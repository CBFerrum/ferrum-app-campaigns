//! Handing marketing emails to the Mail app: the hourly sending budget shared with its
//! newsletters, the personalized message (template, tracked links, open pixel, preheader, signed
//! unsubscribe link and footer, a plain-text part) and the `Communication` record the Mail app
//! delivers after commit — with its retries, account selection and threading.

use crate::render::{self, add_preheader, append_to_body, footer, pixel, rewrite_links};
use crate::util::{self, simple, values};
use ferrum_kernel::{Ctx, Error, Kernel, Result, Tx};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::LazyLock;
use uuid::Uuid;

/// Advisory lock class serializing a tenant's sending budget (with the tenant as the key).
const BUDGET_LOCK: i32 = 0x4341_4D50;

/// Serializes budget checks of a tenant until the transaction ends, so concurrent workers
/// never spend the same hour's room twice.
pub async fn lock_budget(tx: &mut Tx, tenant: Uuid) -> Result<()> {
    sqlx::query("select pg_advisory_xact_lock($1, hashtext($2::text))").bind(BUDGET_LOCK).bind(tenant).execute(&mut **tx).await?;
    Ok(())
}

/// Emails the tenant may still hand over this hour: the Mail app's hourly limit (Communication
/// Settings) minus what campaigns, journeys and its newsletters sent in the last hour.
pub async fn hourly_room(tx: &mut Tx, tenant: Uuid) -> Result<i64> {
    let limit: Option<Option<i64>> =
        sqlx::query_scalar("select newsletter_hourly_limit from e_comms_settings where tenant_id = $1 limit 1").bind(tenant).fetch_optional(&mut **tx).await?;
    let limit = limit.flatten().unwrap_or(500).max(1);
    let used: i64 = sqlx::query_scalar(
        "select (select count(*) from e_campaign_recipient where tenant_id = $1 and communication is not null and queued_at > now() - interval '1 hour') \
         + (select count(*) from comms_newsletter_delivery where tenant_id = $1 and created_at > now() - interval '1 hour')",
    )
    .bind(tenant)
    .fetch_one(&mut **tx)
    .await?;
    Ok((limit - used).max(0))
}

/// An email before personalization.
#[derive(Debug, Clone)]
pub struct Draft {
    pub subject: String,
    pub preheader: Option<String>,
    pub html: String,
    pub text: Option<String>,
}

/// How an email's links are made.
#[derive(Debug, Clone)]
pub struct Links {
    /// Absolute root of the public endpoints.
    pub base: String,
    pub key: Vec<u8>,
    pub track_opens: bool,
    /// URL → link number of the tracked links (`None`: clicks are not tracked).
    pub tracked: Option<HashMap<String, i32>>,
    pub postal_address: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Composed {
    pub subject: String,
    pub html: String,
    pub text: String,
}

pub fn open_url(l: &Links, recipient: Uuid) -> String {
    format!("{}/o/{}.{}.gif", l.base, simple(recipient), util::sign(&l.key, &format!("open:{recipient}")))
}

pub fn click_url(l: &Links, recipient: Uuid, link: i32) -> String {
    format!("{}/c/{}.{link}.{}", l.base, simple(recipient), util::sign(&l.key, &format!("click:{recipient}:{link}")))
}

pub fn unsubscribe_url(l: &Links, recipient: Uuid) -> String {
    format!("{}/u/{}.{}", l.base, simple(recipient), util::sign(&l.key, &format!("unsubscribe:{recipient}")))
}

static ANCHOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<a\b[^>]*?\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')[^>]*>(.*?)</a\s*>"#).expect("anchor pattern"));

/// The plain-text version of an HTML email: link targets stay readable (and tracked).
pub fn html_text(html: &str) -> String {
    let with_urls = ANCHOR.replace_all(html, |c: &regex::Captures<'_>| {
        let url = render::html_unescape(c.get(1).or(c.get(2)).map(|m| m.as_str()).unwrap_or_default().trim());
        let label = &c[3];
        if url.is_empty() || url.starts_with('#') { label.to_string() } else { format!("{label} ({})", util::html_escape(&url)) }
    });
    mail_parser::decoders::html::html_to_text(&with_urls).trim().to_string()
}

