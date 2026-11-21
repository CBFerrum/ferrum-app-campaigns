//! One composed message to named people, sent once under a topic: the operation another app (Intranet's newsletter)
//! calls through the gate. It makes a fixed-membership segment of the addresses that are contacts and may be mailed under
//! the topic (consent, suppression, the topic's own choices: the same condition a campaign's start uses), a campaign to it,
//! and submits that campaign; the upkeep then sends it like any other, within the limits and with the unsubscribe link.
//! It never makes a contact or a consent: an address that is not a contact, or may not be mailed, is counted as skipped.

use crate::topics::Reach;
use crate::util::{self, html_escape, sys, values};
use async_trait::async_trait;
use ferrum_kernel::{Doc, DocHook, Error, HookCtx, Result};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::BTreeSet;
use uuid::Uuid;

const MAX_ADDRESSES: usize = 5000;
const MAX_BODY: usize = 1_000_000;

pub struct TopicHook;

#[async_trait]
impl DocHook for TopicHook {
    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        if name != "send_issue" {
            return Ok(None);
        }
        Ok(Some(send_issue(hc, doc, params).await?))
    }
}

/// Plain text or Markdown paragraphs as email HTML: escaped (template braces too, so the text can never be a template),
/// a blank line between paragraphs, a line break at each newline.
fn body_html(body: &str) -> String {
    body.replace("\r\n", "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", html_escape(p).replace('{', "&#123;").replace('\n', "<br>")))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn send_issue(hc: &mut HookCtx<'_>, topic: &Doc, params: &Value) -> Result<Value> {
    let tenant = hc.ctx.tenant;
    let topic_id = util::doc_id(topic)?;
    let engine = sys(hc.ctx);
    let text = |k: &str| params.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    let reference =
        text("reference").filter(|r| r.chars().count() <= 200).ok_or_else(|| Error::field("reference", "name this issue (at most 200 characters)"))?;
    let subject = text("subject").filter(|v| v.chars().count() <= 998).ok_or_else(|| Error::field("subject", "give the subject (at most 998 characters)"))?;
    let body = text("body").filter(|b| b.len() <= MAX_BODY).ok_or_else(|| Error::field("body", "give the text (at most 1 MB)"))?;
    let addresses: BTreeSet<String> =
        text("recipients").unwrap_or_default().split(['\n', ',', ';']).map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty()).collect();
    if addresses.is_empty() || addresses.len() > MAX_ADDRESSES {
        return Err(Error::field("recipients", format!("give 1 to {MAX_ADDRESSES} addresses, one a line")));
    }
    if let Some(bad) = addresses.iter().find(|a| !a.contains('@') || a.contains(char::is_whitespace) || a.len() > 254) {
        return Err(Error::field("recipients", format!("{bad} is not an email address")));
    }
    // At most once per topic and reference: the row is made first, under the topic's key; a repeat finds it.
    let fresh = sqlx::query("insert into campaigns_issue (tenant_id, topic_id, reference) values ($1, $2, $3) on conflict do nothing")
        .bind(tenant)
        .bind(topic_id)
        .bind(reference)
        .execute(&mut **hc.tx)
        .await?
        .rows_affected();
    if fresh == 0 {
        let (campaign, accepted, skipped): (Option<Uuid>, i32, i32) =
            sqlx::query_as("select campaign_id, accepted, skipped from campaigns_issue where tenant_id = $1 and topic_id = $2 and reference = $3")
                .bind(tenant)
                .bind(topic_id)
                .bind(reference)
                .fetch_one(&mut **hc.tx)
                .await?;
        return Ok(json!({"campaign": campaign, "accepted": accepted, "skipped": skipped, "repeated": true}));
    }
    // A topic that is gone or turned off is refused here, as a campaign's start refuses it.
    let reach = Reach::of(hc.tx, tenant, Some(topic_id)).await?;
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select c.id from e_marketing_contact c where c.tenant_id = ");
    qb.push_bind(tenant).push(" and c.email = any(").push_bind(addresses.iter().cloned().collect::<Vec<_>>()).push(") and ");
    reach.push(&mut qb);
    let ids: Vec<Uuid> = qb.build_query_scalar().fetch_all(&mut **hc.tx).await?;
    let (accepted, skipped) = (ids.len() as i32, (addresses.len() - ids.len()) as i32);
    let mut campaign: Option<Uuid> = None;
    if !ids.is_empty() {
        let name = util::cut(&format!("Issue {}: {reference}", util::s(topic, "topic_name").unwrap_or_default()), 140);
        let segment = util::doc_id(
            &hc.kernel
                .insert_tx(
                    &engine,
                    hc.tx,
                    "MarketingSegment",
                    values([("segment_name", json!(name)), ("source", json!("List")), ("refresh_minutes", json!(0))]),
                )
                .await?,
        )?;
        sqlx::query(
            "insert into campaigns_segment_member (tenant_id, segment_id, contact_id) select $1, $2, x from unnest($3::uuid[]) x on conflict do nothing",
        )
        .bind(tenant)
        .bind(segment)
        .bind(&ids)
        .execute(&mut **hc.tx)
        .await?;
        let made = hc
            .kernel
            .insert_tx(
                &engine,
                hc.tx,
                "EmailCampaign",
                values([
                    ("campaign_name", json!(name)),
                    ("subject", json!(subject)),
                    ("content", json!(body_html(body))),
                    ("topic", json!(topic_id.to_string())),
                    ("audience", json!([{"segment": segment.to_string()}])),
                ]),
            )
            .await?;
        let id = util::doc_id(&made)?;
        hc.kernel.submit_tx(&engine, hc.tx, "EmailCampaign", id, None).await?;
        campaign = Some(id);
    }
    sqlx::query("update campaigns_issue set campaign_id = $4, accepted = $5, skipped = $6 where tenant_id = $1 and topic_id = $2 and reference = $3")
        .bind(tenant)
        .bind(topic_id)
        .bind(reference)
        .bind(campaign)
        .bind(accepted)
        .bind(skipped)
        .execute(&mut **hc.tx)
        .await?;
    Ok(json!({"campaign": campaign, "accepted": accepted, "skipped": skipped}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_becomes_escaped_paragraphs() {
        assert_eq!(
            body_html("Hi <b>all</b>\n\n{{ x }} & more\nsecond line"),
            "<p>Hi &lt;b&gt;all&lt;/b&gt;</p>\n<p>&#123;&#123; x }} &amp; more<br>second line</p>"
        );
    }
}
