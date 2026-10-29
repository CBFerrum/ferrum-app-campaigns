//! Email campaigns. A draft is edited freely; submitting schedules it (content, audience and
//! variants are then fixed by the kernel). When it is due, maintenance materializes the audience
//! into `CampaignRecipient` rows in one statement (deduplicated by contact, only mailable ones,
//! the A/B split decided by a hash of campaign and contact), then sends batches: each batch is
//! one transaction that takes the tenant's budget lock, claims `Pending` recipients with
//! `FOR UPDATE SKIP LOCKED`, re-checks consent and suppression, and inserts one `Communication`
//! per recipient — so an email is handed to the Mail app exactly when its claim commits, and
//! concurrent maintenance runs can never claim (or mail) a recipient twice.

use crate::contact::{self, MAILABLE};
use crate::import;
use crate::segment::{self, Rule, Segment};
use crate::send::{self, Composed, Draft, Links};
use crate::util::{self, b, id, int, rows, s, sys, values};
use async_trait::async_trait;
use ferrum_domain::util::elevated;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result, Tx, with_retry};
use serde_json::{Map, Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Recipients claimed per batch at most.
const BATCH: i64 = 200;
const LABELS: [&str; 5] = ["A", "B", "C", "D", "E"];
/// Largest HTML a campaign may carry (each recipient's copy is stored by the Mail app).
const MAX_CONTENT: usize = 1024 * 1024;

/// A stand-in recipient for previews and test emails.
pub fn sample_contact() -> Value {
    json!({"id": null, "email": "jane.doe@example.com", "first_name": "Jane", "last_name": "Doe", "full_name": "Jane Doe", "attributes": {}})
}

/// The email of a campaign, or of one of its A/B variants.
pub fn draft(doc: &Doc, variant: Option<&str>) -> Draft {
    let mut d = Draft {
        subject: s(doc, "subject").unwrap_or_default().to_string(),
        preheader: s(doc, "preheader").map(str::to_string),
        html: s(doc, "content").unwrap_or_default().to_string(),
        text: s(doc, "text_content").map(str::to_string),
    };
    if let Some(row) = variant.and_then(|v| rows(doc, "variants").into_iter().find(|r| s(r, "variant") == Some(v))) {
        if let Some(subject) = s(&row, "subject") {
            d.subject = subject.to_string();
        }
        if let Some(content) = s(&row, "content") {
            d.html = content.to_string();
            // The campaign's plain text belongs to its own content.
            d.text = None;
        }
    }
    d
}

fn extra(doc: &Doc) -> Map<String, Value> {
    values([("campaign", json!({"name": s(doc, "campaign_name"), "number": s(doc, "number")}))])
}

/// The winning variant: the highest rate of unique opens (or clicks) per email sent; ties go to
/// the higher count, then to the earlier variant. `stats`: label → (sent, opens, clicks).
pub fn pick_winner(labels: &[String], stats: &HashMap<String, (i64, i64, i64)>, by_clicks: bool) -> Option<String> {
    let mut best: Option<(&String, i64, i64)> = None;
    for label in labels {
        let (sent, opens, clicks) = stats.get(label).copied().unwrap_or_default();
        let hits = if by_clicks { clicks } else { opens };
        let sent = sent.max(1);
        let better = match best {
            None => true,
            // hits/sent > best_hits/best_sent, in integers.
            Some((_, bh, bs)) => {
                let (l, r) = (i128::from(hits) * i128::from(bs), i128::from(bh) * i128::from(sent));
                l > r || (l == r && hits > bh)
            }
        };
        if better {
            best = Some((label, hits, sent));
        }
    }
    best.map(|(l, _, _)| l.clone())
}

pub struct CampaignHook;

#[async_trait]
impl DocHook for CampaignHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        // A Mail email template fills what is still blank.
        if let Some(t) = id(doc, "email_template")
            && (s(doc, "subject").is_none() || s(doc, "content").is_none())
        {
            let found: Option<(String, String, bool)> = sqlx::query_as("select subject, body, disabled from e_email_template where tenant_id = $1 and id = $2")
                .bind(hc.ctx.tenant)
                .bind(t)
                .fetch_optional(&mut **hc.tx)
                .await?;
            match found {
                Some((_, _, true)) => return Err(Error::field("email_template", "this email template is disabled")),
                Some((subject, body, false)) => {
                    if s(doc, "subject").is_none() {
                        doc.insert("subject".into(), json!(subject));
                    }
                    if s(doc, "content").is_none() {
                        doc.insert("content".into(), json!(body));
                    }
                }
                None => {}
            }
        }
        let labelled: Vec<Value> = rows(doc, "variants")
            .into_iter()
            .enumerate()
            .map(|(i, mut r)| {
                r.insert("variant".into(), json!(LABELS.get(i).copied().unwrap_or("?")));
                Value::Object(r)
            })
            .collect();
        doc.insert("variants".into(), Value::Array(labelled));
        Ok(())
    }

    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut errors = Vec::new();
        for f in ["subject", "preheader", "content", "text_content"] {
            if let Some(src) = s(doc, f)
                && let Err(m) = crate::render::check(src)
            {
                errors.push(FieldError::new(f, m));
            }
        }
        if s(doc, "content").is_some_and(|c| c.len() > MAX_CONTENT) {
            errors.push(FieldError::new("content", "the content is limited to 1 MB"));
        }
        let audience = rows(doc, "audience");
        for (i, r) in audience.iter().enumerate() {
            if id(r, "segment").is_some() == id(r, "email_group").is_some() {
                errors.push(FieldError::new("segment", "choose a segment or an email group (one per row)").in_row("audience", i));
            }
        }
        if !audience.iter().any(|r| !b(r, "exclude")) {
            errors.push(FieldError::new("audience", "add at least one segment or email group to send to"));
        }
        let variants = rows(doc, "variants");
        if b(doc, "ab_test") {
            if !(2..=LABELS.len()).contains(&variants.len()) {
                errors.push(FieldError::new("variants", "an A/B test needs 2 to 5 variants"));
            }
            for (i, r) in variants.iter().enumerate() {
                for f in ["subject", "content"] {
                    if let Some(src) = s(r, f)
                        && let Err(m) = crate::render::check(src)
                    {
                        errors.push(FieldError::new(f, m).in_row("variants", i));
                    }
                }
                if s(r, "content").is_some_and(|c| c.len() > MAX_CONTENT) {
                    errors.push(FieldError::new("content", "the content is limited to 1 MB").in_row("variants", i));
                }
            }
            match s(doc, "winner_metric").unwrap_or("Opens") {
                "Opens" if !b(doc, "track_opens") => errors.push(FieldError::new("winner_metric", "choosing the winner by opens needs open tracking")),
                "Clicks" if !b(doc, "track_clicks") => errors.push(FieldError::new("winner_metric", "choosing the winner by clicks needs click tracking")),
                _ => {}
            }
        } else if !variants.is_empty() {
            errors.push(FieldError::new("variants", "turn on the A/B test or remove the variants"));
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    async fn before_submit(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        let tenant = hc.ctx.tenant;
        let base = util::site_base(hc.tx, tenant).await?.ok_or_else(send::public_url_missing)?;
        send::check_account(hc.kernel, hc.tx, tenant, id(doc, "email_account")).await?;
        for r in rows(doc, "audience") {
            if let Some(seg) = id(&r, "segment") {
                segment::load(hc.kernel, hc.ctx, hc.tx, seg).await?;
            }
        }
        // Every variant must render for a stand-in contact (runtime errors, fuel).
        let links = Links { base, key: vec![0; 32], track_opens: true, tracked: Some(HashMap::new()), postal_address: None };
        let mut versions = vec![None];
        versions.extend(rows(doc, "variants").iter().filter(|_| b(doc, "ab_test")).map(|r| s(r, "variant").map(str::to_string)));
        for v in versions {
            send::compose(&draft(doc, v.as_deref()), &sample_contact(), &extra(doc), Uuid::nil(), &links)
                .map_err(|e| Error::field("content", format!("{}does not render: {e}", v.map(|v| format!("variant {v} ")).unwrap_or_default())))?;
        }
        Ok(())
    }

    async fn on_submit(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let campaign = util::doc_id(doc)?;
        let mut sources: Vec<String> = vec![s(doc, "content").unwrap_or_default().to_string()];
        sources.extend(rows(doc, "variants").iter().filter_map(|r| s(r, "content").map(str::to_string)));
        send::register_links(hc.tx, hc.ctx.tenant, campaign, &sources.iter().map(String::as_str).collect::<Vec<_>>()).await?;
        let at = s(doc, "schedule_at").map(str::to_string).unwrap_or_else(|| util::ts(util::now()));
        let v = values([("status", json!("Scheduled")), ("schedule_at", json!(at)), ("last_error", Value::Null)]);
        hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "EmailCampaign", campaign, v).await
    }

    async fn before_cancel(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        if s(doc, "status") == Some("Sent") {
            return Err(Error::invalid("this campaign was already sent; there is nothing left to stop"));
        }
        Ok(())
    }

    async fn on_cancel(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let campaign = util::doc_id(doc)?;
        sqlx::query(
            "update e_campaign_recipient set status = 'Cancelled', version = version + 1, updated_at = now() \
             where tenant_id = $1 and campaign = $2 and status in ('Pending', 'Held')",
        )
        .bind(hc.ctx.tenant)
        .bind(campaign)
        .execute(&mut **hc.tx)
        .await?;
        let v = values([("status", json!("Cancelled")), ("finished_at", json!(util::ts(util::now())))]);
        hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "EmailCampaign", campaign, v).await
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        sqlx::query("delete from campaigns_link where tenant_id = $1 and owner_id = $2")
            .bind(hc.ctx.tenant)
            .bind(util::doc_id(doc)?)
            .execute(&mut **hc.tx)
            .await?;
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        match name {
            "preview" => {
                let (contact, variant) = preview_inputs(hc, doc, params).await?;
                let mut tx_base = util::site_base(hc.tx, hc.ctx.tenant).await?;
                let base = match tx_base.take() {
                    Some(b) => b,
                    None => util::site_path(hc.tx, hc.ctx.tenant).await?,
                };
                let links = Links {
                    base,
                    key: vec![0; 32],
                    track_opens: false,
                    tracked: None,
                    postal_address: util::settings(hc.tx, hc.ctx.tenant).await?.postal_address,
                };
                let c = send::compose(&draft(doc, variant.as_deref()), &contact, &extra(doc), Uuid::nil(), &links)?;
                Ok(Some(json!({"subject": c.subject, "html": c.html, "text": c.text, "variant": variant})))
            }
            "send_test" => Ok(Some(send_test(hc, doc, params).await?)),
            _ => Ok(None),
        }
    }
}

