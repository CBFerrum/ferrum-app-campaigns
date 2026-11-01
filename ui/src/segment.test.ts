import { describe, expect, it } from "vitest";
import { blankRule, change, describe as say, operatorsFor, problem, rowsForSave, sameRules, standing, valueKind, type Rule } from "./segment";

const rule = (over: Partial<Rule>): Rule => ({ ...blankRule("Contact Field"), ...over });

describe("which operators fit", () => {
  it("offers text comparisons for a contact's details and date ones for when they were created", () => {
    expect(operatorsFor(rule({ field: "Email" }))).toContain("Ends With");
    expect(operatorsFor(rule({ field: "Email" }))).not.toContain("Before");
    expect(operatorsFor(rule({ field: "Created On" }))).toEqual(["Before", "After", "Within Last Days"]);
  });

  it("offers numbers and dates as well for an attribute, membership for a group and did for engagement", () => {
    const ops = operatorsFor(blankRule("Attribute"));
    expect(["Greater Than", "Less Than", "Before", "Within Last Days", "Contains"].every((o) => ops.includes(o))).toBe(true);
    expect(operatorsFor(blankRule("Email Group"))).toEqual(["Is Member", "Is Not Member"]);
    expect(operatorsFor(blankRule("Engagement"))).toEqual(["Did", "Did Not"]);
  });

  it("asks for a value only where one makes sense", () => {
    expect(["Is Empty", "Is Member", "Did Not"].map((operator) => valueKind({ operator }))).toEqual(["none", "none", "none"]);
    expect(["Equals", "Greater Than", "After", "Within Last Days"].map((operator) => valueKind({ operator }))).toEqual(["text", "number", "date", "days"]);
  });
});

describe("changing a rule", () => {
  it("starts afresh when its kind changes, keeping only which rule it is", () => {
    const next = change(rule({ id: "r1", value: "x" }), { rule_type: "Engagement" });
    expect(next).toMatchObject({ id: "r1", rule_type: "Engagement", operator: "Did", within_days: 30, _touched: true });
    expect(next.value).toBeUndefined();
  });

  it("swaps an operator that no longer fits, and drops a value that no longer applies", () => {
    expect(change(rule({ field: "Email", operator: "Contains", value: "x" }), { field: "Created On" }).operator).toBe("Before");
    expect(change(rule({ value: "x" }), { operator: "Is Empty" }).value).toBe("");
    expect(change(rule({ value: "x" }), { operator: "Starts With" }).value).toBe("x");
    expect(change(rule({ operator: "Before", value: "2026-01-01", field: "Created On" }), { operator: "Within Last Days" }).value).toBe("");
  });
});

describe("what is wrong with a rule", () => {
  it("is nothing for one that is complete", () => {
    expect(problem(rule({ value: "@x.test" }))).toBeNull();
    expect(problem(rule({ operator: "Is Empty" }))).toBeNull();
    expect(problem({ ...blankRule("Attribute"), attribute: "city", value: "Pune" })).toBeNull();
    expect(problem({ ...blankRule("Email Group"), email_group: "g1" })).toBeNull();
    expect(problem(blankRule("Engagement"))).toBeNull();
  });

  it("names the field and says what to do, as the server would", () => {
    expect(problem(rule({ value: " " }))).toEqual({ field: "value", message: "Enter a value" });
    expect(problem({ ...blankRule("Attribute"), attribute: "Not Valid!" })?.field).toBe("attribute");
    expect(problem({ ...blankRule("Attribute"), attribute: "score", operator: "Greater Than", value: "lots" })).toEqual({ field: "value", message: "Enter a number" });
    expect(problem({ ...blankRule("Attribute"), attribute: "born", operator: "Before", value: "1/2/2020" })?.message).toMatch(/YYYY-MM-DD/);
    expect(problem({ ...blankRule("Attribute"), attribute: "born", operator: "Within Last Days", value: "0" })?.field).toBe("value");
    expect(problem(blankRule("Email Group"))).toEqual({ field: "email_group", message: "Choose an email group" });
    expect(problem({ ...blankRule("Engagement"), within_days: 0 })?.field).toBe("within_days");
    expect(problem(rule({ operator: "Greater Than", value: "1" }))?.field).toBe("operator");
  });
});

describe("a rule in words", () => {
  it("reads like something a person would say", () => {
    expect(say(rule({ field: "Email", operator: "Ends With", value: "@x.test" }))).toBe("Email ends with “@x.test”");
    expect(say(rule({ field: "Consent Status", operator: "Is Not Empty" }))).toBe("Consent Status is not empty");
    expect(say(rule({ field: "Created On", operator: "Within Last Days", value: "7" }))).toBe("Created in the last 7 days");
    expect(say(rule({ field: "Created On", operator: "Before", value: "2026-01-01" }))).toBe("Created before 2026-01-01");
    expect(say({ ...blankRule("Attribute"), attribute: "score", operator: "Greater Than", value: "10" })).toBe("score is more than 10");
    expect(say({ ...blankRule("Attribute"), attribute: "city", operator: "Equals", value: "Pune" })).toBe("city is “Pune”");
    expect(say({ ...blankRule("Email Group"), email_group: "g1", operator: "Is Not Member" }, { group: "Newsletter" })).toBe("Is not in the email group “Newsletter”");
    expect(say({ ...blankRule("Engagement"), within_days: 30 })).toBe("Opened any email in the last 30 days");
    expect(say({ ...blankRule("Engagement"), activity: "Clicked", operator: "Did Not", campaign: "c1", within_days: 90 }, { campaign: "Spring sale" })).toBe(
      "Did not click a link in “Spring sale” in the last 90 days",
    );
  });
});

describe("the rows sent to the server", () => {
  it("carry every field of the rule's kind and clear the rest", () => {
    const rows = rowsForSave([
      { id: "r1", ...rule({ field: "Email", operator: "Contains", value: " x.test " }), _titles: { x: "y" } },
      { ...blankRule("Attribute"), attribute: " city ", operator: "Is Empty", value: "left over" },
      { ...blankRule("Engagement"), within_days: 14, campaign: "c1" },
    ]);
    expect(rows[0]).toEqual({ id: "r1", rule_type: "Contact Field", field: "Email", attribute: null, operator: "Contains", value: "x.test", email_group: null, activity: null, within_days: null, campaign: null });
    expect(rows[1]).toMatchObject({ rule_type: "Attribute", attribute: "city", operator: "Is Empty", value: null });
    expect(rows[2]).toMatchObject({ rule_type: "Engagement", activity: "Opened", operator: "Did", within_days: 14, campaign: "c1", value: null, field: null });
  });

  it("tell whether anything changed", () => {
    const a = [rule({ value: "x" })];
    expect(sameRules(a, [{ ...a[0], _titles: {} }])).toBe(true);
    expect(sameRules(a, [rule({ value: "y" })])).toBe(false);
    expect(sameRules(a, [])).toBe(false);
  });
});

describe("a contact in the sample", () => {
  it("says whether they will be emailed, and why not", () => {
    expect(standing("Subscribed", true)).toEqual({ label: "Will be emailed", tone: "green" });
    expect(standing("Subscribed", false).label).toBe("On the suppression list");
    expect(["Pending", "Unsubscribed", "Bounced", "Complained"].map((s) => standing(s, false).label)).toEqual([
      "Not confirmed yet",
      "Unsubscribed",
      "Address bounced",
      "Marked an email as spam",
    ]);
  });
});
