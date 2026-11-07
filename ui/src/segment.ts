// A segment's rules, the way a person builds them: which operators fit which kind of rule, what a
// rule says in plain words, what is wrong with one before the server is asked, and the rows the
// server takes. The server compiles them and stays the judge; this keeps the screen honest while
// the person types. Pure, so it is tested without a browser.

import { validKey } from "./snippets";
import { t } from "@ferrum/client";

export type RuleType = "Contact Field" | "Attribute" | "Email Group" | "Engagement";

export interface Rule {
  id?: string;
  rule_type: RuleType;
  field?: string | null;
  attribute?: string | null;
  operator: string;
  value?: string | null;
  email_group?: string | null;
  activity?: string | null;
  within_days?: number | null;
  campaign?: string | null;
  _titles?: Record<string, string>;
  /** The person has changed it: only then is it told off for being unfinished. The server does not know it. */
  _touched?: boolean;
}

export const RULE_TYPES: { id: RuleType; label: string }[] = [
  { id: "Contact Field", get label() { return t("A detail of the contact"); } },
  { id: "Attribute", get label() { return t("An attribute"); } },
  { id: "Email Group", get label() { return t("An email group"); } },
  { id: "Engagement", get label() { return t("What they did with our emails"); } },
];

export const CONTACT_FIELDS = ["Email", "First Name", "Last Name", "Full Name", "Source", "Consent Status", "Linked Entity", "Created On"];
export const ACTIVITIES = ["Opened", "Clicked", "Was Sent"];

const TEXT_OPS = ["Equals", "Not Equals", "Contains", "Not Contains", "Starts With", "Ends With", "Is Empty", "Is Not Empty"];
const DATE_OPS = ["Before", "After", "Within Last Days"];

/** The operators that make sense for a rule as it stands. */
export function operatorsFor(r: Pick<Rule, "rule_type" | "field">): string[] {
  switch (r.rule_type) {
    case "Contact Field":
      return r.field === "Created On" ? DATE_OPS : TEXT_OPS;
    case "Attribute":
      return [...TEXT_OPS, "Greater Than", "Less Than", ...DATE_OPS];
    case "Email Group":
      return ["Is Member", "Is Not Member"];
    default:
      return ["Did", "Did Not"];
  }
}

export const OPERATOR_WORDS: Record<string, string> = {
  Equals: "is",
  "Not Equals": "is not",
  Contains: "contains",
  "Not Contains": "does not contain",
  "Starts With": "starts with",
  "Ends With": "ends with",
  "Is Empty": "is empty",
  "Is Not Empty": "is not empty",
  "Greater Than": "is more than",
  "Less Than": "is less than",
  Before: "is before",
  After: "is after",
  "Within Last Days": "is within the last (days)",
  "Is Member": "is in",
  "Is Not Member": "is not in",
  Did: "did",
  "Did Not": "did not",
};

export type ValueKind = "none" | "text" | "number" | "date" | "days";

/** What the value box of a rule holds. */
export function valueKind(r: Pick<Rule, "operator">): ValueKind {
  switch (r.operator) {
    case "Is Empty":
    case "Is Not Empty":
    case "Is Member":
    case "Is Not Member":
    case "Did":
    case "Did Not":
      return "none";
    case "Greater Than":
    case "Less Than":
      return "number";
    case "Before":
    case "After":
      return "date";
    case "Within Last Days":
      return "days";
    default:
      return "text";
  }
}

export function blankRule(type: RuleType): Rule {
  switch (type) {
    case "Contact Field":
      return { rule_type: type, field: "Email", operator: "Contains", value: "" };
    case "Attribute":
      return { rule_type: type, attribute: "", operator: "Equals", value: "" };
    case "Email Group":
      return { rule_type: type, email_group: null, operator: "Is Member" };
    default:
      return { rule_type: type, activity: "Opened", operator: "Did", within_days: 30, campaign: null };
  }
}

/** A rule with `patch` applied, and what follows from it: a new kind starts afresh, and an operator
 *  that no longer fits is replaced by the first that does. */
export function change(rule: Rule, patch: Partial<Rule>): Rule {
  if (patch.rule_type && patch.rule_type !== rule.rule_type) return { ...blankRule(patch.rule_type), id: rule.id, _touched: true };
  const next = { ...rule, ...patch, _touched: true };
  const ops = operatorsFor(next);
  if (!ops.includes(next.operator)) next.operator = ops[0];
  if (valueKind(next) === "none") next.value = "";
  else if ((patch.operator || patch.field) && (valueKind(rule) !== valueKind(next) || next.field !== rule.field)) next.value = "";
  return next;
}

const DATE = /^\d{4}-\d{2}-\d{2}$/;

export interface Problem {
  /** The rule's field the problem is in, as the server names it. */
  field: string;
  message: string;
}

