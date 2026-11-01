//! Read-only questions about a campaign, asked by the screens that prepare and judge a send: who
//! would receive it if it started now, and what happened to it since (by hour or by day). Neither
//! writes anything, so neither locks the campaign or leaves an audit entry.

use crate::campaign::{audience as audience_of, draft, extra, push_any, sample_contact, test_size};
use crate::contact::{EMAIL_PATTERN, MAILABLE};
use crate::render;
use crate::segment::{self, Rule, Segment};
use crate::send::{self, Links};
use crate::util::{self, b, id, int, rows, s, sys};
use chrono::{Duration, NaiveDateTime};
use ferrum_kernel::{Ctx, Doc, Error, HookCtx, Result, Tx};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

/// How the people an audience reaches split by what decides whether they are mailed: each is
/// mailed or left out for exactly one reason (the integration test checks that they add up).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tally {
    pub contacts: i64,
    pub mailable: i64,
    pub pending: i64,
    pub unsubscribed: i64,
    pub bounced: i64,
    pub complained: i64,
    /// Subscribed, but on the suppression list.
    pub suppressed: i64,
    /// Of `contacts`: members of the email groups who are not contacts yet; starting the campaign adds them.
    pub new_contacts: i64,
}

/// Counts what `start` would find: the existing contacts, plus the members of `groups` who are not
/// contacts yet (added as `Subscribed`, as `import::adopt_group_members` does when sending starts:
/// keep the two in step), in `include` and not in `exclude`.
async fn tally(tx: &mut Tx, ctx: &Ctx, groups: &[Uuid], include: &[Segment], exclude: &[Segment]) -> Result<Tally> {
    let (tenant, today, tz) = (ctx.tenant, ctx.today(), ctx.schema.timezone.clone());
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
        "with c_all as (select c.tenant_id, c.id, c.email, c.first_name, c.last_name, c.full_name, c.source, c.consent_status, c.record_entity, \
         c.attributes, c.created_at, false as adopted from e_marketing_contact c where c.tenant_id = ",
    );
    qb.push_bind(tenant)
        .push(" union all select ")
        .push_bind(tenant)
        .push(
            ", gen_random_uuid(), x.email, nullif(split_part(x.name, ' ', 1), ''), nullif(btrim(substr(x.name, length(split_part(x.name, ' ', 1)) + 1)), ''), \
             nullif(x.name, ''), 'Email Group', 'Subscribed', null::text, '{}'::jsonb, now(), true \
             from (select distinct on (lower(btrim(m.email))) lower(btrim(m.email)) as email, btrim(coalesce(m.member_name, '')) as name \
             from e_email_group_member m where m.tenant_id = ",
        )
        .push_bind(tenant)
        .push(" and m.email_group = any(")
        .push_bind(groups.to_vec())
        .push(") and not m.unsubscribed order by lower(btrim(m.email)), m.created_at) x where x.email ~ ")
        .push_bind(EMAIL_PATTERN)
        .push(" and not exists (select 1 from e_marketing_contact k where k.tenant_id = ")
        .push_bind(tenant)
        .push(" and k.email = x.email)) ");
    qb.push(format!(
        "select count(*), count(*) filter (where {MAILABLE}), count(*) filter (where c.consent_status = 'Pending'), \
         count(*) filter (where c.consent_status = 'Unsubscribed'), count(*) filter (where c.consent_status = 'Bounced'), \
         count(*) filter (where c.consent_status = 'Complained'), count(*) filter (where c.consent_status = 'Subscribed' and not {MAILABLE}), \
         count(*) filter (where c.adopted) from c_all c where c.tenant_id = "
    ));
    qb.push_bind(tenant).push(" and (");
    push_any(&mut qb, include, today, &tz);
    qb.push(") and not (");
    push_any(&mut qb, exclude, today, &tz);
    qb.push(")");
    let t: (i64, i64, i64, i64, i64, i64, i64, i64) = qb.build_query_as().fetch_one(&mut **tx).await?;
    Ok(Tally { contacts: t.0, mailable: t.1, pending: t.2, unsubscribed: t.3, bounced: t.4, complained: t.5, suppressed: t.6, new_contacts: t.7 })
}

/// The first message of an error, as a person reads it.
fn message(e: &Error) -> String {
    match e {
        Error::Validation(v) => v.first().map(|f| f.message.clone()).unwrap_or_else(|| e.to_string()),
        other => other.to_string(),
    }
}

