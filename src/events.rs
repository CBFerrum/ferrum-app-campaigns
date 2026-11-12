//! Journeys that start, wait for, branch on and end with what happens to records of any entity the workspace has: a
//! journey's event rules name an entity, an event, a condition (the platform's record expressions: `doc`, `old`,
//! `links.<field>`, as rules and automations read them) and the field that names the person.
//!
//! The platform hands every record event to this app's outbox sink after commit, at least once. For each active
//! journey with a rule on the event's entity, the condition is evaluated and the person found among the marketing
//! contacts (never created: a record names a person, it does not give their consent). Then, in one transaction:
//! the trigger rule enrolls them (once per person, journey and rule, keyed so a repeated delivery finds the
//! enrollment it made; a setting allows re-entry after a time); the goal rule ends their run; any rule is kept for the
//! Wait For Event and Branch steps, and wakes a person waiting on it.

use crate::util::{id, rows, s};
use async_trait::async_trait;
use ferrum_kernel::outbox::Event;
use ferrum_kernel::{Ctx, Doc, Error, EventSink, Kernel, Result, Tx, with_retry};
use ferrum_meta::{EffEntity, Expr, FieldType, Schema, Scope};
use serde_json::{Map, Value};
use uuid::Uuid;

/// The record events a rule's `event` takes.
fn events_of(rule_event: &str) -> &'static [&'static str] {
    match rule_event {
        "Created" => &["created"],
        "Updated" => &["updated"],
        "Submitted" => &["submitted"],
        "Cancelled" => &["cancelled"],
        _ => &["created", "updated", "submitted", "cancelled", "transitioned"],
    }
}

/// What is wrong with a condition over records of `ent`: it compiles, reads fields the entity has, links through its
/// link fields, and no settings (a journey reads the record and its links only).
pub fn check_condition(schema: &Schema, ent: &EffEntity, src: &str) -> std::result::Result<Expr, String> {
    let e = Expr::compile(src)?;
    if let Some(f) = e.doc_fields().into_iter().find(|f| ent.field(f).is_none() && !matches!(f.as_str(), "id" | "name" | "number" | "docstatus" | "status")) {
        return Err(format!("{} has no field {f}", ent.def.name));
    }
    if let Some(l) =
        e.links().iter().find(|l| !ent.field(l).is_some_and(|f| f.ftype == FieldType::Link && f.to.as_deref().is_some_and(|t| schema.entity(t).is_some())))
    {
        return Err(format!("links.{l}: {} has no link field {l}", ent.def.name));
    }
    if !e.settings().is_empty() {
        return Err("a journey's condition reads the record and its links, not settings".into());
    }
    Ok(e)
}

/// What is wrong with a person field of `ent` (blank is the record itself).
pub fn check_person_field(ent: &EffEntity, field: Option<&str>) -> Option<String> {
    let f = field?;
    match ent.field(f) {
        Some(d) if matches!(d.ftype, FieldType::Email | FieldType::Link) => None,
        Some(_) => Some(format!("{f} is neither an email field nor a link")),
        None => Some(format!("{} has no field {f}", ent.def.name)),
    }
}

/// The records an expression reads as `links.<field>`: each one as the workspace holds it, its secret fields left out,
/// one level (as the platform binds them for rules, BD-14).
async fn links(k: &Kernel, sys: &Ctx, tx: &mut Tx, ent: &EffEntity, raw: &Value, names: &[String]) -> Result<Value> {
    let mut out = Map::new();
    for name in names {
        let tent = ent.field(name).filter(|f| f.ftype == FieldType::Link).and_then(|f| f.to.as_deref()).and_then(|t| sys.schema.entity(t)).cloned();
        let rid = raw.get(name).and_then(Value::as_str).and_then(|v| v.parse::<Uuid>().ok());
        let found = match (tent, rid) {
            (Some(tent), Some(rid)) => match k.get_tx(sys, tx, &tent.def.name, rid, false).await {
                Ok(mut d) => {
                    for f in tent.def.fields.iter().filter(|f| f.ftype == FieldType::Secret) {
                        d.remove(&f.name);
                    }
                    tent.cel_view(&Value::Object(d), &sys.schema)
                }
                Err(Error::NotFound(_)) => Value::Null,
                Err(e) => return Err(e),
            },
            _ => Value::Null,
        };
        out.insert(name.clone(), found);
    }
    Ok(Value::Object(out))
}

