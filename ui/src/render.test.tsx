// The screens drawn once, on the server's side of React and with answers already in the cache, the
// way they will look when the data has arrived: a screen that cannot draw itself from a real answer
// (a missing field, a list that is not one) fails here, without a browser. What a screen does when
// it is used is not tested here; the logic under it is, in the files beside this one.

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToString } from "react-dom/server";
import { Route, Routes, StaticRouter } from "react-router";
import { describe, expect, it } from "vitest";
import type { Campaign } from "./campaign";
import { Audiences } from "./Audiences";
import { CampaignRoute } from "./CampaignPage";
import { Content } from "./Content";
import { Home } from "./Home";
import { Import } from "./Import";
import { keys } from "./data";
import { ResultsTab } from "./ResultsTab";
import { ReviewTab } from "./ReviewTab";
import { SegmentRoute } from "./SegmentPage";
import { blankRule, rowsForSave, type Rule } from "./segment";
import type { AudienceCheck } from "./review";
import type { Timeline } from "./results";
import { Setup } from "./Setup";
import type { Draft } from "./useDraft";

const render = (ui: ReactNode, seed: (qc: QueryClient) => void = () => {}, at = "/") => {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  seed(qc);
  return renderToString(
    <QueryClientProvider client={qc}>
      <StaticRouter location={at}>{ui}</StaticRouter>
    </QueryClientProvider>,
  );
};

const perms = (over: Record<string, string> = {}) => ({ read: "all", write: "all", create: "all", delete: "all", submit: "all", cancel: "all", export: "all", import: "all", ...over });

const campaign: Campaign = {
  id: "c1",
  version: 4,
  docstatus: 0,
  number: "EC-2026-00001",
  campaign_name: "Spring sale",
  status: "Draft",
  subject: "Hello {{ contact.first_name }}",
  preheader: "Up to 50% off",
  email_account: null,
  content: '<p>Hi</p><a href="https://shop.test/sale">Shop</a><img src="a.png">',
  text_content: null,
  audience: [
    { id: "r1", segment: "s1", email_group: null, exclude: false, _titles: { segment: "Pune customers" } },
    { id: "r2", segment: null, email_group: "g1", exclude: true, _titles: { email_group: "Staff" } },
  ],
  hourly_limit: 0,
  track_opens: true,
  track_clicks: true,
  ab_test: true,
  variants: [
    { id: "v1", variant: "A", subject: "One" },
    { id: "v2", variant: "B", subject: "Two", content: "<p>Two</p>" },
  ],
  test_percent: 20,
  winner_metric: "Opens",
  winner_after_hours: 4,
};

/** A draft as the screens are given it, with nothing to save. */
const draft = (doc: Campaign = campaign): Draft =>
  ({
    doc,
    saved: doc,
    editable: doc.docstatus === 0,
    set: () => {},
    save: async () => doc,
    reload: async () => {},
    keepMine: () => {},
    status: "saved",
    message: null,
    fieldErrors: {},
    rowErrors: [],
    savedAt: null,
    theirs: [],
    dirty: false,
    query: {},
  }) as unknown as Draft;

const check: AudienceCheck = {
  checked_at: "2026-11-01T10:00:00.000Z",
  contacts: 120,
  mailable: 100,
  left_out: { pending: 2, unsubscribed: 10, bounced: 3, complained: 1, suppressed: 4 },
  excluded: 7,
  new_contacts: 3,
  rows: [
    { kind: "segment", id: "s1", name: "Pune customers", exclude: false, contacts: 120, mailable: 100 },
    { kind: "group", id: "g1", name: "Staff", exclude: true, contacts: 7, mailable: 7 },
  ],
  split: { variants: 2, test: 20, rest: 80 },
  rate: { per_hour: 500, workspace: 500, own: 0 },
  ready: { public_url: true, account: null, postal_address: false },
  emails: [
    { variant: null, error: null, bytes: 3000, links: ["https://shop.test/sale"], unsubscribe: "footer" },
    { variant: "A", error: null, bytes: 3000, links: [], unsubscribe: "footer" },
    { variant: "B", error: null, bytes: 3000, links: [], unsubscribe: "footer" },
  ],
};

