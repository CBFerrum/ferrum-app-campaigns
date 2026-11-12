//! Topics and the preference centre. A workspace defines topics (a newsletter, offers, product news); a contact's own
//! choice for a topic is a `TopicSubscription` with its evidence, and without one the topic's default holds, so
//! contacts that existed before a topic keep receiving what they received.
//!
//! "Stop everything" is not a topic: it is the platform's marketing opt-out of the address (see
//! [`crate::contact::MAILABLE`]), one fact every app reads. Topics are the finer choice on top of it. The signed
//! unsubscribe link opens the preference page when the workspace has topics; the one-click request mail clients send
//! (RFC 8058) stays "everything" for this sender.

use crate::channels::Channel;
use crate::contact::{self, MAILABLE, REACHABLE, TEXT_REACHABLE, TEXTABLE};
use crate::util::{self, html_escape, id, s, simple, values};
use async_trait::async_trait;
use ferrum_kernel::retention::Purpose;
use ferrum_kernel::{Ctx, Doc, DocHook, Error, HookCtx, Kernel, Result, SiteResponse, Tx};
use serde_json::{Map, Value, json};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

/// Who a send may reach: everyone who takes promotional mail, or an operational topic's people; narrowed to a topic.
#[derive(Debug, Clone, Default)]
pub struct Reach {
    pub topic: Option<Uuid>,
    pub operational: bool,
    pub channel: Channel,
}

impl Reach {
    /// The reach of a send naming `topic` (none: every contact who takes promotional mail). A topic that is gone or
    /// turned off is refused.
    pub async fn of(tx: &mut Tx, tenant: Uuid, topic: Option<Uuid>) -> Result<Reach> {
        let Some(t) = topic else { return Ok(Reach::default()) };
        let found: Option<(String, bool)> = sqlx::query_as("select category, coalesce(enabled, false) from e_marketing_topic where tenant_id = $1 and id = $2")
            .bind(tenant)
            .bind(t)
            .fetch_optional(&mut **tx)
            .await?;
        match found {
            Some((category, true)) => Ok(Reach { topic: Some(t), operational: category == "Operational", channel: Channel::Email }),
            Some(_) => Err(Error::field("topic", "this topic is turned off")),
            None => Err(Error::field("topic", "this topic does not exist")),
        }
    }

    /// The same, on another channel.
    pub fn on(self, channel: Channel) -> Reach {
        Reach { channel, ..self }
    }

    /// The condition on a contact row `c`.
    pub fn push(&self, qb: &mut QueryBuilder<Postgres>) {
        qb.push(match (self.channel, self.operational) {
            (Channel::Email, false) => MAILABLE,
            (Channel::Email, true) => REACHABLE,
            (_, false) => TEXTABLE,
            (_, true) => TEXT_REACHABLE,
        });
        if let Some(t) = self.topic {
            qb.push(" and ");
            push_in_topic(qb, t);
        }
    }

    /// What the Mail app is told the mail is: an operational topic's goes whatever the marketing choice.
    pub fn purpose(&self) -> Purpose {
        if self.operational { Purpose::Transactional } else { Purpose::Marketing }
    }
}

/// Whether the contact `c` is in topic `t`: its own choice, else the topic's default.
pub fn push_in_topic(qb: &mut QueryBuilder<Postgres>, t: Uuid) {
    qb.push("coalesce((select s.status = 'Subscribed' from e_topic_subscription s where s.tenant_id = c.tenant_id and s.contact = c.id and s.topic = ")
        .push_bind(t)
        .push("), (select t.default_in from e_marketing_topic t where t.tenant_id = c.tenant_id and t.id = ")
        .push_bind(t)
        .push("), false)");
}