/// Whether `e` holds for `raw` (a record of `ent` as stored), `was` what the change replaced.
pub async fn holds(k: &Kernel, sys: &Ctx, tx: &mut Tx, ent: &EffEntity, e: &Expr, raw: &Value, was: Option<&Value>) -> Result<bool> {
    let doc = ent.cel_view(raw, &sys.schema);
    let old = was.and_then(Value::as_object).map(|w| {
        let mut o = raw.clone();
        for (k, v) in w {
            o[k] = v.clone();
        }
        ent.cel_view(&o, &sys.schema)
    });
    let linked = links(k, sys, tx, ent, raw, e.links()).await?;
    let user = sys.user_value();
    e.eval_bool_with(&Scope { doc: &doc, old: old.as_ref(), parent: None, user: &user }, &[("links", &linked)])
        .map_err(|m| Error::invalid(format!("the condition does not evaluate: {m}")))
}

/// The marketing contacts a record names (see the module).
pub async fn people(k: &Kernel, sys: &Ctx, tx: &mut Tx, ent: &EffEntity, raw: &Value, person_field: Option<&str>) -> Result<Vec<Uuid>> {
    let tenant = sys.tenant;
    // The record holding the person: this one, or the one a link names.
    let (holder, hraw): (EffEntity, Value) = match person_field.and_then(|f| ent.field(f)) {
        Some(f) if f.ftype == FieldType::Email => {
            return by_email(tx, tenant, raw.get(&f.name).and_then(Value::as_str).into_iter().collect()).await;
        }
        Some(f) => {
            let Some(rid) = raw.get(&f.name).and_then(Value::as_str).and_then(|v| v.parse::<Uuid>().ok()) else { return Ok(vec![]) };
            let Some(tent) = f.to.as_deref().and_then(|t| sys.schema.entity(t)).cloned() else { return Ok(vec![]) };
            if tent.def.name == "MarketingContact" {
                return Ok(vec![rid]);
            }
            match k.get_tx(sys, tx, &tent.def.name, rid, false).await {
                Ok(d) => ((*tent).clone(), Value::Object(d)),
                Err(Error::NotFound(_)) => return Ok(vec![]),
                Err(e) => return Err(e),
            }
        }
        None if ent.def.name == "MarketingContact" => return Ok(id(raw.as_object().unwrap_or(&Map::new()), "id").into_iter().collect()),
        None => (ent.clone(), raw.clone()),
    };
    let rid = hraw.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    let linked: Vec<Uuid> = sqlx::query_scalar("select id from e_marketing_contact where tenant_id = $1 and record_entity = $2 and record_id = $3")
        .bind(tenant)
        .bind(&holder.def.name)
        .bind(&rid)
        .fetch_all(&mut **tx)
        .await?;
    if !linked.is_empty() {
        return Ok(linked);
    }
    let emails: Vec<&str> =
        holder.def.fields.iter().filter(|f| f.ftype == FieldType::Email).filter_map(|f| hraw.get(&f.name).and_then(Value::as_str)).collect();
    by_email(tx, tenant, emails).await
}

async fn by_email(tx: &mut Tx, tenant: Uuid, emails: Vec<&str>) -> Result<Vec<Uuid>> {
    let emails: Vec<String> = emails.into_iter().map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
    if emails.is_empty() {
        return Ok(vec![]);
    }
    Ok(sqlx::query_scalar("select id from e_marketing_contact where tenant_id = $1 and email = any($2)").bind(tenant).bind(emails).fetch_all(&mut **tx).await?)
}

/// Whether the event rule `n` of `journey` happened to `contact` since `since`.
pub async fn happened(tx: &mut Tx, tenant: Uuid, journey: Uuid, n: i64, contact: Uuid, since: chrono::DateTime<chrono::Utc>) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "select exists (select 1 from campaigns_journey_event where tenant_id = $1 and journey_id = $2 and rule_no = $3 and contact_id = $4 and at >= $5)",
    )
    .bind(tenant)
    .bind(journey)
    .bind(n as i32)
    .bind(contact)
    .bind(since)
    .fetch_one(&mut **tx)
    .await?)
}

