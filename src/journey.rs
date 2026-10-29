//! Journeys: contacts enter on a trigger — joining a Mail email group, entering a segment, an
//! attribute changing, or a date attribute's anniversary — and maintenance moves each
//! enrollment through the steps. An enrollment is advanced in one transaction holding its row
//! (`FOR UPDATE SKIP LOCKED`): the step's effects (an email handed to the Mail app, an attribute
//! or email group change) and the new position commit together, so concurrent or repeated runs
//! never perform a step twice. Contacts leave when they unsubscribe or are suppressed, reach
//! the goal segment, hit an Exit step or run out of steps.

use crate::campaign::{self, Handover};
use crate::contact::{self, MAILABLE};
use crate::import;
use crate::segment::{self, Rule, Segment};
use crate::send::{self, Draft, Links};
use crate::util::{self, b, id, int, rows, s, sys, values};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result, Tx, with_retry};
use serde_json::{Map, Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::HashMap;
use std::time::{Duration as StdDuration, Instant};
use uuid::Uuid;

const MAX_STEPS: usize = 50;
/// Steps one enrollment may take in one run (a loop of branches without waits stops here).
const STEPS_PER_RUN: usize = 25;
/// Enrollments advanced per run at most.
const DUE_PER_RUN: i64 = 500;
/// Fields fixed once a journey was activated: in-flight enrollments point at step numbers.
const FROZEN: [&str; 7] = ["steps", "trigger_type", "trigger_group", "trigger_segment", "trigger_attribute", "trigger_value", "anniversary_offset_days"];

/// When a wait of `amount` hours or days started at `from` ends. Days keep the wall-clock time
/// in the workspace's zone across daylight-saving changes (a day can be 23 or 25 hours); a time
/// that does not exist that day (skipped by the change) moves forward by the gap.
pub fn wait_until(from: DateTime<Utc>, amount: i64, unit: &str, tz: &str) -> DateTime<Utc> {
    if unit == "Hours" {
        return from + Duration::hours(amount);
    }
    let Ok(zone) = tz.parse::<chrono_tz::Tz>() else { return from + Duration::days(amount) };
    let local = from.with_timezone(&zone).naive_local() + Duration::days(amount);
    match zone.from_local_datetime(&local) {
        LocalResult::Single(t) => t.with_timezone(&Utc),
        LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
        LocalResult::None => {
            zone.from_local_datetime(&(local + Duration::hours(1))).earliest().map(|t| t.with_timezone(&Utc)).unwrap_or(from + Duration::days(amount))
        }
    }
}

/// Month-day keys (`MM-DD`) whose anniversary falls on `day`: 29 February counts on the 28th in
/// years without one.
pub fn anniversary_keys(day: NaiveDate) -> Vec<String> {
    let mut keys = vec![day.format("%m-%d").to_string()];
    if day.month() == 2 && day.day() == 28 && NaiveDate::from_ymd_opt(day.year(), 2, 29).is_none() {
        keys.push("02-29".into());
    }
    keys
}

fn step_no(r: &Doc, k: &str) -> Option<usize> {
    r.get(k).and_then(Value::as_i64).filter(|n| *n > 0).map(|n| n as usize)
}

pub struct JourneyHook;

#[async_trait]
impl DocHook for JourneyHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        // Send Email steps: a Mail email template fills a blank subject and content.
        let mut steps = rows(doc, "steps");
        for r in steps.iter_mut() {
            if let Some(t) = id(r, "email_template")
                && (s(r, "subject").is_none() || s(r, "content").is_none())
            {
                let found: Option<(String, String)> =
                    sqlx::query_as("select subject, body from e_email_template where tenant_id = $1 and id = $2 and not disabled")
                        .bind(hc.ctx.tenant)
                        .bind(t)
                        .fetch_optional(&mut **hc.tx)
                        .await?;
                if let Some((subject, body)) = found {
                    if s(r, "subject").is_none() {
                        r.insert("subject".into(), json!(subject));
                    }
                    if s(r, "content").is_none() {
                        r.insert("content".into(), json!(body));
                    }
                }
            }
        }
        doc.insert("steps".into(), Value::Array(steps.into_iter().map(Value::Object).collect()));
        Ok(())
    }

    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut errors = Vec::new();
        let steps = rows(doc, "steps");
        if steps.is_empty() || steps.len() > MAX_STEPS {
            errors.push(FieldError::new("steps", format!("a journey has 1 to {MAX_STEPS} steps")));
        }
        for (i, r) in steps.iter().enumerate() {
            let mut err = |f: &str, m: &str| errors.push(FieldError::new(f, m).in_row("steps", i));
            for f in ["next_step", "else_step"] {
                if step_no(r, f).is_some_and(|n| n > steps.len()) {
                    err(f, "there is no such step");
                }
            }
            match s(r, "step_type").unwrap_or_default() {
                "Send Email" => {
                    if s(r, "subject").is_none() {
                        err("subject", "enter the subject");
                    }
                    if s(r, "content").is_none() {
                        err("content", "write the email");
                    }
                    for f in ["subject", "preheader", "content"] {
                        if let Some(src) = s(r, f)
                            && let Err(m) = crate::render::check(src)
                        {
                            err(f, &m);
                        }
                    }
                }
                "Wait" if int(r, "wait_amount") < 1 => err("wait_amount", "enter how long to wait"),
                "Branch" => match s(r, "branch_on") {
                    None => err("branch_on", "choose what to branch on"),
                    Some("Attribute Equals") if !s(r, "attribute").is_some_and(contact::valid_key) => err("attribute", "enter an attribute key"),
                    Some("In Segment") if id(r, "segment").is_none() => err("segment", "choose the segment"),
                    _ => {}
                },
                "Update Attribute" if !s(r, "attribute").is_some_and(contact::valid_key) => err("attribute", "enter an attribute key"),
                "Add To Group" | "Remove From Group" if id(r, "email_group").is_none() => err("email_group", "choose the email group"),
                _ => {}
            }
        }
        match s(doc, "trigger_type").unwrap_or_default() {
            "Attribute Changed" | "Date Anniversary" if !s(doc, "trigger_attribute").is_some_and(contact::valid_key) => {
                errors.push(FieldError::new("trigger_attribute", "enter an attribute key (lowercase letters, digits and _)"))
            }
            _ => {}
        }
        if !errors.is_empty() {
            return Err(Error::Validation(errors));
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if let Some(old) = old
            && !hc.ctx.is_system
            && s(old, "status").is_some_and(|st| st != "Draft")
            && FROZEN.iter().any(|f| norm(old.get(*f)) != norm(doc.get(*f)))
        {
            return Err(Error::invalid("the trigger and steps are fixed once a journey was activated; duplicate it to change them"));
        }
        Ok(())
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        sqlx::query("delete from campaigns_link where tenant_id = $1 and owner_id = $2")
            .bind(hc.ctx.tenant)
            .bind(util::doc_id(doc)?)
            .execute(&mut **hc.tx)
            .await?;
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        let journey = util::doc_id(doc)?;
        let status = s(doc, "status").unwrap_or("Draft");
        let tenant = hc.ctx.tenant;
        let mut v = Doc::new();
        match name {
            "activate" => {
                if !matches!(status, "Draft" | "Paused") {
                    return Err(Error::invalid("only draft or paused journeys can be activated"));
                }
                let steps = rows(doc, "steps");
                if steps.iter().any(|r| s(r, "step_type") == Some("Send Email")) {
                    util::site_base(hc.tx, tenant).await?.ok_or_else(send::public_url_missing)?;
                    send::check_account(hc.kernel, hc.tx, tenant, id(doc, "email_account")).await?;
                }
                for seg in [id(doc, "trigger_segment"), id(doc, "goal_segment")].into_iter().flatten().chain(steps.iter().filter_map(|r| id(r, "segment"))) {
                    segment::load(hc.kernel, hc.ctx, hc.tx, seg).await?;
                }
                if status == "Draft" {
                    let sources: Vec<&str> = steps.iter().filter_map(|r| r.get("content").and_then(Value::as_str)).collect();
                    send::register_links(hc.tx, tenant, journey, &sources).await?;
                    baseline(hc.kernel, &sys(hc.ctx), hc.tx, doc).await?;
                    v.insert("activated_at".into(), json!(util::ts(util::now())));
                }
                v.insert("status".into(), json!("Active"));
            }
            "pause" => {
                if status != "Active" {
                    return Err(Error::invalid("only active journeys can be paused"));
                }
                v.insert("status".into(), json!("Paused"));
            }
            "stop" => {
                if !matches!(status, "Active" | "Paused") {
                    return Err(Error::invalid("only active or paused journeys can be stopped"));
                }
                sqlx::query(
                    "update e_journey_enrollment set status = 'Exited', exit_reason = 'the journey was stopped', finished_at = now(), \
                     version = version + 1, updated_at = now() where tenant_id = $1 and journey = $2 and status = 'Active'",
                )
                .bind(tenant)
                .bind(journey)
                .execute(&mut **hc.tx)
                .await?;
                v.insert("status".into(), json!("Stopped"));
            }
            _ => return Ok(None),
        }
        hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "MarketingJourney", journey, v.clone()).await?;
        refresh_counts(hc.tx, tenant, Some(journey)).await?;
        Ok(Some(Value::Object(v)))
    }
}

