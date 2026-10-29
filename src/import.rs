//! Audience imports: marketing contacts from a Mail email group, or from customers, CRM contacts
//! or CRM leads (when the CRM app is installed). Each import is one set-based statement that
//! skips addresses already known (case-insensitive) and invalid ones; opt-outs in the source
//! (unsubscribed group members, contacts or leads) arrive as Unsubscribed and suppressed, and an
//! opt-out recorded on a source record also unsubscribes an existing contact. Opt-ins never
//! override an existing contact's consent.

use crate::contact::EMAIL_PATTERN;
use crate::util::{self, id, s, sys, values};
use async_trait::async_trait;
use ferrum_kernel::{Action, Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, Tx, row_scope};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

/// Record sources: entity → (first name, last name, opt-out flag, extra condition) on alias `a`.
fn columns(entity: &str) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    Some(match entity {
        "Customer" => ("a.customer_name", "null::text", "false", " and not a.disabled"),
        "Contact" => ("a.first_name", "a.last_name", "a.unsubscribed", ""),
        "Lead" => ("coalesce(a.first_name, a.lead_name)", "a.last_name", "a.unsubscribed", ""),
        _ => return None,
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub created: i64,
    pub existing: i64,
    pub unsubscribed: i64,
    pub invalid: i64,
}

/// The tail of an import statement: insert the valid addresses not known yet, suppress the
/// opt-outs among them, and count. Expects a `src(email, rid, first_name, last_name, optout)` CTE.
fn push_insert(qb: &mut QueryBuilder<Postgres>, tenant: Uuid, user: Option<Uuid>, label: &str, entity: Option<&str>, consent_source: &str) {
    qb.push(", valid as (select * from src where email ~ ").push_bind(EMAIL_PATTERN).push("), ins as (");
    qb.push(
        "insert into e_marketing_contact (tenant_id, id, email, first_name, last_name, full_name, consent_status, consent_at, consent_source, source, \
         attributes, record_entity, record_id, soft_bounce_count, created_at, updated_at, created_by, updated_by) select ",
    )
    .push_bind(tenant)
    .push(
        ", gen_random_uuid(), v.email, nullif(btrim(v.first_name), ''), nullif(btrim(v.last_name), ''), \
         nullif(btrim(concat_ws(' ', nullif(btrim(v.first_name), ''), nullif(btrim(v.last_name), ''))), ''), \
         case when v.optout then 'Unsubscribed' else 'Subscribed' end, now(), ",
    )
    .push_bind(consent_source.to_string())
    .push(", ")
    .push_bind(label.to_string())
    .push(", '{}'::jsonb, ")
    .push_bind(entity.map(str::to_string))
    .push(", case when ")
    .push_bind(entity.map(str::to_string))
    .push("::text is null then null else v.rid::text end, 0, now(), now(), ")
    .push_bind(user)
    .push(", ")
    .push_bind(user)
    .push(" from valid v on conflict (tenant_id, email) where email is not null do nothing returning id, email, consent_status), sup as (");
    qb.push("insert into e_email_suppression (tenant_id, id, email, reason, source, created_at, updated_at, created_by, updated_by) select ")
        .push_bind(tenant)
        .push(", gen_random_uuid(), i.email, 'Unsubscribed', ")
        .push_bind(consent_source.to_string())
        .push(", now(), now(), ")
        .push_bind(user)
        .push(", ")
        .push_bind(user)
        .push(" from ins i where i.consent_status = 'Unsubscribed' on conflict (tenant_id, email) where email is not null do nothing), log as (")
        .push("insert into campaigns_contact_change (tenant_id, contact_id, field, old_value, new_value, source) select ")
        .push_bind(tenant)
        .push(", i.id, 'consent_status', null, i.consent_status, ")
        .push_bind(consent_source.to_string())
        .push(" from ins i) ")
        .push(
            "select (select count(*) from src), (select count(*) from valid), (select count(*) from ins), \
             (select count(*) from ins where consent_status = 'Unsubscribed')",
        );
}

/// Pushes `(select email, rid, first_name, last_name, optout from <records>) a`: one row per
/// address of the source records `ctx` may read, matching the filters, an opt-out first.
fn push_records(qb: &mut QueryBuilder<Postgres>, ctx: &Ctx, entity: &str, filters: &[ferrum_kernel::Filter]) -> Result<()> {
    let (first, last, optout, extra) = columns(entity).ok_or_else(|| Error::field("source", "unknown source"))?;
    let ent = ctx.schema.entity(entity).cloned().ok_or_else(|| Error::field("source", format!("install the CRM app to import from {entity} records")))?;
    qb.push(format!(
        "(select distinct on (lower(btrim(a.email))) lower(btrim(a.email)) as email, a.id as rid, {first} as first_name, {last} as last_name, \
         {optout} as optout from {} a where a.email is not null{extra} and a.tenant_id = ",
        ferrum_kernel::ddl::q(&ent.table)
    ));
    qb.push_bind(ctx.tenant);
    // The caller's filters, checked against what they may read (a filter on a hidden field, or
    // through a link to records out of their scope, would reveal values).
    for f in filters {
        ferrum_kernel::query::push_filter_for(qb, ctx, &ent, "a", f).map_err(|e| match e {
            Error::Invalid(m) => Error::field("filters", m),
            other => other,
        })?;
    }
    row_scope(ctx, &ent, Action::Read)?.push_sql(qb, "a");
    // An opt-out wins over another record with the same address (customers have no such flag).
    let optout_first = if optout == "false" { String::new() } else { format!("{optout} desc, ") };
    qb.push(format!(" order by lower(btrim(a.email)), {optout_first}a.created_at) a"));
    Ok(())
}

/// Runs an import as `ctx` (who must be able to read the source records).
pub async fn run(k: &Kernel, ctx: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<Outcome> {
    let tenant = ctx.tenant;
    let user = (!ctx.user.is_nil()).then_some(ctx.user);
    let consent_source = util::cut(&format!("import: {}", s(doc, "import_name").unwrap_or("import")), 200);
    let source = s(doc, "source").unwrap_or_default();
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("with src as (select * from ");
    let filters = if source == "Email Group" {
        ctx.require("EmailGroupMember", Action::Read)?;
        let group = id(doc, "email_group").ok_or_else(|| Error::field("email_group", "choose the email group to import"))?;
        qb.push(
            "(select distinct on (lower(btrim(m.email))) lower(btrim(m.email)) as email, m.id as rid, \
             split_part(btrim(coalesce(m.member_name, '')), ' ', 1) as first_name, \
             btrim(substr(btrim(coalesce(m.member_name, '')), length(split_part(btrim(coalesce(m.member_name, '')), ' ', 1)) + 1)) as last_name, \
             m.unsubscribed as optout from e_email_group_member m where m.tenant_id = ",
        );
        qb.push_bind(tenant).push(" and m.email_group = ").push_bind(group).push(" order by lower(btrim(m.email)), m.unsubscribed desc) a)");
        push_insert(&mut qb, tenant, user, "Email Group", None, &consent_source);
        None
    } else {
        ctx.require(source, Action::Read)
            .map_err(|e| if ctx.schema.entity(source).is_none() { Error::field("source", "install the CRM app first") } else { e })?;
        let filters = ferrum_kernel::query::parse_filters(doc.get("filters").unwrap_or(&Value::Null)).map_err(|e| Error::field("filters", e.to_string()))?;
        push_records(&mut qb, ctx, source, &filters)?;
        qb.push(")");
        push_insert(&mut qb, tenant, user, source, Some(source), &consent_source);
        Some(filters)
    };
    let (total, valid, created, created_optouts): (i64, i64, i64, i64) = qb.build_query_as().fetch_one(&mut **tx).await?;
    let mut outcome = Outcome { created, existing: valid - created, unsubscribed: created_optouts, invalid: total - valid };
    let Some(filters) = filters else { return Ok(outcome) };
    // Contacts that already existed: link them to their record ...
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("update e_marketing_contact c set record_entity = ");
    qb.push_bind(source.to_string()).push(", record_id = a.rid::text, version = c.version + 1, updated_at = now() from ");
    push_records(&mut qb, ctx, source, &filters)?;
    qb.push(" where c.tenant_id = ").push_bind(tenant).push(" and c.email = a.email and c.record_entity is null");
    qb.build().execute(&mut **tx).await?;
    // ... and record the opt-outs of their records (through the kernel: suppression, groups, log).
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select c.id from e_marketing_contact c join ");
    push_records(&mut qb, ctx, source, &filters)?;
    qb.push(" on a.email = c.email where a.optout and c.consent_status in ('Subscribed', 'Pending') and c.tenant_id = ").push_bind(tenant);
    let optouts: Vec<Uuid> = qb.build_query_scalar().fetch_all(&mut **tx).await?;
    let sys = sys(ctx);
    for contact in optouts {
        let patch = values([("consent_status", json!("Unsubscribed")), ("consent_source", json!(consent_source))]);
        k.update_tx(&sys, tx, "MarketingContact", contact, patch, None).await?;
        outcome.unsubscribed += 1;
    }
    Ok(outcome)
}

/// Members of these email groups without a marketing contact get one (Subscribed: the Mail
/// app's groups are opt-in mailing lists; unsubscribed members are left alone).
pub async fn adopt_group_members(tx: &mut Tx, tenant: Uuid, groups: &[Uuid]) -> Result<u64> {
    if groups.is_empty() {
        return Ok(0);
    }
    let n = sqlx::query(
        "with ins as (insert into e_marketing_contact (tenant_id, id, email, first_name, last_name, full_name, consent_status, consent_at, consent_source, source, \
         attributes, soft_bounce_count, created_at, updated_at) \
         select $1, gen_random_uuid(), x.email, nullif(split_part(x.name, ' ', 1), ''), nullif(btrim(substr(x.name, length(split_part(x.name, ' ', 1)) + 1)), ''), \
         nullif(x.name, ''), 'Subscribed', now(), 'email group member', 'Email Group', '{}'::jsonb, 0, now(), now() \
         from (select distinct on (lower(btrim(m.email))) lower(btrim(m.email)) as email, btrim(coalesce(m.member_name, '')) as name \
               from e_email_group_member m where m.tenant_id = $1 and m.email_group = any($2) and not m.unsubscribed \
               order by lower(btrim(m.email)), m.created_at) x \
         where x.email ~ $3 on conflict (tenant_id, email) where email is not null do nothing returning id) \
         insert into campaigns_contact_change (tenant_id, contact_id, field, old_value, new_value, source) \
         select $1, ins.id, 'consent_status', null, 'Subscribed', 'email group member' from ins",
    )
    .bind(tenant)
    .bind(groups)
    .bind(EMAIL_PATTERN)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(n)
}

pub struct ImportHook;

#[async_trait]
impl DocHook for ImportHook {
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        match s(doc, "source").unwrap_or_default() {
            "Email Group" => {
                if id(doc, "email_group").is_none() {
                    return Err(Error::field("email_group", "choose the email group to import"));
                }
            }
            other => {
                if hc.ctx.schema.entity(other).is_none() {
                    return Err(Error::field("source", format!("install the CRM app to import from {other} records")));
                }
                let filters =
                    ferrum_kernel::query::parse_filters(doc.get("filters").unwrap_or(&Value::Null)).map_err(|e| Error::field("filters", e.to_string()))?;
                // The same checks the import applies (fields that exist and that the author may read).
                if let Some(ent) = hc.ctx.schema.entity(other).cloned() {
                    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select 1 from x a where true");
                    for f in &filters {
                        ferrum_kernel::query::push_filter_for(&mut qb, hc.ctx, &ent, "a", f).map_err(|e| match e {
                            Error::Invalid(m) => Error::field("filters", m),
                            other => other,
                        })?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "run_import" {
            return Ok(None);
        }
        let o = run(hc.kernel, hc.ctx, hc.tx, doc).await?;
        let v = values([
            ("status", json!("Completed")),
            ("last_run_at", json!(util::ts(util::now()))),
            ("created_count", json!(o.created)),
            ("existing_count", json!(o.existing)),
            ("unsubscribed_count", json!(o.unsubscribed)),
            ("invalid_count", json!(o.invalid)),
        ]);
        hc.kernel.set_values_tx(hc.ctx, hc.tx, "AudienceImport", util::doc_id(doc)?, v.clone()).await?;
        Ok(Some(Value::Object(v)))
    }
}
