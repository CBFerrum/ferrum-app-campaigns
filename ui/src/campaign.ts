// A campaign as the screens use it: its fields, what each status allows, the numbers that follow
// from its counters, and the changes a draft sends back. Pure, so it is tested without a browser.

import { moment, rate } from "./format";
import { t } from "@ferrum/client";

export type Status = "Draft" | "Scheduled" | "Sending" | "Sent" | "Cancelled";
export type Tone = "neutral" | "blue" | "green" | "amber" | "red";

export interface AudienceRow {
  id?: string;
  segment?: string | null;
  email_group?: string | null;
  exclude?: boolean;
  /** Which of the two a row still waiting for its choice is for; the server does not know it. */
  kind?: "segment" | "group";
  _titles?: Record<string, string>;
}

export interface VariantRow {
  id?: string;
  /** A, B, … : the server's, by position. */
  variant?: string | null;
  subject?: string | null;
  content?: string | null;
}

export interface Campaign {
  id: string;
  version: number;
  docstatus: number;
  number?: string | null;
  campaign_name: string;
  status: Status;
  subject: string;
  preheader?: string | null;
  email_account?: string | null;
  email_template?: string | null;
  content: string;
  text_content?: string | null;
  audience: AudienceRow[];
  schedule_at?: string | null;
  hourly_limit?: number | null;
  track_opens: boolean;
  track_clicks: boolean;
  ab_test: boolean;
  variants: VariantRow[];
  test_percent?: number | null;
  winner_metric?: "Opens" | "Clicks" | null;
  winner_after_hours?: number | null;
  winner_variant?: string | null;
  test_sent_at?: string | null;
  winner_chosen_at?: string | null;
  total_recipients?: number | null;
  sent_count?: number | null;
  failed_count?: number | null;
  skipped_count?: number | null;
  unique_opens?: number | null;
  total_opens?: number | null;
  unique_clicks?: number | null;
  total_clicks?: number | null;
  unsubscribe_count?: number | null;
  bounce_count?: number | null;
  complaint_count?: number | null;
  started_at?: string | null;
  finished_at?: string | null;
  last_error?: string | null;
  created_at?: string;
  updated_at?: string;
  _titles?: Record<string, string>;
}

/** What the list of campaigns asks for. */
export const LIST_FIELDS =
  "campaign_name,status,subject,number,docstatus,version,schedule_at,started_at,finished_at,created_at,updated_at,total_recipients,sent_count,bounce_count,unique_opens,unique_clicks,unsubscribe_count,ab_test";

export const STATUS_FILTERS: { id: "" | Status; label: string }[] = [
  { id: "", get label() { return t("All"); } },
  { id: "Draft", get label() { return t("Drafts"); } },
  { id: "Scheduled", get label() { return t("Scheduled"); } },
  { id: "Sending", get label() { return t("Sending"); } },
  { id: "Sent", get label() { return t("Sent"); } },
  { id: "Cancelled", get label() { return t("Cancelled"); } },
];

export const isStatus = (v: string | null | undefined): v is Status => ["Draft", "Scheduled", "Sending", "Sent", "Cancelled"].includes(v ?? "");

/** The `filters` of the list query for a status chip. */
export function listFilters(status: string): string | undefined {
  return isStatus(status) ? JSON.stringify([["status", "=", status]]) : undefined;
}

export function tone(status: string | null | undefined): Tone {
  switch (status) {
    case "Scheduled":
      return "blue";
    case "Sending":
      return "amber";
    case "Sent":
      return "green";
    case "Cancelled":
      return "red";
    default:
      return "neutral";
  }
}

const n = (v: number | null | undefined) => v ?? 0;

/** Emails that reached a mail server and did not bounce. */
export const delivered = (c: Pick<Campaign, "sent_count" | "bounce_count">): number => Math.max(0, n(c.sent_count) - n(c.bounce_count));
export const openRate = (c: Pick<Campaign, "sent_count" | "bounce_count" | "unique_opens">): number | null => rate(n(c.unique_opens), delivered(c));
export const clickRate = (c: Pick<Campaign, "sent_count" | "bounce_count" | "unique_clicks">): number | null => rate(n(c.unique_clicks), delivered(c));

/** How far a send has got: what is sent, failed or skipped, of everyone it was going to. */
export function progress(c: Pick<Campaign, "total_recipients" | "sent_count" | "failed_count" | "skipped_count">): number | null {
  const done = n(c.sent_count) + n(c.failed_count) + n(c.skipped_count);
  const r = rate(done, n(c.total_recipients));
  return r == null ? null : Math.min(100, r);
}

export const isDraft = (c: Pick<Campaign, "docstatus">): boolean => c.docstatus === 0;

/** Whether there is anything to look at: sending has begun, or did before it was stopped. */
export const hasResults = (c: Pick<Campaign, "status" | "started_at">): boolean => c.status === "Sending" || c.status === "Sent" || !!c.started_at;

