//! Delivery outcomes: the Mail app's verdict on each queued email, and bounces and complaints
//! reported by the mail provider (bounce webhook) or by the receiving server at hand-over.
//!
//! Hard bounces and complaints suppress the address at once; soft bounces count up on the
//! contact (an open or click resets the count) and suppress it at the workspace's limit.
//! Consent changes go through the kernel, so the contact's hook records them and applies the
//! suppression list, the email groups and the linked record.

use crate::contact;
use crate::util::{self, values};
use ferrum_kernel::{Ctx, Kernel, Result, Tx};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Hard,
    Soft,
    Complaint,
}

impl Kind {
    pub fn parse(kind: &str, bounce_type: Option<&str>) -> Option<Kind> {
        match kind.trim().to_ascii_lowercase().as_str() {
            "hard" | "hard_bounce" | "permanent" => Some(Kind::Hard),
            "soft" | "soft_bounce" | "transient" => Some(Kind::Soft),
            "complaint" | "spam" | "abuse" | "spamreport" => Some(Kind::Complaint),
            "bounce" | "bounced" => match bounce_type.map(|t| t.trim().to_ascii_lowercase()) {
                Some(t) if t == "soft" || t == "transient" => Some(Kind::Soft),
                _ => Some(Kind::Hard),
            },
            _ => None,
        }
    }
}

/// Whether a rejection at hand-over says the mailbox does not exist (RFC 3463 5.1.x): the
/// receiving server's own hard bounce.
pub fn mailbox_unknown(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    ["5.1.1", "5.1.10", "5.1.0"].iter().any(|code| e.contains(code)) || (e.contains("550") && (e.contains("no such user") || e.contains("user unknown")))
}

/// Queued recipients whose email the Mail app has sent or given up on; a hand-over rejected for
/// an unknown mailbox is a hard bounce. Emails deleted before sending fail after a day.
pub async fn sync(k: &Kernel, sys: &Ctx) -> Result<usize> {
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let done: Vec<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        "update e_campaign_recipient r set status = case when c.status = 'Sent' then 'Sent' else 'Failed' end, sent_at = c.sent_at, \
         error = case when c.status = 'Failed' then c.error end, version = r.version + 1, updated_at = now() \
         from e_communication c where r.tenant_id = $1 and r.status = 'Queued' and c.tenant_id = r.tenant_id and c.id = r.communication \
         and c.status in ('Sent', 'Failed') returning r.id, r.email, r.status, r.error",
    )
    .bind(tenant)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(
        "update e_campaign_recipient set status = 'Failed', error = 'the email was deleted before it was sent', version = version + 1, updated_at = now() \
         where tenant_id = $1 and status = 'Queued' and communication is null and queued_at < now() - interval '1 day'",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await?;
    for (recipient, email, status, error) in &done {
        if status == "Failed" && error.as_deref().is_some_and(mailbox_unknown) {
            apply(k, sys, &mut tx, email, Some(*recipient), Kind::Hard, "rejected by the receiving server").await?;
        }
    }
    tx.commit().await?;
    Ok(done.len())
}

/// Applies one bounce or complaint: to the recipient (when known), its contact and its address.
pub async fn apply(k: &Kernel, sys: &Ctx, tx: &mut Tx, email: &str, recipient: Option<Uuid>, kind: Kind, source: &str) -> Result<()> {
    let tenant = sys.tenant;
    let email = email.trim().to_lowercase();
    if let Some(r) = recipient {
        sqlx::query(
            "update e_campaign_recipient set bounced_at = case when $3 = 'Complaint' then bounced_at else coalesce(bounced_at, now()) end, \
             bounce_type = case when $3 = 'Hard' then 'Hard' when $3 = 'Soft' then coalesce(bounce_type, 'Soft') else bounce_type end, \
             complained_at = case when $3 = 'Complaint' then coalesce(complained_at, now()) else complained_at end, version = version + 1, updated_at = now() \
             where tenant_id = $1 and id = $2",
        )
        .bind(tenant)
        .bind(r)
        .bind(format!("{kind:?}"))
        .execute(&mut **tx)
        .await?;
    }
    let found: Option<(Uuid, String, i64)> =
        sqlx::query_as("select id, consent_status, coalesce(soft_bounce_count, 0) from e_marketing_contact where tenant_id = $1 and email = $2 for update")
            .bind(tenant)
            .bind(&email)
            .fetch_optional(&mut **tx)
            .await?;
    let source = util::cut(source, 150);
    let Some((contact, consent, soft)) = found else {
        // Not a contact (any more): the address is still never mailed again.
        match kind {
            Kind::Hard => contact::suppress(tx, tenant, &email, "Hard Bounce", &source).await?,
            Kind::Complaint => contact::suppress(tx, tenant, &email, "Complaint", &source).await?,
            Kind::Soft => {}
        }
        return Ok(());
    };
    let patch = match kind {
        Kind::Complaint if consent != "Complained" => {
            Some(values([("consent_status", json!("Complained")), ("consent_source", json!(format!("complaint: {source}")))]))
        }
        Kind::Hard if matches!(consent.as_str(), "Subscribed" | "Pending") => {
            Some(values([("consent_status", json!("Bounced")), ("consent_source", json!(format!("hard bounce: {source}")))]))
        }
        Kind::Soft => {
            let limit = util::settings(tx, tenant).await?.soft_bounce_limit;
            let count = soft + 1;
            if count >= limit && matches!(consent.as_str(), "Subscribed" | "Pending") {
                Some(values([
                    ("soft_bounce_count", json!(count)),
                    ("consent_status", json!("Bounced")),
                    ("consent_source", json!(format!("soft bounces: {count} in a row ({source})"))),
                ]))
            } else {
                k.set_values_tx(sys, tx, "MarketingContact", contact, values([("soft_bounce_count", json!(count))])).await?;
                None
            }
        }
        _ => None,
    };
    match patch {
        Some(p) => {
            k.update_tx(sys, tx, "MarketingContact", contact, p, None).await?;
        }
        // Already opted out or bounced: the address is suppressed all the same.
        None if kind == Kind::Hard => contact::suppress(tx, tenant, &email, "Hard Bounce", &source).await?,
        None => {}
    }
    Ok(())
}

/// A human open or click proves the mailbox works: soft bounces start counting again.
pub async fn engaged(tx: &mut Tx, tenant: Uuid, contact: Uuid) -> Result<()> {
    sqlx::query("update e_marketing_contact set soft_bounce_count = 0 where tenant_id = $1 and id = $2 and soft_bounce_count > 0")
        .bind(tenant)
        .bind(contact)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_provider_events() {
        assert_eq!(Kind::parse("hard", None), Some(Kind::Hard));
        assert_eq!(Kind::parse("Bounce", Some("Transient")), Some(Kind::Soft));
        assert_eq!(Kind::parse("bounce", Some("Permanent")), Some(Kind::Hard));
        assert_eq!(Kind::parse("spamreport", None), Some(Kind::Complaint));
        assert_eq!(Kind::parse("delivered", None), None);
        assert!(mailbox_unknown("SMTP: permanent error (550): 5.1.1 No such user"));
        assert!(mailbox_unknown("550 User unknown"));
        assert!(!mailbox_unknown("SMTP: permanent error (554): 5.7.1 Message rejected as spam"));
        assert!(!mailbox_unknown("the SMTP server did not answer in time"));
    }
}
