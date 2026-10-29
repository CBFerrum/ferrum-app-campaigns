//! Reports: campaign performance, A/B test results, link clicks, the journey funnel and audience
//! growth. Each runs in a read-only transaction and only over records the caller may read.

use async_trait::async_trait;
use chrono::{Duration, NaiveDate, NaiveTime};
use ferrum_domain::reports::{Params, ReportSource, col, read_tx};
use ferrum_kernel::{Action, Ctx, Error, Kernel, Result, row_scope};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::HashMap;
use uuid::Uuid;

pub struct CampaignReports;

fn date(name: &str, label: &str) -> Value {
    json!({"name": name, "label": label, "type": "date"})
}

fn link(name: &str, label: &str, to: &str) -> Value {
    json!({"name": name, "label": label, "type": "link", "to": to})
}

/// `part` as a percentage of `whole`, two decimals, in integer arithmetic.
pub fn pct(part: i64, whole: i64) -> Value {
    if whole <= 0 {
        return json!("0.00");
    }
    let bp = (part * 10_000 + whole / 2) / whole;
    json!(format!("{}.{:02}", bp / 100, bp % 100))
}

/// The period asked for; the last `days` days (to today, inclusive) by default.
fn period(p: &Params<'_>, days: i64) -> Result<(NaiveDate, NaiveDate)> {
    let to = p.date("to_date")?.unwrap_or_else(ferrum_domain::reports::today);
    let from = p.date("from_date")?.unwrap_or(to - Duration::days(days - 1));
    if from > to {
        return Err(Error::field("from_date", "is after the end date"));
    }
    Ok((from, to))
}

fn required(p: &Params<'_>, k: &str) -> Result<Uuid> {
    p.uuid(k)?.ok_or_else(|| Error::field(k, "is required"))
}

#[derive(sqlx::FromRow)]
struct Perf {
    id: Uuid,
    number: Option<String>,
    campaign_name: String,
    status: Option<String>,
    started_on: Option<NaiveDate>,
    recipients: i64,
    sent: i64,
    failed: i64,
    skipped: i64,
    unique_opens: i64,
    total_opens: i64,
    machine_opens: i64,
    unique_clicks: i64,
    total_clicks: i64,
    unsubscribes: i64,
    hard_bounces: i64,
    soft_bounces: i64,
    complaints: i64,
}

/// The performance numbers of one campaign (or of everything), as a report row.
fn perf_row(r: &Perf) -> Value {
    let bounces = r.hard_bounces + r.soft_bounces;
    let delivered = (r.sent - bounces).max(0);
    json!({
        "campaign": r.campaign_name, "campaign_id": r.id, "number": r.number, "status": r.status, "started_on": r.started_on.map(|d| d.to_string()),
        "recipients": r.recipients, "sent": r.sent, "delivered": delivered, "failed": r.failed, "skipped": r.skipped,
        "unique_opens": r.unique_opens, "total_opens": r.total_opens, "open_rate": pct(r.unique_opens, delivered), "machine_opens": r.machine_opens,
        "unique_clicks": r.unique_clicks, "total_clicks": r.total_clicks, "click_rate": pct(r.unique_clicks, delivered),
        "click_to_open": pct(r.unique_clicks, r.unique_opens), "unsubscribes": r.unsubscribes, "unsubscribe_rate": pct(r.unsubscribes, delivered),
        "bounces": bounces, "hard_bounces": r.hard_bounces, "soft_bounces": r.soft_bounces, "bounce_rate": pct(bounces, r.sent),
        "complaints": r.complaints, "complaint_rate": pct(r.complaints, delivered),
    })
}

const COUNTS: &str = "count(r.id) as recipients, count(r.id) filter (where r.status = 'Sent') as sent, count(r.id) filter (where r.status = 'Failed') as failed, \
     count(r.id) filter (where r.status = 'Skipped') as skipped, count(r.id) filter (where r.opened_at is not null) as unique_opens, \
     coalesce(sum(r.open_count), 0)::bigint as total_opens, coalesce(sum(r.machine_open_count), 0)::bigint as machine_opens, \
     count(r.id) filter (where r.clicked_at is not null) as unique_clicks, coalesce(sum(r.click_count), 0)::bigint as total_clicks, \
     count(r.id) filter (where r.unsubscribed_at is not null) as unsubscribes, count(r.id) filter (where r.bounce_type = 'Hard') as hard_bounces, \
     count(r.id) filter (where r.bounce_type = 'Soft') as soft_bounces, count(r.id) filter (where r.complained_at is not null) as complaints";

