//! Revenue from email: where ERP is installed, a submitted sales order (or sales invoice, by the workspace's setting)
//! of a customer is credited to the last email touch of the same person within the attribution window before it was
//! made: a click (or, by the setting, an open; automatic opens never count). The person is a contact that came from
//! that customer, or whose address is the customer's email.
//!
//! **No double counting**: each order is credited once, to one email (one row per order, its primary key), so two
//! campaigns that touched the same person split nothing and count nothing twice: the later touch takes it. A cancelled
//! order loses its credit. Amounts are the order's company-currency total, summed exactly by the database.

use crate::util::sys;
use ferrum_kernel::{Ctx, Kernel, Result};
use uuid::Uuid;

/// Credits the recent orders not credited yet, takes back cancelled ones, and refreshes the totals on campaigns and
/// journeys. Orders made in the last window (plus a day) are looked at, so a run costs what that period holds.
pub async fn run(k: &Kernel, ctx: &Ctx) -> Result<u64> {
    let sys = sys(ctx);
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let (document, days, opens): (Option<String>, Option<i64>, Option<String>) =
        sqlx::query_as("select attribution_document, attribution_days, attribution_touch from e_campaign_settings where tenant_id = $1 limit 1")
            .bind(tenant)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or_default();
    let entity = if document.as_deref() == Some("Sales Invoice") { "SalesInvoice" } else { "SalesOrder" };
    // Without ERP there is nothing to credit.
    let Some(ent) = sys.schema.entity(entity).filter(|e| !e.table.is_empty() && e.field("customer").is_some() && e.field("base_grand_total").is_some()) else {
        tx.rollback().await?;
        return Ok(0);
    };
    let days = days.unwrap_or(5).clamp(1, 90) as i32;
    let opens = opens.as_deref() == Some("Clicks and Opens");
    let table = &ent.table;
    let customers = sys.schema.entity("Customer").map(|e| e.table.clone()).unwrap_or_else(|| "e_customer".into());
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "delete from campaigns_attribution a where a.tenant_id = $1 and a.order_entity = $2 \
         and not exists (select 1 from {table} o where o.tenant_id = a.tenant_id and o.id = a.order_id and o.docstatus = 1)"
    )))
    .bind(tenant)
    .bind(entity)
    .execute(&mut *tx)
    .await?;
    // Touches: every click, and (by the setting) the first and the last open, of one person's emails.
    let touches = format!(
        "select r.id as recipient_id, coalesce(r.campaign, r.journey) as owner_id, k.link_no, r.contact as contact_id, k.clicked_at as at \
         from campaigns_click k join e_campaign_recipient r on r.tenant_id = k.tenant_id and r.id = k.recipient_id where k.tenant_id = $1{}",
        if opens {
            " union all select r.id, coalesce(r.campaign, r.journey), null, r.contact, t.at from e_campaign_recipient r \
             cross join lateral (values (r.opened_at), (r.last_opened_at)) t(at) where r.tenant_id = $1 and t.at is not null"
        } else {
            ""
        }
    );
    let credited = sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into campaigns_attribution (tenant_id, order_entity, order_id, recipient_id, owner_id, link_no, contact_id, amount, touched_at, ordered_at) \
         select distinct on (o.id) $1, $2, o.id, t.recipient_id, t.owner_id, t.link_no, t.contact_id, coalesce(o.base_grand_total, 0), t.at, o.created_at \
         from {table} o join ({touches}) t on t.at <= o.created_at and t.at > o.created_at - make_interval(days => $3) \
         join e_marketing_contact c on c.tenant_id = o.tenant_id and c.id = t.contact_id \
         where o.tenant_id = $1 and o.docstatus = 1 and o.created_at > now() - make_interval(days => $3 + 1) \
         and ((c.record_entity = 'Customer' and c.record_id = o.customer::text) \
              or exists (select 1 from {customers} u where u.tenant_id = o.tenant_id and u.id = o.customer and lower(u.email) = c.email)) \
         order by o.id, t.at desc, t.recipient_id on conflict do nothing"
    )))
    .bind(tenant)
    .bind(entity)
    .bind(days)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    for target in ["e_email_campaign", "e_marketing_journey"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "update {target} x set attributed_orders = s.n, attributed_revenue = s.total, updated_at = now() \
             from (select o.id, count(a.order_id) as n, coalesce(sum(a.amount), 0) as total from {target} o \
                   left join campaigns_attribution a on a.tenant_id = o.tenant_id and a.owner_id = o.id where o.tenant_id = $1 group by o.id) s \
             where x.tenant_id = $1 and x.id = s.id and (x.attributed_orders, x.attributed_revenue) is distinct from (s.n, s.total)"
        )))
        .bind(tenant)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(credited)
}

/// `url` with the campaign's tracking parameters added: those the link already has are kept.
pub fn tag(url: &str, params: &[(&str, &str)]) -> String {
    let (base, fragment) = match url.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (url, None),
    };
    let have: Vec<String> = base.split_once('?').map(|(_, q)| form_urlencoded::parse(q.as_bytes()).map(|(k, _)| k.into_owned()).collect()).unwrap_or_default();
    let mut add = form_urlencoded::Serializer::new(String::new());
    for (k, v) in params.iter().filter(|(k, v)| !v.is_empty() && !have.iter().any(|h| h == k)) {
        add.append_pair(k, v);
    }
    let add = add.finish();
    let mut out = base.to_string();
    if !add.is_empty() {
        out.push(if base.contains('?') { '&' } else { '?' });
        out.push_str(&add);
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// The tracking parameters of a campaign that adds them: `[(name, value)]`.
pub async fn utm_of(tx: &mut ferrum_kernel::Tx, tenant: Uuid, owner: Uuid) -> Result<Vec<(&'static str, String)>> {
    let row: Option<(String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "select campaign_name, utm_campaign, utm_source, utm_medium from e_email_campaign where tenant_id = $1 and id = $2 and coalesce(utm_tags, false)",
    )
    .bind(tenant)
    .bind(owner)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(match row {
        Some((name, campaign, source, medium)) => vec![
            ("utm_campaign", campaign.filter(|c| !c.trim().is_empty()).unwrap_or(name)),
            ("utm_source", source.unwrap_or_default()),
            ("utm_medium", medium.unwrap_or_default()),
        ],
        None => vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::tag;

    #[test]
    fn tracking_parameters_keep_the_links_own() {
        let p = [("utm_campaign", "Spring sale"), ("utm_source", "newsletter"), ("utm_medium", "email")];
        assert_eq!(tag("https://shop.test/a", &p), "https://shop.test/a?utm_campaign=Spring+sale&utm_source=newsletter&utm_medium=email");
        assert_eq!(tag("https://shop.test/a?x=1&utm_source=fb#top", &p), "https://shop.test/a?x=1&utm_source=fb&utm_campaign=Spring+sale&utm_medium=email#top");
        assert_eq!(tag("https://shop.test/a", &[("utm_source", "")]), "https://shop.test/a");
    }
}
