//! Marketing contacts, consent and the suppression list.
//!
//! A contact's `consent_status` decides whether campaigns and journeys may mail it; the
//! suppression list blocks an address whatever its consent. Every consent change — through the
//! records API or the engine (unsubscribe links, bounces, complaints, confirmations), all of
//! which go through the kernel — runs [`ContactHook::after_save`], which logs it (audience growth)
//! and applies what it implies: opt-outs, bounces and complaints are added to the suppression
//! list, opt-outs and complaints also leave the Mail app's email groups (so newsletters honour
//! them) and set the linked record's own `unsubscribed` flag (CRM contacts and leads).

use crate::util::{self, id, s, sys, values};
use async_trait::async_trait;
use ferrum_kernel::{Doc, DocHook, Error, FieldError, HookCtx, Result, Tx};
use ferrum_meta::FieldType;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use uuid::Uuid;

/// SQL condition on a contact row aliased `c`: campaigns and journeys may mail it.
pub const MAILABLE: &str =
    "(c.consent_status = 'Subscribed' and not exists (select 1 from e_email_suppression x where x.tenant_id = c.tenant_id and x.email = c.email))";

/// Postgres pattern of an address worth importing (the kernel's email rule).
pub const EMAIL_PATTERN: &str = "^[^@[:space:]]{1,64}@[^@[:space:]]+\\.[^@[:space:]]+$";

const MAX_ATTRIBUTES: usize = 100;
const MAX_ATTRIBUTE_BYTES: usize = 16 * 1024;
static KEY: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[a-z][a-z0-9_]{0,39}$").expect("key pattern"));

/// Whether `k` is a valid attribute key.
pub fn valid_key(k: &str) -> bool {
    KEY.is_match(k)
}

/// The attributes of a contact, checked: an object of scalar values under valid keys (nulls drop).
pub fn check_attributes(v: Option<&Value>) -> std::result::Result<Map<String, Value>, String> {
    let m = match v {
        None | Some(Value::Null) => return Ok(Map::new()),
        Some(Value::Object(m)) => m,
        Some(_) => return Err("must be an object of {key: value}".into()),
    };
    if m.len() > MAX_ATTRIBUTES {
        return Err(format!("at most {MAX_ATTRIBUTES} attributes"));
    }
    let mut out = Map::new();
    for (k, v) in m {
        if !valid_key(k) {
            return Err(format!("'{k}' is not a valid key: use lowercase letters, digits and _ (starting with a letter, at most 40)"));
        }
        match v {
            Value::Null => {}
            Value::String(t) if t.chars().count() > 1000 => return Err(format!("'{k}' is longer than 1000 characters")),
            Value::String(_) | Value::Number(_) | Value::Bool(_) => {
                out.insert(k.clone(), v.clone());
            }
            _ => return Err(format!("'{k}' must be text, a number or true/false")),
        }
    }
    if Value::Object(out.clone()).to_string().len() > MAX_ATTRIBUTE_BYTES {
        return Err("attributes are limited to 16 KB".into());
    }
    Ok(out)
}

/// An attribute value as text (how rules and triggers compare it).
pub fn attr_text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

fn full_name(first: Option<&str>, last: Option<&str>) -> Option<String> {
    let n = [first, last].into_iter().flatten().map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
    (!n.is_empty()).then_some(n)
}

/// Consent changes a person may make through the records API (`old` → `new`). Opting back in is
/// the contact's own act (a confirmed signup); bounces and complaints are recorded by the engine.
fn user_may_set(old: Option<&str>, new: &str) -> bool {
    match old {
        None => matches!(new, "Subscribed" | "Pending" | "Unsubscribed"),
        Some(o) if o == new => true,
        Some(_) if new == "Unsubscribed" => true,
        // A mailbox that bounced can be fixed (or the bounce was a provider error).
        Some("Bounced") => new == "Subscribed",
        Some(_) => false,
    }
}

/// The suppression reason a consent status implies.
fn suppression_reason(status: &str, source: Option<&str>) -> Option<&'static str> {
    match status {
        "Unsubscribed" => Some("Unsubscribed"),
        "Complained" => Some("Complaint"),
        "Bounced" if source.is_some_and(|s| s.starts_with("soft bounces")) => Some("Soft Bounces"),
        "Bounced" => Some("Hard Bounce"),
        _ => None,
    }
}

