//! Campaigns: email marketing on top of the Mail app — contacts with consent evidence, segments
//! compiled to parameterized SQL, campaigns with A/B tests sent in claimed batches, journeys,
//! open and click tracking, bounces and complaints, a suppression list and double opt-in forms.
//!
//! Emails are `Communication` records of the Mail app (`comms`), which delivers them after commit
//! with its accounts, retries and limits. Hooks are registered per entity, so a tenant without
//! the package never runs them; the public endpoints are a `Site` (see packages/campaigns/README.md).

mod campaign;
mod contact;
mod delivery;
mod import;
mod journey;
mod public;
mod render;
mod reports;
mod segment;
mod send;
mod util;

pub use public::machine_open;

use ferrum_domain::reports::ReportSource;
use ferrum_kernel::{Ctx, Error, HookRegistry, Kernel, Result};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Time one maintenance round may spend per tenant advancing journeys and sending campaign
/// batches (work started before the budget runs out is finished).
const JOURNEY_BUDGET: Duration = Duration::from_secs(15);
const SEND_BUDGET: Duration = Duration::from_secs(30);

include!(concat!(env!("OUT_DIR"), "/packages.rs"));

/// This app's manifest (`app.yaml`).
pub const MANIFEST: &str = include_str!("../app.yaml");

/// This app's metadata packages, embedded at build time.
pub fn packages() -> std::result::Result<Vec<ferrum_meta::Package>, ferrum_meta::MetaError> {
    ferrum_meta::catalog::load_embedded(EMBEDDED)
}

/// Registers this app's document hooks and its public endpoints.
pub fn register(hooks: &mut HookRegistry) {
    hooks
        .register("MarketingContact", Arc::new(contact::ContactHook))
        .register("EmailSuppression", Arc::new(contact::SuppressionHook))
        .register("MarketingSegment", Arc::new(segment::SegmentHook))
        .register("AudienceImport", Arc::new(import::ImportHook))
        .register("EmailCampaign", Arc::new(campaign::CampaignHook))
        .register("CampaignRecipient", Arc::new(public::RecipientGuard))
        .register("MarketingJourney", Arc::new(journey::JourneyHook))
        .register("JourneyEnrollment", Arc::new(journey::EnrollmentHook))
        .register("SubscriptionForm", Arc::new(public::FormHook))
        .register("CampaignSettings", Arc::new(public::SettingsHook))
        .add_site(util::PACKAGE, Arc::new(public::CampaignsSite));
}

/// This app's reports.
pub fn reports() -> Vec<Arc<dyn ReportSource>> {
    vec![Arc::new(reports::CampaignReports)]
}

/// Periodic upkeep: every minute, on one replica at a time (advisory lock); must be idempotent and cheap.
/// Per tenant: record delivery outcomes, move journeys, start and send campaigns, purge old
/// bookkeeping. A failing step is logged and does not stop the others; the first error is returned.
pub async fn maintenance(k: &Kernel) -> Result<()> {
    let mut tx = k.db.system_tx().await?;
    let tenants: Vec<Uuid> = sqlx::query_scalar("select id from tenant where status = 'active' order by id").fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let mut first_error: Option<Error> = None;
    for tenant in tenants {
        let schema = match k.registry.schema(tenant).await {
            Ok(schema) if schema.packages.iter().any(|p| p == util::PACKAGE) => schema,
            Ok(_) => continue,
            Err(e) => {
                first_error.get_or_insert(e);
                continue;
            }
        };
        let sys = Ctx::system(tenant, schema);
        let steps = [
            ("delivery", delivery::sync(k, &sys).await),
            ("journeys", journey::run(k, &sys, JOURNEY_BUDGET).await),
            ("campaigns", campaign::run(k, &sys, SEND_BUDGET).await),
            ("purge", purge(k, tenant).await),
        ];
        for (step, outcome) in steps {
            if let Err(e) = outcome {
                tracing::warn!(%tenant, step, "campaigns maintenance failed: {e}");
                first_error.get_or_insert(e);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Old webhook deduplication keys and contact change history.
async fn purge(k: &Kernel, tenant: Uuid) -> Result<usize> {
    let mut tx = k.db.tenant_tx(tenant).await?;
    let a = sqlx::query("delete from campaigns_webhook_event where tenant_id = $1 and received_at < now() - interval '30 days'")
        .bind(tenant)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    let b = sqlx::query("delete from campaigns_contact_change where tenant_id = $1 and changed_at < now() - interval '25 months'")
        .bind(tenant)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    tx.commit().await?;
    Ok((a + b) as usize)
}

#[cfg(test)]
mod tests {
    #[test]
    fn packages_are_valid() {
        let mut all = ferrum_domain::packages().expect("platform packages");
        all.extend(ferrum_app_comms::packages().expect("mail packages"));
        all.extend(super::packages().expect("app packages"));
        let catalog = ferrum_meta::Catalog::from_packages(all).unwrap_or_else(|e| panic!("{e}"));
        let schema = ferrum_meta::effective::build(&catalog, &["campaigns".to_string()], &[], 1).unwrap_or_else(|e| panic!("{e}"));
        assert!(schema.warnings.is_empty(), "{:?}", schema.warnings);
        assert!(schema.entity("CampaignSettings").is_some_and(|e| e.secret_fields().count() == 2));
        assert!(schema.entity("EmailCampaign").is_some_and(|e| e.is_submittable()));
        assert!(super::MANIFEST.contains("packages: [campaigns]"));
    }
}
