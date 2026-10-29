//! Segments: rules over a contact's whitelisted fields, its attributes, its email group
//! memberships and its engagement, compiled to one parameterized SQL condition on the contact
//! row (alias `c`). Every column and operator comes from a fixed table keyed by the rule's select
//! values; attribute keys, values, groups and day counts are bound parameters — nothing a user
//! writes ever becomes SQL text.

use crate::contact::{self, MAILABLE};
use crate::util::{self, id, rows, s, sys, values};
use async_trait::async_trait;
use chrono::{Duration, NaiveDate};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, Result, Tx};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::str::FromStr;
use uuid::Uuid;

/// Contact fields rules may use, by their option label: the SQL column of each.
const FIELDS: &[(&str, &str)] = &[
    ("Email", "c.email"),
    ("First Name", "c.first_name"),
    ("Last Name", "c.last_name"),
    ("Full Name", "c.full_name"),
    ("Source", "c.source"),
    ("Consent Status", "c.consent_status"),
    ("Linked Entity", "c.record_entity"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOp {
    Equals,
    NotEquals,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    IsEmpty,
    IsNotEmpty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    Column(&'static str),
    Attribute(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOp {
    Before(NaiveDate),
    After(NaiveDate),
    WithinDays(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Opened,
    Clicked,
    Sent,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    Text(Subject, TextOp, String),
    /// Attribute greater than (`true`) or less than a number.
    Number(String, bool, Decimal),
    /// `None`: the contact's creation date; `Some(key)`: a date attribute (YYYY-MM-DD).
    Date(Option<String>, DateOp),
    Group(Uuid, bool),
    Engagement {
        activity: Activity,
        did: bool,
        days: i32,
        campaign: Option<Uuid>,
    },
}

/// A segment ready to filter contacts.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub all: bool,
    pub rules: Vec<Rule>,
}

fn text_op(op: &str) -> Option<TextOp> {
    Some(match op {
        "Equals" => TextOp::Equals,
        "Not Equals" => TextOp::NotEquals,
        "Contains" => TextOp::Contains,
        "Not Contains" => TextOp::NotContains,
        "Starts With" => TextOp::StartsWith,
        "Ends With" => TextOp::EndsWith,
        "Is Empty" => TextOp::IsEmpty,
        "Is Not Empty" => TextOp::IsNotEmpty,
        _ => return None,
    })
}

fn date_op(op: &str, value: Option<&str>) -> std::result::Result<Option<DateOp>, String> {
    let date = || value.and_then(|v| NaiveDate::parse_from_str(v.trim(), "%Y-%m-%d").ok()).ok_or_else(|| "enter a date as YYYY-MM-DD".to_string());
    Ok(Some(match op {
        "Before" => DateOp::Before(date()?),
        "After" => DateOp::After(date()?),
        "Within Last Days" => DateOp::WithinDays(
            value
                .and_then(|v| v.trim().parse::<i32>().ok())
                .filter(|d| (1..=36_500).contains(d))
                .ok_or_else(|| "enter a number of days (1 to 36500)".to_string())?,
        ),
        _ => return Ok(None),
    }))
}

/// Parses one rule row; the message says what is wrong with it.
pub fn parse_rule(r: &Doc) -> std::result::Result<Rule, (String, String)> {
    let err = |f: &str, m: &str| Err((f.to_string(), m.to_string()));
    let op = s(r, "operator").unwrap_or_default();
    let value = s(r, "value");
    let needs_value = |op: TextOp| !matches!(op, TextOp::IsEmpty | TextOp::IsNotEmpty);
    match s(r, "rule_type").unwrap_or("Contact Field") {
        "Contact Field" => {
            let Some(field) = s(r, "field") else { return err("field", "choose a contact field") };
            if field == "Created On" {
                return match date_op(op, value) {
                    Ok(Some(d)) => Ok(Rule::Date(None, d)),
                    Ok(None) => err("operator", "use Before, After or Within Last Days with Created On"),
                    Err(m) => err("value", &m),
                };
            }
            let Some((_, col)) = FIELDS.iter().find(|(label, _)| *label == field) else { return err("field", "unknown contact field") };
            match text_op(op) {
                Some(t) if needs_value(t) && value.is_none() => err("value", "enter a value"),
                Some(t) => Ok(Rule::Text(Subject::Column(col), t, value.unwrap_or_default().to_string())),
                None => err("operator", "use a text comparison with contact fields"),
            }
        }
        "Attribute" => {
            let Some(key) = s(r, "attribute").map(str::trim).filter(|k| contact::valid_key(k)) else {
                return err("attribute", "enter an attribute key (lowercase letters, digits and _)");
            };
            if let Some(t) = text_op(op) {
                return if needs_value(t) && value.is_none() {
                    err("value", "enter a value")
                } else {
                    Ok(Rule::Text(Subject::Attribute(key.into()), t, value.unwrap_or_default().into()))
                };
            }
            if matches!(op, "Greater Than" | "Less Than") {
                return match value.and_then(|v| Decimal::from_str(v.trim()).ok()) {
                    Some(n) => Ok(Rule::Number(key.into(), op == "Greater Than", n)),
                    None => err("value", "enter a number"),
                };
            }
            match date_op(op, value) {
                Ok(Some(d)) => Ok(Rule::Date(Some(key.into()), d)),
                Ok(None) => err("operator", "this operator does not apply to attributes"),
                Err(m) => err("value", &m),
            }
        }
        "Email Group" => match (id(r, "email_group"), op) {
            (None, _) => err("email_group", "choose an email group"),
            (Some(g), "Is Member") => Ok(Rule::Group(g, true)),
            (Some(g), "Is Not Member") => Ok(Rule::Group(g, false)),
            _ => err("operator", "use Is Member or Is Not Member with email groups"),
        },
        "Engagement" => {
            let activity = match s(r, "activity") {
                Some("Opened") => Activity::Opened,
                Some("Clicked") => Activity::Clicked,
                Some("Was Sent") => Activity::Sent,
                _ => return err("activity", "choose Opened, Clicked or Was Sent"),
            };
            let did = match op {
                "Did" => true,
                "Did Not" => false,
                _ => return err("operator", "use Did or Did Not with engagement"),
            };
            let days = r.get("within_days").and_then(Value::as_i64).filter(|d| (1..=3650).contains(d));
            let Some(days) = days else { return err("within_days", "enter the number of days (1 to 3650)") };
            Ok(Rule::Engagement { activity, did, days: days as i32, campaign: id(r, "campaign") })
        }
        _ => err("rule_type", "unknown rule type"),
    }
}

/// Parses a segment document; errors point at the offending rows.
pub fn parse(doc: &Doc) -> std::result::Result<Segment, Vec<FieldError>> {
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for (i, r) in rows(doc, "rules").iter().enumerate() {
        match parse_rule(r) {
            Ok(rule) => rules.push(rule),
            Err((f, m)) => errors.push(FieldError::new(f, m).in_row("rules", i)),
        }
    }
    if errors.is_empty() { Ok(Segment { all: s(doc, "match_type") != Some("Any"), rules }) } else { Err(errors) }
}

/// `%`, `_` and `\` match themselves in a LIKE pattern.
fn like_escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn push_subject(qb: &mut QueryBuilder<Postgres>, subject: &Subject) {
    match subject {
        Subject::Column(col) => {
            qb.push(*col);
        }
        Subject::Attribute(key) => {
            qb.push("(c.attributes ->> ").push_bind(key.clone()).push(")");
        }
    }
}

impl Segment {
    /// Appends the condition, in parentheses. `today` and `tz` are the workspace's (dates in
    /// rules are local dates).
    pub fn push(&self, qb: &mut QueryBuilder<Postgres>, today: NaiveDate, tz: &str) {
        if self.rules.is_empty() {
            qb.push("true");
            return;
        }
        qb.push("(");
        for (i, rule) in self.rules.iter().enumerate() {
            if i > 0 {
                qb.push(if self.all { " and " } else { " or " });
            }
            qb.push("(");
            push_rule(qb, rule, today, tz);
            qb.push(")");
        }
        qb.push(")");
    }
}

fn push_rule(qb: &mut QueryBuilder<Postgres>, rule: &Rule, today: NaiveDate, tz: &str) {
    match rule {
        Rule::Text(subject, op, value) => match op {
            TextOp::Equals | TextOp::NotEquals => {
                qb.push("lower(coalesce(");
                push_subject(qb, subject);
                qb.push(", '')) ").push(if *op == TextOp::Equals { "=" } else { "<>" }).push(" lower(").push_bind(value.clone()).push(")");
            }
            TextOp::Contains | TextOp::NotContains | TextOp::StartsWith | TextOp::EndsWith => {
                let v = like_escape(value);
                let p = match op {
                    TextOp::StartsWith => format!("{v}%"),
                    TextOp::EndsWith => format!("%{v}"),
                    _ => format!("%{v}%"),
                };
                qb.push("coalesce(");
                push_subject(qb, subject);
                qb.push(", '') ").push(if *op == TextOp::NotContains { "not ilike " } else { "ilike " }).push_bind(p);
            }
            TextOp::IsEmpty | TextOp::IsNotEmpty => {
                qb.push("coalesce(");
                push_subject(qb, subject);
                qb.push(", '') ").push(if *op == TextOp::IsEmpty { "= ''" } else { "<> ''" });
            }
        },
        Rule::Number(key, greater, n) => {
            let subject = Subject::Attribute(key.clone());
            qb.push("(case when btrim(coalesce(");
            push_subject(qb, &subject);
            qb.push(", '')) ~ '^-?[0-9]+(\\.[0-9]+)?$' then btrim(");
            push_subject(qb, &subject);
            qb.push(")::numeric end) ").push(if *greater { "> " } else { "< " }).push_bind(*n);
        }
        Rule::Date(None, op) => match op {
            DateOp::Before(d) | DateOp::After(d) => {
                qb.push("(c.created_at at time zone ").push_bind(tz.to_string()).push(")::date ");
                qb.push(if matches!(op, DateOp::Before(_)) { "< " } else { "> " }).push_bind(*d);
            }
            DateOp::WithinDays(n) => {
                qb.push("c.created_at >= now() - make_interval(days => ").push_bind(*n).push(")");
            }
        },
        Rule::Date(Some(key), op) => {
            // ISO dates compare as text; anything else in the attribute never matches.
            let subject = Subject::Attribute(key.clone());
            qb.push("(case when coalesce(");
            push_subject(qb, &subject);
            qb.push(", '') ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}' then left(");
            push_subject(qb, &subject);
            qb.push(", 10) end) ");
            match op {
                DateOp::Before(d) => {
                    qb.push("< ").push_bind(d.to_string());
                }
                DateOp::After(d) => {
                    qb.push("> ").push_bind(d.to_string());
                }
                DateOp::WithinDays(n) => {
                    qb.push("between ").push_bind((today - Duration::days(i64::from(*n))).to_string()).push(" and ").push_bind(today.to_string());
                }
            }
        }
        Rule::Group(group, member) => {
            qb.push(if *member { "exists" } else { "not exists" });
            qb.push(" (select 1 from e_email_group_member m where m.tenant_id = c.tenant_id and lower(m.email) = c.email and not m.unsubscribed and m.email_group = ")
                .push_bind(*group)
                .push(")");
        }
        Rule::Engagement { activity, did, days, campaign } => {
            let col = match activity {
                Activity::Opened => "r.last_opened_at",
                Activity::Clicked => "r.last_clicked_at",
                Activity::Sent => "r.sent_at",
            };
            qb.push(if *did { "exists" } else { "not exists" });
            qb.push(format!(
                " (select 1 from e_campaign_recipient r where r.tenant_id = c.tenant_id and r.contact = c.id and {col} >= now() - make_interval(days => "
            ))
            .push_bind(*days)
            .push(")");
            if let Some(cid) = campaign {
                qb.push(" and r.campaign = ").push_bind(*cid);
            }
            qb.push(")");
        }
    }
}

/// Loads and parses a stored segment.
pub async fn load(k: &Kernel, ctx: &Ctx, tx: &mut Tx, segment: Uuid) -> Result<(String, Segment)> {
    let doc = k.get_tx(&sys(ctx), tx, "MarketingSegment", segment, false).await?;
    let name = s(&doc, "segment_name").unwrap_or_default().to_string();
    let parsed = parse(&doc).map_err(|_| Error::invalid(format!("the segment {name} has invalid rules; fix them first")))?;
    Ok((name, parsed))
}

/// `(contacts, mailable)` in a segment, and the first contacts by address.
pub async fn count(tx: &mut Tx, ctx: &Ctx, seg: &Segment, sample: i64) -> Result<(i64, i64, Vec<Value>)> {
    let today = ctx.today();
    let tz = ctx.schema.timezone.clone();
    let mut qb: QueryBuilder<Postgres> =
        QueryBuilder::new(format!("select count(*), count(*) filter (where {MAILABLE}) from e_marketing_contact c where c.tenant_id = "));
    qb.push_bind(ctx.tenant).push(" and ");
    seg.push(&mut qb, today, &tz);
    let (total, mailable): (i64, i64) = qb.build_query_as().fetch_one(&mut **tx).await?;
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(format!(
        "select jsonb_build_object('id', c.id, 'email', c.email, 'full_name', c.full_name, 'consent_status', c.consent_status, 'mailable', {MAILABLE}) \
         from e_marketing_contact c where c.tenant_id = "
    ));
    qb.push_bind(ctx.tenant).push(" and ");
    seg.push(&mut qb, today, &tz);
    qb.push(" order by c.email limit ").push_bind(sample);
    let rows: Vec<Value> = qb.build_query_scalar().fetch_all(&mut **tx).await?;
    Ok((total, mailable, rows))
}

pub struct SegmentHook;

#[async_trait]
impl DocHook for SegmentHook {
    async fn validate(&self, _hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        parse(doc).map(|_| ()).map_err(Error::Validation)
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, _params: &Value) -> Result<Option<Value>> {
        if name != "preview" {
            return Ok(None);
        }
        let seg = parse(doc).map_err(Error::Validation)?;
        let (total, mailable, sample) = count(hc.tx, hc.ctx, &seg, 20).await?;
        let counted_at = util::ts(util::now());
        let v = values([("contact_count", json!(total)), ("mailable_count", json!(mailable)), ("counted_at", json!(counted_at))]);
        hc.kernel.set_values_tx(hc.ctx, hc.tx, "MarketingSegment", util::doc_id(doc)?, v).await?;
        Ok(Some(json!({"contact_count": total, "mailable_count": mailable, "counted_at": counted_at, "sample": sample})))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(v: Value) -> Doc {
        v.as_object().cloned().unwrap()
    }

    fn sql(seg: &Segment) -> String {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select 1 from e_marketing_contact c where ");
        seg.push(&mut qb, NaiveDate::from_ymd_opt(2026, 3, 10).unwrap(), "Asia/Kolkata");
        qb.sql().as_str().to_string()
    }

    #[test]
    fn rules_parse_with_their_own_errors() {
        let ok = |v: Value| parse_rule(&row(v)).unwrap();
        let bad = |v: Value| parse_rule(&row(v)).unwrap_err().0;
        assert_eq!(
            ok(json!({"rule_type": "Contact Field", "field": "Email", "operator": "Ends With", "value": "@x.test"})),
            Rule::Text(Subject::Column("c.email"), TextOp::EndsWith, "@x.test".into())
        );
        assert_eq!(
            ok(json!({"rule_type": "Attribute", "attribute": "score", "operator": "Greater Than", "value": "9.5"})),
            Rule::Number("score".into(), true, Decimal::from_str("9.5").unwrap())
        );
        assert_eq!(
            ok(json!({"rule_type": "Contact Field", "field": "Created On", "operator": "Within Last Days", "value": "30"})),
            Rule::Date(None, DateOp::WithinDays(30))
        );
        assert_eq!(
            bad(json!({"rule_type": "Attribute", "attribute": "x'); drop table e_marketing_contact; --", "operator": "Equals", "value": "1"})),
            "attribute"
        );
        assert_eq!(bad(json!({"rule_type": "Contact Field", "field": "Email", "operator": "Greater Than", "value": "1"})), "operator");
        assert_eq!(bad(json!({"rule_type": "Contact Field", "field": "Email", "operator": "Equals"})), "value");
        assert_eq!(bad(json!({"rule_type": "Attribute", "attribute": "born", "operator": "Before", "value": "1/2/2020"})), "value");
        assert_eq!(bad(json!({"rule_type": "Email Group", "operator": "Is Member"})), "email_group");
        assert_eq!(bad(json!({"rule_type": "Engagement", "activity": "Opened", "operator": "Did"})), "within_days");
        assert_eq!(bad(json!({"rule_type": "Engagement", "activity": "Opened", "operator": "Equals", "within_days": 3})), "operator");
    }

    #[test]
    fn values_are_bound_never_spliced() {
        let evil = "x' or '1'='1'); drop table e_marketing_contact; --";
        let seg = Segment {
            all: false,
            rules: vec![
                Rule::Text(Subject::Attribute(evil.into()), TextOp::Contains, evil.into()),
                Rule::Text(Subject::Column("c.first_name"), TextOp::Equals, evil.into()),
                Rule::Number("score".into(), false, Decimal::ONE),
                Rule::Date(Some("born".into()), DateOp::WithinDays(7)),
                Rule::Group(Uuid::nil(), false),
                Rule::Engagement { activity: Activity::Clicked, did: true, days: 30, campaign: Some(Uuid::nil()) },
            ],
        };
        let text = sql(&seg);
        assert!(!text.contains("drop table") && !text.contains("'1'='1'"), "{text}");
        assert!(text.contains("(c.attributes ->> $1)") && text.contains(" or "), "{text}");
        assert_eq!(text.matches('$').count(), 14, "{text}");
        assert_eq!(sql(&Segment { all: true, rules: vec![] }), "select 1 from e_marketing_contact c where true");
        assert_eq!(like_escape("50%_off\\"), "50\\%\\_off\\\\");
    }
}