#[async_trait]
impl ReportSource for CampaignReports {
    fn catalog(&self) -> Vec<Value> {
        vec![
            json!({"name": "campaign_performance", "label": "Campaign Performance", "module": "marketing", "entity": "EmailCampaign",
                   "params": [date("from_date", "Started From"), date("to_date", "Started To"), link("campaign", "Campaign", "EmailCampaign")]}),
            json!({"name": "ab_test_results", "label": "A/B Test Results", "module": "marketing", "entity": "EmailCampaign",
                   "params": [link("campaign", "Campaign", "EmailCampaign")]}),
            json!({"name": "link_clicks", "label": "Link Clicks", "module": "marketing", "entity": "EmailCampaign",
                   "params": [link("campaign", "Campaign", "EmailCampaign"), link("journey", "Journey", "MarketingJourney")]}),
            json!({"name": "journey_funnel", "label": "Journey Funnel", "module": "marketing", "entity": "MarketingJourney",
                   "params": [link("journey", "Journey", "MarketingJourney")]}),
            json!({"name": "audience_growth", "label": "Audience Growth", "module": "marketing", "entity": "MarketingContact",
                   "params": [date("from_date", "From Date"), date("to_date", "To Date")]}),
        ]
    }

    async fn run(&self, k: &Kernel, ctx: &Ctx, name: &str, params: &HashMap<String, String>) -> Result<Value> {
        let p = Params { raw: params };
        match name {
            "campaign_performance" => performance(k, ctx, &p).await,
            "ab_test_results" => ab_results(k, ctx, &p).await,
            "link_clicks" => link_clicks(k, ctx, &p).await,
            "journey_funnel" => funnel(k, ctx, &p).await,
            "audience_growth" => growth(k, ctx, &p).await,
            _ => Err(Error::NotFound(format!("report {name}"))),
        }
    }
}

async fn performance(k: &Kernel, ctx: &Ctx, p: &Params<'_>) -> Result<Value> {
    let (from, to) = period(p, 90)?;
    let tz = ctx.schema.timezone.clone();
    let mut tx = read_tx(k, ctx).await?;
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(format!(
        "select c.id, c.number, c.campaign_name, c.status, (c.started_at at time zone '{}')::date as started_on, {COUNTS} \
         from e_email_campaign c left join e_campaign_recipient r on r.tenant_id = c.tenant_id and r.campaign = c.id where c.docstatus > 0 \
         and c.started_at is not null and c.tenant_id = ",
        tz.replace('\'', "")
    ));
    qb.push_bind(ctx.tenant);
    match p.uuid("campaign")? {
        Some(c) => {
            qb.push(" and c.id = ").push_bind(c);
        }
        None => {
            qb.push(" and (c.started_at at time zone ").push_bind(tz.clone()).push(")::date between ").push_bind(from).push(" and ").push_bind(to);
        }
    }
    let ent = k.entity(ctx, "EmailCampaign")?;
    row_scope(ctx, &ent, Action::Read)?.push_sql(&mut qb, "c");
    qb.push(" group by c.tenant_id, c.id order by c.started_at desc");
    let data: Vec<Perf> = qb.build_query_as().fetch_all(&mut *tx).await?;
    tx.rollback().await?;
    let total = data.iter().fold(
        Perf {
            id: Uuid::nil(),
            number: None,
            campaign_name: "Total".into(),
            status: None,
            started_on: None,
            recipients: 0,
            sent: 0,
            failed: 0,
            skipped: 0,
            unique_opens: 0,
            total_opens: 0,
            machine_opens: 0,
            unique_clicks: 0,
            total_clicks: 0,
            unsubscribes: 0,
            hard_bounces: 0,
            soft_bounces: 0,
            complaints: 0,
        },
        |mut t, r| {
            t.recipients += r.recipients;
            t.sent += r.sent;
            t.failed += r.failed;
            t.skipped += r.skipped;
            t.unique_opens += r.unique_opens;
            t.total_opens += r.total_opens;
            t.machine_opens += r.machine_opens;
            t.unique_clicks += r.unique_clicks;
            t.total_clicks += r.total_clicks;
            t.unsubscribes += r.unsubscribes;
            t.hard_bounces += r.hard_bounces;
            t.soft_bounces += r.soft_bounces;
            t.complaints += r.complaints;
            t
        },
    );
    Ok(json!({
        "columns": [col("campaign", "Campaign", "data"), col("started_on", "Started", "date"), col("sent", "Sent", "int"), col("delivered", "Delivered", "int"),
                    col("unique_opens", "Unique Opens", "int"), col("total_opens", "Total Opens", "int"), col("open_rate", "Open Rate %", "percent"),
                    col("unique_clicks", "Unique Clicks", "int"), col("total_clicks", "Total Clicks", "int"), col("click_rate", "CTR %", "percent"),
                    col("click_to_open", "Click-to-Open %", "percent"), col("unsubscribes", "Unsubscribes", "int"), col("bounces", "Bounces", "int"),
                    col("complaints", "Complaints", "int"), col("machine_opens", "Automatic Opens", "int")],
        "rows": data.iter().map(perf_row).collect::<Vec<_>>(),
        "totals": perf_row(&total),
        "meta": {"from_date": from.to_string(), "to_date": to.to_string()},
    }))
}

