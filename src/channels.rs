//! Text and chat-app messages from campaigns and journey steps, through the platform (docs/builder/decisions.md
//! BD-20): a message template the workspace edits, its channel connection, the number's own opt-out on that channel,
//! quiet hours and hourly limits are all the platform's. This app chooses who (its audience and topics), hands each
//! person to `messages::send` in the transaction that claims them, and reads the outcome back from the platform's
//! delivery log. No provider is named here.

use crate::util::{self, s};
use ferrum_kernel::messages::{self, Message, Recipient};
use ferrum_kernel::{Ctx, Doc, Error, Kernel, Result, Tx};
use serde_json::Value;
use uuid::Uuid;

/// Text and chat-app messages the platform sends a workspace an hour unless its `messages` setting says otherwise
/// (the platform's own default; it skips what passes it, so campaigns pace themselves to stay under it).
const PLATFORM_PER_HOUR: i64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    #[default]
    Email,
    Text,
    Chat,
}

impl Channel {
    /// A campaign's or step's `channel` (blank: email).
    pub fn of(doc: &Doc) -> Channel {
        match s(doc, "channel") {
            Some("Text Message") => Channel::Text,
            Some("Chat App") => Channel::Chat,
            _ => Channel::Email,
        }
    }

    /// The platform's name for it.
    pub fn platform(self) -> &'static str {
        match self {
            Channel::Email => "email",
            Channel::Text => "text",
            Channel::Chat => "chat",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::Email => "Email",
            Channel::Text => "Text Message",
            Channel::Chat => "Chat App",
        }
    }
}

/// Whether template `key` can carry this send: it exists (the workspace's own or edited copy, else a package's
/// default), has a body for the channel, is about a Marketing Contact or no record, and is promotional unless the send
/// is operational (an operational template would pass every opt-out).
pub async fn check_template(tx: &mut Tx, ctx: &Ctx, key: &str, channel: Channel, operational: bool) -> Result<()> {
    let own: Option<Value> =
        sqlx::query_scalar("select def from message_template where tenant_id = $1 and key = $2").bind(ctx.tenant).bind(key).fetch_optional(&mut **tx).await?;
    let def = match own {
        Some(v) => serde_json::from_value::<ferrum_meta::MessageDef>(v).ok(),
        None => ctx.schema.messages.iter().find(|m| m.key == key).cloned(),
    };
    let Some(def) = def else { return Err(Error::field("message_template", format!("there is no message template {key}"))) };
    if !def.order().iter().any(|c| c == channel.platform()) {
        return Err(Error::field("message_template", format!("{key} has no {} body", channel.label().to_lowercase())));
    }
    if def.entity.as_deref().is_some_and(|e| e != "MarketingContact") {
        return Err(Error::field("message_template", format!("{key} is about another kind of record")));
    }
    if !operational && def.category != ferrum_meta::MessageCategory::Promotional {
        return Err(Error::field("message_template", format!("{key} is operational: marketing goes out on promotional templates, which respect opt-outs")));
    }
    Ok(())
}

/// Text and chat-app messages the workspace may still send this hour, by the platform's limit.
pub async fn room(tx: &mut Tx, tenant: Uuid) -> Result<i64> {
    let (limit, used): (Option<i64>, i64) = sqlx::query_as(
        "select (select (settings->'messages'->>'per_hour')::bigint from tenant where id = $1), \
                (select count(*) from message_send where tenant_id = $1 and channel in ('text', 'chat') and state <> 'skipped' \
                 and created_at > now() - interval '1 hour')",
    )
    .bind(tenant)
    .fetch_one(&mut **tx)
    .await?;
    Ok((limit.unwrap_or(PLATFORM_PER_HOUR) - used).max(0))
}

/// Hands one person to the platform in the caller's transaction: the template about their contact record, to their
/// number. Returns the platform's send and what the recipient row becomes (`Queued`, or `Skipped` and why). `record`:
/// the record that started a journey, which the template may read as `values.record`.
#[allow(clippy::too_many_arguments)]
pub async fn hand_over(
    k: &Kernel,
    sys: &Ctx,
    tx: &mut Tx,
    channel: Channel,
    template: &str,
    contact: Uuid,
    phone: Option<&str>,
    record: Option<Value>,
) -> Result<(Option<Uuid>, &'static str, Option<String>)> {
    let Some(phone) = phone.map(str::trim).filter(|p| !p.is_empty()) else { return Ok((None, "Skipped", Some("no phone number".into()))) };
    let m = Message {
        template: template.to_string(),
        entity: Some("MarketingContact".into()),
        record: Some(contact),
        to: vec![Recipient { phone: Some(phone.to_string()), ..Default::default() }],
        channels: vec![channel.platform().to_string()],
        values: record.map(|r| serde_json::Map::from_iter([("record".to_string(), r)])).unwrap_or_default(),
        ..Default::default()
    };
    let sent = messages::send(k, tx, sys, &m).await?;
    let Some(first) = sent.first() else { return Ok((None, "Skipped", Some("no recipient".into()))) };
    Ok(match first.state.as_str() {
        "skipped" => (Some(first.id), "Skipped", first.reason.clone()),
        "failed" => (Some(first.id), "Failed", first.reason.clone()),
        _ => (Some(first.id), "Queued", None),
    })
}

/// Text and chat-app recipients from the platform's delivery log: sent, delivered, failed or skipped (an opt-out, a
/// limit, quiet hours that outlasted nothing), moving forward only.
pub async fn sync(k: &Kernel, sys: &Ctx) -> Result<u64> {
    let mut tx = k.db.tenant_tx(sys.tenant).await?;
    let n = sqlx::query(
        "update e_campaign_recipient r set status = case m.state when 'failed' then 'Failed' when 'skipped' then 'Skipped' else 'Sent' end, \
         sent_at = coalesce(r.sent_at, case when m.state in ('sent', 'delivered') then m.updated_at end), \
         delivered_at = coalesce(r.delivered_at, case when m.state = 'delivered' then m.updated_at end), \
         error = case when m.state = 'failed' then left(m.error, 500) else r.error end, \
         skip_reason = case when m.state = 'skipped' then left(m.reason, 500) else r.skip_reason end, version = r.version + 1, updated_at = now() \
         from message_send m where r.tenant_id = $1 and r.message_send is not null and m.tenant_id = r.tenant_id and m.id = r.message_send::uuid \
         and ((r.status = 'Queued' and m.state in ('sent', 'delivered', 'failed', 'skipped')) or (r.status = 'Sent' and m.state = 'delivered' and r.delivered_at is null))",
    )
    .bind(sys.tenant)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(n)
}

/// The recipient row of a text or chat-app message.
pub async fn record(tx: &mut Tx, tenant: Uuid, recipient: Uuid, channel: Channel, outcome: &(Option<Uuid>, &'static str, Option<String>)) -> Result<()> {
    sqlx::query(
        "update e_campaign_recipient set channel = $3, message_send = $4, status = $5, skip_reason = case when $5 = 'Skipped' then $6 end, \
         error = case when $5 = 'Failed' then $6 end, queued_at = case when $5 = 'Queued' then coalesce(queued_at, now()) end, updated_at = now() \
         where tenant_id = $1 and id = $2",
    )
    .bind(tenant)
    .bind(recipient)
    .bind(channel.label())
    .bind(outcome.0.map(|u| u.to_string()))
    .bind(outcome.1)
    .bind(outcome.2.as_deref().map(|w| util::cut(w, 500)))
    .execute(&mut **tx)
    .await?;
    Ok(())
}