/// The record that started an enrollment, as stored now (`None`: none, or gone).
pub async fn trigger_record(
    k: &Kernel,
    sys: &Ctx,
    tx: &mut Tx,
    entity: Option<&str>,
    record: Option<&str>,
) -> Result<Option<(std::sync::Arc<EffEntity>, Value)>> {
    let (Some(entity), Some(rid)) = (entity, record.and_then(|r| r.parse::<Uuid>().ok())) else { return Ok(None) };
    let Some(ent) = sys.schema.entity(entity).cloned() else { return Ok(None) };
    match k.get_tx(sys, tx, entity, rid, false).await {
        Ok(d) => Ok(Some((ent, Value::Object(d)))),
        Err(Error::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

pub struct JourneySink;

#[async_trait]
impl EventSink for JourneySink {
    async fn on_event(&self, k: &Kernel, ev: &Event) -> Result<()> {
        let Some((entity, event)) = ev.topic.split_once('.') else { return Ok(()) };
        let Some(record) = ev.record_id else { return Ok(()) };
        // Nothing of ours, and nothing a journey can listen to, without the package.
        let schema = k.registry.schema(ev.tenant_id).await?;
        if !schema.packages.iter().any(|p| p == crate::util::PACKAGE) || entity.starts_with("Journey") || entity == "CampaignRecipient" {
            return Ok(());
        }
        let Some(ent) = schema.entity(entity).cloned() else { return Ok(()) };
        let sys = Ctx::system(ev.tenant_id, schema);
        // The journeys listening to this entity, read before any work.
        let mut tx = k.db.tenant_tx(ev.tenant_id).await?;
        let listening: Vec<Uuid> = sqlx::query_scalar(
            "select distinct j.id from e_marketing_journey j join e_marketing_journey__event_rules r on r.tenant_id = j.tenant_id and r.parent_id = j.id \
             where j.tenant_id = $1 and j.status = 'Active' and r.entity_name = $2",
        )
        .bind(ev.tenant_id)
        .bind(entity)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let raw = ev.payload.get("doc").cloned().unwrap_or(Value::Null);
        let was = ev.payload.get("was").cloned();
        for journey in listening {
            with_retry(|| take(k, &sys, ev, journey, &ent, event, record, &raw, was.as_ref())).await?;
        }
        Ok(())
    }
}

/// One journey's share of one event, in one transaction.
#[allow(clippy::too_many_arguments)]
async fn take(k: &Kernel, sys: &Ctx, ev: &Event, journey: Uuid, ent: &EffEntity, event: &str, record: Uuid, raw: &Value, was: Option<&Value>) -> Result<()> {
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let doc = k.get_tx(sys, &mut tx, "MarketingJourney", journey, true).await?;
    if s(&doc, "status") != Some("Active") {
        tx.rollback().await?;
        return Ok(());
    }
    // The record as the event carried it; a large one is read as it is now.
    let raw = if raw.is_object() { raw.clone() } else { Value::Object(k.get_tx(sys, &mut tx, &ent.def.name, record, false).await.unwrap_or_default()) };
    let trigger = (s(&doc, "trigger_type") == Some("Record Event")).then(|| crate::util::int(&doc, "trigger_rule").max(1));
    let goal = doc.get("goal_rule").and_then(Value::as_i64);
    let waits: Vec<i64> =
        rows(&doc, "steps").iter().enumerate().filter(|(_, r)| s(r, "step_type") == Some("Wait For Event")).map(|(i, _)| i as i64 + 1).collect();
    for (i, r) in rows(&doc, "event_rules").iter().enumerate() {
        let n = i as i64 + 1;
        if s(r, "entity_name") != Some(ent.def.name.as_str()) || !events_of(s(r, "event").unwrap_or("Created")).contains(&event) {
            continue;
        }
        if let Some(src) = s(r, "condition") {
            let e = check_condition(&sys.schema, ent, src).map_err(Error::invalid)?;
            if !holds(k, sys, &mut tx, ent, &e, &raw, was).await? {
                continue;
            }
        }
        let people = people(k, sys, &mut tx, ent, &raw, s(r, "person_field")).await?;
        if people.is_empty() {
            continue;
        }
        if trigger == Some(n) {
            enroll(&mut tx, tenant, &doc, journey, n, ev.id, &people, &ent.def.name, record).await?;
        }
        // Kept for the steps that wait for it or branch on it, for people in the journey.
        sqlx::query(
            "insert into campaigns_journey_event (tenant_id, journey_id, rule_no, contact_id, entity, record_id, event_id) \
             select $1, $2, $3, e.contact, $5, $6, $7 from e_journey_enrollment e where e.tenant_id = $1 and e.journey = $2 and e.contact = any($4) \
             and e.status = 'Active' on conflict do nothing",
        )
        .bind(tenant)
        .bind(journey)
        .bind(n as i32)
        .bind(&people)
        .bind(&ent.def.name)
        .bind(record)
        .bind(ev.id)
        .execute(&mut *tx)
        .await?;
        if goal == Some(n) {
            sqlx::query(
                "update e_journey_enrollment set status = 'Goal Reached', finished_at = now(), waiting = false, version = version + 1, updated_at = now() \
                 where tenant_id = $1 and journey = $2 and contact = any($3) and status = 'Active'",
            )
            .bind(tenant)
            .bind(journey)
            .bind(&people)
            .execute(&mut *tx)
            .await?;
        }
        // Someone waiting for this event goes on now, not when the wait runs out.
        let waiting_here: Vec<i64> = waits
            .iter()
            .copied()
            .filter(|step| rows(&doc, "steps").get(*step as usize - 1).and_then(|r| r.get("event_rule")).and_then(Value::as_i64) == Some(n))
            .collect();
        if !waiting_here.is_empty() {
            sqlx::query(
                "update e_journey_enrollment set next_run_at = now(), updated_at = now() where tenant_id = $1 and journey = $2 and contact = any($3) \
                 and status = 'Active' and waiting and current_step = any($4)",
            )
            .bind(tenant)
            .bind(journey)
            .bind(&people)
            .bind(&waiting_here)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Enrolls `people` (mailable ones) for rule `n`: once per person, journey and rule (`event:<n>`), or with re-entry once
/// per event (`event:<n>:<event id>`) when their last entry is older than the journey's re-entry hours. One active run
/// per person and journey at a time (a unique index), so two events at once enroll once.
#[allow(clippy::too_many_arguments)]
async fn enroll(tx: &mut Tx, tenant: Uuid, doc: &Doc, journey: Uuid, n: i64, event: i64, people: &[Uuid], entity: &str, record: Uuid) -> Result<()> {
    let reentry = crate::util::b(doc, "allow_reentry");
    let gate = crate::journey::gate(doc);
    let key = if reentry { format!("event:{n}:{event}") } else { format!("event:{n}") };
    let q = format!(
        "insert into e_journey_enrollment (tenant_id, id, journey, contact, email, status, current_step, waiting, next_run_at, entered_at, steps_done, \
         trigger_key, trigger_entity, trigger_record, created_at, updated_at) \
         select $1, gen_random_uuid(), $2, c.id, c.email, 'Active', 1, false, now(), now(), 0, $4, $5, $6, now(), now() \
         from e_marketing_contact c where c.tenant_id = $1 and c.id = any($3) and {gate} \
         and ($7::int is null or not exists (select 1 from e_journey_enrollment p where p.tenant_id = c.tenant_id and p.journey = $2 and p.contact = c.id \
         and p.entered_at > now() - make_interval(hours => $7::int))) on conflict do nothing"
    );
    sqlx::query(sqlx::AssertSqlSafe(q))
        .bind(tenant)
        .bind(journey)
        .bind(people)
        .bind(key)
        .bind(entity)
        .bind(record.to_string())
        .bind(reentry.then(|| crate::util::int(doc, "reentry_hours").clamp(0, 87_600) as i32))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// What is wrong with a journey's event rules and the numbers that point at them, as field errors.
pub fn problems(schema: &Schema, doc: &Doc) -> Vec<ferrum_kernel::FieldError> {
    use ferrum_kernel::FieldError;
    let mut errors = Vec::new();
    let rules = rows(doc, "event_rules");
    for (i, r) in rules.iter().enumerate() {
        let Some(ent) = s(r, "entity_name").and_then(|e| schema.entity(e)) else {
            errors.push(FieldError::new("entity", "not an entity of this workspace").in_row("event_rules", i));
            continue;
        };
        if let Some(m) = check_person_field(ent, s(r, "person_field")) {
            errors.push(FieldError::new("person_field", m).in_row("event_rules", i));
        }
        if let Some(Err(m)) = s(r, "condition").map(|c| check_condition(schema, ent, c)) {
            errors.push(FieldError::new("condition", m).in_row("event_rules", i));
        }
    }
    let known = |n: Option<i64>| n.is_none_or(|n| n >= 1 && n as usize <= rules.len());
    if s(doc, "trigger_type") == Some("Record Event") && !known(Some(crate::util::int(doc, "trigger_rule").max(1))) {
        errors.push(FieldError::new("trigger_rule", "add the event rule that starts the journey"));
    }
    if !known(doc.get("goal_rule").and_then(Value::as_i64)) {
        errors.push(FieldError::new("goal_rule", "there is no such event rule"));
    }
    let trigger_entity = rules.get(crate::util::int(doc, "trigger_rule").max(1) as usize - 1).and_then(|r| s(r, "entity_name")).and_then(|e| schema.entity(e));
    for (i, r) in rows(doc, "steps").iter().enumerate() {
        let wants = s(r, "step_type") == Some("Wait For Event") || s(r, "branch_on") == Some("Event Happened") && s(r, "step_type") == Some("Branch");
        if wants && !(r.get("event_rule").and_then(Value::as_i64).is_some() && known(r.get("event_rule").and_then(Value::as_i64))) {
            errors.push(FieldError::new("event_rule", "choose one of the journey's event rules").in_row("steps", i));
        }
        if s(r, "step_type") == Some("Branch") && s(r, "branch_on") == Some("Record Condition") {
            match (s(r, "condition"), trigger_entity.filter(|_| s(doc, "trigger_type") == Some("Record Event"))) {
                (None, _) => errors.push(FieldError::new("condition", "write the condition").in_row("steps", i)),
                (_, None) => errors.push(FieldError::new("condition", "a record condition needs a journey started by a record event").in_row("steps", i)),
                (Some(c), Some(ent)) => {
                    if let Err(m) = check_condition(schema, ent, c) {
                        errors.push(FieldError::new("condition", m).in_row("steps", i));
                    }
                }
            }
        }
    }
    errors
}