pub struct ContactHook;

#[async_trait]
impl DocHook for ContactHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        for f in ["first_name", "last_name"] {
            if let Some(v) = s(doc, f).map(|v| v.trim().to_string()) {
                doc.insert(f.into(), json!(v));
            }
        }
        doc.insert("full_name".into(), json!(full_name(s(doc, "first_name"), s(doc, "last_name"))));
        if doc.get("attributes").is_none_or(Value::is_null) {
            doc.insert("attributes".into(), json!({}));
        }
        // The stored consent and its evidence; an update's document already carries them.
        let stored: Option<(String, Option<String>, Option<String>)> = match id(doc, "id").filter(|_| doc.contains_key("created_at")) {
            Some(c) => {
                sqlx::query_as("select consent_status, consent_source, consent_ip_hash from e_marketing_contact where tenant_id = $1 and id = $2")
                    .bind(hc.ctx.tenant)
                    .bind(c)
                    .fetch_optional(&mut **hc.tx)
                    .await?
            }
            None => None,
        };
        let (old_status, old_source, old_ip) = match &stored {
            Some((st, src, ip)) => (Some(st.as_str()), src.as_deref(), ip.as_deref()),
            None => (None, None, None),
        };
        if old_status != s(doc, "consent_status") {
            doc.insert("consent_at".into(), json!(util::ts(util::now())));
            // The engine passes its own evidence (what happened, from which address); people are
            // named, and stale evidence of an earlier change never sticks to this one.
            if !hc.ctx.is_system || s(doc, "consent_source").is_none() || s(doc, "consent_source") == old_source {
                let who = if hc.ctx.is_system { "system".to_string() } else { format!("user: {}", hc.ctx.email) };
                doc.insert("consent_source".into(), json!(who));
            }
            if !hc.ctx.is_system || s(doc, "consent_ip_hash") == old_ip {
                doc.insert("consent_ip_hash".into(), Value::Null);
            }
        }
        Ok(())
    }

    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut errors = Vec::new();
        if let Err(m) = check_attributes(doc.get("attributes")) {
            errors.push(FieldError::new("attributes", m));
        }
        match (s(doc, "record_entity"), s(doc, "record_id")) {
            (None, None) => {}
            (Some(e), Some(r)) => match (hc.ctx.schema.entity(e), r.parse::<Uuid>()) {
                (None, _) => errors.push(FieldError::new("record_entity", format!("{e} is not an installed entity"))),
                (Some(_), Err(_)) => errors.push(FieldError::new("record_id", "must be a record id")),
                (Some(_), Ok(rid)) => {
                    let old: Option<(Option<String>, Option<String>)> = match id(doc, "id").filter(|_| doc.contains_key("created_at")) {
                        Some(c) => {
                            sqlx::query_as("select record_entity, record_id from e_marketing_contact where tenant_id = $1 and id = $2")
                                .bind(hc.ctx.tenant)
                                .bind(c)
                                .fetch_optional(&mut **hc.tx)
                                .await?
                        }
                        None => None,
                    };
                    let unchanged = old.is_some_and(|(oe, or)| oe.as_deref() == Some(e) && or.as_deref() == Some(r));
                    if !hc.ctx.is_system && !unchanged && hc.kernel.get_tx(hc.ctx, hc.tx, e, rid, false).await.is_err() {
                        errors.push(FieldError::new("record_id", "the record does not exist or you cannot read it"));
                    }
                }
            },
            _ => errors.push(FieldError::new("record_id", "set both the entity and the record, or neither")),
        }
        if !errors.is_empty() {
            return Err(Error::Validation(errors));
        }
        if !hc.ctx.is_system {
            let old: Option<String> = match id(doc, "id").filter(|_| doc.contains_key("created_at")) {
                Some(c) => {
                    sqlx::query_scalar("select consent_status from e_marketing_contact where tenant_id = $1 and id = $2")
                        .bind(hc.ctx.tenant)
                        .bind(c)
                        .fetch_optional(&mut **hc.tx)
                        .await?
                }
                None => None,
            };
            let new = s(doc, "consent_status").unwrap_or("Subscribed");
            if !user_may_set(old.as_deref(), new) {
                let msg = match old.as_deref() {
                    Some(o) => format!("a contact who is {} can only opt back in themselves, by confirming a subscription form", o.to_lowercase()),
                    None => format!("new contacts start as Subscribed, Pending or Unsubscribed (not {new})"),
                };
                return Err(Error::field("consent_status", msg));
            }
        }
        Ok(())
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        let tenant = hc.ctx.tenant;
        let contact = util::doc_id(doc)?;
        let email = s(doc, "email").unwrap_or_default().to_string();
        let (before, after) = (old.and_then(|o| s(o, "consent_status")), s(doc, "consent_status").unwrap_or("Subscribed"));
        // The first status is logged too: audience growth counts movements in and out of Subscribed.
        if before != Some(after) {
            log_change(hc.tx, tenant, contact, "consent_status", before, Some(after), s(doc, "consent_source")).await?;
        }
        if before != Some(after)
            && let Some(reason) = suppression_reason(after, s(doc, "consent_source"))
        {
            suppress(hc.tx, tenant, &email, reason, s(doc, "consent_source").unwrap_or("consent change")).await?;
            if matches!(after, "Unsubscribed" | "Complained") {
                leave_groups(hc.tx, tenant, &email).await?;
                flag_record(hc, doc).await?;
            }
        }
        // Attribute changes start journeys ("Attribute Changed").
        let empty = Map::new();
        let new_attrs = doc.get("attributes").and_then(Value::as_object).unwrap_or(&empty);
        let old_attrs = old.and_then(|o| o.get("attributes")).and_then(Value::as_object).unwrap_or(&empty);
        let mut keys: Vec<&String> = new_attrs.keys().chain(old_attrs.keys()).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let (o, n) = (attr_text(old_attrs.get(k)), attr_text(new_attrs.get(k)));
            if o != n {
                log_change(hc.tx, tenant, contact, &format!("attr:{k}"), o.as_deref(), n.as_deref(), None).await?;
            }
        }
        Ok(())
    }
}