export type Tab = "setup" | "content" | "review" | "results";
export const TABS: Tab[] = ["setup", "content", "review", "results"];
export const isTab = (v: string | null | undefined): v is Tab => TABS.includes(v as Tab);

/** Where a campaign opens: a draft to work on, a scheduled one to check, a sent one to judge. */
export function defaultTab(c: Pick<Campaign, "status" | "started_at">): Tab {
  if (c.status === "Scheduled") return "review";
  if (c.status === "Sending" || c.status === "Sent") return "results";
  return c.status === "Cancelled" && c.started_at ? "results" : "setup";
}

/** One line of when: what it is waiting for, or what it did. */
export function whenLine(c: Pick<Campaign, "status" | "schedule_at" | "started_at" | "finished_at">, locale?: string): string {
  switch (c.status) {
    case "Scheduled":
      return c.schedule_at ? `Scheduled for ${moment(c.schedule_at, locale)}` : "Scheduled: starts within a minute";
    case "Sending":
      return `Sending since ${moment(c.started_at, locale)}`;
    case "Sent":
      return `Sent ${moment(c.finished_at ?? c.started_at, locale)}`;
    case "Cancelled":
      return c.started_at ? `Stopped ${moment(c.finished_at ?? c.started_at, locale)}` : "Cancelled before it started";
    default:
      return c.schedule_at ? `Draft, planned for ${moment(c.schedule_at, locale)}` : "Draft";
  }
}

// ── the changes a draft sends back ──

/** Audience rows as the server takes them (a row without its segment or group is not one yet). */
export function audienceForSave(rows: AudienceRow[]): AudienceRow[] {
  return rows
    .filter((r) => r.segment || r.email_group)
    .map((r) => ({ ...(r.id ? { id: r.id } : {}), segment: r.segment ?? null, email_group: r.email_group ?? null, exclude: !!r.exclude }));
}

/** Variants as the server takes them: it letters them by position itself. */
export function variantsForSave(rows: VariantRow[]): VariantRow[] {
  return rows.map((r) => ({ ...(r.id ? { id: r.id } : {}), subject: r.subject ?? "", content: r.content?.trim() ? r.content : null }));
}

const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/** Why edits cannot be saved yet, if they cannot: a row waiting for its choice. */
export function incomplete(edits: Partial<Campaign>): string | null {
  if (edits.audience?.some((r) => !r.segment && !r.email_group)) return "Choose a segment or an email group for every audience row.";
  if (edits.variants?.some((r) => !r.subject?.trim())) return "Give every version a subject.";
  return null;
}

/** The edits that can be saved now, and why the rest cannot yet. A table with a row still waiting
 *  for its choice stays as it is, and so does the A/B switch when its versions are not ready: the
 *  server wants the two together. */
export function savable(edits: Partial<Campaign>): { ready: Partial<Campaign>; waiting: string | null } {
  const ready = { ...edits };
  if (edits.audience && incomplete({ audience: edits.audience })) delete ready.audience;
  if (edits.variants && incomplete({ variants: edits.variants })) {
    delete ready.variants;
    delete ready.ab_test;
  }
  return { ready, waiting: incomplete(edits) };
}

/** What to send for `edits` on top of `base`: only what changed; a cleared field is null. */
export function patchOf(base: Campaign, edits: Partial<Campaign>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(edits) as [keyof Campaign, unknown][]) {
    if (key === "audience") {
      const rows = audienceForSave((value as AudienceRow[]) ?? []);
      if (!same(rows, audienceForSave(base.audience ?? []))) out.audience = rows;
    } else if (key === "variants") {
      const rows = variantsForSave((value as VariantRow[]) ?? []);
      if (!same(rows, variantsForSave(base.variants ?? []))) out.variants = rows;
    } else if (!same(base[key], value)) {
      out[key] = value === "" ? null : value;
    }
  }
  return out;
}

/** A new draft made from `c`: the same email to the same audience, with nothing of its sending. */
export function duplicateBody(c: Campaign): Record<string, unknown> {
  const body: Record<string, unknown> = {
    campaign_name: `Copy of ${c.campaign_name}`.slice(0, 140),
    subject: c.subject,
    preheader: c.preheader ?? null,
    email_account: c.email_account ?? null,
    content: c.content,
    text_content: c.text_content ?? null,
    audience: audienceForSave(c.audience.map(({ id: _id, ...row }) => row)),
    hourly_limit: c.hourly_limit ?? 0,
    track_opens: c.track_opens,
    track_clicks: c.track_clicks,
    ab_test: c.ab_test,
  };
  if (c.ab_test) {
    body.variants = variantsForSave(c.variants.map(({ id: _id, ...row }) => row));
    body.test_percent = c.test_percent ?? 20;
    body.winner_metric = c.winner_metric ?? "Opens";
    body.winner_after_hours = c.winner_after_hours ?? 4;
  }
  return body;
}
