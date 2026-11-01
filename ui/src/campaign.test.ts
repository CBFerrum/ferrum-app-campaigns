import { describe, expect, it } from "vitest";
import { clickRate, defaultTab, delivered, duplicateBody, incomplete, listFilters, openRate, patchOf, progress, savable, tone, whenLine, type Campaign } from "./campaign";

const base: Campaign = {
  id: "c1",
  version: 3,
  docstatus: 0,
  campaign_name: "Spring sale",
  status: "Draft",
  subject: "Hello {{ contact.first_name }}",
  preheader: "Up to 50% off",
  email_account: null,
  content: "<p>Hi</p>",
  text_content: null,
  audience: [
    { id: "r1", segment: "s1", email_group: null, exclude: false, _titles: { segment: "Pune" } },
    { id: "r2", segment: "s2", email_group: null, exclude: true },
  ],
  hourly_limit: 0,
  track_opens: true,
  track_clicks: true,
  ab_test: false,
  variants: [],
};

describe("a campaign's counters", () => {
  const c = { sent_count: 100, bounce_count: 4, unique_opens: 48, unique_clicks: 12, total_recipients: 120, failed_count: 5, skipped_count: 5 };

  it("count what was delivered, not just sent, as the report does", () => {
    expect(delivered(c)).toBe(96);
    expect(openRate(c)).toBe(50);
    expect(clickRate(c)).toBe(12.5);
    expect(openRate({ sent_count: 0, bounce_count: 0, unique_opens: 0 })).toBeNull();
    expect(delivered({ sent_count: 2, bounce_count: 5 })).toBe(0);
  });

  it("say how far a send has got, never past the end", () => {
    expect(progress(c)).toBeCloseTo(91.67, 1);
    expect(progress({ total_recipients: 0 })).toBeNull();
    expect(progress({ total_recipients: 10, sent_count: 12 })).toBe(100);
  });
});

describe("where a campaign opens and what it says about itself", () => {
  it("opens a draft to write, a scheduled one to check and a sent one to judge", () => {
    expect(defaultTab({ status: "Draft" })).toBe("setup");
    expect(defaultTab({ status: "Scheduled" })).toBe("review");
    expect(defaultTab({ status: "Sending" })).toBe("results");
    expect(defaultTab({ status: "Sent" })).toBe("results");
    expect(defaultTab({ status: "Cancelled", started_at: "2026-11-01T10:00:00Z" })).toBe("results");
    expect(defaultTab({ status: "Cancelled" })).toBe("setup");
  });

  it("filters the list by a status chip, and ignores anything else", () => {
    expect(JSON.parse(listFilters("Sent")!)).toEqual([["status", "=", "Sent"]]);
    expect(listFilters("")).toBeUndefined();
    expect(listFilters("Everything")).toBeUndefined();
  });

  it("tells a status from its colour", () => {
    expect(["Draft", "Scheduled", "Sending", "Sent", "Cancelled", "?"].map(tone)).toEqual(["neutral", "blue", "amber", "green", "red", "neutral"]);
  });

  it("says when in a line", () => {
    const at = "2026-11-07T10:30:00Z";
    expect(whenLine({ status: "Draft" })).toBe("Draft");
    expect(whenLine({ status: "Scheduled", schedule_at: at }, "en-GB")).toMatch(/^Scheduled for .*Nov/);
    expect(whenLine({ status: "Scheduled" })).toBe("Scheduled: starts within a minute");
    expect(whenLine({ status: "Sent", started_at: at, finished_at: at }, "en-GB")).toMatch(/^Sent .*Nov/);
    expect(whenLine({ status: "Cancelled" })).toBe("Cancelled before it started");
  });
});

