//! Helpers shared by the package: document access, time, small HTML pages, signed tokens and
//! the workspace's settings (ours, and the Mail app's public URL and hourly limit).

use chrono::{DateTime, SecondsFormat, Utc};
pub use ferrum_domain::util::{b, id, rows, s, sys};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, SiteResponse, Tx};
use hmac::{Hmac, Mac};
use serde_json::{Map, Value, json};
use sha2::Sha256;
use uuid::Uuid;

pub const PACKAGE: &str = "campaigns";

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// RFC 3339 UTC with milliseconds, the kernel's canonical date-time form.
pub fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn time(doc: &Doc, k: &str) -> Option<DateTime<Utc>> {
    s(doc, k).and_then(|v| ferrum_kernel::value::parse_datetime(v).ok())
}

pub fn int(doc: &Doc, k: &str) -> i64 {
    doc.get(k).and_then(Value::as_i64).unwrap_or(0)
}

pub fn doc_id(doc: &Doc) -> Result<Uuid> {
    id(doc, "id").ok_or_else(|| Error::internal("document without id"))
}

/// Text cut to `max` characters.
pub fn cut(v: &str, max: usize) -> String {
    v.chars().take(max).collect()
}

pub fn obj(v: Value) -> Doc {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// A document from `(field, value)` pairs.
pub fn values<const N: usize>(pairs: [(&str, Value); N]) -> Doc {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// A small self-contained HTML page (no scripts, no external resources).
pub fn page(status: u16, title: &str, body_html: &str) -> SiteResponse {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><title>{t}</title><style>body{{font:15px/1.5 system-ui,-apple-system,Segoe UI,sans-serif;\
         color:#1f2328;background:#f6f8fa;margin:0;padding:48px 16px}}main{{max-width:460px;margin:auto;background:#fff;border:1px solid #d0d7de;\
         border-radius:8px;padding:24px}}h1{{font-size:20px;margin:0 0 12px}}button{{font:inherit;padding:8px 16px;border-radius:6px;border:1px solid #1f6feb;\
         background:#1f6feb;color:#fff;cursor:pointer}}</style></head><body><main><h1>{t}</h1>{body_html}</main></body></html>",
        t = html_escape(title)
    );
    let mut r = SiteResponse::text(status, "text/html; charset=utf-8", html);
    r.headers.push(("content-security-policy".into(), "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'".into()));
    r.headers.push(("cache-control".into(), "no-store".into()));
    r.headers.push(("x-frame-options".into(), "DENY".into()));
    r.headers.push(("referrer-policy".into(), "no-referrer".into()));
    r
}

/// The one answer to a link that is unknown, tampered with or expired: nobody learns which.
pub fn invalid_link() -> SiteResponse {
    page(404, "This link is not valid", "<p>The link is incomplete, was changed or has expired. Please use the link from a recent email.</p>")
}

// ───────────────────────────── signed tokens ─────────────────────────────

/// The tenant's HMAC key for links (tracking, unsubscribe, confirmation): a secret field of
/// Campaign Settings, created on first use.
pub async fn signing_key(k: &Kernel, tenant: Uuid) -> Result<Vec<u8>> {
    let sys = Ctx::system(tenant, k.registry.schema(tenant).await?);
    for _ in 0..2 {
        let mut tx = k.db.tenant_tx(tenant).await?;
        let settings: Option<Uuid> =
            sqlx::query_scalar("select id from e_campaign_settings where tenant_id = $1 limit 1 for update").bind(tenant).fetch_optional(&mut *tx).await?;
        if let Some(id) = settings
            && let Some(key) = k.secret(&mut tx, tenant, "CampaignSettings", id, "signing_key").await?
        {
            tx.commit().await?;
            return hex::decode(key).map_err(Error::internal);
        }
        let fresh = hex::encode(rand::random::<[u8; 32]>());
        let patch = values([("signing_key", json!(fresh))]);
        let written = match settings {
            Some(id) => k.update_tx(&sys, &mut tx, "CampaignSettings", id, patch, None).await,
            None => k.insert_tx(&sys, &mut tx, "CampaignSettings", patch).await,
        };
        match written {
            Ok(_) => {
                tx.commit().await?;
                return hex::decode(fresh).map_err(Error::internal);
            }
            // The settings were created at the same moment elsewhere: read theirs.
            Err(Error::Conflict(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(Error::Retryable("signing key".into()))
}

fn mac(key: &[u8], message: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    mac
}

/// 128-bit HMAC-SHA256 tag of `message`, hex.
pub fn sign(key: &[u8], message: &str) -> String {
    hex::encode(&mac(key, message).finalize().into_bytes()[..16])
}

/// Constant-time check of a tag made by [`sign`].
pub fn verify(key: &[u8], message: &str, tag: &str) -> bool {
    match hex::decode(tag.trim()) {
        Ok(raw) if raw.len() == 16 => mac(key, message).verify_truncated_left(&raw).is_ok(),
        _ => false,
    }
}

/// Keyed hash of a network address: consent evidence without keeping the address.
pub fn ip_hash(key: &[u8], ip: &str) -> String {
    hex::encode(mac(key, &format!("ip:{}", ip.trim())).finalize().into_bytes())
}

/// A uuid written without dashes (as in tokens).
pub fn simple(u: Uuid) -> String {
    u.simple().to_string()
}

/// Splits a signed token `part.part….tag` into its parts when the tag over `message(parts)` checks out.
pub fn open_token<'a>(key: &[u8], token: &'a str, parts: usize, message: impl Fn(&[&str]) -> Option<String>) -> Option<Vec<&'a str>> {
    let all: Vec<&str> = token.split('.').collect();
    if all.len() != parts + 1 || token.len() > 200 {
        return None;
    }
    let (body, tag) = all.split_at(parts);
    let msg = message(body)?;
    verify(key, &msg, tag[0]).then(|| body.to_vec())
}

// ───────────────────────────── settings ─────────────────────────────

#[derive(Debug, Clone)]
pub struct Settings {
    pub postal_address: Option<String>,
    pub soft_bounce_limit: i64,
    pub link_expiry_days: i64,
    pub confirmation_hours: i64,
}

pub async fn settings(tx: &mut Tx, tenant: Uuid) -> Result<Settings> {
    let row: Option<(Option<String>, Option<i64>, Option<i64>, Option<i64>)> =
        sqlx::query_as("select postal_address, soft_bounce_limit, link_expiry_days, confirmation_hours from e_campaign_settings where tenant_id = $1 limit 1")
            .bind(tenant)
            .fetch_optional(&mut **tx)
            .await?;
    let (postal, soft, expiry, confirm) = row.unwrap_or_default();
    Ok(Settings {
        postal_address: postal.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()),
        soft_bounce_limit: soft.unwrap_or(3).clamp(1, 20),
        link_expiry_days: expiry.unwrap_or(365).clamp(30, 3650),
        confirmation_hours: confirm.unwrap_or(72).clamp(1, 720),
    })
}

/// `/api/site/{slug}/campaigns`, the root of the package's public endpoints.
pub async fn site_path(tx: &mut Tx, tenant: Uuid) -> Result<String> {
    let slug: String = sqlx::query_scalar("select slug from tenant where id = $1").bind(tenant).fetch_one(&mut **tx).await?;
    Ok(format!("/api/site/{slug}/{PACKAGE}"))
}

/// The absolute address of the public endpoints, from the Mail app's Public URL (Communication
/// Settings); `None` until that is set.
pub async fn site_base(tx: &mut Tx, tenant: Uuid) -> Result<Option<String>> {
    let public: Option<Option<String>> =
        sqlx::query_scalar("select public_url from e_comms_settings where tenant_id = $1 limit 1").bind(tenant).fetch_optional(&mut **tx).await?;
    let Some(base) = public.flatten().map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty()) else { return Ok(None) };
    Ok(Some(format!("{base}{}", site_path(tx, tenant).await?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_bind_key_and_message() {
        let key = b"0123456789abcdef0123456789abcdef";
        let tag = sign(key, "open:1");
        assert_eq!(tag.len(), 32);
        assert!(verify(key, "open:1", &tag));
        assert!(!verify(key, "open:2", &tag), "another message");
        assert!(!verify(b"another key, another tenant......", "open:1", &tag), "another key");
        assert!(!verify(key, "open:1", &tag[..30]) && !verify(key, "open:1", "zz") && !verify(key, "open:1", ""));
        let mut flipped = tag.clone().into_bytes();
        flipped[5] = if flipped[5] == b'a' { b'b' } else { b'a' };
        assert!(!verify(key, "open:1", std::str::from_utf8(&flipped).unwrap()));
        let token = format!("abc.7.{}", sign(key, "click:abc:7"));
        assert_eq!(open_token(key, &token, 2, |p| Some(format!("click:{}:{}", p[0], p[1]))), Some(vec!["abc", "7"]));
        assert_eq!(open_token(key, &token.replace(".7.", ".8."), 2, |p| Some(format!("click:{}:{}", p[0], p[1]))), None);
        assert_eq!(open_token(key, &token, 1, |p| Some(p[0].to_string())), None, "wrong shape");
        assert_ne!(ip_hash(key, "10.0.0.1"), ip_hash(key, "10.0.0.2"));
        assert_eq!(ip_hash(key, "10.0.0.1").len(), 64);
        assert_eq!(html_escape("<a href=\"x\">'&"), "&lt;a href=&quot;x&quot;&gt;&#39;&amp;");
    }
}