/// The contact (by id, or the stand-in) and variant a preview or test email shows.
async fn preview_inputs(hc: &mut HookCtx<'_>, doc: &Doc, params: &Value) -> Result<(Value, Option<String>)> {
    let variant = params.get("variant").and_then(Value::as_str).map(str::to_string);
    if let Some(v) = &variant
        && !rows(doc, "variants").iter().any(|r| s(r, "variant") == Some(v))
    {
        return Err(Error::field("variant", format!("this campaign has no variant {v}")));
    }
    let contact = match params.get("contact").and_then(Value::as_str) {
        Some(c) => {
            let cid: Uuid = c.parse().map_err(|_| Error::field("contact", "must be a contact id"))?;
            let d = hc.kernel.get_tx(hc.ctx, hc.tx, "MarketingContact", cid, false).await?;
            contact::template_view(&d)
        }
        None => sample_contact(),
    };
    Ok((contact, variant))
}

/// Sends the campaign (or a variant) to up to 10 addresses, untracked, subject prefixed "[Test]".
async fn send_test(hc: &mut HookCtx<'_>, doc: &Doc, params: &Value) -> Result<Value> {
    let addresses: Vec<String> = match params.get("emails").or(params.get("to")) {
        Some(Value::String(v)) => v.split([',', ';', '\n']).map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|e| e.trim().to_lowercase()).filter(|e| !e.is_empty()).collect(),
        _ => vec![],
    };
    if addresses.is_empty() || addresses.len() > 10 {
        return Err(Error::field("emails", "send a test to 1 to 10 addresses"));
    }
    if let Some(bad) = addresses.iter().find(|e| !e.contains('@') || e.contains(char::is_whitespace) || e.len() > 254) {
        return Err(Error::field("emails", format!("{bad} is not a valid email address")));
    }
    let (contact, variant) = preview_inputs(hc, doc, params).await?;
    let tenant = hc.ctx.tenant;
    send::check_account(hc.kernel, hc.tx, tenant, id(doc, "email_account")).await?;
    let base = match util::site_base(hc.tx, tenant).await? {
        Some(b) => b,
        None => util::site_path(hc.tx, tenant).await?,
    };
    let links = Links {
        base,
        key: util::signing_key(hc.kernel, tenant).await?,
        track_opens: false,
        tracked: None,
        postal_address: util::settings(hc.tx, tenant).await?.postal_address,
    };
    let mut d = draft(doc, variant.as_deref());
    d.subject = format!("[Test] {}", d.subject);
    let campaign = util::doc_id(doc)?;
    // As the person asking (named as the author, told if it fails), with the engine's rights.
    let author = elevated(hc.ctx);
    let mut sent = Vec::new();
    for to in &addresses {
        let c = send::compose(&d, &contact, &extra(doc), Uuid::now_v7(), &links)?;
        sent.push(send::queue(hc.kernel, &author, hc.tx, id(doc, "email_account"), to, &c, ("EmailCampaign", campaign)).await?);
    }
    Ok(json!({"sent": sent.len(), "communications": sent}))
}