describe("what a draft sends back", () => {
  it("sends only what changed", () => {
    expect(patchOf(base, {})).toEqual({});
    expect(patchOf(base, { subject: "Hello {{ contact.first_name }}", preheader: "Up to 50% off" })).toEqual({});
    expect(patchOf(base, { subject: "New subject", track_opens: false })).toEqual({ subject: "New subject", track_opens: false });
  });

  it("sends a cleared field as null, so the server leaves it blank", () => {
    expect(patchOf(base, { preheader: "" })).toEqual({ preheader: null });
    expect(patchOf(base, { schedule_at: null })).toEqual({});
  });

  it("sends the audience whole, rows with their ids, without the titles or any half-chosen row", () => {
    const rows = [...base.audience, { segment: null, email_group: "g1", exclude: false }, { segment: null, email_group: null }];
    expect(patchOf(base, { audience: rows }).audience).toEqual([
      { id: "r1", segment: "s1", email_group: null, exclude: false },
      { id: "r2", segment: "s2", email_group: null, exclude: true },
      { segment: null, email_group: "g1", exclude: false },
    ]);
    expect(patchOf(base, { audience: base.audience.map((r) => ({ ...r, _titles: undefined })) })).toEqual({});
  });

  it("lets the server letter the variants", () => {
    const v = patchOf({ ...base, ab_test: true }, { variants: [{ variant: "A", subject: "One", content: "  " }, { subject: "Two", content: "<p>x</p>" }] });
    expect(v.variants).toEqual([
      { subject: "One", content: null },
      { subject: "Two", content: "<p>x</p>" },
    ]);
  });

  it("waits for a row to be finished before saving", () => {
    expect(incomplete({})).toBeNull();
    expect(incomplete({ audience: [{ segment: "s1" }] })).toBeNull();
    expect(incomplete({ audience: [{ segment: "s1" }, { segment: null, email_group: null }] })).toMatch(/segment or an email group/);
    expect(incomplete({ variants: [{ subject: "A" }, { subject: " " }] })).toMatch(/subject/);
  });
});

describe("what can be saved while something is unfinished", () => {
  it("saves the rest and holds back a table with a row still waiting", () => {
    const edits = { subject: "New", audience: [{ segment: "s1" }, { segment: null, email_group: null }] };
    const { ready, waiting } = savable(edits);
    expect(ready).toEqual({ subject: "New" });
    expect(waiting).toMatch(/segment or an email group/);
    expect(savable({ audience: [{ segment: "s1" }] })).toEqual({ ready: { audience: [{ segment: "s1" }] }, waiting: null });
  });

  it("keeps the A/B switch with its versions, which the server wants together", () => {
    const { ready } = savable({ ab_test: true, track_clicks: false, variants: [{ subject: "A" }, { subject: "" }] });
    expect(ready).toEqual({ track_clicks: false });
    expect(savable({ ab_test: true, variants: [{ subject: "A" }, { subject: "B" }] }).ready).toHaveProperty("ab_test", true);
    expect(savable({ ab_test: false, variants: [] }).ready).toEqual({ ab_test: false, variants: [] });
  });
});

describe("a copy of a campaign", () => {
  it("keeps the email and the audience and nothing of its sending", () => {
    const copy = duplicateBody({ ...base, status: "Sent", sent_count: 90, started_at: "2026-11-01T10:00:00Z", schedule_at: "2026-11-01T09:00:00Z" } as Campaign);
    expect(copy).toMatchObject({ campaign_name: "Copy of Spring sale", subject: base.subject, content: base.content, track_opens: true });
    expect(copy.audience).toEqual([
      { segment: "s1", email_group: null, exclude: false },
      { segment: "s2", email_group: null, exclude: true },
    ]);
    for (const key of ["status", "sent_count", "started_at", "schedule_at", "version", "id", "number", "variants"]) expect(copy).not.toHaveProperty(key);
  });

  it("keeps an A/B test with its settings, and drops the winner", () => {
    const copy = duplicateBody({
      ...base,
      ab_test: true,
      variants: [{ id: "v1", variant: "A", subject: "One" }, { id: "v2", variant: "B", subject: "Two", content: "<p>b</p>" }],
      test_percent: 30,
      winner_metric: "Clicks",
      winner_after_hours: 6,
      winner_variant: "B",
    });
    expect(copy.variants).toEqual([
      { subject: "One", content: null },
      { subject: "Two", content: "<p>b</p>" },
    ]);
    expect([copy.test_percent, copy.winner_metric, copy.winner_after_hours]).toEqual([30, "Clicks", 6]);
    expect(copy).not.toHaveProperty("winner_variant");
  });
});
