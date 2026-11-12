//! Segments: rules over a contact's whitelisted fields, its attributes, its email group
//! memberships and its engagement, compiled to one parameterized SQL condition on the contact
//! row (alias `c`). Every column and operator comes from a fixed table keyed by the rule's select
//! values; attribute keys, values, groups and day counts are bound parameters — nothing a user
//! writes ever becomes SQL text.

use crate::contact::{self, MAILABLE};
use crate::util::{self, id, rows, s, sys, values};
use async_trait::async_trait;
use chrono::{Duration, NaiveDate};
use ferrum_kernel::store::{entity_predicate, field_expr};
use ferrum_kernel::{Ctx, Doc, DocHook, Error, FieldError, HookCtx, Kernel, ListQuery, Result, Tx};
use ferrum_meta::{FieldType, Schema};
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

/// A comparison of a record's field (or an aggregate), by what the value is.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmp {
    Text(TextOp, String),
    /// Greater than (`Some(true)`), less than (`Some(false)`) or equal to (`None`) a number.
    Number(Option<bool>, Decimal),
    Date(DateOp),
}

/// How records of another entity name the person (SQL over the record `r` and the contact `c`).
#[derive(Debug, Clone, PartialEq)]
pub enum Person {
    /// An email field.
    Email(String),
    /// A link to a Marketing Contact.
    Contact(String),
    /// A link to the record contacts came from (its entity, the link expression).
    Record(String, String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Measure {
    Count,
    Sum(String),
    Latest(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// A field of the contact (`entity` None) or of the record it came from. SQL fragments come from the schema
    /// (table, its record predicate, the field's typed expression on `r`); values are bound.
    Field {
        entity: Option<String>,
        table: String,
        pred: String,
        expr: String,
        cmp: Cmp,
    },
    /// An aggregate over records of another entity that name the person.
    Related {
        table: String,
        pred: String,
        submitted: bool,
        person: Person,
        measure: Measure,
        period: Option<(String, i32)>,
        matching: Option<(String, String)>,
        cmp: Cmp,
    },
    /// The members of a segment worked out from a query (`campaigns_segment_member`).
    Members(Uuid),
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
pub fn parse_rule(r: &Doc, schema: &Schema) -> std::result::Result<Rule, (String, String)> {
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
        "Record Field" => {
            let Some(ent) = s(r, "entity_name").and_then(|e| schema.entity(e)) else { return err("entity_name", "name an entity of this workspace") };
            let Some(field) = s(r, "field_name").and_then(|f| ent.field(f)) else { return err("field_name", "name a field of that entity") };
            let Some(expr) = field_expr(ent, &field.name, "r") else { return err("field_name", "a table cannot be compared") };
            let cmp = compare(field.ftype, op, value)?;
            let entity = (ent.def.name != "MarketingContact").then(|| ent.def.name.clone());
            Ok(Rule::Field { entity, table: ent.table.clone(), pred: entity_predicate(ent, "r"), expr, cmp })
        }
        "Related Records" => {
            let Some(ent) = s(r, "entity_name").and_then(|e| schema.entity(e)).filter(|e| !e.table.is_empty()) else {
                return err("entity_name", "name an entity of this workspace");
            };
            let typed = |k: &str| s(r, k).and_then(|f| ent.field(f)).and_then(|f| Some((f.ftype, field_expr(ent, &f.name, "r")?)));
            let person = match (s(r, "link_field").and_then(|f| ent.field(f)), typed("link_field")) {
                (Some(f), Some((FieldType::Email, e))) if f.ftype == FieldType::Email => Person::Email(e),
                (Some(f), Some((FieldType::Link, e))) if f.to.as_deref() == Some("MarketingContact") => Person::Contact(e),
                (Some(f), Some((FieldType::Link, e))) => Person::Record(f.to.clone().unwrap_or_default(), e),
                _ => return err("link_field", "name the field that names the person: an email field or a link"),
            };
            let measure = match s(r, "aggregate").unwrap_or("Count") {
                "Count" => Measure::Count,
                agg => match typed("measure_field") {
                    Some((t, e)) if agg == "Sum" && matches!(t, FieldType::Int | FieldType::Decimal | FieldType::Currency | FieldType::Percent) => {
                        Measure::Sum(e)
                    }
                    Some((t, e)) if agg == "Latest" && matches!(t, FieldType::Date | FieldType::Datetime) => Measure::Latest(e),
                    _ => return err("measure_field", "name a number field to sum, or a date field for the latest"),
                },
            };
            let cmp = compare(if matches!(measure, Measure::Latest(_)) { FieldType::Date } else { FieldType::Decimal }, op, value)?;
            let period = match (s(r, "date_field"), r.get("within_days").and_then(Value::as_i64)) {
                (None, _) => None,
                (Some(_), days) => match (typed("date_field"), days.filter(|d| (1..=36_500).contains(d))) {
                    (Some((FieldType::Date | FieldType::Datetime, e)), Some(d)) => Some((e, d as i32)),
                    (_, Some(_)) => return err("date_field", "name a date field of that entity"),
                    (_, None) => return err("within_days", "enter the number of days (1 to 36500)"),
                },
            };
            let matching = match (s(r, "match_field"), typed("match_field")) {
                (None, _) => None,
                (Some(_), Some((_, e))) => Some((e, s(r, "match_value").unwrap_or_default().to_string())),
                (Some(_), None) => return err("match_field", "name a field of that entity"),
            };
            Ok(Rule::Related {
                table: ent.table.clone(),
                pred: entity_predicate(ent, "r"),
                submitted: ent.def.submittable,
                person,
                measure,
                period,
                matching,
                cmp,
            })
        }
        _ => err("rule_type", "unknown rule type"),
    }
}

/// The comparison a rule's operator and value make on a value of type `t`.
fn compare(t: FieldType, op: &str, value: Option<&str>) -> std::result::Result<Cmp, (String, String)> {
    let err = |f: &str, m: &str| Err((f.to_string(), m.to_string()));
    match t {
        FieldType::Int | FieldType::Decimal | FieldType::Currency | FieldType::Percent => {
            let greater = match op {
                "Greater Than" => Some(true),
                "Less Than" => Some(false),
                "Equals" => None,
                _ => return err("operator", "use Greater Than, Less Than or Equals with numbers"),
            };
            match value.and_then(|v| Decimal::from_str(v.trim()).ok()) {
                Some(n) => Ok(Cmp::Number(greater, n)),
                None => err("value", "enter a number"),
            }
        }
        FieldType::Date | FieldType::Datetime => match date_op(op, value) {
            Ok(Some(d)) => Ok(Cmp::Date(d)),
            Ok(None) => err("operator", "use Before, After or Within Last Days with dates"),
            Err(m) => err("value", &m),
        },
        _ => match text_op(op) {
            Some(t) if !matches!(t, TextOp::IsEmpty | TextOp::IsNotEmpty) && value.is_none() => err("value", "enter a value"),
            Some(t) => Ok(Cmp::Text(t, value.unwrap_or_default().to_string())),
            None => err("operator", "use a text comparison with this field"),
        },
    }
}

/// Parses a segment document; errors point at the offending rows.
pub fn parse(doc: &Doc, schema: &Schema) -> std::result::Result<Segment, Vec<FieldError>> {
    if s(doc, "source") == Some("Query") {
        return match (id(doc, "id"), query_of(doc, schema)) {
            (_, Err(m)) => Err(vec![FieldError::new("query", m)]),
            (Some(sid), Ok(_)) => Ok(Segment { all: true, rules: vec![Rule::Members(sid)] }),
            // Not saved yet: nobody, until it is worked out.
            (None, Ok(_)) => Ok(Segment { all: true, rules: vec![Rule::Members(Uuid::nil())] }),
        };
    }
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    for (i, r) in rows(doc, "rules").iter().enumerate() {
        match parse_rule(r, schema) {
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

/// `<subject> <cmp>`, the subject pushed by `subject` (it may bind values of its own).
fn push_cmp(qb: &mut QueryBuilder<Postgres>, subject: &dyn Fn(&mut QueryBuilder<Postgres>), cmp: &Cmp, today: NaiveDate) {
    match cmp {
        Cmp::Text(op, value) => {
            let (open, close) = match op {
                TextOp::Equals | TextOp::NotEquals => ("lower(coalesce((", ")::text, ''))"),
                _ => ("coalesce((", ")::text, '')"),
            };
            qb.push(open);
            subject(qb);
            qb.push(close);
            match op {
                TextOp::Equals | TextOp::NotEquals => {
                    qb.push(if *op == TextOp::Equals { " = lower(" } else { " <> lower(" }).push_bind(value.clone()).push(")");
                }
                TextOp::IsEmpty => {
                    qb.push(" = ''");
                }
                TextOp::IsNotEmpty => {
                    qb.push(" <> ''");
                }
                _ => {
                    let v = like_escape(value);
                    let p = match op {
                        TextOp::StartsWith => format!("{v}%"),
                        TextOp::EndsWith => format!("%{v}"),
                        _ => format!("%{v}%"),
                    };
                    qb.push(if *op == TextOp::NotContains { " not ilike " } else { " ilike " }).push_bind(p);
                }
            }
        }
        Cmp::Number(greater, n) => {
            qb.push("(");
            subject(qb);
            qb.push(")::numeric ").push(match greater {
                Some(true) => "> ",
                Some(false) => "< ",
                None => "= ",
            });
            qb.push_bind(*n);
        }
        Cmp::Date(op) => {
            qb.push("(");
            subject(qb);
            qb.push(")::date ");
            match op {
                DateOp::Before(d) => {
                    qb.push("< ").push_bind(*d);
                }
                DateOp::After(d) => {
                    qb.push("> ").push_bind(*d);
                }
                DateOp::WithinDays(n) => {
                    qb.push("between ").push_bind(today - Duration::days(i64::from(*n))).push(" and ").push_bind(today);
                }
            }
        }
    }
}

fn push_rule(qb: &mut QueryBuilder<Postgres>, rule: &Rule, today: NaiveDate, tz: &str) {
    match rule {
        Rule::Field { entity, table, pred, expr, cmp } => {
            qb.push(format!("exists (select 1 from {table} r where r.tenant_id = c.tenant_id{pred} and "));
            match entity {
                None => qb.push("r.id = c.id"),
                Some(e) => qb.push("c.record_entity = ").push_bind(e.clone()).push(" and r.id::text = c.record_id"),
            };
            qb.push(" and ");
            push_cmp(
                qb,
                &|qb| {
                    qb.push(expr.as_str());
                },
                cmp,
                today,
            );
            qb.push(")");
        }
        Rule::Related { table, pred, submitted, person, measure, period, matching, cmp } => {
            let subject = |qb: &mut QueryBuilder<Postgres>| {
                let agg = match measure {
                    Measure::Count => "count(*)".to_string(),
                    Measure::Sum(e) => format!("coalesce(sum({e}), 0)"),
                    Measure::Latest(e) => format!("max({e})"),
                };
                qb.push(format!("(select {agg} from {table} r where r.tenant_id = c.tenant_id{pred}"));
                if *submitted {
                    qb.push(" and r.docstatus = 1");
                }
                match person {
                    Person::Email(e) => qb.push(format!(" and lower({e}) = c.email")),
                    Person::Contact(e) => qb.push(format!(" and {e} = c.id")),
                    Person::Record(entity, e) => qb.push(" and c.record_entity = ").push_bind(entity.clone()).push(format!(" and ({e})::text = c.record_id")),
                };
                if let Some((e, days)) = period {
                    qb.push(format!(" and ({e})::date >= ")).push_bind(today - Duration::days(i64::from(*days)));
                }
                if let Some((e, v)) = matching {
                    qb.push(format!(" and lower(coalesce(({e})::text, '')) = lower(")).push_bind(v.clone()).push(")");
                }
                qb.push(")");
            };
            push_cmp(qb, &subject, cmp, today);
        }
        Rule::Members(seg) => {
            qb.push("exists (select 1 from campaigns_segment_member m where m.tenant_id = c.tenant_id and m.contact_id = c.id and m.segment_id = ")
                .push_bind(*seg)
                .push(")");
        }
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
    let parsed = parse(&doc, &ctx.schema).map_err(|_| Error::invalid(format!("the segment {name} has invalid rules; fix them first")))?;
    Ok((name, parsed))
}

// ───────────────────────────── query segments and membership ─────────────────────────────

/// Rows a query segment reads at most, a page at a time.
const MAX_QUERY_ROWS: usize = 100_000;
const PAGE: u32 = 1000;

/// A query segment's entity and filters: a query of rows (grouping, totals and pivots are refused).
pub fn query_of(doc: &Doc, schema: &Schema) -> std::result::Result<(String, Value), String> {
    let q = doc.get("query").filter(|q| q.is_object()).ok_or("save a query")?;
    let entity = q
        .get("entity")
        .and_then(Value::as_str)
        .filter(|e| schema.entity(e).is_some_and(|e| !e.table.is_empty()))
        .ok_or("the query's entity is not one of this workspace")?;
    if ["group_by", "measures", "pivot", "formulas"].iter().any(|k| q.get(*k).is_some_and(|v| !v.is_null() && v != &json!([]))) {
        return Err("a segment's query lists records: grouping and totals are not people".into());
    }
    let filters = q.get("filters").cloned().unwrap_or_else(|| json!({}));
    ferrum_kernel::query::parse_filters(&filters).map_err(|e| format!("its filters: {e}"))?;
    if let Some(m) = crate::events::check_person_field(schema.entity(entity).ok_or("no entity")?, s(doc, "query_person_field")) {
        return Err(m);
    }
    Ok((entity.to_string(), filters))
}

/// The contacts a query segment's rows name, read as the workspace a page at a time: by an email column, a link to a
/// Marketing Contact, a link to the record a contact came from, or (blank) the rows themselves.
pub async fn query_contacts(k: &Kernel, sys: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<Vec<Uuid>> {
    let (entity, filters) = query_of(doc, &sys.schema).map_err(|m| Error::field("query", m))?;
    let ent = sys.schema.entity(&entity).cloned().ok_or_else(|| Error::field("query", "no entity"))?;
    let person = s(doc, "query_person_field");
    let column = person.unwrap_or("id").to_string();
    let (mut found, mut cursor) = (Vec::<String>::new(), None);
    loop {
        let q = ListQuery {
            filters: ferrum_kernel::query::parse_filters(&filters)?,
            fields: Some(vec!["id".into(), column.clone()]),
            limit: Some(PAGE),
            cursor: cursor.take(),
            ..Default::default()
        };
        let page = k.list(sys, &entity, &q).await?;
        let full = page.rows.len() as u32 == PAGE;
        found.extend(page.rows.iter().filter_map(|r| r.get(&column).and_then(Value::as_str).map(str::to_string)));
        if found.len() > MAX_QUERY_ROWS {
            return Err(Error::field("query", format!("the query finds more than {MAX_QUERY_ROWS} rows: narrow it")));
        }
        match page.next_cursor {
            Some(c) if full => cursor = Some(c),
            _ => break,
        }
    }
    let tenant = sys.tenant;
    let field = person.and_then(|f| ent.field(f));
    let by_id = match field {
        Some(f) => f.ftype == FieldType::Link && f.to.as_deref() == Some("MarketingContact"),
        None => entity == "MarketingContact",
    };
    let ids: Vec<Uuid> = if field.is_some_and(|f| f.ftype == FieldType::Email) {
        let emails: Vec<String> = found.iter().map(|v| v.trim().to_lowercase()).collect();
        sqlx::query_scalar("select id from e_marketing_contact where tenant_id = $1 and email = any($2)").bind(tenant).bind(emails).fetch_all(&mut **tx).await?
    } else if by_id {
        sqlx::query_scalar("select id from e_marketing_contact where tenant_id = $1 and id::text = any($2)")
            .bind(tenant)
            .bind(&found)
            .fetch_all(&mut **tx)
            .await?
    } else {
        let from = field.and_then(|f| f.to.clone()).unwrap_or(entity);
        sqlx::query_scalar("select id from e_marketing_contact where tenant_id = $1 and record_entity = $2 and record_id = any($3)")
            .bind(tenant)
            .bind(from)
            .bind(&found)
            .fetch_all(&mut **tx)
            .await?
    };
    Ok(ids)
}

/// Works a segment's membership out again in the caller's transaction and records who entered and who left it:
/// `(entered, left)`. A query segment's membership is what campaigns and journeys read; a rule segment's is history
/// (its rules are evaluated when it is used).
pub async fn refresh(k: &Kernel, sys: &Ctx, tx: &mut Tx, doc: &Doc) -> Result<(u64, u64)> {
    let sid = util::doc_id(doc)?;
    let tenant = sys.tenant;
    let ids: Vec<Uuid> = if s(doc, "source") == Some("Query") {
        query_contacts(k, sys, tx, doc).await?
    } else {
        let seg = parse(doc, &sys.schema).map_err(Error::Validation)?;
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("select c.id from e_marketing_contact c where c.tenant_id = ");
        qb.push_bind(tenant).push(" and ");
        seg.push(&mut qb, sys.today(), &sys.schema.timezone);
        qb.build_query_scalar().fetch_all(&mut **tx).await?
    };
    let entered = sqlx::query(
        "with ins as (insert into campaigns_segment_member (tenant_id, segment_id, contact_id) select $1, $2, x from unnest($3::uuid[]) x \
         on conflict do nothing returning contact_id) \
         insert into campaigns_segment_change (tenant_id, segment_id, contact_id, change) select $1, $2, contact_id, 'entered' from ins",
    )
    .bind(tenant)
    .bind(sid)
    .bind(&ids)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    let left = sqlx::query(
        "with gone as (delete from campaigns_segment_member m where m.tenant_id = $1 and m.segment_id = $2 and not (m.contact_id = any($3)) \
         returning contact_id) insert into campaigns_segment_change (tenant_id, segment_id, contact_id, change) select $1, $2, contact_id, 'left' from gone",
    )
    .bind(tenant)
    .bind(sid)
    .bind(&ids)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    k.set_values_tx(sys, tx, "MarketingSegment", sid, values([("refreshed_at", json!(util::ts(util::now())))])).await?;
    Ok((entered, left))
}

/// A query segment an audience names is worked out again before it is used.
pub async fn refresh_if_query(k: &Kernel, sys: &Ctx, tx: &mut Tx, segment: Uuid) -> Result<()> {
    let doc = k.get_tx(sys, tx, "MarketingSegment", segment, true).await?;
    if s(&doc, "source") == Some("Query") {
        refresh(k, sys, tx, &doc).await?;
    }
    Ok(())
}

/// Maintenance: the segments whose schedule is due, each in its own transaction; one that fails is said and skipped.
pub async fn run(k: &Kernel, sys: &Ctx) -> Result<usize> {
    let mut tx = k.db.tenant_tx(sys.tenant).await?;
    let due: Vec<Uuid> = sqlx::query_scalar(
        "select id from e_marketing_segment where tenant_id = $1 and coalesce(refresh_minutes, 0) > 0 \
         and (refreshed_at is null or refreshed_at < now() - make_interval(mins => refresh_minutes::int)) order by refreshed_at nulls first, id limit 50",
    )
    .bind(sys.tenant)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let mut done = 0;
    for sid in due {
        let mut tx = k.db.tenant_tx(sys.tenant).await?;
        let doc = k.get_tx(sys, &mut tx, "MarketingSegment", sid, true).await?;
        match refresh(k, sys, &mut tx, &doc).await {
            Ok(_) => {
                tx.commit().await?;
                done += 1;
            }
            Err(e) => {
                tx.rollback().await?;
                tracing::warn!(tenant = %sys.tenant, segment = %sid, "segment not refreshed: {e}");
            }
        }
    }
    Ok(done)
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
    async fn validate(&self, hc: &mut HookCtx<'_>, doc: &Doc) -> Result<()> {
        parse(doc, &hc.ctx.schema).map_err(Error::Validation)?;
        // Who saves a query must be able to read its rows; it is then worked out as the workspace.
        if let (Some("Query"), Ok((entity, filters)), false) = (s(doc, "source"), query_of(doc, &hc.ctx.schema), hc.ctx.is_system) {
            let q = ListQuery { filters: ferrum_kernel::query::parse_filters(&filters)?, limit: Some(1), ..Default::default() };
            if let Err(e) = hc.kernel.list(hc.ctx, &entity, &q).await {
                return Err(Error::field("query", format!("you cannot read these records: {e}")));
            }
        }
        Ok(())
    }

    async fn action(&self, hc: &mut HookCtx<'_>, name: &str, doc: &Doc, params: &Value) -> Result<Option<Value>> {
        match name {
            "preview" => {
                if s(doc, "source") == Some("Query") {
                    refresh(hc.kernel, &sys(hc.ctx), hc.tx, doc).await?;
                }
                let seg = parse(doc, &hc.ctx.schema).map_err(Error::Validation)?;
                let (total, mailable, sample) = count(hc.tx, hc.ctx, &seg, 20).await?;
                let counted_at = util::ts(util::now());
                let v = values([("contact_count", json!(total)), ("mailable_count", json!(mailable)), ("counted_at", json!(counted_at))]);
                hc.kernel.set_values_tx(hc.ctx, hc.tx, "MarketingSegment", util::doc_id(doc)?, v).await?;
                Ok(Some(json!({"contact_count": total, "mailable_count": mailable, "counted_at": counted_at, "sample": sample})))
            }
            // The same count for rules being edited: nothing is saved, so it can follow every change.
            "count" => {
                let rules = params.get("rules").unwrap_or(&Value::Null);
                if !rules.is_array() {
                    return Err(Error::field("rules", "send the rules as a list"));
                }
                let candidate = values([("match_type", params.get("match_type").cloned().unwrap_or_else(|| json!("All"))), ("rules", rules.clone())]);
                let seg = parse(&candidate, &hc.ctx.schema).map_err(Error::Validation)?;
                let (total, mailable, sample) = count(hc.tx, hc.ctx, &seg, 20).await?;
                Ok(Some(json!({"contact_count": total, "mailable_count": mailable, "sample": sample})))
            }
            _ => Ok(None),
        }
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
        let schema = Schema::default();
        let ok = |v: Value| parse_rule(&row(v), &schema).unwrap();
        let bad = |v: Value| parse_rule(&row(v), &schema).unwrap_err().0;
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