/// A value for comparison: missing and null alike, child rows without their row ids, positions
/// and empty values (the kernel manages those).
fn norm(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Array(a)) => Value::Array(
            a.iter()
                .map(|r| match r {
                    Value::Object(m) => Value::Object(
                        m.iter()
                            .filter(|(k, v)| !matches!(k.as_str(), "id" | "idx" | "parent_id" | "ext") && !v.is_null())
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    ),
                    other => other.clone(),
                })
                .collect(),
        ),
        Some(other) => other.clone(),
    }
}

/// At first activation: who is already in the group or segment does not "join" it (unless the
/// journey includes them), and attribute changes count from now on.
async fn baseline(k: &Kernel, sys: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<()> {
    let journey = util::doc_id(doc)?;
    let tenant = sys.tenant;
    match s(doc, "trigger_type").unwrap_or_default() {
        "Joined Email Group" | "Entered Segment" if !b(doc, "include_existing") => {
            let Some(members) = membership(k, sys, tx, doc).await? else { return Ok(()) };
            let mut qb: QueryBuilder<Postgres> =
                QueryBuilder::new("insert into campaigns_journey_member (tenant_id, journey_id, contact_id) select c.tenant_id, ");
            qb.push_bind(journey).push(" , c.id from e_marketing_contact c where c.tenant_id = ").push_bind(tenant).push(" and ");
            members.push(&mut qb, sys.today(), &sys.schema.timezone);
            qb.push(" on conflict do nothing");
            qb.build().execute(&mut **tx).await?;
        }
        "Attribute Changed" => {
            let last: i64 =
                sqlx::query_scalar("select coalesce(max(id), 0) from campaigns_contact_change where tenant_id = $1").bind(tenant).fetch_one(&mut **tx).await?;
            k.set_values_tx(sys, tx, "MarketingJourney", journey, values([("trigger_cursor", json!(last))])).await?;
        }
        _ => {}
    }
    Ok(())
}

/// The trigger's group or segment as a condition (group members get contacts first).
async fn membership(k: &Kernel, sys: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<Option<Segment>> {
    Ok(match s(doc, "trigger_type").unwrap_or_default() {
        "Joined Email Group" => match id(doc, "trigger_group") {
            Some(g) => {
                import::adopt_group_members(tx, sys.tenant, &[g]).await?;
                Some(Segment { all: true, rules: vec![Rule::Group(g, true)] })
            }
            None => None,
        },
        "Entered Segment" => match id(doc, "trigger_segment") {
            Some(seg) => Some(segment::load(k, sys, tx, seg).await?.1),
            None => None,
        },
        _ => None,
    })
}

/// Enrollments are the journey's to manage; managers take a contact out with the action.
pub struct EnrollmentHook;

#[async_trait]
impl DocHook for EnrollmentHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if !hc.ctx.is_system {
            return Err(Error::Forbidden("journey enrollments are managed by their journey; use Take Out Of Journey".into()));
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "exit_journey" {
            return Ok(None);
        }
        if s(doc, "status") != Some("Active") {
            return Err(Error::invalid("this contact is no longer in the journey"));
        }
        let v = values([
            ("status", json!("Exited")),
            ("exit_reason", json!(util::cut(&format!("taken out by {}", hc.ctx.email), 200))),
            ("finished_at", json!(util::ts(util::now()))),
            ("waiting", json!(false)),
        ]);
        hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "JourneyEnrollment", util::doc_id(doc)?, v.clone()).await?;
        Ok(Some(Value::Object(v)))
    }
}

