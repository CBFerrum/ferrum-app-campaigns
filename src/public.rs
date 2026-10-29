//! The public endpoints, `/api/site/{workspace}/campaigns/...` — each authenticated by an HMAC
//! (keyed per workspace, checked in constant time) and acting with the system context:
//!
//! * `GET /o/{recipient}.{tag}.gif` — the open pixel: always the same image, counted only when
//!   the tag is valid and the link has not expired; automatic opens are counted apart.
//! * `GET /c/{recipient}.{link}.{tag}` — a click: redirects to link number `link` of that email's
//!   campaign or journey, so it can only ever lead to a URL written in that content.
//! * `GET|POST /u/{recipient}.{tag}` — unsubscribe: GET asks (link scanners only GET), POST — the
//!   button or a mail client's one-click request — unsubscribes.
//! * `POST /subscribe/{form}` — double opt-in signup (JSON or a browser form, with a honeypot);
//!   `GET|POST /confirm/{token}` confirms it.
//! * `POST /webhooks/bounces` — bounces and complaints from the mail provider, signed with the
//!   webhook secret.
//!
//! Unknown, tampered and expired links all get the same answer.

use crate::contact;
use crate::delivery::{self, Kind};
use crate::send;
use crate::util::{self, html_escape, id, page, s, simple, sys, values};
use async_trait::async_trait;
use chrono::Utc;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result, Site, SiteRequest, SiteResponse, Tx};
use hmac::{Hmac, Mac};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use uuid::Uuid;

/// 1×1 transparent GIF.
const GIF: [u8; 43] = [
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x21, 0xf9, 0x04, 0x01, 0x00, 0x00, 0x00,
    0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3b,
];
/// Webhook signatures older (or newer) than this are refused: a captured request cannot be replayed later.
const WEBHOOK_TOLERANCE_SECS: i64 = 300;
const MAX_WEBHOOK_EVENTS: usize = 1000;
/// Confirmation emails per address at most once in this many minutes.
const CONFIRMATION_PAUSE_MINUTES: i64 = 10;
static EMAIL: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^[^@\s]{1,64}@[^@\s]+\.[^@\s]+$").expect("email pattern"));

/// User agents of tools, scanners and link checkers (case-insensitive substrings). Mail
/// providers' image proxies (Gmail, Yahoo) fetch when the person opens, so they are not here.
const TOOLS: &[&str] = &[
    "bot",
    "crawler",
    "spider",
    "preview",
    "scanner",
    "curl",
    "wget",
    "python",
    "java/",
    "go-http-client",
    "okhttp",
    "libwww",
    "httpclient",
    "headless",
    "phantomjs",
    "barracuda",
    "mimecast",
    "proofpoint",
    "symantec",
    "forcepoint",
    "trendmicro",
    "sophos",
    "fortinet",
    "cisco",
    "existence discovery",
];

/// Whether an open looks automatic: no user agent or a tool's, the bare `Mozilla/5.0` of Apple
/// Mail Privacy Protection prefetching every image, or an image fetched within two seconds of
/// sending (security gateways scan mail as it arrives).
pub fn machine_open(user_agent: Option<&str>, seconds_after_send: Option<i64>) -> bool {
    let ua = user_agent.map(str::trim).unwrap_or_default();
    if ua.is_empty() || ua == "Mozilla/5.0" {
        return true;
    }
    let lower = ua.to_ascii_lowercase();
    TOOLS.iter().any(|t| lower.contains(t)) || seconds_after_send.is_some_and(|s| s <= 2)
}

pub struct CampaignsSite;

#[async_trait]
impl Site for CampaignsSite {
    async fn handle(&self, k: &Kernel, req: SiteRequest) -> Result<SiteResponse> {
        // Everything signed-in users do goes through the records API; these are public only.
        if req.user.is_some() {
            return Err(Error::NotFound("endpoint".into()));
        }
        let path: Vec<&str> = req.path.split('/').filter(|p| !p.is_empty()).collect();
        match (req.method.as_str(), path.as_slice()) {
            ("GET" | "HEAD", ["o", token]) => Ok(open(k, &req, token).await),
            ("GET" | "HEAD", ["c", token]) => click(k, &req, token).await,
            ("GET", ["u", token]) => unsubscribe(k, &req, token, false).await,
            ("POST", ["u", token]) => unsubscribe(k, &req, token, true).await,
            ("POST", ["subscribe", form]) => subscribe(k, &req, form).await,
            ("GET", ["confirm", token]) => confirm(k, &req, token, false).await,
            ("POST", ["confirm", token]) => confirm(k, &req, token, true).await,
            ("POST", ["webhooks", "bounces"]) => webhook(k, &req).await,
            _ => Err(Error::NotFound("page".into())),
        }
    }
}