// ───────────────────────────── sending (maintenance) ─────────────────────────────

/// One maintenance round for a tenant's campaigns: start due ones, choose A/B winners, send
/// batches within `budget`, close finished ones and refresh their counters.
pub async fn run(k: &Kernel, sys: &Ctx, budget: Duration) -> Result<usize> {
    let started = Instant::now();
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let due: Vec<Uuid> = sqlx::query_scalar(
        "select id from e_email_campaign where tenant_id = $1 and docstatus = 1 and status = 'Scheduled' and coalesce(schedule_at, now()) <= now() \
         order by schedule_at, id",
    )
    .bind(tenant)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    for c in due {
        with_retry(|| start(k, sys, c)).await?;
    }
    choose_winners(k, sys).await?;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let sending: Vec<Uuid> = sqlx::query_scalar(
        "select c.id from e_email_campaign c where c.tenant_id = $1 and c.docstatus = 1 and c.status = 'Sending' \
         and exists (select 1 from e_campaign_recipient r where r.tenant_id = c.tenant_id and r.campaign = c.id and r.status = 'Pending') \
         order by c.started_at, c.id",
    )
    .bind(tenant)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut queued = 0;
    'campaigns: for c in sending {
        let doc = k.get(sys, "EmailCampaign", c, false).await?;
        let links = match send::links_for(k, tenant, c, b(&doc, "track_opens"), b(&doc, "track_clicks")).await {
            Ok(l) => l,
            Err(e) => {
                record_error(k, tenant, c, &e.to_string()).await?;
                continue;
            }
        };
        loop {
            if started.elapsed() > budget {
                break 'campaigns;
            }
            let n = send_batch(k, sys, c, &links).await?;
            queued += n;
            if n == 0 {
                break;
            }
        }
    }
    finish(k, tenant).await?;
    refresh_stats(k, tenant).await?;
    Ok(queued)
}