// ───────────────────────────── maintenance ─────────────────────────────

/// One round for a tenant's journeys: enroll from the triggers, apply goals and exits, advance
/// due enrollments within `budget`, refresh the counters.
pub async fn run(k: &Kernel, sys: &Ctx, budget: StdDuration) -> Result<usize> {
    let started = Instant::now();
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let active: Vec<Uuid> = sqlx::query_scalar("select id from e_marketing_journey where tenant_id = $1 and status = 'Active' order by id")
        .bind(tenant)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;
    for j in &active {
        with_retry(|| enroll(k, sys, *j)).await?;
    }
    leave(k, sys).await?;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let due: Vec<Uuid> = sqlx::query_scalar(
        "select e.id from e_journey_enrollment e join e_marketing_journey j on j.tenant_id = e.tenant_id and j.id = e.journey and j.status = 'Active' \
         where e.tenant_id = $1 and e.status = 'Active' and e.next_run_at <= now() order by e.next_run_at, e.id limit $2",
    )
    .bind(tenant)
    .bind(DUE_PER_RUN)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let cache = Cache::default();
    let mut moved = 0;
    for e in due {
        if started.elapsed() > budget {
            break;
        }
        if with_retry(|| advance(k, sys, e, &cache)).await? {
            moved += 1;
        }
    }
    let mut tx = k.db.tenant_tx(tenant).await?;
    refresh_counts(&mut tx, tenant, None).await?;
    tx.commit().await?;
    Ok(moved)
}