/// Whether `contact` is in `topic` now.
pub async fn is_in(tx: &mut Tx, tenant: Uuid, contact: Uuid, topic: Uuid) -> Result<bool> {
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select exists (select 1 from e_marketing_contact c where c.tenant_id = ");
    qb.push_bind(tenant).push(" and c.id = ").push_bind(contact).push(" and ");
    push_in_topic(&mut qb, topic);
    qb.push(")");
    Ok(qb.build_query_scalar().fetch_one(&mut **tx).await?)
}

/// Records the person's choice for a topic (as the engine, with how and where it was made), when it changes it.
pub async fn choose(k: &Kernel, sys: &Ctx, tx: &mut Tx, contact: Uuid, topic: Uuid, status: &str, method: &str, source: &str) -> Result<bool> {
    let found: Option<(Uuid, String)> =
        sqlx::query_as("select id, status from e_topic_subscription where tenant_id = $1 and contact = $2 and topic = $3 for update")
            .bind(sys.tenant)
            .bind(contact)
            .bind(topic)
            .fetch_optional(&mut **tx)
            .await?;
    let patch = values([("status", json!(status)), ("method", json!(method)), ("source", json!(util::cut(source, 200)))]);
    match found {
        Some((_, old)) if old == status => Ok(false),
        Some((sid, _)) => {
            k.update_tx(sys, tx, "TopicSubscription", sid, patch, None).await?;
            Ok(true)
        }
        None => {
            let mut doc = patch;
            doc.insert("contact".into(), json!(contact.to_string()));
            doc.insert("topic".into(), json!(topic.to_string()));
            k.insert_tx(sys, tx, "TopicSubscription", doc).await?;
            Ok(true)
        }
    }
}

pub struct SubscriptionHook;

#[async_trait]
impl DocHook for SubscriptionHook {
    async fn before_validate(&self, hc: &mut HookCtx<'_>, doc: &mut Doc) -> Result<()> {
        if let Some(c) = id(doc, "contact") {
            let email: Option<String> = sqlx::query_scalar("select email from e_marketing_contact where tenant_id = $1 and id = $2")
                .bind(hc.ctx.tenant)
                .bind(c)
                .fetch_optional(&mut **hc.tx)
                .await?;
            doc.insert("email".into(), json!(email));
        }
        let old: Option<String> = match id(doc, "id").filter(|_| doc.contains_key("created_at")) {
            Some(sid) => {
                sqlx::query_scalar("select status from e_topic_subscription where tenant_id = $1 and id = $2")
                    .bind(hc.ctx.tenant)
                    .bind(sid)
                    .fetch_optional(&mut **hc.tx)
                    .await?
            }
            None => None,
        };
        if old.as_deref() != s(doc, "status") {
            doc.insert("changed_at".into(), json!(util::ts(util::now())));
            // People are named; the engine passes how and where.
            if !hc.ctx.is_system {
                doc.insert("method".into(), json!("Staff"));
                doc.insert("source".into(), json!(util::cut(&format!("user: {}", hc.ctx.email), 200)));
            }
        }
        if !hc.ctx.is_system && !contact::user_may_set(old.as_deref(), s(doc, "status").unwrap_or("Subscribed")) {
            return Err(Error::field("status", "someone who left a topic can only join it again themselves, on their preference page or a signup form"));
        }
        Ok(())
    }
}

// ───────────────────────────── the preference page ─────────────────────────────

/// The workspace's topics on the page, in order: `(id, name, description, default_in, operational)`.
async fn listed(tx: &mut Tx, tenant: Uuid) -> Result<Vec<(Uuid, String, Option<String>, bool, bool)>> {
    Ok(sqlx::query_as(
        "select id, topic_name, description, coalesce(default_in, false), category = 'Operational' from e_marketing_topic \
         where tenant_id = $1 and coalesce(enabled, false) order by sort_order nulls first, topic_name",
    )
    .bind(tenant)
    .fetch_all(&mut **tx)
    .await?)
}

/// Whether the person stopped everything: their contact is not subscribed, or their address is opted out of marketing.
async fn stopped(k: &Kernel, tx: &mut Tx, tenant: Uuid, contact: Uuid, email: &str) -> Result<bool> {
    let status: Option<String> = sqlx::query_scalar("select consent_status from e_marketing_contact where tenant_id = $1 and id = $2")
        .bind(tenant)
        .bind(contact)
        .fetch_optional(&mut **tx)
        .await?;
    Ok(status.as_deref() != Some("Subscribed") || !k.may_contact_for(tx, tenant, email, Purpose::Marketing).await?)
}

/// The page a signed unsubscribe link opens when the workspace has topics: the person's topics to tick and "stop
/// everything". `None` without topics (the plain unsubscribe question is asked).
pub async fn page(k: &Kernel, tx: &mut Tx, tenant: Uuid, contact: Uuid, email: &str, what: &str) -> Result<Option<SiteResponse>> {
    let topics = listed(tx, tenant).await?;
    if topics.is_empty() {
        return Ok(None);
    }
    let all = stopped(k, tx, tenant, contact, email).await?;
    let mut rows = String::new();
    for (t, name, description, _, _) in &topics {
        let on = is_in(tx, tenant, contact, *t).await?;
        rows.push_str(&format!(
            "<p><label><input type=\"checkbox\" name=\"t_{}\"{}> <strong>{}</strong></label>{}</p>",
            simple(*t),
            if on { " checked" } else { "" },
            html_escape(name),
            description.as_deref().map(|d| format!("<br><small>{}</small>", html_escape(d))).unwrap_or_default()
        ));
    }
    // ponytail: the page's few words are fixed in English; the platform has no render-only call for a package's
    // message template (its preview fills values with placeholders), so they cannot be a template yet.
    Ok(Some(util::page(
        200,
        "Your email preferences",
        &format!(
            "<p>Choose what <strong>{}</strong> receives from us{what}.</p><form method=\"post\"><input type=\"hidden\" name=\"prefs\" value=\"1\">{rows}\
             <p><label><input type=\"checkbox\" name=\"stop_all\"{}> <strong>Stop everything</strong></label><br><small>No marketing email at all, \
             whatever is ticked above</small></p><button type=\"submit\">Save</button></form>",
            html_escape(email),
            if all { " checked" } else { "" }
        ),
    )))
}

/// Saves what the person ticked on their page (`fields`: the posted form). `name` says which email they came from.
pub async fn save(
    k: &Kernel,
    sys: &Ctx,
    tx: &mut Tx,
    contact: Uuid,
    email: &str,
    fields: &Map<String, Value>,
    from: &str,
    ip_hash: &str,
    key: &[u8],
) -> Result<SiteResponse> {
    let tenant = sys.tenant;
    let source = util::cut(&format!("preference page{from}"), 200);
    for (t, _, _, _, _) in listed(tx, tenant).await? {
        let want = fields.get(&format!("t_{}", simple(t))).is_some();
        if want != is_in(tx, tenant, contact, t).await? {
            choose(k, sys, tx, contact, t, if want { "Subscribed" } else { "Unsubscribed" }, "Preference Page", &source).await?;
        }
    }
    let stop = fields.get("stop_all").is_some();
    let status: Option<String> = sqlx::query_scalar("select consent_status from e_marketing_contact where tenant_id = $1 and id = $2 for update")
        .bind(tenant)
        .bind(contact)
        .fetch_optional(&mut **tx)
        .await?;
    match (stop, status.as_deref()) {
        (true, Some("Subscribed" | "Pending")) => {
            let patch = values([("consent_status", json!("Unsubscribed")), ("consent_source", json!(source)), ("consent_ip_hash", json!(ip_hash))]);
            k.update_tx(sys, tx, "MarketingContact", contact, patch, None).await?;
        }
        (true, _) => {}
        // Their own word, on the page their signed link opened: they take marketing mail again. A link in a forwarded
        // mail would let someone else say it, so where the workspace confirms re-subscribing, the status changes only
        // when the person confirms the mail this sends.
        (false, Some("Unsubscribed")) => {
            if util::settings(tx, tenant).await?.double_opt_in {
                return ask_again(k, sys, tx, contact, email, key).await;
            }
            let patch = values([
                ("consent_status", json!("Subscribed")),
                ("consent_source", json!(format!("re-subscription from the {source}"))),
                ("consent_ip_hash", json!(ip_hash)),
            ]);
            k.update_tx(sys, tx, "MarketingContact", contact, patch, None).await?;
            sqlx::query("delete from e_email_suppression where tenant_id = $1 and email = $2 and reason = 'Unsubscribed'")
                .bind(tenant)
                .bind(email)
                .execute(&mut **tx)
                .await?;
        }
        // Opted out elsewhere (another app, staff): the one fact is the platform's, so it is changed there.
        (false, Some("Subscribed")) if !k.may_contact_for(tx, tenant, email, Purpose::Marketing).await? => {
            if util::settings(tx, tenant).await?.double_opt_in {
                return ask_again(k, sys, tx, contact, email, key).await;
            }
            k.record_consent_tx(tx, sys, email, Purpose::Marketing, true, &util::cut(&format!("Campaigns: re-subscription from the {source}"), 200)).await?;
        }
        _ => {}
    }
    Ok(util::page(200, "Your preferences are saved", &format!("<p>Thank you: we keep to what you chose for <strong>{}</strong>.</p>", html_escape(email))))
}

/// The message a re-subscribing confirmation link signs.
pub fn reconfirm_message(contact: Uuid, expires: i64) -> String {
    format!("reconfirm:{contact}:{expires}")
}

/// Mails the person a link to confirm taking marketing mail again (at most one in a few minutes), and answers the page.
async fn ask_again(k: &Kernel, sys: &Ctx, tx: &mut Tx, contact: Uuid, email: &str, key: &[u8]) -> Result<SiteResponse> {
    let tenant = sys.tenant;
    let sent: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("select confirmation_sent_at from e_marketing_contact where tenant_id = $1 and id = $2")
            .bind(tenant)
            .bind(contact)
            .fetch_one(&mut **tx)
            .await?;
    if sent.is_none_or(|at| chrono::Utc::now() - at >= chrono::Duration::minutes(crate::public::CONFIRMATION_PAUSE_MINUTES)) {
        let base = util::site_base(tx, tenant).await?.ok_or_else(|| Error::Unavailable("confirmation mail cannot be sent yet".into()))?;
        let settings = util::settings(tx, tenant).await?;
        let expires = (chrono::Utc::now() + chrono::Duration::hours(settings.confirmation_hours)).timestamp();
        let url = format!("{base}/reconfirm/{}.{expires}.{}", simple(contact), util::sign(key, &reconfirm_message(contact, expires)));
        let view: Value = sqlx::query_scalar(
            "select jsonb_build_object('email', email, 'first_name', first_name, 'last_name', last_name, 'full_name', full_name) \
             from e_marketing_contact where tenant_id = $1 and id = $2",
        )
        .bind(tenant)
        .bind(contact)
        .fetch_one(&mut **tx)
        .await?;
        let ctx = json!({"contact": view, "doc": view, "confirm_url": url});
        let content = settings.reconfirm_content.unwrap_or_else(|| {
            "<p>Hello{% if contact.first_name %} {{ contact.first_name }}{% endif %},</p><p>Please confirm that you want our emails again:</p>\
             <p><a href=\"{{ confirm_url }}\">Yes, subscribe me</a></p><p>If this was not you, ignore this email: nothing changes without your confirmation.</p>"
                .to_string()
        });
        let html = crate::render::render_html(&content, &ctx)?;
        let subject = crate::render::render_line(settings.reconfirm_subject.as_deref().unwrap_or("Please confirm you want our emails again"), &ctx)?;
        let composed = crate::send::Composed { subject, text: format!("{}\n\nConfirm: {url}\n", crate::send::html_text(&html)), html, unsubscribe: None };
        crate::send::queue(k, sys, tx, None, email, &composed, ("MarketingContact", contact), crate::send::Kind::Transactional).await?;
        k.set_values_tx(sys, tx, "MarketingContact", contact, values([("confirmation_sent_at", json!(util::ts(util::now())))])).await?;
    }
    Ok(util::page(200, "Check your inbox", "<p>We sent you an email: you take our emails again when you confirm it. Until then nothing changes.</p>"))
}

// ───────────────────────────── erasure ─────────────────────────────

/// After an erasure has blanked a contact, it leaves every journey at once (the platform calls this in the erasure's
/// transaction, after the records).
pub struct Erased;

#[async_trait]
impl ferrum_kernel::privacy::PersonalHeld for Erased {
    fn app(&self) -> &'static str {
        "campaigns"
    }

    async fn report(&self, _tx: &mut Tx, _tenant: Uuid, _account: Option<Uuid>, _emails: &[String]) -> Result<Vec<Value>> {
        Ok(vec![])
    }

    async fn erase(&self, tx: &mut Tx, tenant: Uuid, _account: Option<Uuid>, _emails: &[String], _reason: &str) -> Result<u64> {
        let erased: Vec<Uuid> = sqlx::query_scalar(
            "select c.id from e_marketing_contact c where c.tenant_id = $1 and c.email like '%@erased.invalid' \
             and exists (select 1 from e_journey_enrollment e where e.tenant_id = c.tenant_id and e.contact = c.id and e.status = 'Active')",
        )
        .bind(tenant)
        .fetch_all(&mut **tx)
        .await?;
        if erased.is_empty() {
            return Ok(0);
        }
        contact::exit_journeys(tx, tenant, &erased, "erased").await
    }
}