const timeline: Timeline = {
  interval: "hour",
  timezone: "UTC",
  at: ["2026-11-01T10:00:00", "2026-11-01T11:00:00", "2026-11-01T12:00:00"],
  series: { sent: [60, 40, 0], opens: [5, 20, 10], clicks: [0, 4, 2], unsubscribes: [0, 1, 0], bounces: [1, 0, 0], complaints: [0, 0, 0] },
  variants: { A: { sent: [30, 20, 0], opens: [2, 8, 5], clicks: [0, 1, 1] }, B: { sent: [30, 20, 0], opens: [3, 12, 5], clicks: [0, 3, 1] } },
};

const perf = { campaign_id: "c1", recipients: 120, sent: 100, delivered: 99, failed: 0, skipped: 20, unique_opens: 35, total_opens: 50, machine_opens: 4, unique_clicks: 6, total_clicks: 8, unsubscribes: 1, bounces: 1, hard_bounces: 1, soft_bounces: 0, complaints: 0 };

describe("the campaign screens, drawn from real answers", () => {
  it("draws the setup of a draft, with who it goes to and the A/B test", () => {
    const html = render(<Setup draft={draft()} editable check={check} />);
    for (const text of ["Spring sale", "Preview text", "Who gets it", "100 of 120", "7 people", "Subject of version B", "Test versions", "Count opens"]) expect(html).toContain(text);
    expect(html).not.toContain("undefined");
  });

  it("draws the content tab with its editor and a preview waiting for the server", () => {
    const html = render(<Content draft={draft()} editable />);
    expect(html).toContain("Insert");
    expect(html).toContain("Blocks");
    expect(html).toContain("Version A only");
    expect(html).toContain("Building the preview");
  });

  it("draws the review: who gets it, why the others do not, what to fix and the way to send", () => {
    const html = render(<ReviewTab draft={draft()} editable canSubmit canCancel go={() => {}} onTest={() => {}} onCancel={() => {}} />, (qc) => qc.setQueryData(["campaigns", "check", "c1", 4], check));
    for (const text of ["100", "will get this email", "Unsubscribed", "On the suppression list", "Left out by your exclusion rules", "Pune customers", "Your postal address is not set", "1 picture without alternative text", "Send to 100 people", "As soon as I confirm"]) {
      expect(html).toContain(text);
    }
    expect(html).not.toContain("undefined");
  });

  it("does not let a person without the right to send do it, and says who can", () => {
    const html = render(<ReviewTab draft={draft()} editable canSubmit={false} canCancel={false} go={() => {}} onTest={() => {}} onCancel={() => {}} />, (qc) => qc.setQueryData(["campaigns", "check", "c1", 4], check));
    expect(html).toContain("A Marketing Manager sends campaigns");
    expect(html).not.toContain("Send to 100 people");
  });

  it("blocks the send, in words, when nobody would get it", () => {
    const none = { ...check, mailable: 0, contacts: 0 };
    const html = render(<ReviewTab draft={draft()} editable canSubmit canCancel go={() => {}} onTest={() => {}} onCancel={() => {}} />, (qc) => qc.setQueryData(["campaigns", "check", "c1", 4], none));
    expect(html).toContain("Nobody would receive this");
    expect(html).toContain("disabled");
  });

  it("draws the results of a sent campaign: numbers, the A/B test and the links", () => {
    const sent: Campaign = { ...campaign, docstatus: 1, status: "Sent", winner_variant: "B", test_sent_at: "2026-11-01T10:00:00Z", started_at: "2026-11-01T10:00:00Z" };
    const html = render(<ResultsTab doc={sent} />, (qc) => {
      qc.setQueryData(["campaigns", "perf", "c1", 4], { rows: [perf] });
      qc.setQueryData(["campaigns", "timeline", "c1", "auto"], timeline);
      qc.setQueryData(["campaigns", "ab", "c1"], {
        rows: [
          { variant: "A", subject: "One", recipients: 60, delivered: 60, unique_opens: 15, unique_clicks: 2, winner: false },
          { variant: "B", subject: "Two", recipients: 60, delivered: 59, unique_opens: 20, unique_clicks: 4, winner: true },
        ],
      });
      qc.setQueryData(["campaigns", "links", "c1"], { rows: [{ link_no: 1, url: "https://shop.test/sale?x=1", unique_clicks: 6, total_clicks: 8 }] });
    });
    for (const text of ["Delivered", "Opened", "Spam complaints", "Emails sent each hour", "Opened and clicked", "Version B won", "Winner", "shop.test", "Running total"]) expect(html).toContain(text);
    expect(html).not.toContain("undefined");
  });

  it("says there are no results before anything was sent", () => {
    const html = render(<ResultsTab doc={campaign} />, (qc) => {
      qc.setQueryData(["campaigns", "perf", "c1", 4], { rows: [] });
      qc.setQueryData(["campaigns", "timeline", "c1", "auto"], { ...timeline, at: [], series: { sent: [], opens: [], clicks: [], unsubscribes: [], bounces: [], complaints: [] }, variants: {} });
    });
    expect(html).toContain("No results yet");
  });
});