async fn ab_results(k: &Kernel, ctx: &Ctx, p: &Params<'_>) -> Result<Value> {
    let campaign = required(p, "campaign")?;
    let doc = k.get(ctx, "EmailCampaign", campaign, false).await?;
    let mut tx = read_tx(k, ctx).await?;
    let data: Vec<(String, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "select r.variant, count(*) filter (where r.status <> 'Held'), count(*) filter (where r.status = 'Sent' and r.bounced_at is null), \
         count(*) filter (where r.opened_at is not null), count(*) filter (where r.clicked_at is not null), coalesce(sum(r.click_count), 0)::bigint \
         from e_campaign_recipient r where r.tenant_id = $1 and r.campaign = $2 and r.variant is not null group by r.variant order by r.variant",
    )
    .bind(ctx.tenant)
    .bind(campaign)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    let winner = doc.get("winner_variant").and_then(Value::as_str).map(str::to_string);
    let subjects: HashMap<String, String> = ferrum_domain::util::rows(&doc, "variants")
        .iter()
        .filter_map(|r| Some((r.get("variant")?.as_str()?.to_string(), r.get("subject")?.as_str()?.to_string())))
        .collect();
    let rows: Vec<Value> = data
        .iter()
        .map(|(v, recipients, delivered, opens, clicks, total_clicks)| {
            json!({"variant": v, "subject": subjects.get(v), "recipients": recipients, "delivered": delivered, "unique_opens": opens,
                   "open_rate": pct(*opens, *delivered), "unique_clicks": clicks, "click_rate": pct(*clicks, *delivered), "total_clicks": total_clicks,
                   "winner": winner.as_deref() == Some(v.as_str())})
        })
        .collect();
    Ok(json!({
        "columns": [col("variant", "Variant", "data"), col("subject", "Subject", "data"), col("recipients", "Recipients", "int"), col("delivered", "Delivered", "int"),
                    col("unique_opens", "Unique Opens", "int"), col("open_rate", "Open Rate %", "percent"), col("unique_clicks", "Unique Clicks", "int"),
                    col("click_rate", "Click Rate %", "percent"), col("winner", "Winner", "check")],
        "rows": rows,
        "meta": {"campaign": doc.get("campaign_name"), "ab_test": doc.get("ab_test"), "winner_metric": doc.get("winner_metric"), "test_percent": doc.get("test_percent"),
                 "winner": winner, "test_sent_at": doc.get("test_sent_at"), "winner_chosen_at": doc.get("winner_chosen_at")},
    }))
}

