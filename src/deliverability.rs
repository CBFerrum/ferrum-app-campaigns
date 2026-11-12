//! Deliverability per sending domain: what its DNS says (asked of the Mail app's domain check, which knows the records
//! its mail server signs with), whether the visible sender is the domain mail is signed for, a warm-up that limits a new
//! domain's daily volume, a seed-list send, and the refusal to send a promotional campaign from a domain whose check
//! failed unless an administrator overrides it with a reason. Nothing names a mailbox provider.

use crate::util::{self, id, s, values};
use async_trait::async_trait;
use chrono::NaiveDate;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Result, SiteRequest, Tx};
use serde_json::{Map, Value, json};
use uuid::Uuid;

/// The largest daily limit a warm-up works out (beyond it the domain is as good as unlimited).
const WARMUP_CEILING: i64 = 10_000_000;

/// The daily limit of a warm-up on `day`: the first day's volume grown by `growth`% a day, none after `days` days.
pub fn warmup_limit(start: NaiveDate, first: i64, growth: i64, days: i64, day: NaiveDate) -> Option<i64> {
    let n = (day - start).num_days();
    if n < 0 {
        return Some(0);
    }
    if n >= days {
        return None;
    }
    let mut limit = first.max(1);
    for _ in 0..n {
        limit = (limit * (100 + growth.max(0)) / 100).min(WARMUP_CEILING);
    }
    Some(limit)
}

/// The domain of the account a campaign sends from (its own, else the workspace's default outgoing account).
pub async fn domain_of(tx: &mut Tx, tenant: Uuid, account: Option<Uuid>) -> Result<Option<String>> {
    let address: Option<String> = match account {
        Some(a) => sqlx::query_scalar("select email_address from e_email_account where tenant_id = $1 and id = $2").bind(tenant).bind(a).fetch_optional(&mut **tx).await?,
        None => sqlx::query_scalar(
            "select email_address from e_email_account where tenant_id = $1 and enable_outgoing and default_outgoing and company is null order by created_at limit 1",
        )
        .bind(tenant)
        .fetch_optional(&mut **tx)
        .await?,
    };
    Ok(address.and_then(|a| a.rsplit_once('@').map(|(_, d)| d.trim().to_lowercase())).filter(|d| !d.is_empty()))
}

/// How many more emails a campaign from `account` may hand over today under its domain's warm-up (`None`: no limit).
/// Counted over every campaign sending from that domain today, in the workspace's day.
pub async fn warmup_room(tx: &mut Tx, ctx: &Ctx, account: Option<Uuid>) -> Result<Option<i64>> {
    let Some(domain) = domain_of(tx, ctx.tenant, account).await? else { return Ok(None) };
    let row: Option<(Option<NaiveDate>, Option<i64>, Option<i64>, Option<i64>)> =
        sqlx::query_as("select warmup_start, warmup_first_day, warmup_growth_percent, warmup_days from e_sending_domain where tenant_id = $1 and domain = $2")
            .bind(ctx.tenant)
            .bind(&domain)
            .fetch_optional(&mut **tx)
            .await?;
    let Some((Some(start), first, growth, days)) = row else { return Ok(None) };
    let today = ctx.today();
    let Some(limit) = warmup_limit(start, first.unwrap_or(50), growth.unwrap_or(50), days.unwrap_or(30), today) else { return Ok(None) };
    let used: i64 = sqlx::query_scalar(
        "select count(*) from e_campaign_recipient r join e_email_campaign c on c.tenant_id = r.tenant_id and c.id = r.campaign \
         left join e_email_account a on a.tenant_id = c.tenant_id and a.id = c.email_account \
         where r.tenant_id = $1 and r.communication is not null and (r.queued_at at time zone $3)::date = $4 \
         and lower(split_part(coalesce(a.email_address, (select d.email_address from e_email_account d where d.tenant_id = r.tenant_id and d.enable_outgoing \
             and d.default_outgoing and d.company is null order by d.created_at limit 1)), '@', 2)) = $2",
    )
    .bind(ctx.tenant)
    .bind(&domain)
    .bind(&ctx.schema.timezone)
    .bind(today)
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some((limit - used).max(0)))
}

/// Refuses a promotional email campaign from a domain whose check failed, unless an administrator overrode it with a
/// reason. A domain never checked (or without a record) is not refused: existing workspaces keep sending.
pub async fn gate(tx: &mut Tx, tenant: Uuid, doc: &Doc) -> Result<()> {
    let Some(domain) = domain_of(tx, tenant, id(doc, "email_account")).await? else { return Ok(()) };
    let status: Option<Option<String>> = sqlx::query_scalar("select check_status from e_sending_domain where tenant_id = $1 and domain = $2")
        .bind(tenant)
        .bind(&domain)
        .fetch_optional(&mut **tx)
        .await?;
    if status.flatten().as_deref() != Some("Failing") || util::b(doc, "deliverability_override") && s(doc, "override_reason").is_some() {
        return Ok(());
    }
    Err(Error::field(
        "email_account",
        format!("{domain} failed its DNS check (Sending Domain): fix its records and check again, or an administrator sends anyway with a reason"),
    ))
}