/// Sets the linked record's own "do not send marketing" flag, when it has one.
async fn flag_record(hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
    let (Some(entity), Some(record)) = (s(doc, "record_entity"), id(doc, "record_id")) else { return Ok(()) };
    let flaggable = hc.ctx.schema.entity(entity).is_some_and(|e| !e.custom && e.field("unsubscribed").is_some_and(|f| f.ftype == FieldType::Check));
    if !flaggable {
        return Ok(());
    }
    match hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, entity, record, values([("unsubscribed", json!(true))])).await {
        Ok(()) | Err(Error::NotFound(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Records a consent or attribute change (`field`: `consent_status` or `attr:<key>`).
pub async fn log_change(tx: &mut Tx, tenant: Uuid, contact: Uuid, field: &str, old: Option<&str>, new: Option<&str>, source: Option<&str>) -> Result<()> {
    sqlx::query("insert into campaigns_contact_change (tenant_id, contact_id, field, old_value, new_value, source) values ($1, $2, $3, $4, $5, $6)")
        .bind(tenant)
        .bind(contact)
        .bind(field)
        .bind(old)
        .bind(new)
        .bind(source)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Adds an address to the suppression list (an existing entry keeps its first reason).
pub async fn suppress(tx: &mut Tx, tenant: Uuid, email: &str, reason: &str, source: &str) -> Result<()> {
    sqlx::query(
        "insert into e_email_suppression (tenant_id, id, email, reason, source, created_at, updated_at) values ($1, $2, $3, $4, $5, now(), now()) \
         on conflict (tenant_id, email) where email is not null do nothing",
    )
    .bind(tenant)
    .bind(Uuid::now_v7())
    .bind(email.trim().to_lowercase())
    .bind(reason)
    .bind(util::cut(source, 200))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Lifts the automatic suppressions of an address after a confirmed opt-in (manual blocks stay).
pub async fn lift(tx: &mut Tx, tenant: Uuid, email: &str) -> Result<()> {
    sqlx::query("delete from e_email_suppression where tenant_id = $1 and email = $2 and reason <> 'Manual'")
        .bind(tenant)
        .bind(email.trim().to_lowercase())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The address leaves every Mail email group (as the Mail app's own unsubscribe does).
async fn leave_groups(tx: &mut Tx, tenant: Uuid, email: &str) -> Result<()> {
    sqlx::query(
        "update e_email_group_member set unsubscribed = true, version = version + 1, updated_at = now() \
         where tenant_id = $1 and lower(email) = $2 and not unsubscribed",
    )
    .bind(tenant)
    .bind(email.trim().to_lowercase())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A contact as templates see it: `{email, first_name, last_name, full_name, attributes}`.
pub fn template_view(doc: &Doc) -> Value {
    json!({
        "id": doc.get("id"),
        "email": doc.get("email"),
        "first_name": doc.get("first_name"),
        "last_name": doc.get("last_name"),
        "full_name": doc.get("full_name"),
        "attributes": doc.get("attributes").filter(|v| v.is_object()).cloned().unwrap_or_else(|| json!({})),
    })
}

/// Contacts by id with what sending needs, and whether each is mailable right now.
pub async fn load_for_sending(tx: &mut Tx, tenant: Uuid, ids: &[Uuid]) -> Result<Vec<(Uuid, Value, bool)>> {
    let rows: Vec<(Uuid, Value, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "select c.id, jsonb_build_object('id', c.id, 'email', c.email, 'first_name', c.first_name, 'last_name', c.last_name, 'full_name', c.full_name, \
         'attributes', coalesce(c.attributes, '{{}}'::jsonb)), {MAILABLE} from e_marketing_contact c where c.tenant_id = $1 and c.id = any($2)"
    )))
    .bind(tenant)
    .bind(ids)
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows)
}

/// Suppression entries: people name themselves as the source.
pub struct SuppressionHook;

#[async_trait]
impl DocHook for SuppressionHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if !doc.contains_key("created_at") && !hc.ctx.is_system {
            doc.insert("source".into(), json!(format!("user: {}", hc.ctx.email)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_are_flat_and_keyed() {
        assert_eq!(check_attributes(None).unwrap(), Map::new());
        let ok = check_attributes(Some(&json!({"city": "Pune", "score": 42, "vip": true, "gone": null}))).unwrap();
        assert_eq!(Value::Object(ok), json!({"city": "Pune", "score": 42, "vip": true}));
        assert!(check_attributes(Some(&json!({"City": "x"}))).is_err());
        assert!(check_attributes(Some(&json!({"a'; drop table x; --": "x"}))).is_err());
        assert!(check_attributes(Some(&json!({"nested": {"a": 1}}))).is_err());
        assert!(check_attributes(Some(&json!(["a"]))).is_err());
        assert!(check_attributes(Some(&json!({"long": "x".repeat(1001)}))).is_err());
        assert_eq!(attr_text(Some(&json!(42))).as_deref(), Some("42"));
        assert_eq!(attr_text(Some(&json!("x"))).as_deref(), Some("x"));
        assert_eq!(attr_text(Some(&Value::Null)), None);
    }

    #[test]
    fn people_cannot_opt_others_back_in() {
        assert!(user_may_set(None, "Subscribed") && user_may_set(None, "Unsubscribed") && !user_may_set(None, "Complained"));
        assert!(user_may_set(Some("Subscribed"), "Unsubscribed") && user_may_set(Some("Complained"), "Unsubscribed"));
        assert!(!user_may_set(Some("Unsubscribed"), "Subscribed") && !user_may_set(Some("Complained"), "Subscribed"));
        assert!(!user_may_set(Some("Pending"), "Subscribed"), "pending contacts confirm themselves");
        assert!(user_may_set(Some("Bounced"), "Subscribed") && user_may_set(Some("Bounced"), "Bounced"));
        assert_eq!(suppression_reason("Bounced", Some("soft bounces: 3 in a row")), Some("Soft Bounces"));
        assert_eq!(suppression_reason("Bounced", Some("bounce webhook")), Some("Hard Bounce"));
        assert_eq!(suppression_reason("Subscribed", None), None);
        assert_eq!(full_name(Some(" Asha "), None).as_deref(), Some("Asha"));
        assert_eq!(full_name(None, Some("")), None);
    }
}