describe("the other screens, drawn from real answers", () => {
  it("lists campaigns with how each is doing", () => {
    const row = (over: Partial<Campaign>) => ({ ...campaign, ...over, audience: undefined, variants: undefined });
    const html = render(<Home />, (qc) => {
      qc.setQueryData(["meta", "EmailCampaign"], { name: "EmailCampaign", permissions: { read: "all", write: "all", create: "all", delete: "all", submit: "all", cancel: "all", export: "all", import: "none" } });
      qc.setQueryData([...keys.list, "", ""], {
        pages: [
          {
            total: 3,
            rows: [
              row({ id: "a", status: "Sending", docstatus: 1, total_recipients: 200, sent_count: 80, unique_opens: 20, unique_clicks: 3, bounce_count: 1, started_at: "2026-11-01T10:00:00Z" }),
              row({ id: "b", status: "Sent", docstatus: 1, sent_count: 100, bounce_count: 2, unique_opens: 40, unique_clicks: 5, started_at: "2026-11-01T10:00:00Z", finished_at: "2026-11-01T12:00:00Z" }),
              row({ id: "c", status: "Draft", updated_at: "2026-11-01T09:00:00Z" }),
            ],
          },
        ],
        pageParams: [undefined],
      });
    });
    for (const text of ["3 campaigns", "Sending", "Sent", "Draft", "80 of 200", "Drafts", "New campaign", "A/B test"]) expect(html).toContain(text);
  });

  it("shows who can be emailed, segments and how the audience moved", () => {
    const html = render(<Audiences />, (qc) => {
      qc.setQueryData(["meta", "MarketingSegment"], { name: "MarketingSegment", permissions: { read: "all", write: "all", create: "all", delete: "all", submit: "none", cancel: "none", export: "none", import: "none" } });
      qc.setQueryData(["meta", "MarketingContact"], { name: "MarketingContact", permissions: { read: "all", write: "all", create: "all", delete: "none", submit: "none", cancel: "none", export: "all", import: "all" } });
      qc.setQueryData(["campaigns", "standing"], { subscribed: 4812, pending: 3, unsubscribed: 120, away: 14, suppressed: 90 });
      qc.setQueryData(["campaigns", "growth"], { rows: [{ day: "2026-10-31", subscribed: 5, unsubscribed: 1, bounced: 0, complained: 0 }] });
      qc.setQueryData(keys.segments, { rows: [{ id: "s1", segment_name: "Pune customers", match_type: "All", contact_count: 120, mailable_count: 100, counted_at: "2026-11-01T10:00:00Z" }] });
    });
    for (const text of ["4,812", "Pune customers", "Import contacts", "New segment", "The last 30 days", "Not confirmed yet"]) expect(html).toContain(text);
  });

  it("starts the import at the file", () => {
    const html = render(<Import />, (qc) =>
      qc.setQueryData(["meta", "MarketingContact"], { name: "MarketingContact", permissions: { read: "all", write: "all", create: "all", delete: "none", submit: "none", cancel: "none", export: "all", import: "all" } }),
    );
    for (const text of ["Choose the file", "Choose a CSV file", "Or paste them", "File", "Columns", "Consent", "Check", "Import"]) expect(html).toContain(text);
  });

  it("does not offer the import to someone who may not", () => {
    const html = render(<Import />, (qc) =>
      qc.setQueryData(["meta", "MarketingContact"], { name: "MarketingContact", permissions: { read: "all", write: "all", create: "all", delete: "none", submit: "none", cancel: "none", export: "none", import: "none" } }),
    );
    expect(html).toContain("You cannot import contacts");
  });
});