/** What is wrong with a rule that the server would refuse; null when it is fine. */
export function problem(r: Rule): Problem | null {
  if (!operatorsFor(r).includes(r.operator)) return { field: "operator", message: "Choose how to compare" };
  const value = (r.value ?? "").trim();
  switch (r.rule_type) {
    case "Contact Field":
      if (!r.field) return { field: "field", message: "Choose a detail" };
      break;
    case "Attribute":
      if (!validKey((r.attribute ?? "").trim())) return { field: "attribute", message: "Use lowercase letters, digits and _, starting with a letter" };
      break;
    case "Email Group":
      return r.email_group ? null : { field: "email_group", message: "Choose an email group" };
    default: {
      if (!r.activity) return { field: "activity", message: "Choose what they did" };
      const days = Number(r.within_days);
      return Number.isInteger(days) && days >= 1 && days <= 3650 ? null : { field: "within_days", message: "Enter a number of days, 1 to 3650" };
    }
  }
  switch (valueKind(r)) {
    case "text":
      return value ? null : { field: "value", message: "Enter a value" };
    case "number":
      return value !== "" && Number.isFinite(Number(value)) ? null : { field: "value", message: "Enter a number" };
    case "date":
      return DATE.test(value) ? null : { field: "value", message: "Enter a date as YYYY-MM-DD" };
    case "days": {
      const days = Number(value);
      return Number.isInteger(days) && days >= 1 && days <= 36500 ? null : { field: "value", message: "Enter a number of days, 1 to 36500" };
    }
    default:
      return null;
  }
}

/** A rule in a sentence. `names` turn linked records (an email group, a campaign) into what a person calls them. */
export function describe(r: Rule, names: { group?: string; campaign?: string } = {}): string {
  const v = (r.value ?? "").trim();
  const quoted = `“${v}”`;
  switch (r.rule_type) {
    case "Contact Field": {
      const subject = r.field === "Created On" ? "Created" : r.field ?? "A detail";
      if (r.field === "Created On") return r.operator === "Within Last Days" ? `Created in the last ${v || "…"} days` : `Created ${r.operator === "Before" ? "before" : "after"} ${v || "…"}`;
      return `${subject} ${OPERATOR_WORDS[r.operator] ?? r.operator}${valueKind(r) === "none" ? "" : ` ${quoted}`}`;
    }
    case "Attribute": {
      const key = (r.attribute ?? "").trim() || "…";
      if (r.operator === "Within Last Days") return `${key} is within the last ${v || "…"} days`;
      const number = valueKind(r) === "number" || valueKind(r) === "date";
      return `${key} ${OPERATOR_WORDS[r.operator] ?? r.operator}${valueKind(r) === "none" ? "" : ` ${number ? v || "…" : quoted}`}`;
    }
    case "Email Group":
      return `${r.operator === "Is Not Member" ? "Is not in" : "Is in"} the email group ${names.group ? `“${names.group}”` : "…"}`;
    default: {
      const did = r.operator === "Did";
      const verb = { Opened: did ? "Opened" : "Did not open", Clicked: did ? "Clicked a link in" : "Did not click a link in", "Was Sent": did ? "Was sent" : "Was not sent" }[
        r.activity ?? "Opened"
      ];
      const what = r.campaign ? `“${names.campaign ?? "a campaign"}”` : "any email";
      return `${verb} ${what} in the last ${r.within_days ?? "…"} days`;
    }
  }
}

/** The rows the server takes: every field of the rule's kind, the others cleared. */
export function rowsForSave(rules: Rule[]): Record<string, unknown>[] {
  return rules.map((r) => {
    const row: Record<string, unknown> = {
      ...(r.id ? { id: r.id } : {}),
      rule_type: r.rule_type,
      field: null,
      attribute: null,
      operator: r.operator,
      value: null,
      email_group: null,
      activity: null,
      within_days: null,
      campaign: null,
    };
    const value = valueKind(r) === "none" ? null : (r.value ?? "").trim();
    if (r.rule_type === "Contact Field") Object.assign(row, { field: r.field, value });
    else if (r.rule_type === "Attribute") Object.assign(row, { attribute: (r.attribute ?? "").trim(), value });
    else if (r.rule_type === "Email Group") row.email_group = r.email_group;
    else Object.assign(row, { activity: r.activity, within_days: r.within_days == null ? null : Number(r.within_days), campaign: r.campaign ?? null });
    return row;
  });
}

export const sameRules = (a: Rule[], b: Rule[]): boolean => JSON.stringify(rowsForSave(a)) === JSON.stringify(rowsForSave(b));

export type ContactTone = "green" | "amber" | "red" | "neutral";

/** What to say about a contact in a segment's sample: will they be emailed, and if not, why. */
export function standing(consent: string, mailable: boolean): { label: string; tone: ContactTone } {
  if (mailable) return { label: t("Will be emailed"), tone: "green" };
  switch (consent) {
    case "Subscribed":
      return { label: t("On the suppression list"), tone: "amber" };
    case "Pending":
      return { label: t("Not confirmed yet"), tone: "amber" };
    case "Unsubscribed":
      return { label: t("Unsubscribed"), tone: "neutral" };
    case "Bounced":
      return { label: t("Address bounced"), tone: "red" };
    case "Complained":
      return { label: t("Marked an email as spam"), tone: "red" };
    default:
      return { label: consent || t("Unknown"), tone: "neutral" };
  }
}