async fn record_error(k: &Kernel, tenant: Uuid, campaign: Uuid, message: &str) -> Result<()> {
    let mut tx = k.db.tenant_tx(tenant).await?;
    sqlx::query("update e_email_campaign set last_error = $3, updated_at = now() where tenant_id = $1 and id = $2 and last_error is distinct from $3")
        .bind(tenant)
        .bind(campaign)
        .bind(message)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::warn!(%tenant, %campaign, "campaign cannot send: {message}");
    Ok(())
}

/// Pushes `(p1) or (p2) …` (or `false`).
fn push_any(qb: &mut QueryBuilder<Postgres>, parts: &[Segment], today: chrono::NaiveDate, tz: &str) {
    if parts.is_empty() {
        qb.push("false");
        return;
    }
    for (i, seg) in parts.iter().enumerate() {
        if i > 0 {
            qb.push(" or ");
        }
        seg.push(qb, today, tz);
    }
}

/// The campaign's audience as conditions: (included, excluded, groups to adopt).
async fn audience(k: &Kernel, sys: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<(Vec<Segment>, Vec<Segment>, Vec<Uuid>)> {
    let (mut include, mut exclude, mut groups) = (Vec::new(), Vec::new(), Vec::new());
    for r in rows(doc, "audience") {
        let seg = match (id(&r, "segment"), id(&r, "email_group")) {
            (Some(sid), _) => segment::load(k, sys, tx, sid).await?.1,
            (None, Some(g)) => {
                if !b(&r, "exclude") {
                    groups.push(g);
                }
                Segment { all: true, rules: vec![Rule::Group(g, true)] }
            }
            (None, None) => continue,
        };
        if b(&r, "exclude") { exclude.push(seg) } else { include.push(seg) }
    }
    Ok((include, exclude, groups))
}

/// Scheduled → Sending: the audience becomes recipients (mailable contacts, each once).
async fn start(k: &Kernel, sys: &Ctx, campaign: Uuid) -> Result<()> {
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let doc = k.get_tx(sys, &mut tx, "EmailCampaign", campaign, true).await?;
    if s(&doc, "status") != Some("Scheduled") || int(&doc, "docstatus") != 1 {
        tx.rollback().await?;
        return Ok(());
    }
    let (include, exclude, groups) = match audience(k, sys, &mut tx, &doc).await {
        Ok(a) => a,
        Err(e) => {
            tx.rollback().await?;
            return record_error(k, tenant, campaign, &e.to_string()).await;
        }
    };
    import::adopt_group_members(&mut tx, tenant, &groups).await?;
    let (today, tz) = (sys.today(), sys.schema.timezone.clone());
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("with audience as (select c.id, c.email from e_marketing_contact c where c.tenant_id = ");
    qb.push_bind(tenant).push(format!(" and {MAILABLE} and ("));
    push_any(&mut qb, &include, today, &tz);
    qb.push(") and not (");
    push_any(&mut qb, &exclude, today, &tz);
    // Ranked by a hash of campaign and contact: the A/B split is the same however often it runs.
    qb.push(")), ranked as (select a.id, a.email, row_number() over (order by sha256(convert_to(")
        .push_bind(campaign.to_string())
        .push(" || a.id::text, 'UTF8')), a.id) as rn, count(*) over () as n from audience a) ")
        .push(
            "insert into e_campaign_recipient (tenant_id, id, campaign, contact, email, variant, status, open_count, machine_open_count, click_count, \
             created_at, updated_at) select ",
        )
        .push_bind(tenant)
        .push(", gen_random_uuid(), ")
        .push_bind(campaign)
        .push(", r.id, r.email, ");
    let labels: Vec<String> = rows(&doc, "variants").iter().filter_map(|r| s(r, "variant").map(str::to_string)).collect();
    if b(&doc, "ab_test") && labels.len() >= 2 {
        let push_test = |qb: &mut QueryBuilder<Postgres>| {
            qb.push("r.rn <= greatest((r.n * ")
                .push_bind(int(&doc, "test_percent").clamp(1, 90))
                .push(" + 99) / 100, least(r.n, ")
                .push_bind(labels.len() as i64)
                .push("))");
        };
        qb.push("case when ");
        push_test(&mut qb);
        qb.push(" then (").push_bind(labels.clone()).push("::text[])[(((r.rn - 1) % ").push_bind(labels.len() as i64).push(") + 1)::int] end, case when ");
        push_test(&mut qb);
        qb.push(" then 'Pending' else 'Held' end");
    } else {
        qb.push("null, 'Pending'");
    }
    qb.push(", 0, 0, 0, now(), now() from ranked r on conflict (tenant_id, campaign, contact) do nothing");
    qb.build().execute(&mut *tx).await?;
    let total: i64 = sqlx::query_scalar("select count(*) from e_campaign_recipient where tenant_id = $1 and campaign = $2")
        .bind(tenant)
        .bind(campaign)
        .fetch_one(&mut *tx)
        .await?;
    let v =
        values([("status", json!("Sending")), ("started_at", json!(util::ts(util::now()))), ("total_recipients", json!(total)), ("last_error", Value::Null)]);
    k.set_values_tx(sys, &mut tx, "EmailCampaign", campaign, v).await?;
    tx.commit().await?;
    tracing::info!(%tenant, %campaign, total, "campaign started");
    Ok(())
}

/// One batch of a sending campaign; returns how many emails were handed to the Mail app.
pub async fn send_batch(k: &Kernel, sys: &Ctx, campaign: Uuid, links: &Links) -> Result<usize> {
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    send::lock_budget(&mut tx, tenant).await?;
    // Locks the campaign too: a cancel waits for this batch, and this batch sees a cancel.
    let doc = k.get_tx(sys, &mut tx, "EmailCampaign", campaign, true).await?;
    if s(&doc, "status") != Some("Sending") || int(&doc, "docstatus") != 1 {
        tx.rollback().await?;
        return Ok(0);
    }
    let mut room = send::hourly_room(&mut tx, tenant).await?.min(BATCH);
    let own_limit = int(&doc, "hourly_limit");
    if own_limit > 0 {
        let used: i64 = sqlx::query_scalar(
            "select count(*) from e_campaign_recipient where tenant_id = $1 and campaign = $2 and communication is not null and queued_at > now() - interval '1 hour'",
        )
        .bind(tenant)
        .bind(campaign)
        .fetch_one(&mut *tx)
        .await?;
        room = room.min(own_limit - used);
    }
    if room <= 0 {
        tx.rollback().await?;
        return Ok(0);
    }
    // A materialized CTE runs once: an `IN (… LIMIT n FOR UPDATE SKIP LOCKED)` subquery may be
    // rescanned by the join, skip the rows this statement just updated and claim more than `n`.
    let claimed: Vec<(Uuid, Uuid, String, Option<String>)> = sqlx::query_as(
        "with due as materialized (select id from e_campaign_recipient where tenant_id = $1 and campaign = $2 and status = 'Pending' \
         order by id limit $3 for update skip locked) \
         update e_campaign_recipient r set status = 'Queued', queued_at = now(), version = r.version + 1, updated_at = now() \
         from due where r.tenant_id = $1 and r.id = due.id returning r.id, r.contact, r.email, r.variant",
    )
    .bind(tenant)
    .bind(campaign)
    .bind(room)
    .fetch_all(&mut *tx)
    .await?;
    if claimed.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }
    let ids: Vec<Uuid> = claimed.iter().map(|c| c.1).collect();
    let contacts: HashMap<Uuid, (Value, bool)> = contact::load_for_sending(&mut tx, tenant, &ids).await?.into_iter().map(|(i, v, m)| (i, (v, m))).collect();
    let account = id(&doc, "email_account");
    let extra = extra(&doc);
    let mut drafts: HashMap<Option<String>, Draft> = HashMap::new();
    let mut queued = 0;
    for (recipient, contact_id, email, variant) in claimed {
        let d = drafts.entry(variant.clone()).or_insert_with(|| draft(&doc, variant.as_deref())).clone();
        let outcome = match contacts.get(&contact_id) {
            Some((view, true)) => hand_over(k, sys, &mut tx, account, &email, &d, view, &extra, recipient, links, contact_id).await,
            Some((_, false)) => Ok(Handover::Skipped("unsubscribed or suppressed since the campaign started")),
            None => Ok(Handover::Skipped("the contact was deleted")),
        };
        match outcome {
            Ok(Handover::Queued(comm)) => {
                sqlx::query("update e_campaign_recipient set communication = $3 where tenant_id = $1 and id = $2")
                    .bind(tenant)
                    .bind(recipient)
                    .bind(comm)
                    .execute(&mut *tx)
                    .await?;
                queued += 1;
            }
            Ok(Handover::Skipped(why)) => skip(&mut tx, tenant, recipient, "Skipped", why).await?,
            Err(e) => skip(&mut tx, tenant, recipient, "Failed", &e.to_string()).await?,
        }
    }
    tx.commit().await?;
    Ok(queued)
}