describe("the pages, drawn from a record in the cache", () => {
  const page = (at: string, doc: Campaign, rights = perms()) =>
    render(
      <Routes>
        <Route path="/c/:id" element={<CampaignRoute />} />
      </Routes>,
      (qc) => {
        qc.setQueryData(["meta", "EmailCampaign"], { name: "EmailCampaign", permissions: rights });
        qc.setQueryData(keys.campaign("c1"), doc);
        qc.setQueryData(["campaigns", "check", "c1", 4], check);
      },
      at,
    );

  it("opens a draft on its setup, with the way on to the review", () => {
    const html = page("/c/c1", campaign);
    for (const text of ["Spring sale", "EC-2026-00001", "Draft", "Setup", "Content", "Review and send", "Results", "Send a test", "All changes saved", "A/B test"]) expect(html).toContain(text);
    expect(html).not.toContain("undefined");
  });

  it("calls the review a review, not a send, for someone who cannot send", () => {
    const html = page("/c/c1", campaign, perms({ submit: "none", cancel: "none" }));
    expect(html).not.toContain("Review and send");
    expect(html).toContain(">Review<");
  });

  it("opens a scheduled campaign on its review, and a sent one on its results", () => {
    const scheduled = { ...campaign, docstatus: 1, status: "Scheduled" as const, schedule_at: "2099-01-01T10:00:00Z" };
    expect(page("/c/c1", scheduled)).toContain("The content and the audience are fixed now");
    const sent = { ...campaign, docstatus: 1, status: "Sent" as const, started_at: "2026-11-01T10:00:00Z" };
    const html = page("/c/c1", sent);
    expect(html).toContain("Duplicate");
    expect(html).toContain("Loading the results");
  });

  it("opens a cancelled campaign with a way to start again", () => {
    const html = page("/c/c1?tab=setup", { ...campaign, docstatus: 2, status: "Cancelled" });
    expect(html).toContain("Amend");
    expect(html).toContain("Duplicate");
  });

  it("draws a segment: its rules in words, the count and who is in it", () => {
    const rules: Rule[] = [
      { ...blankRule("Contact Field"), id: "r1", field: "Email", operator: "Ends With", value: "@x.test" },
      { ...blankRule("Engagement"), id: "r2", within_days: 30 },
    ];
    const segment = { id: "s1", version: 2, segment_name: "Pune customers", description: "Who we mail first", match_type: "All" as const, rules, contact_count: 120, mailable_count: 100, counted_at: "2026-11-01T10:00:00Z" };
    const html = render(
      <Routes>
        <Route path="/s/:id" element={<SegmentRoute />} />
      </Routes>,
      (qc) => {
        qc.setQueryData(["meta", "MarketingSegment"], { name: "MarketingSegment", permissions: perms() });
        qc.setQueryData(keys.segment("s1"), segment);
        qc.setQueryData(["campaigns", "segment-count", "s1", JSON.stringify({ match: "All", rules: rowsForSave(rules) })], {
          contact_count: 120,
          mailable_count: 100,
          sample: [
            { id: "p1", email: "asha@x.test", full_name: "Asha Rao", consent_status: "Subscribed", mailable: true },
            { id: "p2", email: "ben@x.test", full_name: null, consent_status: "Unsubscribed", mailable: false },
          ],
        });
      },
      "/s/s1",
    );
    for (const text of ["Pune customers", "Who is in it", "All of these", "Email ends with “@x.test”", "Opened any email in the last 30 days", "120", "people match", "asha@x.test", "Will be emailed", "Unsubscribed", "Save changes", "Where it is used"]) {
      expect(html).toContain(text);
    }
    expect(html).not.toContain("undefined");
  });
});