async fn link_clicks(k: &Kernel, ctx: &Ctx, p: &Params<'_>) -> Result<Value> {
    let owner = match (p.uuid("campaign")?, p.uuid("journey")?) {
        (Some(c), _) => {
            k.get(ctx, "EmailCampaign", c, false).await?;
            c
        }
        (None, Some(j)) => {
            ctx.require("MarketingJourney", Action::Read)?;
            k.get(ctx, "MarketingJourney", j, false).await?;
            j
        }
        _ => return Err(Error::field("campaign", "choose a campaign or a journey")),
    };
    let mut tx = read_tx(k, ctx).await?;
    let data: Vec<(i32, String, i64, i64)> = sqlx::query_as(
        "select l.link_no, l.url, count(distinct x.recipient_id), count(x.recipient_id) from campaigns_link l \
         left join campaigns_click x on x.tenant_id = l.tenant_id and x.owner_id = l.owner_id and x.link_no = l.link_no \
         where l.tenant_id = $1 and l.owner_id = $2 group by l.link_no, l.url order by 4 desc, l.link_no",
    )
    .bind(ctx.tenant)
    .bind(owner)
    .fetch_all(&mut *tx)
    .await?;
    tx.rollback().await?;
    Ok(json!({
        "columns": [col("link_no", "#", "int"), col("url", "Link", "data"), col("unique_clicks", "Unique Clicks", "int"), col("total_clicks", "Total Clicks", "int")],
        "rows": data.iter().map(|(n, url, u, t)| json!({"link_no": n, "url": url, "unique_clicks": u, "total_clicks": t})).collect::<Vec<_>>(),
        "totals": {"unique_clicks": data.iter().map(|d| d.2).sum::<i64>(), "total_clicks": data.iter().map(|d| d.3).sum::<i64>()},
    }))
}

async fn funnel(k: &Kernel, ctx: &Ctx, p: &Params<'_>) -> Result<Value> {
    let journey = required(p, "journey")?;
    let doc = k.get(ctx, "MarketingJourney", journey, false).await?;
    let mut tx = read_tx(k, ctx).await?;
    let (enrolled, active, completed, goal, exited): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "select count(*), count(*) filter (where status = 'Active'), count(*) filter (where status = 'Completed'), \
         count(*) filter (where status = 'Goal Reached'), count(*) filter (where status = 'Exited') from e_journey_enrollment where tenant_id = $1 and journey = $2",
    )
    .bind(ctx.tenant)
    .bind(journey)
    .fetch_one(&mut *tx)
    .await?;
    let reached: HashMap<i32, i64> = sqlx::query_as::<_, (i32, i64)>(
        "select step, count(distinct enrollment_id) from campaigns_journey_log where tenant_id = $1 and journey_id = $2 group by step",
    )
    .bind(ctx.tenant)
    .bind(journey)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let mail: HashMap<i64, (i64, i64, i64)> = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "select journey_step, count(*) filter (where communication is not null), count(*) filter (where opened_at is not null), \
         count(*) filter (where clicked_at is not null) from e_campaign_recipient where tenant_id = $1 and journey = $2 group by journey_step",
    )
    .bind(ctx.tenant)
    .bind(journey)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(s, a, b, c)| (s, (a, b, c)))
    .collect();
    tx.rollback().await?;
    let rows: Vec<Value> = ferrum_domain::util::rows(&doc, "steps")
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let n = i + 1;
            let (sent, opens, clicks) = mail.get(&(n as i64)).copied().unwrap_or_default();
            let reach = reached.get(&(n as i32)).copied().unwrap_or_default();
            json!({"step": n, "step_type": r.get("step_type"), "label": r.get("label"), "reached": reach, "reached_rate": pct(reach, enrolled),
                   "emails_sent": sent, "unique_opens": opens, "unique_clicks": clicks, "open_rate": pct(opens, sent), "click_rate": pct(clicks, sent)})
        })
        .collect();
    Ok(json!({
        "columns": [col("step", "Step", "int"), col("step_type", "Type", "data"), col("label", "Label", "data"), col("reached", "Reached", "int"),
                    col("reached_rate", "Reached %", "percent"), col("emails_sent", "Emails Sent", "int"), col("unique_opens", "Unique Opens", "int"),
                    col("unique_clicks", "Unique Clicks", "int")],
        "rows": rows,
        "totals": {"enrolled": enrolled, "active": active, "completed": completed, "goal_reached": goal, "exited": exited, "conversion_rate": pct(goal, enrolled)},
        "meta": {"journey": doc.get("journey_name"), "status": doc.get("status")},
    }))
}