/// Journeys and their links, loaded once per run.
#[derive(Default)]
struct Cache {
    journeys: std::sync::Mutex<HashMap<Uuid, std::sync::Arc<(Doc, Option<Links>)>>>,
}

async fn journey_of(k: &Kernel, sys: &Ctx, cache: &Cache, journey: Uuid) -> Result<std::sync::Arc<(Doc, Option<Links>)>> {
    if let Some(hit) = cache.journeys.lock().map_err(|_| Error::internal("journey cache"))?.get(&journey) {
        return Ok(hit.clone());
    }
    let doc = k.get(sys, "MarketingJourney", journey, false).await?;
    let sends = rows(&doc, "steps").iter().any(|r| s(r, "step_type") == Some("Send Email"));
    let links = if sends { send::links_for(k, sys.tenant, journey, b(&doc, "track_opens"), b(&doc, "track_clicks")).await.ok() } else { None };
    let entry = std::sync::Arc::new((doc, links));
    cache.journeys.lock().map_err(|_| Error::internal("journey cache"))?.insert(journey, entry.clone());
    Ok(entry)
}

/// Inserts enrollments for `contacts` (a query yielding `c` rows with a `trigger_key`), mailable
/// ones only; an existing enrollment with that key, or an active one, wins.
fn push_enroll(qb: &mut QueryBuilder<Postgres>, tenant: Uuid, journey: Uuid) {
    qb.push(
        "insert into e_journey_enrollment (tenant_id, id, journey, contact, email, status, current_step, waiting, next_run_at, entered_at, steps_done, \
         trigger_key, created_at, updated_at) select ",
    )
    .push_bind(tenant)
    .push(", gen_random_uuid(), ")
    .push_bind(journey)
    .push(", c.id, c.email, 'Active', 1, false, now(), now(), 0, x.trigger_key, now(), now() from ");
}