pub enum Handover {
    Queued(Uuid),
    Skipped(&'static str),
}

/// Composes and queues one recipient's email inside a savepoint: a failure (a template error
/// for this contact, an address the Mail app rejects) undoes only this recipient's work.
pub async fn hand_over(
    k: &Kernel,
    sys: &Ctx,
    tx: &mut Tx,
    account: Option<Uuid>,
    email: &str,
    d: &Draft,
    contact: &Value,
    extra: &Map<String, Value>,
    recipient: Uuid,
    links: &Links,
    contact_id: Uuid,
) -> Result<Handover> {
    let composed: Composed = send::compose(d, contact, extra, recipient, links)?;
    sqlx::query("savepoint campaigns_recipient").execute(&mut **tx).await?;
    match send::queue(k, sys, tx, account, email, &composed, ("MarketingContact", contact_id)).await {
        Ok(comm) => {
            sqlx::query("release savepoint campaigns_recipient").execute(&mut **tx).await?;
            Ok(Handover::Queued(comm))
        }
        Err(e) => {
            sqlx::query("rollback to savepoint campaigns_recipient").execute(&mut **tx).await?;
            Err(e)
        }
    }
}

async fn skip(tx: &mut Tx, tenant: Uuid, recipient: Uuid, status: &str, why: &str) -> Result<()> {
    sqlx::query(
        "update e_campaign_recipient set status = $3, skip_reason = case when $3 = 'Skipped' then $4 end, error = case when $3 = 'Failed' then $4 end, \
         queued_at = null where tenant_id = $1 and id = $2",
    )
    .bind(tenant)
    .bind(recipient)
    .bind(status)
    .bind(util::cut(why, 500))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A/B tests: once every test email is out, wait `winner_after_hours`, then give the held
/// recipients the variant with the best open (or click) rate.
async fn choose_winners(k: &Kernel, sys: &Ctx) -> Result<()> {
    let tenant = sys.tenant;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let open: Vec<Uuid> = sqlx::query_scalar(
        "select id from e_email_campaign where tenant_id = $1 and docstatus = 1 and status = 'Sending' and ab_test and winner_variant is null",
    )
    .bind(tenant)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    for c in open {
        let mut tx = k.db.tenant_tx(tenant).await?;
        let doc = k.get_tx(sys, &mut tx, "EmailCampaign", c, true).await?;
        let testing: i64 = sqlx::query_scalar(
            "select count(*) from e_campaign_recipient where tenant_id = $1 and campaign = $2 and variant is not null and status in ('Pending', 'Queued')",
        )
        .bind(tenant)
        .bind(c)
        .fetch_one(&mut *tx)
        .await?;
        if testing > 0 {
            tx.rollback().await?;
            continue;
        }
        let Some(sent_at) = util::time(&doc, "test_sent_at") else {
            k.set_values_tx(sys, &mut tx, "EmailCampaign", c, values([("test_sent_at", json!(util::ts(util::now())))])).await?;
            tx.commit().await?;
            continue;
        };
        if util::now() < sent_at + chrono::Duration::hours(int(&doc, "winner_after_hours").max(1)) {
            tx.rollback().await?;
            continue;
        }
        let stats: Vec<(String, i64, i64, i64)> = sqlx::query_as(
            "select variant, count(*) filter (where status = 'Sent' and bounced_at is null), count(*) filter (where opened_at is not null), \
             count(*) filter (where clicked_at is not null) from e_campaign_recipient where tenant_id = $1 and campaign = $2 and variant is not null \
             and status <> 'Held' group by variant",
        )
        .bind(tenant)
        .bind(c)
        .fetch_all(&mut *tx)
        .await?;
        let stats: HashMap<String, (i64, i64, i64)> = stats.into_iter().map(|(v, a, b, c)| (v, (a, b, c))).collect();
        let labels: Vec<String> = rows(&doc, "variants").iter().filter_map(|r| s(r, "variant").map(str::to_string)).collect();
        let Some(winner) = pick_winner(&labels, &stats, s(&doc, "winner_metric") == Some("Clicks")) else {
            tx.rollback().await?;
            continue;
        };
        let released = sqlx::query(
            "update e_campaign_recipient set variant = $3, status = 'Pending', version = version + 1, updated_at = now() \
             where tenant_id = $1 and campaign = $2 and status = 'Held'",
        )
        .bind(tenant)
        .bind(c)
        .bind(&winner)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        k.set_values_tx(sys, &mut tx, "EmailCampaign", c, values([("winner_variant", json!(winner)), ("winner_chosen_at", json!(util::ts(util::now())))]))
            .await?;
        tx.commit().await?;
        tracing::info!(%tenant, campaign = %c, %winner, released, "A/B test decided");
    }
    Ok(())
}

/// Sending → Sent once nobody waits (and an A/B test has its winner).
async fn finish(k: &Kernel, tenant: Uuid) -> Result<()> {
    let mut tx = k.db.tenant_tx(tenant).await?;
    sqlx::query(
        "update e_email_campaign c set status = 'Sent', finished_at = now(), version = c.version + 1, updated_at = now() \
         where c.tenant_id = $1 and c.docstatus = 1 and c.status = 'Sending' and (not c.ab_test or c.winner_variant is not null) \
         and not exists (select 1 from e_campaign_recipient r where r.tenant_id = c.tenant_id and r.campaign = c.id and r.status in ('Pending', 'Held'))",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The counters on campaigns that are sending or finished in the last 30 days (opens and clicks
/// keep coming), written only when they changed.
async fn refresh_stats(k: &Kernel, tenant: Uuid) -> Result<()> {
    let mut tx = k.db.tenant_tx(tenant).await?;
    sqlx::query(
        "update e_email_campaign c set total_recipients = x.total, sent_count = x.sent, failed_count = x.failed, skipped_count = x.skipped, \
         unique_opens = x.uo, total_opens = x.tot_o, unique_clicks = x.uc, total_clicks = x.tot_c, unsubscribe_count = x.unsub, \
         bounce_count = x.bounced, complaint_count = x.complained, updated_at = now() \
         from (select r.campaign, count(*) as total, count(*) filter (where r.status = 'Sent') as sent, count(*) filter (where r.status = 'Failed') as failed, \
               count(*) filter (where r.status = 'Skipped') as skipped, count(*) filter (where r.opened_at is not null) as uo, \
               coalesce(sum(r.open_count), 0) as tot_o, count(*) filter (where r.clicked_at is not null) as uc, coalesce(sum(r.click_count), 0) as tot_c, \
               count(*) filter (where r.unsubscribed_at is not null) as unsub, count(*) filter (where r.bounced_at is not null) as bounced, \
               count(*) filter (where r.complained_at is not null) as complained \
               from e_campaign_recipient r where r.tenant_id = $1 and r.campaign in (select id from e_email_campaign where tenant_id = $1 and docstatus > 0 \
               and (status = 'Sending' or finished_at > now() - interval '30 days')) group by r.campaign) x \
         where c.tenant_id = $1 and c.id = x.campaign and (c.total_recipients, c.sent_count, c.failed_count, c.skipped_count, c.unique_opens, c.total_opens, \
         c.unique_clicks, c.total_clicks, c.unsubscribe_count, c.bounce_count, c.complaint_count) is distinct from \
         (x.total, x.sent, x.failed, x.skipped, x.uo, x.tot_o, x.uc, x.tot_c, x.unsub, x.bounced, x.complained)",
    )
    .bind(tenant)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test share `start` computes in SQL: `pct`% of the audience rounded up, at least one per variant.
    fn test_size(n: i64, pct: i64, variants: i64) -> i64 {
        ((n * pct + 99) / 100).max(n.min(variants))
    }

    #[test]
    fn test_share_and_winner() {
        assert_eq!(test_size(20, 50, 2), 10);
        assert_eq!(test_size(7, 20, 2), 2, "rounded up");
        assert_eq!(test_size(3, 10, 2), 2, "at least one per variant");
        assert_eq!(test_size(1, 10, 3), 1);
        assert_eq!(test_size(0, 50, 2), 0);
        let labels: Vec<String> = ["A", "B", "C"].iter().map(|l| l.to_string()).collect();
        let stats = HashMap::from([("A".to_string(), (10, 3, 1)), ("B".to_string(), (9, 3, 2)), ("C".to_string(), (10, 2, 2))]);
        assert_eq!(pick_winner(&labels, &stats, false).as_deref(), Some("B"), "3/9 beats 3/10");
        assert_eq!(pick_winner(&labels, &stats, true).as_deref(), Some("B"), "2/9 beats 2/10");
        let tie = HashMap::from([("A".to_string(), (10, 2, 0)), ("B".to_string(), (20, 4, 0))]);
        assert_eq!(pick_winner(&labels[..2], &tie, false).as_deref(), Some("B"), "same rate: more opens");
        assert_eq!(pick_winner(&labels[..2], &HashMap::new(), false).as_deref(), Some("A"), "nothing measured: the first");
        assert_eq!(pick_winner(&[], &HashMap::new(), false), None);
    }

    #[test]
    fn variants_override_subject_and_content() {
        let doc: Doc = json!({"subject": "Base", "content": "<p>base</p>", "text_content": "base", "preheader": "pre",
            "variants": [{"variant": "A", "subject": "Hello A"}, {"variant": "B", "subject": "Hello B", "content": "<p>b</p>"}]})
        .as_object()
        .cloned()
        .unwrap();
        let a = draft(&doc, Some("A"));
        assert_eq!((a.subject.as_str(), a.html.as_str(), a.text.as_deref()), ("Hello A", "<p>base</p>", Some("base")));
        let bb = draft(&doc, Some("B"));
        assert_eq!((bb.subject.as_str(), bb.html.as_str(), bb.text.as_deref(), bb.preheader.as_deref()), ("Hello B", "<p>b</p>", None, Some("pre")));
        assert_eq!(draft(&doc, None).subject, "Base");
    }
}