/// Personalizes `d` for one recipient: the template context has `contact` (also as `doc`, the
/// Mail app's name for it), `unsubscribe_url` and `extra`. Content without its own unsubscribe
/// link gets the footer; static links become tracked click links; the pixel goes last.
pub fn compose(d: &Draft, contact: &Value, extra: &Map<String, Value>, recipient: Uuid, l: &Links) -> Result<Composed> {
    let unsubscribe = unsubscribe_url(l, recipient);
    let mut ctx = Map::new();
    ctx.insert("contact".into(), contact.clone());
    ctx.insert("doc".into(), contact.clone());
    ctx.insert("unsubscribe_url".into(), json!(unsubscribe));
    for (k, v) in extra {
        ctx.insert(k.clone(), v.clone());
    }
    let ctx = Value::Object(ctx);
    let subject = render::render_line(&d.subject, &ctx)?;
    let mut html = render::render_html(&d.html, &ctx)?;
    if !d.html.contains("unsubscribe_url") {
        html = append_to_body(&html, &footer(&unsubscribe, l.postal_address.as_deref()));
    }
    if let Some(map) = &l.tracked {
        html = rewrite_links(&html, |u| map.get(u).map(|n| click_url(l, recipient, *n)));
    }
    let mut text = match &d.text {
        Some(t) => render::render_text(t, &ctx)?,
        None => html_text(&html),
    };
    if !text.contains(&unsubscribe) {
        text.push_str(&format!("\n\n--\nUnsubscribe: {unsubscribe}\n"));
    }
    if let Some(p) = &d.preheader {
        let p = render::render_line(p, &ctx)?;
        if !p.is_empty() {
            html = add_preheader(&html, &p);
        }
    }
    if l.track_opens {
        html = append_to_body(&html, &pixel(&open_url(l, recipient)));
    }
    if subject.is_empty() {
        return Err(Error::invalid("the subject renders empty"));
    }
    Ok(Composed { subject, html, text })
}

/// Queues the email as an outgoing `Communication` on the contact's timeline, in the caller's
/// transaction: the Mail app sends it after commit (never if the transaction rolls back).
pub async fn queue(k: &Kernel, ctx: &Ctx, tx: &mut Tx, account: Option<Uuid>, to: &str, c: &Composed, reference: (&str, Uuid)) -> Result<Uuid> {
    let id = Uuid::now_v7();
    let doc = values([
        ("id", json!(id.to_string())),
        ("channel", json!("Email")),
        ("direction", json!("Sent")),
        ("email_account", json!(account.map(|a| a.to_string()))),
        ("recipients", json!(to)),
        ("subject", json!(util::cut(&c.subject, 998))),
        ("content", json!(c.html)),
        ("text_content", json!(c.text)),
        ("reference_entity", json!(reference.0)),
        ("reference_id", json!(reference.1.to_string())),
    ]);
    k.insert_tx(ctx, tx, "Communication", doc).await?;
    Ok(id)
}