/// Who this campaign would be mailed to if it started now, counted the way sending counts, and
/// whether it could start: the numbers behind "Send to N people?".
pub async fn audience(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<Value> {
    let ctx = hc.ctx;
    let tenant = ctx.tenant;
    let engine = sys(ctx);
    // One line per audience row, counted alone.
    let mut lines = Vec::new();
    for r in rows(doc, "audience") {
        let exclude = b(&r, "exclude");
        let (kind, row_id, name, seg, groups) = match (id(&r, "segment"), id(&r, "email_group")) {
            (Some(sid), _) => {
                let (name, seg) = segment::load(hc.kernel, &engine, hc.tx, sid).await?;
                ("segment", sid, name, seg, vec![])
            }
            (None, Some(g)) => {
                let name: Option<String> = sqlx::query_scalar("select group_name from e_email_group where tenant_id = $1 and id = $2")
                    .bind(tenant)
                    .bind(g)
                    .fetch_optional(&mut **hc.tx)
                    .await?;
                ("group", g, name.unwrap_or_default(), Segment { all: true, rules: vec![Rule::Group(g, true)] }, if exclude { vec![] } else { vec![g] })
            }
            (None, None) => continue,
        };
        let t = tally(hc.tx, ctx, &groups, &[seg], &[]).await?;
        lines.push(json!({"kind": kind, "id": row_id, "name": name, "exclude": exclude, "contacts": t.contacts, "mailable": t.mailable}));
    }
    let (include, exclude, groups) = audience_of(hc.kernel, &engine, hc.tx, doc).await?;
    let all = tally(hc.tx, ctx, &groups, &include, &exclude).await?;
    let before = tally(hc.tx, ctx, &groups, &include, &[]).await?;

    // Whether it could go out now, and what each version of the email will be.
    let base = util::site_base(hc.tx, tenant).await?;
    let postal_address = util::settings(hc.tx, tenant).await?.postal_address.is_some();
    let account = send::check_account(hc.kernel, hc.tx, tenant, id(doc, "email_account")).await.err().map(|e| message(&e));
    let links = Links {
        base: match &base {
            Some(url) => url.clone(),
            None => util::site_path(hc.tx, tenant).await?,
        },
        key: vec![0; 32],
        track_opens: true,
        tracked: Some(HashMap::new()),
        postal_address: None,
    };
    let mut versions = vec![None];
    if b(doc, "ab_test") {
        versions.extend(rows(doc, "variants").iter().filter_map(|r| s(r, "variant").map(|v| Some(v.to_string()))));
    }
    let emails: Vec<Value> = versions
        .into_iter()
        .map(|v| {
            let d = draft(doc, v.as_deref());
            let composed = send::compose(&d, &sample_contact(), &extra(doc), Uuid::nil(), &links);
            // Only static links are numbered and tracked when the campaign is scheduled.
            let tracked = if b(doc, "track_clicks") { render::static_links(&d.html) } else { vec![] };
            json!({
                "variant": v,
                "error": composed.as_ref().err().map(message),
                "bytes": composed.as_ref().ok().map(|c| c.html.len()),
                "links": tracked,
                "unsubscribe": if d.html.contains("unsubscribe_url") { "own" } else { "footer" },
            })
        })
        .collect();

    let workspace = send::hourly_limit(hc.tx, tenant).await?;
    let own = int(doc, "hourly_limit");
    let variants = rows(doc, "variants").len() as i64;
    let split = b(doc, "ab_test").then(|| {
        let test = test_size(all.mailable, int(doc, "test_percent").clamp(1, 90), variants);
        json!({"variants": variants, "test": test, "rest": all.mailable - test})
    });
    Ok(json!({
        "checked_at": util::ts(util::now()),
        "contacts": all.contacts,
        "mailable": all.mailable,
        "left_out": {"pending": all.pending, "unsubscribed": all.unsubscribed, "bounced": all.bounced, "complained": all.complained, "suppressed": all.suppressed},
        "excluded": before.contacts - all.contacts,
        "new_contacts": all.new_contacts,
        "rows": lines,
        "split": split,
        "rate": {"per_hour": if own > 0 { workspace.min(own) } else { workspace }, "workspace": workspace, "own": own},
        "ready": {"public_url": base.is_some(), "account": account, "postal_address": postal_address},
        "emails": emails,
    }))
}

/// Hours a timeline shows hour by hour at most; a campaign that ran longer is shown by day.
const HOURLY_SPAN: i64 = 14 * 24;
/// Days a timeline shows at most.
const DAILY_SPAN: i64 = 366;

/// One bucket's counts, by kind of event.
const KINDS: [&str; 6] = ["sent", "opens", "clicks", "unsubscribes", "bounces", "complaints"];

/// What happened to a campaign's emails, hour by hour or day by day (local to the workspace):
/// when they went out and when people first opened, first clicked, unsubscribed, bounced or
/// complained (one event per recipient, like the counters). Columns, not rows: `at[i]` goes with
/// `series.<kind>[i]`, and `variants.<label>` repeats the three that A/B tests compare.
pub async fn timeline(hc: &mut HookCtx<'_>, doc: &Doc, params: &Value) -> Result<Value> {
    let tenant = hc.ctx.tenant;
    let campaign = util::doc_id(doc)?;
    let tz = hc.ctx.schema.timezone.clone();
    let wanted = match params.get("interval").and_then(Value::as_str) {
        None | Some("auto") => None,
        Some("hour") => Some(true),
        Some("day") => Some(false),
        Some(_) => return Err(Error::field("interval", "use hour, day or auto")),
    };
    let found: Vec<(String, Option<String>, NaiveDateTime, i64)> = sqlx::query_as(
        "select e.kind, e.variant, date_trunc('hour', e.at at time zone $3), count(*) from ( \
           select 'sent' as kind, variant, sent_at as at from e_campaign_recipient where tenant_id = $1 and campaign = $2 and sent_at is not null \
           union all select 'opens', variant, opened_at from e_campaign_recipient where tenant_id = $1 and campaign = $2 and opened_at is not null \
           union all select 'clicks', variant, clicked_at from e_campaign_recipient where tenant_id = $1 and campaign = $2 and clicked_at is not null \
           union all select 'unsubscribes', variant, unsubscribed_at from e_campaign_recipient where tenant_id = $1 and campaign = $2 and unsubscribed_at is not null \
           union all select 'bounces', variant, bounced_at from e_campaign_recipient where tenant_id = $1 and campaign = $2 and bounced_at is not null \
           union all select 'complaints', variant, complained_at from e_campaign_recipient where tenant_id = $1 and campaign = $2 and complained_at is not null \
         ) e group by 1, 2, 3",
    )
    .bind(tenant)
    .bind(campaign)
    .bind(&tz)
    .fetch_all(&mut **hc.tx)
    .await?;
    let now: NaiveDateTime = sqlx::query_scalar("select date_trunc('hour', now() at time zone $1)").bind(&tz).fetch_one(&mut **hc.tx).await?;
    let first = found.iter().map(|f| f.2).min();
    // A campaign still sending keeps its chart open up to now.
    let last = found.iter().map(|f| f.2).max().map(|l| if s(doc, "status") == Some("Sending") { l.max(now) } else { l });
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(json!({"interval": "hour", "timezone": tz, "at": [], "series": empty_series(0), "variants": {}}));
    };
    let hours = (last - first).num_hours() + 1;
    let hourly = match wanted {
        Some(h) => h && hours <= HOURLY_SPAN,
        None => hours <= 72,
    };
    // Buckets are the first hour or day of each run: days start at local midnight.
    let bucket = |t: NaiveDateTime| if hourly { t } else { t.date().and_hms_opt(0, 0, 0).unwrap_or(t) };
    let step = if hourly { Duration::hours(1) } else { Duration::days(1) };
    let (mut start, end) = (bucket(first), bucket(last));
    // Only the last DAILY_SPAN days are drawn.
    if !hourly && (end - start).num_days() >= DAILY_SPAN {
        start = end - Duration::days(DAILY_SPAN - 1);
    }
    let mut at = Vec::new();
    let mut t = start;
    while t <= end {
        at.push(t);
        t += step;
    }
    let index: HashMap<NaiveDateTime, usize> = at.iter().enumerate().map(|(i, t)| (*t, i)).collect();
    let mut series = empty_series(at.len());
    let mut variants: BTreeMap<String, Value> = BTreeMap::new();
    for (kind, variant, when, n) in found {
        let Some(&i) = index.get(&bucket(when)) else { continue };
        series[kind.as_str()][i] = json!(series[kind.as_str()][i].as_i64().unwrap_or(0) + n);
        // Test emails go to the variants; the rest of the audience gets the winner, which is a variant too.
        if let (Some(v), "sent" | "opens" | "clicks") = (variant, kind.as_str()) {
            let entry = variants.entry(v).or_insert_with(|| empty_series(at.len()));
            entry[kind.as_str()][i] = json!(entry[kind.as_str()][i].as_i64().unwrap_or(0) + n);
        }
    }
    let local = |t: &NaiveDateTime| t.format("%Y-%m-%dT%H:%M:%S").to_string();
    Ok(json!({
        "interval": if hourly { "hour" } else { "day" },
        "timezone": tz,
        "at": at.iter().map(local).collect::<Vec<_>>(),
        "series": series,
        "variants": variants,
    }))
}

/// `{kind: [0; n]}` for every kind of event.
fn empty_series(n: usize) -> Value {
    Value::Object(KINDS.iter().map(|k| (k.to_string(), json!(vec![0; n]))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_start_empty_and_keep_every_kind() {
        let s = empty_series(3);
        for k in KINDS {
            assert_eq!(s[k], json!([0, 0, 0]), "{k}");
        }
        assert_eq!(empty_series(0)["sent"], json!([]));
    }

    #[test]
    fn errors_read_as_one_message() {
        assert_eq!(message(&Error::field("email_account", "does not exist")), "does not exist");
        assert_eq!(message(&Error::invalid("set the Public URL")), "set the Public URL");
    }
}