/// Only an administrator may set or change the override.
pub fn override_allowed(ctx: &Ctx, old: Option<&Doc>, doc: &Doc) -> Result<()> {
    let set = |d: Option<&Doc>, f: &str| d.and_then(|d| d.get(f)).filter(|v| !v.is_null() && **v != json!(false) && **v != json!("")).cloned();
    let changed = |f: &str| set(old, f) != set(Some(doc), f);
    if !ctx.is_system && !ctx.is_admin && (changed("deliverability_override") || changed("override_reason")) {
        return Err(Error::field("deliverability_override", "only an administrator may send despite failing domain checks"));
    }
    Ok(())
}

pub struct DomainHook;

#[async_trait]
impl DocHook for DomainHook {
    async fn before_validate(&self, _hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if let Some(d) = s(doc, "domain").map(|d| d.trim().trim_start_matches('@').to_lowercase()) {
            doc.insert("domain".into(), json!(d));
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "check" {
            return Ok(None);
        }
        let domain = s(doc, "domain").unwrap_or_default().to_string();
        // The Mail app's own check of its domain, asked as the person pressing (it needs the right to Mail's settings).
        let site = hc.kernel.hooks.site("comms").ok_or_else(|| Error::invalid("the Mail app does not check domains here"))?;
        let req = SiteRequest {
            tenant: hc.ctx.tenant,
            user: Some(hc.ctx.clone()),
            method: "GET".into(),
            path: "/domain".into(),
            query: Map::new(),
            headers: Map::new(),
            body: vec![],
            client_ip: String::new(),
        };
        let answer = site.handle(hc.kernel, req).await?;
        let report: Value = serde_json::from_slice(&answer.body).unwrap_or(Value::Null);
        let v = judge(&domain, &report);
        hc.kernel.set_values_tx(&util::sys(hc.ctx), hc.tx, "SendingDomain", util::doc_id(doc)?, v.clone()).await?;
        Ok(Some(Value::Object(v)))
    }
}

/// What the Mail app's report says of `domain`: each record's status, the DMARC policy, alignment, and the verdict.
pub fn judge(domain: &str, report: &Value) -> Doc {
    let signed = report.get("domain").and_then(Value::as_str).map(str::to_lowercase);
    let now = json!(util::ts(util::now()));
    // Aligned (relaxed): the visible sender's domain is the signing domain or below it.
    let aligned = signed.as_deref().is_some_and(|d| domain == d || domain.ends_with(&format!(".{d}")));
    if !aligned {
        let why = match signed {
            Some(d) => format!("Mail signs for {d}, not {domain}: mail from {domain} is not aligned with its signature, and its records are not checked here."),
            None => "The Mail app has no domain of its own to sign for here, so nothing could be checked.".to_string(),
        };
        return values([
            ("check_status", json!(if report.get("domain").is_some_and(Value::is_string) { "Failing" } else { "Not Checked" })),
            ("aligned", json!(false)),
            ("checked_at", now),
            ("check_notes", json!(why)),
        ]);
    }
    let check = |id: &str| report["checks"].as_array().and_then(|c| c.iter().find(|x| x["id"] == json!(id))).cloned().unwrap_or(Value::Null);
    let status = |id: &str| check(id)["status"].as_str().unwrap_or("missing").to_string();
    let policy = check("dmarc")["found"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|t| t.split(';').map(str::trim).find_map(|p| p.strip_prefix("p=").map(str::to_lowercase)));
    let (spf, dkim) = (status("spf"), status("dkim"));
    let verdict = if [&spf, &dkim].iter().any(|s| s.as_str() == "unknown") {
        "Not Checked"
    } else if spf == "ok" && dkim == "ok" && policy.is_some() {
        "Passing"
    } else {
        "Failing"
    };
    values([
        ("check_status", json!(verdict)),
        ("spf", json!(spf)),
        ("dkim", json!(dkim)),
        ("dmarc_policy", json!(policy)),
        ("aligned", json!(true)),
        ("checked_at", now),
        ("check_notes", Value::Null),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_domain_grows_its_daily_volume_then_has_none() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let day = |n| start + chrono::Duration::days(n);
        assert_eq!(warmup_limit(start, 50, 50, 30, day(0)), Some(50));
        assert_eq!(warmup_limit(start, 50, 50, 30, day(1)), Some(75));
        assert_eq!(warmup_limit(start, 50, 50, 30, day(2)), Some(112));
        assert_eq!(warmup_limit(start, 50, 50, 30, day(30)), None, "warmed up");
        assert_eq!(warmup_limit(start, 50, 50, 30, day(-1)), Some(0), "not started");
        assert_eq!(warmup_limit(start, 1_000_000, 400, 365, day(300)), Some(WARMUP_CEILING));
    }

    #[test]
    fn the_mail_report_is_judged_for_the_domain() {
        let report = json!({"domain": "acme.test", "checks": [{"id": "spf", "status": "ok"}, {"id": "dkim", "status": "ok"},
            {"id": "dmarc", "status": "ok", "found": ["v=DMARC1; p=quarantine; rua=mailto:x@acme.test"]}]});
        let v = judge("news.acme.test", &report);
        assert_eq!((v["check_status"].clone(), v["dmarc_policy"].clone(), v["aligned"].clone()), (json!("Passing"), json!("quarantine"), json!(true)));
        let mut no_policy = report.clone();
        no_policy["checks"][2]["found"] = json!([]);
        assert_eq!(judge("acme.test", &no_policy)["check_status"], json!("Failing"), "a published policy is required");
        assert_eq!(judge("other.test", &report)["check_status"], json!("Failing"), "not aligned with the signature");
        assert_eq!(judge("acme.test", &json!({"domain": null}))["check_status"], json!("Not Checked"));
    }
}
