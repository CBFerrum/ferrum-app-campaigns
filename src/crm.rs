//! Telling CRM what a lead did with Campaigns' mail: the first human open and the first click of each email sent to a
//! contact that came from a CRM lead, once each, through CRM's operation (the gate, never CRM's tables). Done by the
//! upkeep, so the tracking pixel and the click redirect stay independent of CRM. Where the `campaigns_crm` package is not
//! installed, or CRM does not offer the operation at a version this was built for (`uses` in that package), nothing is
//! asked and the feature is absent; the next round asks again, so an upgrade of CRM turns it on.

use crate::util::{self, values};
use ferrum_kernel::capability::Invocation;
use ferrum_kernel::{Ctx, Error, Kernel, Result};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

/// CRM's operation that records a marketing engagement on a lead (declared in `campaigns_crm`'s `uses`).
pub(crate) const RECORD_ENGAGEMENT: &str = "crm.lead.record_engagement";
/// The package that carries the connection.
const CONNECTOR: &str = "campaigns_crm";
/// Emails told in one upkeep round.
const PER_ROUND: i64 = 200;

pub async fn run(k: &Kernel, sys: &Ctx) -> Result<usize> {
    if !sys.schema.packages.iter().any(|p| p == CONNECTOR) {
        return Ok(0);
    }
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let rows = sqlx::query(
        "select * from (\
           select r.id as recipient, c.record_id as lead, 'Email Opened' as kind, 'crm_open_told_at' as mark, r.opened_at as at, \
                  coalesce(camp.campaign_name, j.journey_name, '') as name \
           from e_campaign_recipient r join e_marketing_contact c on c.tenant_id = r.tenant_id and c.id = r.contact \
           left join e_email_campaign camp on camp.tenant_id = r.tenant_id and camp.id = r.campaign \
           left join e_marketing_journey j on j.tenant_id = r.tenant_id and j.id = r.journey \
           where r.tenant_id = $1 and c.record_entity = 'Lead' and c.record_id is not null and r.opened_at is not null and r.crm_open_told_at is null \
           union all \
           select r.id, c.record_id, 'Email Clicked', 'crm_click_told_at', r.clicked_at, coalesce(camp.campaign_name, j.journey_name, '') \
           from e_campaign_recipient r join e_marketing_contact c on c.tenant_id = r.tenant_id and c.id = r.contact \
           left join e_email_campaign camp on camp.tenant_id = r.tenant_id and camp.id = r.campaign \
           left join e_marketing_journey j on j.tenant_id = r.tenant_id and j.id = r.journey \
           where r.tenant_id = $1 and c.record_entity = 'Lead' and c.record_id is not null and r.clicked_at is not null and r.crm_click_told_at is null \
         ) x order by at limit $2",
    )
    .bind(tenant)
    .bind(PER_ROUND)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    let mut told = 0;
    for row in rows {
        let recipient: Uuid = row.get("recipient");
        let (lead, kind, mark, name): (String, String, String, String) = (row.get("lead"), row.get("kind"), row.get("mark"), row.get("name"));
        let at: chrono::DateTime<chrono::Utc> = row.get("at");
        let Ok(lead) = lead.parse::<Uuid>() else { continue };
        // One transaction for the call and the mark: no state "told but not marked"; a lost mark is answered by the gate's receipt.
        let mut tx = k.db.tenant_tx(tenant).await?;
        let key = format!("engage:{recipient}:{}", if kind == "Email Opened" { "open" } else { "click" });
        let input = json!({"id": lead.to_string(), "params": {"kind": kind, "source": util::cut(&name, 140), "occurred_at": util::ts(at)}});
        let answer = k.call_tx(&crate::keys::handle(k)?, sys, &mut tx, &Invocation::new(RECORD_ENGAGEMENT, input).key(key)).await;
        match answer {
            Ok(_) => {}
            // CRM is not installed here, not at a version this was built for, or the use was not granted: the feature is absent.
            Err(e) if matches!(e.code(), "capability_unavailable" | "forbidden") => {
                tx.rollback().await?;
                tracing::info!(%tenant, "CRM does not take engagement here: {e}");
                return Ok(told);
            }
            // The lead is gone (deleted or merged): the mark is set all the same, so it is not asked for ever.
            Err(Error::NotFound(_)) => {}
            Err(e) => {
                tx.rollback().await?;
                tracing::warn!(%tenant, %recipient, "engagement not told to CRM: {e}");
                continue;
            }
        }
        k.set_values_tx(sys, &mut tx, "CampaignRecipient", recipient, values([(mark.as_str(), json!(util::ts(util::now())))])).await?;
        tx.commit().await?;
        told += 1;
    }
    Ok(told)
}