async fn enroll(k: &Kernel, sys: &Ctx, journey: Uuid) -> Result<()> {
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let doc = k.get_tx(sys, &mut tx, "MarketingJourney", journey, true).await?;
    if s(&doc, "status") != Some("Active") {
        tx.rollback().await?;
        return Ok(());
    }
    let (today, tz) = (sys.today(), sys.schema.timezone.clone());
    let reentry = b(&doc, "allow_reentry");
    match s(&doc, "trigger_type").unwrap_or_default() {
        "Joined Email Group" | "Entered Segment" => {
            let Some(members) = membership(k, sys, &mut tx, &doc).await? else {
                tx.rollback().await?;
                return Ok(());
            };
            if reentry {
                // Who left may join again (and then enters again).
                let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("delete from campaigns_journey_member s where s.tenant_id = ");
                qb.push_bind(tenant).push(" and s.journey_id = ").push_bind(journey);
                qb.push(" and not exists (select 1 from e_marketing_contact c where c.tenant_id = s.tenant_id and c.id = s.contact_id and ");
                members.push(&mut qb, today, &tz);
                qb.push(")");
                qb.build().execute(&mut *tx).await?;
            }
            let mut qb: QueryBuilder<Postgres> =
                QueryBuilder::new("with fresh as (insert into campaigns_journey_member (tenant_id, journey_id, contact_id) select c.tenant_id, ");
            qb.push_bind(journey).push(", c.id from e_marketing_contact c where c.tenant_id = ").push_bind(tenant).push(" and ");
            members.push(&mut qb, today, &tz);
            qb.push(" on conflict do nothing returning contact_id, seen_at) ");
            push_enroll(&mut qb, tenant, journey);
            qb.push(
                "(select f.contact_id, 'entry:' || (extract(epoch from f.seen_at) * 1000000)::bigint as trigger_key from fresh f) x \
                 join e_marketing_contact c on c.tenant_id = ",
            )
            .push_bind(tenant)
            .push(format!(" and c.id = x.contact_id where {MAILABLE} on conflict do nothing"));
            qb.build().execute(&mut *tx).await?;
        }
        "Attribute Changed" => {
            let key = format!("attr:{}", s(&doc, "trigger_attribute").unwrap_or_default());
            let cursor = int(&doc, "trigger_cursor");
            let last: Option<i64> = sqlx::query_scalar(
                "select max(id) from (select id from campaigns_contact_change where tenant_id = $1 and field = $2 and id > $3 order by id limit 1000) x",
            )
            .bind(tenant)
            .bind(&key)
            .bind(cursor)
            .fetch_one(&mut *tx)
            .await?;
            if let Some(last) = last {
                let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("");
                push_enroll(&mut qb, tenant, journey);
                qb.push("(select contact_id, 'change:' || id as trigger_key from campaigns_contact_change where tenant_id = ")
                    .push_bind(tenant)
                    .push(" and field = ")
                    .push_bind(key.clone())
                    .push(" and id > ")
                    .push_bind(cursor)
                    .push(" and id <= ")
                    .push_bind(last)
                    .push(" and new_value is not null");
                if let Some(value) = s(&doc, "trigger_value") {
                    qb.push(" and lower(new_value) = lower(").push_bind(value.to_string()).push(")");
                }
                qb.push(") x join e_marketing_contact c on c.tenant_id = ").push_bind(tenant).push(format!(" and c.id = x.contact_id where {MAILABLE}"));
                if !reentry {
                    qb.push(" and not exists (select 1 from e_journey_enrollment p where p.tenant_id = c.tenant_id and p.journey = ")
                        .push_bind(journey)
                        .push(" and p.contact = c.id)");
                }
                qb.push(" on conflict do nothing");
                qb.build().execute(&mut *tx).await?;
                k.set_values_tx(sys, &mut tx, "MarketingJourney", journey, values([("trigger_cursor", json!(last))])).await?;
            }
        }
        "Date Anniversary" => {
            let target = today - Duration::days(int(&doc, "anniversary_offset_days"));
            let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("");
            push_enroll(&mut qb, tenant, journey);
            qb.push("(select ")
                .push_bind(format!("date:{}", target.year()))
                .push("::text as trigger_key) x cross join e_marketing_contact c where c.tenant_id = ")
                .push_bind(tenant);
            qb.push(format!(" and {MAILABLE} and coalesce(c.attributes ->> "))
                .push_bind(s(&doc, "trigger_attribute").unwrap_or_default().to_string())
                .push(", '') ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}' and substring(c.attributes ->> ")
                .push_bind(s(&doc, "trigger_attribute").unwrap_or_default().to_string())
                .push(" from 6 for 5) = any(")
                .push_bind(anniversary_keys(target))
                .push(") on conflict do nothing");
            qb.build().execute(&mut *tx).await?;
        }
        _ => {}
    }
    tx.commit().await?;
    Ok(())
}

/// Set-based exits: contacts who reached a goal segment, then contacts no longer mailable.
async fn leave(k: &Kernel, sys: &Ctx) -> Result<()> {
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let goals: Vec<(Uuid, Uuid)> =
        sqlx::query_as("select id, goal_segment from e_marketing_journey where tenant_id = $1 and status in ('Active', 'Paused') and goal_segment is not null")
            .bind(tenant)
            .fetch_all(&mut *tx)
            .await?;
    for (journey, seg) in goals {
        let Ok((_, goal)) = segment::load(k, sys, &mut tx, seg).await else { continue };
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
            "update e_journey_enrollment e set status = 'Goal Reached', finished_at = now(), waiting = false, version = e.version + 1, updated_at = now() \
             from e_marketing_contact c where e.tenant_id = ",
        );
        qb.push_bind(tenant)
            .push(" and e.journey = ")
            .push_bind(journey)
            .push(" and e.status = 'Active' and c.tenant_id = e.tenant_id and c.id = e.contact and ");
        goal.push(&mut qb, sys.today(), &sys.schema.timezone);
        qb.build().execute(&mut *tx).await?;
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "update e_journey_enrollment e set status = 'Exited', exit_reason = 'unsubscribed or suppressed', finished_at = now(), waiting = false, \
         version = e.version + 1, updated_at = now() where e.tenant_id = $1 and e.status = 'Active' \
         and not exists (select 1 from e_marketing_contact c where c.tenant_id = e.tenant_id and c.id = e.contact and {MAILABLE})"
    )))
    .bind(tenant)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The enrollment's state while it moves.
struct State {
    status: String,
    step: usize,
    waiting: bool,
    next_run_at: DateTime<Utc>,
    last_recipient: Option<Uuid>,
    steps_done: i64,
    exit_reason: Option<String>,
}