fn header<'a>(req: &'a SiteRequest, name: &str) -> Option<&'a str> {
    req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).and_then(|(_, v)| v.as_str())
}

async fn system(k: &Kernel, tenant: Uuid) -> Result<Ctx> {
    Ok(Ctx::system(tenant, k.registry.schema(tenant).await?))
}

/// A recipient from a signed link, while the link is valid: `(recipient, contact, owner, email)`.
async fn recipient(tx: &mut Tx, tenant: Uuid, rid: Uuid) -> Result<Option<(Uuid, Uuid, Uuid, String, Option<String>)>> {
    let expiry = util::settings(tx, tenant).await?.link_expiry_days;
    Ok(sqlx::query_as(
        "select r.id, r.contact, coalesce(r.campaign, r.journey), r.email, coalesce(c.campaign_name, j.journey_name) from e_campaign_recipient r \
         left join e_email_campaign c on c.tenant_id = r.tenant_id and c.id = r.campaign left join e_marketing_journey j on j.tenant_id = r.tenant_id and j.id = r.journey \
         where r.tenant_id = $1 and r.id = $2 and r.communication is not null and coalesce(r.queued_at, r.created_at) > now() - make_interval(days => $3::int)",
    )
    .bind(tenant)
    .bind(rid)
    .bind(expiry as i32)
    .fetch_optional(&mut **tx)
    .await?)
}

fn parse_uuid(simple: &str) -> Option<Uuid> {
    (simple.len() == 32).then(|| Uuid::try_parse(simple).ok()).flatten()
}

// ───────────────────────────── opens and clicks ─────────────────────────────

async fn open(k: &Kernel, req: &SiteRequest, token: &str) -> SiteResponse {
    if let Err(e) = record_open(k, req, token.strip_suffix(".gif").unwrap_or(token)).await {
        tracing::warn!(tenant = %req.tenant, "open not recorded: {e}");
    }
    SiteResponse {
        status: 200,
        content_type: "image/gif".into(),
        headers: vec![("cache-control".into(), "no-store, no-cache, must-revalidate, max-age=0".into()), ("expires".into(), "0".into())],
        body: GIF.to_vec(),
    }
}