/// Numbers the static links of `sources` for `owner` (a campaign or journey), keeping the
/// numbers links already have.
pub async fn register_links(tx: &mut Tx, tenant: Uuid, owner: Uuid, sources: &[&str]) -> Result<()> {
    let mut urls: Vec<String> = Vec::new();
    for src in sources {
        for u in render::static_links(src) {
            if !urls.contains(&u) && urls.len() < render::MAX_LINKS {
                urls.push(u);
            }
        }
    }
    for url in urls {
        sqlx::query(
            "insert into campaigns_link (tenant_id, owner_id, link_no, url) \
             select $1, $2, coalesce(max(link_no), 0) + 1, $3 from campaigns_link where tenant_id = $1 and owner_id = $2 on conflict do nothing",
        )
        .bind(tenant)
        .bind(owner)
        .bind(&url)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// The tracked links of a campaign or journey: URL → number.
pub async fn links_of(tx: &mut Tx, tenant: Uuid, owner: Uuid) -> Result<HashMap<String, i32>> {
    let rows: Vec<(String, i32)> =
        sqlx::query_as("select url, link_no from campaigns_link where tenant_id = $1 and owner_id = $2").bind(tenant).bind(owner).fetch_all(&mut **tx).await?;
    Ok(rows.into_iter().collect())
}

/// Everything links need, for a campaign or journey sending now. Fails until the Mail app's
/// Public URL is set (links must reach this server from the internet).
pub async fn links_for(k: &Kernel, tenant: Uuid, owner: Uuid, track_opens: bool, track_clicks: bool) -> Result<Links> {
    let key = util::signing_key(k, tenant).await?;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let base = util::site_base(&mut tx, tenant).await?.ok_or_else(public_url_missing)?;
    let postal_address = util::settings(&mut tx, tenant).await?.postal_address;
    let tracked = if track_clicks { Some(links_of(&mut tx, tenant, owner).await?) } else { None };
    tx.commit().await?;
    Ok(Links { base, key, track_opens, tracked, postal_address })
}

pub fn public_url_missing() -> Error {
    Error::invalid("set the Public URL in Communication Settings (Mail app) first: tracking and unsubscribe links must reach this server")
}

/// Whether the workspace can send: the account (or the default outgoing one, or the server's
/// own mailer) is able to.
pub async fn check_account(k: &Kernel, tx: &mut Tx, tenant: Uuid, account: Option<Uuid>) -> Result<()> {
    let found: Option<(String, bool, Option<String>)> = match account {
        Some(a) => sqlx::query_as("select account_name, enable_outgoing, smtp_host from e_email_account where tenant_id = $1 and id = $2")
            .bind(tenant)
            .bind(a)
            .fetch_optional(&mut **tx)
            .await?
            .or(Some((String::new(), false, None))),
        None => {
            sqlx::query_as(
                "select account_name, enable_outgoing, smtp_host from e_email_account where tenant_id = $1 and enable_outgoing and default_outgoing \
                 and company is null order by created_at limit 1",
            )
            .bind(tenant)
            .fetch_optional(&mut **tx)
            .await?
        }
    };
    match found {
        Some((name, true, Some(_))) if !name.is_empty() => Ok(()),
        Some((name, _, _)) if !name.is_empty() => Err(Error::field("email_account", format!("{name} does not send mail (no SMTP server or sending is off)"))),
        Some(_) => Err(Error::field("email_account", "does not exist")),
        None if k.mailer.is_some() => Ok(()),
        None => Err(Error::field("email_account", "set up an outgoing email account in the Mail app first (or choose one)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(track: bool) -> Links {
        Links {
            base: "https://erp.test/api/site/acme/campaigns".into(),
            key: b"0123456789abcdef0123456789abcdef".to_vec(),
            track_opens: track,
            tracked: track.then(|| HashMap::from([("https://shop.test/sale".to_string(), 1)])),
            postal_address: Some("Acme Ltd, 1 Main St".into()),
        }
    }

    #[test]
    fn composes_a_personal_tracked_email() {
        let d = Draft {
            subject: "Hi {{ contact.first_name }}".into(),
            preheader: Some("Only for {{ contact.attributes.city }}".into()),
            html: "<html><body><p>Dear {{ doc.first_name }}</p><a href=\"https://shop.test/sale\">Shop</a><a href=\"{{ contact.attributes.url }}\">Mine</a></body></html>".into(),
            text: None,
        };
        let r = Uuid::now_v7();
        let contact = json!({"first_name": "Asha", "attributes": {"city": "Pune", "url": "https://shop.test/sale"}});
        let c = compose(&d, &contact, &Map::new(), r, &links(true)).unwrap();
        assert_eq!(c.subject, "Hi Asha");
        let l = links(true);
        assert!(c.html.contains(&util::html_escape(&click_url(&l, r, 1))), "{}", c.html);
        assert_eq!(c.html.matches("/c/").count(), 2, "the dynamic link renders to a tracked URL too: tracking matches rendered URLs");
        assert!(c.html.contains(&util::html_escape(&unsubscribe_url(&l, r))) && c.html.contains("Acme Ltd, 1 Main St"), "footer");
        assert!(c.html.contains("Only for Pune") && c.html.contains(&open_url(&l, r)));
        assert!(c.html.find("Only for Pune").unwrap() < c.html.find("Dear Asha").unwrap(), "preheader first");
        assert!(c.text.contains("Dear Asha") && c.text.contains(&unsubscribe_url(&l, r)), "{}", c.text);
        assert!(c.text.contains("Shop (https://erp.test/api/site/acme/campaigns/c/"), "{}", c.text);
        // Without tracking: original links, no pixel, still the unsubscribe link.
        let plain = compose(&d, &contact, &Map::new(), r, &links(false)).unwrap();
        assert!(plain.html.contains("href=\"https://shop.test/sale\"") && !plain.html.contains("/o/") && plain.html.contains("/u/"));
        // Content with its own unsubscribe link gets no footer.
        let own = Draft { html: "<p><a href=\"{{ unsubscribe_url }}\">Leave</a></p>".into(), preheader: None, ..d.clone() };
        let c = compose(&own, &contact, &Map::new(), r, &links(false)).unwrap();
        assert!(!c.html.contains("Acme Ltd") && c.html.contains(&util::html_escape(&unsubscribe_url(&l, r))));
        assert!(compose(&Draft { subject: "{{ contact.nope }}".into(), ..own }, &contact, &Map::new(), r, &links(false)).is_err(), "empty subject");
    }
}
