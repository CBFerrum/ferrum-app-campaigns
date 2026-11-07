// Judging a send: the headline numbers, the series over time, the A/B outcome and the links, from
// what the server answers (the email_campaign_performance, ab_test_results and link_clicks reports and
// the campaign's timeline action). Pure, so the wording and the arithmetic are tested without a browser.

import { cumulative, ratesOver, type Counts, type Interval } from "./chart";
import { count, percent, rate, span } from "./format";
import { t } from "@ferrum/client";

export type Kind = "sent" | "opens" | "clicks" | "unsubscribes" | "bounces" | "complaints";

export interface Timeline {
  interval: Interval;
  timezone: string;
  /** The start of each bucket, in the workspace's wall-clock time. */
  at: string[];
  series: Record<Kind, Counts>;
  /** Per version of an A/B test. */
  variants: Record<string, { sent: Counts; opens: Counts; clicks: Counts }>;
}

/** The report's row for one campaign. */
export interface Perf {
  recipients: number;
  sent: number;
  delivered: number;
  failed: number;
  skipped: number;
  unique_opens: number;
  total_opens: number;
  machine_opens: number;
  unique_clicks: number;
  total_clicks: number;
  unsubscribes: number;
  bounces: number;
  hard_bounces: number;
  soft_bounces: number;
  complaints: number;
}

export interface Tile {
  id: string;
  label: string;
  value: string;
  /** What it is a share of. */
  sub?: string;
  /** Something to know about the number. */
  note?: string;
}

/** The headline numbers, each with its share, worked out the way the server's report does. */
export function tiles(p: Perf): Tile[] {
  const share = (part: number, whole: number) => percent(rate(part, whole));
  return [
    {
      id: "sent",
      label: t("Sent"),
      value: count(p.sent),
      sub: t("of {value}", { value: String((p.recipients === 1 ? t("1 recipient") : t("{n} recipients", { n: count(p.recipients) }))) }),
      note: p.failed + p.skipped > 0 ? [p.failed ? t("{value} failed", { value: count(p.failed) }) : "", p.skipped ? t("{value} skipped", { value: count(p.skipped) }) : ""].filter(Boolean).join(", ") : undefined,
    },
    { id: "delivered", label: t("Delivered"), value: count(p.delivered), sub: t("{value} of sent", { value: share(p.delivered, p.sent) }) },
    {
      id: "opened",
      label: t("Opened"),
      value: count(p.unique_opens),
      sub: t("{value} of delivered", { value: share(p.unique_opens, p.delivered) }),
      note: p.machine_opens > 0 ? t("{value} more were automatic (privacy proxies, scanners) and are not counted", { value: count(p.machine_opens) }) : undefined,
    },
    {
      id: "clicked",
      label: t("Clicked"),
      value: count(p.unique_clicks),
      sub: t("{value} of delivered", { value: share(p.unique_clicks, p.delivered) }),
      note: p.unique_opens > 0 ? t("{value} of those who opened", { value: share(p.unique_clicks, p.unique_opens) }) : undefined,
    },
    { id: "unsubscribed", label: t("Unsubscribed"), value: count(p.unsubscribes), sub: t("{value} of delivered", { value: share(p.unsubscribes, p.delivered) }) },
    {
      id: "bounced",
      label: t("Bounced"),
      value: count(p.bounces),
      sub: t("{value} hard, {value2} soft", { value: count(p.hard_bounces), value2: count(p.soft_bounces) }),
      note: p.bounces > 0 ? t("{value} of sent", { value: share(p.bounces, p.sent) }) : undefined,
    },
    { id: "complaints", label: t("Spam complaints"), value: count(p.complaints), sub: t("{value} of delivered", { value: share(p.complaints, p.delivered) }) },
  ];
}

/** A series per bucket, or as a running total. */
export const shape = (values: Counts, running: boolean): Counts => (running ? cumulative(values) : values);

/** Whether any bucket of a series is not zero. */
export const hasAny = (values: Counts): boolean => values.some((v) => v > 0);

/** Open (or click) rate of each version as the send went on: opens so far of emails sent so far. */
export function variantRates(tl: Timeline, kind: "opens" | "clicks"): { variant: string; values: number[] }[] {
  return Object.entries(tl.variants)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([variant, v]) => ({ variant, values: ratesOver(cumulative(v[kind]), cumulative(v.sent)) }));
}

export interface AbRow {
  variant: string;
  subject: string | null;
  recipients: number;
  delivered: number;
  unique_opens: number;
  unique_clicks: number;
  winner: boolean;
}

export interface AbBar {
  variant: string;
  subject: string | null;
  delivered: number;
  hits: number;
  rate: number | null;
  winner: boolean;
}

/** Each version's bar for the metric the test is decided by (a click needs no open, so the two are separate). */
export function abBars(rows: AbRow[], metric: "Opens" | "Clicks"): AbBar[] {
  return rows.map((r) => {
    const hits = metric === "Clicks" ? r.unique_clicks : r.unique_opens;
    return { variant: r.variant, subject: r.subject, delivered: r.delivered, hits, rate: rate(hits, r.delivered), winner: r.winner };
  });
}

/** Where an A/B test stands, in a sentence. */
export function abStanding(
  c: { winner_variant?: string | null; test_sent_at?: string | null; winner_after_hours?: number | null; winner_chosen_at?: string | null },
  now = Date.now(),
): string {
  if (c.winner_variant) return t("Version {winner_variant} won and went to everyone else.", { winner_variant: c.winner_variant });
  if (!c.test_sent_at) return "Waiting for every test email to go out. Then the clock for the winner starts.";
  const left = (new Date(c.test_sent_at).getTime() + (c.winner_after_hours ?? 4) * 3_600_000 - now) / 3_600_000;
  return left > 0 ? t("The winner is chosen in {left}.", { left: span(left) }) : "The winner is chosen at the next round, within a minute.";
}

export interface LinkRow {
  link_no: number;
  url: string;
  unique_clicks: number;
  total_clicks: number;
}

/** A link's clicks as a share of those who got the email, and of the busiest link (for its bar). */
export function linkShares(rows: LinkRow[], delivered: number): (LinkRow & { share: number | null; relative: number })[] {
  const top = Math.max(0, ...rows.map((r) => r.unique_clicks));
  return rows.map((r) => ({ ...r, share: rate(r.unique_clicks, delivered), relative: top > 0 ? r.unique_clicks / top : 0 }));
}

/** The host of a URL, for a short label; the whole text when it is not one. */
export function host(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