async fn log(tx: &mut Tx, tenant: Uuid, journey: Uuid, enrollment: Uuid, step: usize, outcome: &str) -> Result<()> {
    sqlx::query("insert into campaigns_journey_log (tenant_id, journey_id, enrollment_id, step, outcome) values ($1, $2, $3, $4, $5)")
        .bind(tenant)
        .bind(journey)
        .bind(enrollment)
        .bind(step as i32)
        .bind(util::cut(outcome, 200))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn in_segment(k: &Kernel, sys: &Ctx, tx: &mut Tx, seg: Uuid, contact: Uuid) -> Result<bool> {
    let (_, seg) = segment::load(k, sys, tx, seg).await?;
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select exists (select 1 from e_marketing_contact c where c.tenant_id = ");
    qb.push_bind(sys.tenant).push(" and c.id = ").push_bind(contact).push(" and ");
    seg.push(&mut qb, sys.today(), &sys.schema.timezone);
    qb.push(")");
    Ok(qb.build_query_scalar().fetch_one(&mut **tx).await?)
}

/// Moves one due enrollment as far as it can go now; false when it was not due any more.
async fn advance(k: &Kernel, sys: &Ctx, enrollment: Uuid, cache: &Cache) -> Result<bool> {
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let row: Option<(Uuid, Uuid, String, i64, bool, Option<Uuid>, i64)> = sqlx::query_as(
        "select journey, contact, email, coalesce(current_step, 1), waiting, last_recipient, coalesce(steps_done, 0) from e_journey_enrollment \
         where tenant_id = $1 and id = $2 and status = 'Active' and next_run_at <= now() for update skip locked",
    )
    .bind(tenant)
    .bind(enrollment)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((journey, contact_id, email, step, waiting, last_recipient, steps_done)) = row else {
        tx.rollback().await?;
        return Ok(false);
    };
    let entry = journey_of(k, sys, cache, journey).await?;
    let (jdoc, links) = (&entry.0, entry.1.as_ref());
    let steps = rows(jdoc, "steps");
    let mut st =
        State { status: "Active".into(), step: step.max(1) as usize, waiting, next_run_at: util::now(), last_recipient, steps_done, exit_reason: None };
    for _ in 0..STEPS_PER_RUN {
        let contact_row: Option<(Value, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "select jsonb_build_object('id', c.id, 'email', c.email, 'first_name', c.first_name, 'last_name', c.last_name, 'full_name', c.full_name, \
             'attributes', coalesce(c.attributes, '{{}}'::jsonb)), {MAILABLE} from e_marketing_contact c where c.tenant_id = $1 and c.id = $2"
        )))
        .bind(tenant)
        .bind(contact_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((view, mailable)) = contact_row else { break };
        if !mailable {
            st.status = "Exited".into();
            st.exit_reason = Some("unsubscribed or suppressed".into());
            break;
        }
        if let Some(goal) = id(jdoc, "goal_segment")
            && in_segment(k, sys, &mut tx, goal, contact_id).await?
        {
            st.status = "Goal Reached".into();
            break;
        }
        let Some(r) = steps.get(st.step - 1) else {
            st.status = "Completed".into();
            break;
        };
        let here = st.step;
        let next = step_no(r, "next_step").unwrap_or(here + 1);
        match s(r, "step_type").unwrap_or_default() {
            "Send Email" => {
                let Some(links) = links else {
                    // The Public URL went missing: try again later.
                    st.next_run_at = util::now() + Duration::minutes(10);
                    break;
                };
                send::lock_budget(&mut tx, tenant).await?;
                if send::hourly_room(&mut tx, tenant).await? <= 0 {
                    st.next_run_at = util::now() + Duration::minutes(5);
                    break;
                }
                let recipient = Uuid::now_v7();
                sqlx::query(
                    "insert into e_campaign_recipient (tenant_id, id, journey, journey_step, enrollment, contact, email, status, queued_at, \
                     open_count, machine_open_count, click_count, created_at, updated_at) values ($1, $2, $3, $4, $5, $6, $7, 'Queued', now(), 0, 0, 0, now(), now())",
                )
                .bind(tenant)
                .bind(recipient)
                .bind(journey)
                .bind(here as i64)
                .bind(enrollment)
                .bind(contact_id)
                .bind(&email)
                .execute(&mut *tx)
                .await?;
                let d = Draft {
                    subject: s(r, "subject").unwrap_or_default().to_string(),
                    preheader: s(r, "preheader").map(str::to_string),
                    html: s(r, "content").unwrap_or_default().to_string(),
                    text: None,
                };
                let extra: Map<String, Value> = values([("journey", json!({"name": s(jdoc, "journey_name")}))]);
                match campaign::hand_over(k, sys, &mut tx, id(jdoc, "email_account"), &email, &d, &view, &extra, recipient, links, contact_id).await {
                    Ok(Handover::Queued(comm)) => {
                        sqlx::query("update e_campaign_recipient set communication = $3 where tenant_id = $1 and id = $2")
                            .bind(tenant)
                            .bind(recipient)
                            .bind(comm)
                            .execute(&mut *tx)
                            .await?;
                        log(&mut tx, tenant, journey, enrollment, here, "sent").await?;
                    }
                    Ok(Handover::Skipped(_)) => {}
                    Err(e) => {
                        sqlx::query("update e_campaign_recipient set status = 'Failed', error = $3, queued_at = null where tenant_id = $1 and id = $2")
                            .bind(tenant)
                            .bind(recipient)
                            .bind(util::cut(&e.to_string(), 500))
                            .execute(&mut *tx)
                            .await?;
                        log(&mut tx, tenant, journey, enrollment, here, &format!("failed: {e}")).await?;
                    }
                }
                st.last_recipient = Some(recipient);
                st.step = next;
            }
            "Wait" => {
                if st.waiting {
                    st.waiting = false;
                    log(&mut tx, tenant, journey, enrollment, here, "waited").await?;
                    st.step = next;
                } else {
                    st.waiting = true;
                    st.next_run_at = wait_until(util::now(), int(r, "wait_amount").max(1), s(r, "wait_unit").unwrap_or("Days"), &sys.schema.timezone);
                    log(&mut tx, tenant, journey, enrollment, here, "waiting").await?;
                    st.steps_done += 1;
                    break;
                }
            }
            "Branch" => {
                let holds = match s(r, "branch_on").unwrap_or_default() {
                    "Opened Last Email" | "Clicked Last Email" => match st.last_recipient {
                        Some(lr) => {
                            let (opened, clicked): (bool, bool) = sqlx::query_as(
                                "select opened_at is not null, clicked_at is not null from e_campaign_recipient where tenant_id = $1 and id = $2",
                            )
                            .bind(tenant)
                            .bind(lr)
                            .fetch_optional(&mut *tx)
                            .await?
                            .unwrap_or((false, false));
                            if s(r, "branch_on") == Some("Opened Last Email") { opened } else { clicked }
                        }
                        None => false,
                    },
                    "Attribute Equals" => {
                        let attr = view.get("attributes").and_then(|a| a.get(s(r, "attribute").unwrap_or_default()));
                        contact::attr_text(attr).unwrap_or_default().to_lowercase() == s(r, "value").unwrap_or_default().to_lowercase()
                    }
                    "In Segment" => match id(r, "segment") {
                        Some(seg) => in_segment(k, sys, &mut tx, seg, contact_id).await?,
                        None => false,
                    },
                    _ => false,
                };
                log(&mut tx, tenant, journey, enrollment, here, if holds { "yes" } else { "no" }).await?;
                st.step = if holds { next } else { step_no(r, "else_step").unwrap_or(steps.len() + 1) };
            }
            "Update Attribute" => {
                let key = s(r, "attribute").unwrap_or_default().to_string();
                let mut attrs = view.get("attributes").and_then(Value::as_object).cloned().unwrap_or_default();
                match s(r, "value") {
                    Some(v) => attrs.insert(key, json!(v)),
                    None => attrs.remove(&key),
                };
                k.update_tx(sys, &mut tx, "MarketingContact", contact_id, values([("attributes", Value::Object(attrs))]), None).await?;
                log(&mut tx, tenant, journey, enrollment, here, "updated").await?;
                st.step = next;
            }
            "Add To Group" => {
                if let Some(g) = id(r, "email_group") {
                    let known: Option<Uuid> =
                        sqlx::query_scalar("select id from e_email_group_member where tenant_id = $1 and email_group = $2 and lower(email) = $3")
                            .bind(tenant)
                            .bind(g)
                            .bind(&email)
                            .fetch_optional(&mut *tx)
                            .await?;
                    // Someone who left the group stays out (their opt-out is not undone).
                    if known.is_none() {
                        let member = values([
                            ("email_group", json!(g.to_string())),
                            ("email", json!(email)),
                            ("member_name", view.get("full_name").cloned().unwrap_or(Value::Null)),
                        ]);
                        k.insert_tx(sys, &mut tx, "EmailGroupMember", member).await?;
                    }
                }
                log(&mut tx, tenant, journey, enrollment, here, "added").await?;
                st.step = next;
            }
            "Remove From Group" => {
                if let Some(g) = id(r, "email_group") {
                    let known: Option<Uuid> =
                        sqlx::query_scalar("select id from e_email_group_member where tenant_id = $1 and email_group = $2 and lower(email) = $3")
                            .bind(tenant)
                            .bind(g)
                            .bind(&email)
                            .fetch_optional(&mut *tx)
                            .await?;
                    if let Some(m) = known {
                        k.delete_tx(sys, &mut tx, "EmailGroupMember", m, None).await?;
                    }
                }
                log(&mut tx, tenant, journey, enrollment, here, "removed").await?;
                st.step = next;
            }
            _ => {
                log(&mut tx, tenant, journey, enrollment, here, "exit").await?;
                st.status = "Completed".into();
                st.steps_done += 1;
                break;
            }
        }
        st.steps_done += 1;
    }
    let finished = st.status != "Active";
    sqlx::query(
        "update e_journey_enrollment set status = $3, current_step = $4, waiting = $5, next_run_at = $6, last_recipient = $7, steps_done = $8, \
         exit_reason = coalesce($9, exit_reason), finished_at = case when $3 <> 'Active' then now() end, version = version + 1, updated_at = now() \
         where tenant_id = $1 and id = $2",
    )
    .bind(tenant)
    .bind(enrollment)
    .bind(&st.status)
    .bind(st.step as i64)
    .bind(st.waiting && !finished)
    .bind(st.next_run_at)
    .bind(st.last_recipient)
    .bind(st.steps_done)
    .bind(&st.exit_reason)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// The counters on journeys (one, or every one not stopped).
async fn refresh_counts(tx: &mut Tx, tenant: Uuid, journey: Option<Uuid>) -> Result<()> {
    sqlx::query(
        "update e_marketing_journey j set enrolled_count = x.enrolled, active_count = x.active, completed_count = x.completed, goal_count = x.goal, \
         exited_count = x.exited, updated_at = now() \
         from (select j2.id, count(e.id) as enrolled, count(e.id) filter (where e.status = 'Active') as active, \
               count(e.id) filter (where e.status = 'Completed') as completed, count(e.id) filter (where e.status = 'Goal Reached') as goal, \
               count(e.id) filter (where e.status = 'Exited') as exited from e_marketing_journey j2 \
               left join e_journey_enrollment e on e.tenant_id = j2.tenant_id and e.journey = j2.id \
               where j2.tenant_id = $1 and ($2::uuid is null or j2.id = $2) and ($2::uuid is not null or j2.status <> 'Stopped') group by j2.id) x \
         where j.tenant_id = $1 and j.id = x.id and (j.enrolled_count, j.active_count, j.completed_count, j.goal_count, j.exited_count) \
         is distinct from (x.enrolled, x.active, x.completed, x.goal, x.exited)",
    )
    .bind(tenant)
    .bind(journey)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn day_waits_keep_the_local_time_across_dst() {
        let ny = "America/New_York";
        // Spring forward (8 March 2026): 10:00 EST → 10:00 EDT is 23 hours.
        assert_eq!(wait_until(at("2026-03-07T15:00:00Z"), 1, "Days", ny), at("2026-03-08T14:00:00Z"));
        // Fall back (1 November 2026): 10:00 EDT → 10:00 EST is 25 hours.
        assert_eq!(wait_until(at("2026-10-31T14:00:00Z"), 1, "Days", ny), at("2026-11-01T15:00:00Z"));
        // 02:30 does not exist on 8 March: an hour later, 03:30 EDT.
        assert_eq!(wait_until(at("2026-03-07T07:30:00Z"), 1, "Days", ny), at("2026-03-08T07:30:00Z"));
        // 01:30 happens twice on 1 November: the first one (EDT).
        assert_eq!(wait_until(at("2026-10-31T05:30:00Z"), 1, "Days", ny), at("2026-11-01T05:30:00Z"));
        // Hours are exact durations; an unknown zone falls back to UTC days.
        assert_eq!(wait_until(at("2026-03-07T15:00:00Z"), 24, "Hours", ny), at("2026-03-08T15:00:00Z"));
        assert_eq!(wait_until(at("2026-03-07T15:00:00Z"), 2, "Days", "Nowhere/Land"), at("2026-03-09T15:00:00Z"));
    }

    #[test]
    fn leap_day_anniversaries() {
        assert_eq!(anniversary_keys(NaiveDate::from_ymd_opt(2027, 2, 28).unwrap()), vec!["02-28", "02-29"]);
        assert_eq!(anniversary_keys(NaiveDate::from_ymd_opt(2028, 2, 28).unwrap()), vec!["02-28"]);
        assert_eq!(anniversary_keys(NaiveDate::from_ymd_opt(2028, 2, 29).unwrap()), vec!["02-29"]);
        assert_eq!(anniversary_keys(NaiveDate::from_ymd_opt(2026, 7, 4).unwrap()), vec!["07-04"]);
    }
}