async fn growth(k: &Kernel, ctx: &Ctx, p: &Params<'_>) -> Result<Value> {
    let (from, to) = period(p, 30)?;
    if (to - from).num_days() > 731 {
        return Err(Error::field("from_date", "choose at most two years"));
    }
    let tz = ctx.schema.timezone.clone();
    let (start, end) = (from.and_time(NaiveTime::MIN), (to + Duration::days(1)).and_time(NaiveTime::MIN));
    let mut tx = read_tx(k, ctx).await?;
    // Local days: the bounds are local midnights in the workspace's zone.
    let created: HashMap<NaiveDate, i64> = sqlx::query_as::<_, (NaiveDate, i64)>(
        "select (created_at at time zone $2)::date, count(*) from e_marketing_contact where tenant_id = $1 \
         and created_at >= ($3::timestamp at time zone $2) and created_at < ($4::timestamp at time zone $2) group by 1",
    )
    .bind(ctx.tenant)
    .bind(&tz)
    .bind(start)
    .bind(end)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    // Movements into and out of the subscribed audience (a contact's first status counts as one).
    let moves: HashMap<NaiveDate, (i64, i64, i64, i64)> = sqlx::query_as::<_, (NaiveDate, i64, i64, i64, i64)>(
        "select (changed_at at time zone $2)::date, count(*) filter (where new_value = 'Subscribed' and old_value is distinct from 'Subscribed'), \
         count(*) filter (where old_value = 'Subscribed' and new_value = 'Unsubscribed'), count(*) filter (where old_value = 'Subscribed' and new_value = 'Bounced'), \
         count(*) filter (where old_value = 'Subscribed' and new_value = 'Complained') from campaigns_contact_change where tenant_id = $1 \
         and field = 'consent_status' and changed_at >= ($3::timestamp at time zone $2) and changed_at < ($4::timestamp at time zone $2) group by 1",
    )
    .bind(ctx.tenant)
    .bind(&tz)
    .bind(start)
    .bind(end)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(d, a, b, c, e)| (d, (a, b, c, e)))
    .collect();
    let (subscribed_now, total_now): (i64, i64) =
        sqlx::query_as("select count(*) filter (where consent_status = 'Subscribed'), count(*) from e_marketing_contact where tenant_id = $1")
            .bind(ctx.tenant)
            .fetch_one(&mut *tx)
            .await?;
    tx.rollback().await?;
    let mut rows = Vec::new();
    let mut totals = [0i64; 6];
    let mut day = from;
    while day <= to {
        let new = created.get(&day).copied().unwrap_or_default();
        let (subscribed, unsubscribed, bounced, complained) = moves.get(&day).copied().unwrap_or_default();
        let net = subscribed - unsubscribed - bounced - complained;
        for (t, v) in totals.iter_mut().zip([new, subscribed, unsubscribed, bounced, complained, net]) {
            *t += v;
        }
        rows.push(json!({"day": day.to_string(), "new_contacts": new, "subscribed": subscribed, "unsubscribed": unsubscribed, "bounced": bounced,
                         "complained": complained, "net_change": net}));
        day += Duration::days(1);
    }
    Ok(json!({
        "columns": [col("day", "Day", "date"), col("new_contacts", "New Contacts", "int"), col("subscribed", "New Subscribers", "int"),
                    col("unsubscribed", "Unsubscribed", "int"), col("bounced", "Bounced", "int"), col("complained", "Complained", "int"),
                    col("net_change", "Net Change", "int")],
        "rows": rows,
        "totals": {"new_contacts": totals[0], "subscribed": totals[1], "unsubscribed": totals[2], "bounced": totals[3], "complained": totals[4], "net_change": totals[5]},
        "meta": {"from_date": from.to_string(), "to_date": to.to_string(), "timezone": tz, "subscribed_now": subscribed_now, "contacts_now": total_now},
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_round_half_up() {
        assert_eq!(pct(1, 3), json!("33.33"));
        assert_eq!(pct(2, 3), json!("66.67"));
        assert_eq!(pct(5, 5), json!("100.00"));
        assert_eq!(pct(1, 0), json!("0.00"));
    }
}