async fn record_open(k: &Kernel, req: &SiteRequest, token: &str) -> Result<()> {
    let key = util::signing_key(k, req.tenant).await?;
    let Some(rid) = util::open_token(&key, token, 1, |p| Some(format!("open:{}", parse_uuid(p[0])?))).and_then(|p| parse_uuid(p[0])) else { return Ok(()) };
    let mut tx = k.db.tenant_tx(req.tenant).await?;
    let Some((rid, contact, ..)) = recipient(&mut tx, req.tenant, rid).await? else { return Ok(()) };
    let sent_at: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
        "select coalesce(c.sent_at, r.sent_at) from e_campaign_recipient r left join e_communication c on c.tenant_id = r.tenant_id and c.id = r.communication \
         where r.tenant_id = $1 and r.id = $2",
    )
    .bind(req.tenant)
    .bind(rid)
    .fetch_one(&mut *tx)
    .await?;
    let machine = machine_open(header(req, "user-agent"), sent_at.map(|t| (Utc::now() - t).num_seconds()));
    sqlx::query(
        "update e_campaign_recipient set open_count = open_count + case when $3 then 0 else 1 end, \
         machine_open_count = machine_open_count + case when $3 then 1 else 0 end, \
         opened_at = case when $3 then opened_at else coalesce(opened_at, now()) end, last_opened_at = case when $3 then last_opened_at else now() end, \
         updated_at = now() where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant)
    .bind(rid)
    .bind(machine)
    .execute(&mut *tx)
    .await?;
    if !machine {
        delivery::engaged(&mut tx, req.tenant, contact).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn click(k: &Kernel, req: &SiteRequest, token: &str) -> Result<SiteResponse> {
    let key = util::signing_key(k, req.tenant).await?;
    let parsed = util::open_token(&key, token, 2, |p| Some(format!("click:{}:{}", parse_uuid(p[0])?, p[1].parse::<i32>().ok()?)));
    let Some((rid, link)) = parsed.and_then(|p| Some((parse_uuid(p[0])?, p[1].parse::<i32>().ok()?))) else { return Ok(util::invalid_link()) };
    let mut tx = k.db.tenant_tx(req.tenant).await?;
    let Some((rid, contact, owner, ..)) = recipient(&mut tx, req.tenant, rid).await? else { return Ok(util::invalid_link()) };
    // Only a URL of this email's own content: the link number picks it, the request cannot.
    let url: Option<String> = sqlx::query_scalar("select url from campaigns_link where tenant_id = $1 and owner_id = $2 and link_no = $3")
        .bind(req.tenant)
        .bind(owner)
        .bind(link)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(url) = url else { return Ok(util::invalid_link()) };
    // A click proves the email was opened, even with images blocked.
    sqlx::query(
        "update e_campaign_recipient set click_count = click_count + 1, clicked_at = coalesce(clicked_at, now()), last_clicked_at = now(), \
         open_count = case when opened_at is null then open_count + 1 else open_count end, opened_at = coalesce(opened_at, now()), \
         last_opened_at = coalesce(last_opened_at, now()), updated_at = now() where tenant_id = $1 and id = $2",
    )
    .bind(req.tenant)
    .bind(rid)
    .execute(&mut *tx)
    .await?;
    sqlx::query("insert into campaigns_click (tenant_id, recipient_id, owner_id, link_no) values ($1, $2, $3, $4)")
        .bind(req.tenant)
        .bind(rid)
        .bind(owner)
        .bind(link)
        .execute(&mut *tx)
        .await?;
    delivery::engaged(&mut tx, req.tenant, contact).await?;
    tx.commit().await?;
    Ok(SiteResponse {
        status: 302,
        content_type: "text/plain; charset=utf-8".into(),
        headers: vec![("location".into(), url), ("cache-control".into(), "no-store".into()), ("referrer-policy".into(), "no-referrer".into())],
        body: vec![],
    })
}

// ───────────────────────────── unsubscribe ─────────────────────────────

async fn unsubscribe(k: &Kernel, req: &SiteRequest, token: &str, confirmed: bool) -> Result<SiteResponse> {
    let key = util::signing_key(k, req.tenant).await?;
    let Some(rid) = util::open_token(&key, token, 1, |p| Some(format!("unsubscribe:{}", parse_uuid(p[0])?))).and_then(|p| parse_uuid(p[0])) else {
        return Ok(util::invalid_link());
    };
    let mut tx = k.db.tenant_tx(req.tenant).await?;
    let Some((rid, contact, _, email, name)) = recipient(&mut tx, req.tenant, rid).await? else { return Ok(util::invalid_link()) };
    let what = name.map(|n| format!(" (you received “{}”)", html_escape(&n))).unwrap_or_default();
    if !confirmed {
        tx.rollback().await?;
        return Ok(page(
            200,
            "Unsubscribe",
            &format!(
                "<p>Stop sending marketing emails to <strong>{}</strong>{what}?</p><form method=\"post\"><button type=\"submit\">Unsubscribe</button></form>",
                html_escape(&email)
            ),
        ));
    }
    sqlx::query("update e_campaign_recipient set unsubscribed_at = coalesce(unsubscribed_at, now()), updated_at = now() where tenant_id = $1 and id = $2")
        .bind(req.tenant)
        .bind(rid)
        .execute(&mut *tx)
        .await?;
    let consent: Option<String> = sqlx::query_scalar("select consent_status from e_marketing_contact where tenant_id = $1 and id = $2 for update")
        .bind(req.tenant)
        .bind(contact)
        .fetch_optional(&mut *tx)
        .await?;
    let sys = system(k, req.tenant).await?;
    match consent.as_deref() {
        Some("Unsubscribed" | "Complained") => {}
        Some(_) => {
            let source = format!("unsubscribe link{}", name_suffix(&mut tx, req.tenant, rid).await?);
            let patch = values([
                ("consent_status", json!("Unsubscribed")),
                ("consent_source", json!(util::cut(&source, 200))),
                ("consent_ip_hash", json!(util::ip_hash(&key, &req.client_ip))),
            ]);
            k.update_tx(&sys, &mut tx, "MarketingContact", contact, patch, None).await?;
        }
        // The contact was deleted since: the address is still never mailed again.
        None => contact::suppress(&mut tx, req.tenant, &email, "Unsubscribed", "unsubscribe link").await?,
    }
    tx.commit().await?;
    tracing::info!(tenant = %req.tenant, recipient = %rid, "unsubscribed");
    Ok(page(200, "You are unsubscribed", &format!("<p><strong>{}</strong> will not receive our marketing emails any more.</p>", html_escape(&email))))
}

async fn name_suffix(tx: &mut Tx, tenant: Uuid, rid: Uuid) -> Result<String> {
    let name: Option<Option<String>> = sqlx::query_scalar(
        "select coalesce(c.number || ' ' || c.campaign_name, 'journey ' || j.journey_name) from e_campaign_recipient r \
         left join e_email_campaign c on c.tenant_id = r.tenant_id and c.id = r.campaign left join e_marketing_journey j on j.tenant_id = r.tenant_id and j.id = r.journey \
         where r.tenant_id = $1 and r.id = $2",
    )
    .bind(tenant)
    .bind(rid)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(name.flatten().map(|n| format!(": {n}")).unwrap_or_default())
}

// ───────────────────────────── double opt-in ─────────────────────────────

/// The fields of a JSON body or a browser form, and whether it came from a browser form.
fn form_fields(req: &SiteRequest) -> Result<(Map<String, Value>, bool)> {
    let ctype = header(req, "content-type").unwrap_or_default().to_ascii_lowercase();
    if ctype.starts_with("application/x-www-form-urlencoded") {
        let mut out = Map::new();
        for (k, v) in form_urlencoded::parse(&req.body) {
            out.insert(k.into_owned(), json!(v.into_owned()));
        }
        return Ok((out, true));
    }
    if req.body.iter().all(u8::is_ascii_whitespace) {
        return Ok((Map::new(), false));
    }
    match serde_json::from_slice::<Value>(&req.body) {
        Ok(Value::Object(m)) => Ok((m, false)),
        _ => Err(Error::invalid("send a JSON object or a form (application/x-www-form-urlencoded)")),
    }
}

fn field<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

/// The same answer whatever happened (new, known, already subscribed, suppressed, a bot).
fn accepted(browser: bool, thank_you: Option<String>) -> SiteResponse {
    let message = "Thanks! Please check your inbox and confirm your subscription.";
    match (browser, thank_you) {
        (true, Some(url)) => {
            SiteResponse { status: 303, content_type: "text/plain; charset=utf-8".into(), headers: vec![("location".into(), url)], body: vec![] }
        }
        (true, None) => page(200, "Almost done", &format!("<p>{message}</p>")),
        (false, _) => SiteResponse::json(202, &json!({"status": "pending", "message": message})),
    }
}

fn confirm_message(contact: Uuid, form: Uuid, expires: i64) -> String {
    format!("confirm:{contact}:{form}:{expires}")
}

async fn subscribe(k: &Kernel, req: &SiteRequest, form: &str) -> Result<SiteResponse> {
    let form: Uuid = form.parse().map_err(|_| Error::NotFound("form".into()))?;
    let (fields, browser) = form_fields(req)?;
    let tenant = req.tenant;
    let key = util::signing_key(k, tenant).await?;
    let mut tx = k.db.tenant_tx(tenant).await?;
    let found: Option<Value> = sqlx::query_scalar("select to_jsonb(f) from e_subscription_form f where f.tenant_id = $1 and f.id = $2 and f.enabled")
        .bind(tenant)
        .bind(form)
        .fetch_optional(&mut *tx)
        .await?;
    let f = found.map(util::obj).ok_or_else(|| Error::NotFound("form".into()))?;
    let thank_you = s(&f, "thank_you_url").map(str::to_string);
    // A filled honeypot is a bot: it is told what everyone is told.
    if field(&fields, "website").is_some() {
        tx.rollback().await?;
        return Ok(accepted(browser, thank_you));
    }
    let email = field(&fields, "email").map(str::to_lowercase).unwrap_or_default();
    if email.len() > 254 || !EMAIL.is_match(&email) {
        return Err(Error::field("email", "enter a valid email address"));
    }
    let mut attributes = Map::new();
    for key in s(&f, "allowed_attributes").unwrap_or_default().split(',').map(str::trim).filter(|k| contact::valid_key(k)) {
        if let Some(v) = field(&fields, key).or_else(|| fields.get("attributes").and_then(|a| a.get(key)).and_then(Value::as_str)) {
            attributes.insert(key.to_string(), json!(util::cut(v, 200)));
        }
    }
    let base = util::site_base(&mut tx, tenant).await?.ok_or_else(|| Error::Unavailable("this form cannot take signups yet".into()))?;
    let sys = system(k, tenant).await?;
    let known: Option<(Uuid, String, Option<chrono::DateTime<Utc>>)> =
        sqlx::query_as("select id, consent_status, confirmation_sent_at from e_marketing_contact where tenant_id = $1 and email = $2 for update")
            .bind(tenant)
            .bind(&email)
            .fetch_optional(&mut *tx)
            .await?;
    let blocked: Option<String> = sqlx::query_scalar("select reason from e_email_suppression where tenant_id = $1 and email = $2 and reason <> 'Unsubscribed'")
        .bind(tenant)
        .bind(&email)
        .fetch_optional(&mut *tx)
        .await?;
    let in_group = match id(&f, "email_group") {
        Some(g) => {
            sqlx::query_scalar::<_, bool>(
                "select exists (select 1 from e_email_group_member where tenant_id = $1 and email_group = $2 and lower(email) = $3 and not unsubscribed)",
            )
            .bind(tenant)
            .bind(g)
            .bind(&email)
            .fetch_one(&mut *tx)
            .await?
        }
        None => true,
    };
    let contact = match known {
        // Complaints, hard bounces and manual blocks get no mail from a form anyone can fill in.
        _ if blocked.is_some() => None,
        Some((_, consent, _)) if consent == "Complained" => None,
        Some((_, consent, _)) if consent == "Subscribed" && in_group => None,
        Some((_, _, Some(at))) if Utc::now() - at < chrono::Duration::minutes(CONFIRMATION_PAUSE_MINUTES) => None,
        // An existing contact is not changed by an unauthenticated request; confirming is what counts.
        Some((cid, ..)) => Some(cid),
        None => {
            let name = |k: &str| field(&fields, k).map(|v| json!(util::cut(v, 100))).unwrap_or(Value::Null);
            let doc = values([
                ("email", json!(email)),
                ("first_name", name("first_name")),
                ("last_name", name("last_name")),
                ("attributes", Value::Object(attributes)),
                ("consent_status", json!("Pending")),
                ("consent_source", json!(util::cut(&format!("form: {}", s(&f, "form_name").unwrap_or_default()), 200))),
                ("source", json!("Subscription Form")),
            ]);
            Some(util::doc_id(&k.insert_tx(&sys, &mut tx, "MarketingContact", doc).await?)?)
        }
    };
    if let Some(cid) = contact {
        let hours = util::settings(&mut tx, tenant).await?.confirmation_hours;
        let expires = (Utc::now() + chrono::Duration::hours(hours)).timestamp();
        let token = format!("{}.{}.{expires}.{}", simple(cid), simple(form), util::sign(&key, &confirm_message(cid, form, expires)));
        let url = format!("{base}/confirm/{token}");
        let view: Value = sqlx::query_scalar(
            "select jsonb_build_object('email', email, 'first_name', first_name, 'last_name', last_name, 'full_name', full_name, 'attributes', attributes) \
             from e_marketing_contact where tenant_id = $1 and id = $2",
        )
        .bind(tenant)
        .bind(cid)
        .fetch_one(&mut *tx)
        .await?;
        let ctx = json!({"contact": view, "doc": view, "confirm_url": url, "form_name": s(&f, "form_name")});
        let content = s(&f, "confirmation_content").map(str::to_string).unwrap_or_else(|| {
            "<p>Hello{% if contact.first_name %} {{ contact.first_name }}{% endif %},</p><p>Please confirm that you want to receive our emails \
             ({{ form_name }}):</p><p><a href=\"{{ confirm_url }}\">Yes, subscribe me</a></p><p>If you did not ask for this, ignore this email: \
             nothing happens without your confirmation.</p>"
                .to_string()
        });
        let html = crate::render::render_html(&content, &ctx)?;
        let composed = send::Composed {
            subject: crate::render::render_line(s(&f, "confirmation_subject").unwrap_or("Please confirm your subscription"), &ctx)?,
            text: format!("{}\n\nConfirm: {url}\n", send::html_text(&html)),
            html,
        };
        send::queue(k, &sys, &mut tx, id(&f, "email_account"), &email, &composed, ("MarketingContact", cid)).await?;
        k.set_values_tx(&sys, &mut tx, "MarketingContact", cid, values([("confirmation_sent_at", json!(util::ts(util::now())))])).await?;
    }
    sqlx::query("update e_subscription_form set submission_count = coalesce(submission_count, 0) + 1 where tenant_id = $1 and id = $2")
        .bind(tenant)
        .bind(form)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(accepted(browser, thank_you))
}

async fn confirm(k: &Kernel, req: &SiteRequest, token: &str, confirmed: bool) -> Result<SiteResponse> {
    let tenant = req.tenant;
    let key = util::signing_key(k, tenant).await?;
    let parsed = util::open_token(&key, token, 3, |p| Some(confirm_message(parse_uuid(p[0])?, parse_uuid(p[1])?, p[2].parse().ok()?)));
    let Some((cid, form, expires)) = parsed.and_then(|p| Some((parse_uuid(p[0])?, parse_uuid(p[1])?, p[2].parse::<i64>().ok()?))) else {
        return Ok(util::invalid_link());
    };
    if expires < Utc::now().timestamp() {
        return Ok(util::invalid_link());
    }
    let mut tx = k.db.tenant_tx(tenant).await?;
    let found: Option<Value> = sqlx::query_scalar("select to_jsonb(f) from e_subscription_form f where f.tenant_id = $1 and f.id = $2 and f.enabled")
        .bind(tenant)
        .bind(form)
        .fetch_optional(&mut *tx)
        .await?;
    let contact: Option<(String, String)> = sqlx::query_as("select email, consent_status from e_marketing_contact where tenant_id = $1 and id = $2 for update")
        .bind(tenant)
        .bind(cid)
        .fetch_optional(&mut *tx)
        .await?;
    let (Some(f), Some((email, consent))) = (found.map(util::obj), contact) else { return Ok(util::invalid_link()) };
    let form_name = s(&f, "form_name").unwrap_or_default().to_string();
    if !confirmed {
        tx.rollback().await?;
        return Ok(page(
            200,
            "Confirm your subscription",
            &format!(
                "<p>Subscribe <strong>{}</strong> to {}?</p><form method=\"post\"><button type=\"submit\">Yes, subscribe me</button></form>",
                html_escape(&email),
                html_escape(&form_name)
            ),
        ));
    }
    let sys = system(k, tenant).await?;
    let mut changed = false;
    if consent != "Subscribed" {
        let patch = values([
            ("consent_status", json!("Subscribed")),
            ("consent_source", json!(util::cut(&format!("double opt-in: {form_name}"), 200))),
            ("consent_ip_hash", json!(util::ip_hash(&key, &req.client_ip))),
        ]);
        k.update_tx(&sys, &mut tx, "MarketingContact", cid, patch, None).await?;
        changed = true;
    }
    // Their own confirmed opt-in lifts automatic suppressions (manual blocks stay).
    contact::lift(&mut tx, tenant, &email).await?;
    if let Some(g) = id(&f, "email_group") {
        let member: Option<(Uuid, bool)> =
            sqlx::query_as("select id, unsubscribed from e_email_group_member where tenant_id = $1 and email_group = $2 and lower(email) = $3")
                .bind(tenant)
                .bind(g)
                .bind(&email)
                .fetch_optional(&mut *tx)
                .await?;
        match member {
            Some((m, true)) => {
                k.update_tx(&sys, &mut tx, "EmailGroupMember", m, values([("unsubscribed", json!(false))]), None).await?;
                changed = true;
            }
            Some(_) => {}
            None => {
                let full: Option<String> = sqlx::query_scalar("select full_name from e_marketing_contact where tenant_id = $1 and id = $2")
                    .bind(tenant)
                    .bind(cid)
                    .fetch_one(&mut *tx)
                    .await?;
                k.insert_tx(
                    &sys,
                    &mut tx,
                    "EmailGroupMember",
                    values([("email_group", json!(g.to_string())), ("email", json!(email)), ("member_name", json!(full))]),
                )
                .await?;
                changed = true;
            }
        }
    }
    if changed {
        sqlx::query("update e_subscription_form set confirmed_count = coalesce(confirmed_count, 0) + 1 where tenant_id = $1 and id = $2")
            .bind(tenant)
            .bind(form)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(match s(&f, "confirmed_url") {
        Some(url) => {
            SiteResponse { status: 303, content_type: "text/plain; charset=utf-8".into(), headers: vec![("location".into(), url.to_string())], body: vec![] }
        }
        None => page(
            200,
            "You are subscribed",
            &format!("<p>Thank you: <strong>{}</strong> is subscribed to {}.</p>", html_escape(&email), html_escape(&form_name)),
        ),
    })
}

// ───────────────────────────── bounce webhook ─────────────────────────────

/// Checks `X-Campaigns-Signature: t=<unix seconds>,v1=<hex HMAC-SHA256 of "<t>.<body>">`.
pub fn signature_valid(secret: &str, header: Option<&str>, body: &[u8], now: i64) -> bool {
    let Some(header) = header else { return false };
    let (mut t, mut v1) = (None, None);
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", v)) => t = v.trim().parse::<i64>().ok(),
            Some(("v1", v)) => v1 = hex::decode(v.trim()).ok(),
            _ => {}
        }
    }
    let (Some(t), Some(sig)) = (t, v1) else { return false };
    if (now - t).abs() > WEBHOOK_TOLERANCE_SECS {
        return false;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(t.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

async fn webhook(k: &Kernel, req: &SiteRequest) -> Result<SiteResponse> {
    let tenant = req.tenant;
    let refused = || Err(Error::Unauthorized("invalid or missing signature".into()));
    let mut tx = k.db.tenant_tx(tenant).await?;
    let settings: Option<Uuid> =
        sqlx::query_scalar("select id from e_campaign_settings where tenant_id = $1 limit 1").bind(tenant).fetch_optional(&mut *tx).await?;
    let secret = match settings {
        Some(sid) => k.secret(&mut tx, tenant, "CampaignSettings", sid, "webhook_secret").await?,
        None => None,
    };
    let Some(secret) = secret.filter(|s| s.len() >= 16) else { return refused() };
    if !signature_valid(&secret, header(req, "x-campaigns-signature"), &req.body, Utc::now().timestamp()) {
        return refused();
    }
    let body: Value = serde_json::from_slice(&req.body).map_err(|_| Error::invalid("the body must be JSON"))?;
    let events: Vec<Value> = match body {
        Value::Array(a) => a,
        Value::Object(ref m) if m.get("events").is_some_and(Value::is_array) => m["events"].as_array().cloned().unwrap_or_default(),
        Value::Object(_) => vec![body],
        _ => return Err(Error::invalid("send an event, a list of events or {\"events\": [...]}")),
    };
    if events.len() > MAX_WEBHOOK_EVENTS {
        return Err(Error::invalid(format!("at most {MAX_WEBHOOK_EVENTS} events per request")));
    }
    let sys = system(k, tenant).await?;
    let (mut processed, mut duplicates, mut ignored) = (0, 0, 0);
    for ev in &events {
        let get = |k: &str| ev.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
        let Some(kind) = get("type").and_then(|t| Kind::parse(t, get("bounce_type"))) else {
            ignored += 1;
            continue;
        };
        let message_id = get("message_id").map(|m| m.trim_matches(['<', '>']).to_string());
        let by_id = match get("recipient").and_then(|r| r.parse::<Uuid>().ok()) {
            Some(r) => {
                sqlx::query_as::<_, (Uuid, String)>("select id, email from e_campaign_recipient where tenant_id = $1 and id = $2")
                    .bind(tenant)
                    .bind(r)
                    .fetch_optional(&mut *tx)
                    .await?
            }
            None => None,
        };
        let by_message = match (&by_id, &message_id) {
            (None, Some(mid)) => {
                sqlx::query_as::<_, (Uuid, String)>(
                    "select r.id, r.email from e_campaign_recipient r join e_communication c on c.tenant_id = r.tenant_id and c.id = r.communication \
                 where r.tenant_id = $1 and c.message_id = $2",
                )
                .bind(tenant)
                .bind(mid)
                .fetch_optional(&mut *tx)
                .await?
            }
            _ => None,
        };
        let email = get("email").map(str::to_lowercase);
        let by_email = match (&by_id, &by_message, &email) {
            (None, None, Some(e)) => {
                sqlx::query_as::<_, (Uuid, String)>(
                    "select id, email from e_campaign_recipient where tenant_id = $1 and email = $2 and ($3::uuid is null or campaign = $3) \
                 and communication is not null order by queued_at desc nulls last limit 1",
                )
                .bind(tenant)
                .bind(e)
                .bind(get("campaign").and_then(|c| c.parse::<Uuid>().ok()))
                .fetch_optional(&mut *tx)
                .await?
            }
            _ => None,
        };
        let found = by_id.or(by_message).or(by_email);
        let Some(address) = found.as_ref().map(|f| f.1.clone()).or(email) else {
            ignored += 1;
            continue;
        };
        let key = match get("id") {
            Some(event_id) => format!("id:{}", util::cut(event_id, 200)),
            None => format!(
                "h:{}",
                hex::encode(Sha256::digest(
                    format!("{kind:?}|{address}|{}|{}", message_id.unwrap_or_default(), get("timestamp").unwrap_or_default()).as_bytes()
                ))
            ),
        };
        let fresh = sqlx::query("insert into campaigns_webhook_event (tenant_id, event_key) values ($1, $2) on conflict do nothing")
            .bind(tenant)
            .bind(&key)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if fresh == 0 {
            duplicates += 1;
            continue;
        }
        let source = format!("bounce webhook{}", get("reason").map(|r| format!(": {r}")).unwrap_or_default());
        delivery::apply(k, &sys, &mut tx, &address, found.map(|f| f.0), kind, &source).await?;
        processed += 1;
    }
    tx.commit().await?;
    Ok(SiteResponse::json(200, &json!({"processed": processed, "duplicates": duplicates, "ignored": ignored})))
}

// ───────────────────────────── forms, settings, recipients ─────────────────────────────

pub struct FormHook;

#[async_trait]
impl DocHook for FormHook {
    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        let mut errors = Vec::new();
        for key in s(doc, "allowed_attributes").unwrap_or_default().split(',').map(str::trim).filter(|k| !k.is_empty()) {
            if !contact::valid_key(key) || key == "website" {
                errors
                    .push(FieldError::new("allowed_attributes", format!("'{key}' cannot be a form attribute (lowercase letters, digits and _; not website)")));
            }
        }
        for f in ["confirmation_subject", "confirmation_content"] {
            if let Some(src) = s(doc, f)
                && let Err(m) = crate::render::check(src)
            {
                errors.push(FieldError::new(f, m));
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(Error::Validation(errors)) }
    }

    async fn after_save(&self, hc: &mut HookCtx<'_>, doc: &Doc, old: Option<&Doc>) -> Result<()> {
        if old.is_none() {
            let endpoint = format!("{}/subscribe/{}", util::site_path(hc.tx, hc.ctx.tenant).await?, util::doc_id(doc)?);
            hc.kernel.set_values_tx(&sys(hc.ctx), hc.tx, "SubscriptionForm", util::doc_id(doc)?, values([("endpoint", json!(endpoint))])).await?;
        }
        Ok(())
    }
}

pub struct SettingsHook;

#[async_trait]
impl DocHook for SettingsHook {
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "rotate_webhook_secret" {
            return Ok(None);
        }
        let secret = hex::encode(rand::random::<[u8; 32]>());
        hc.kernel.update_tx(&sys(hc.ctx), hc.tx, "CampaignSettings", util::doc_id(doc)?, values([("webhook_secret", json!(secret))]), None).await?;
        let (path, url) = (util::site_path(hc.tx, hc.ctx.tenant).await?, util::site_base(hc.tx, hc.ctx.tenant).await?);
        Ok(Some(json!({
            "secret": secret,
            "path": format!("{path}/webhooks/bounces"),
            "url": url.map(|u| format!("{u}/webhooks/bounces")),
            "note": "Shown only now. Sign each POST with the header X-Campaigns-Signature: t=<unix seconds>,v1=<hex HMAC-SHA256 of \"<t>.<raw body>\" with this secret>.",
        })))
    }
}

/// Recipients are the engine's record of what was sent: nobody edits or deletes them.
pub struct RecipientGuard;

#[async_trait]
impl DocHook for RecipientGuard {
    async fn validate(&self, hc: &mut HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if hc.ctx.is_system { Ok(()) } else { Err(Error::Forbidden("campaign recipients are written by the app".into())) }
    }

    async fn before_delete(&self, hc: &mut HookCtx<'_>, _doc: &Doc) -> Result<()> {
        if hc.ctx.is_system { Ok(()) } else { Err(Error::Forbidden("campaign recipients are kept as the record of what was sent".into())) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_opens() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148";
        assert!(!machine_open(Some(iphone), Some(3600)));
        assert!(
            !machine_open(Some("Mozilla/5.0 (Windows NT 5.1; rv:11.0) Gecko Firefox/11.0 (via ggpht.com GoogleImageProxy)"), Some(60)),
            "Gmail fetches on open"
        );
        assert!(machine_open(Some("Mozilla/5.0"), Some(3600)), "Apple Mail Privacy Protection");
        assert!(machine_open(None, None) && machine_open(Some("  "), None));
        assert!(machine_open(Some("python-requests/2.31"), Some(600)));
        assert!(machine_open(Some("Barracuda Sentinel (EE)"), None));
        assert!(machine_open(Some(iphone), Some(1)), "fetched as it arrived");
        assert!(!machine_open(Some(iphone), None));
    }

    #[test]
    fn webhook_signatures() {
        let secret = "0123456789abcdef0123456789abcdef";
        let body = br#"{"type":"hard","email":"a@b.test"}"#;
        let sign = |t: i64, body: &[u8]| {
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
            mac.update(format!("{t}.").as_bytes());
            mac.update(body);
            format!("t={t},v1={}", hex::encode(mac.finalize().into_bytes()))
        };
        let now = 1_800_000_000;
        assert!(signature_valid(secret, Some(&sign(now, body)), body, now));
        assert!(signature_valid(secret, Some(&sign(now - 299, body)), body, now));
        assert!(!signature_valid(secret, Some(&sign(now - 301, body)), body, now), "replayed later");
        assert!(!signature_valid(secret, Some(&sign(now, br#"{"type":"hard","email":"x@b.test"}"#)), body, now), "another body");
        assert!(!signature_valid("another secret, another tenant!!", Some(&sign(now, body)), body, now));
        assert!(
            !signature_valid(secret, None, body, now)
                && !signature_valid(secret, Some("t=1,v1=zz"), body, now)
                && !signature_valid(secret, Some("garbage"), body, now)
        );
    }
}
