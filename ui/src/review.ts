// Before a campaign goes out: who it will reach, who it leaves out and why, and what would stop it
// or is worth a look. The server counts (the `audience_check` action counts the way sending
// counts); this only words it. Pure, so the wording is tested without a browser.

import type { Campaign, Tab } from "./campaign";
import { size, span, count } from "./format";
import { t } from "@ferrum/client";

export interface CheckRow {
  kind: "segment" | "group";
  id: string;
  name: string;
  exclude: boolean;
  contacts: number;
  mailable: number;
}

export interface CheckEmail {
  /** null: the campaign's own email; "A", "B", …: a version of an A/B test. */
  variant: string | null;
  error: string | null;
  bytes: number | null;
  /** The links the sender will track in this version. */
  links: string[];
  unsubscribe: "own" | "footer";
}

export interface AudienceCheck {
  checked_at: string;
  contacts: number;
  mailable: number;
  left_out: { pending: number; unsubscribed: number; bounced: number; complained: number; suppressed: number };
  excluded: number;
  new_contacts: number;
  rows: CheckRow[];
  split: { variants: number; test: number; rest: number } | null;
  rate: { per_hour: number; workspace: number; own: number };
  ready: { public_url: boolean; account: string | null; postal_address: boolean };
  emails: CheckEmail[];
}

export interface LeftOut {
  key: keyof AudienceCheck["left_out"] | "excluded";
  label: string;
  count: number;
  note: string;
}

/** Why people in the audience are not mailed, each reason with its count, the biggest worry first. */
export function leftOut(check: AudienceCheck): LeftOut[] {
  const l = check.left_out;
  const all: LeftOut[] = [
    { key: "unsubscribed", label: t("Unsubscribed"), count: l.unsubscribed, note: "They opted out. Only they can opt back in." },
    { key: "complained", label: t("Marked an earlier email as spam"), count: l.complained, note: "Mailing them again would hurt your sending reputation." },
    { key: "bounced", label: t("Their address bounced"), count: l.bounced, note: "Emails to them were rejected." },
    { key: "suppressed", label: t("On the suppression list"), count: l.suppressed, note: "Blocked by hand, or after repeated bounces." },
    { key: "pending", label: t("Not confirmed yet"), count: l.pending, note: "They signed up but have not confirmed their address." },
    { key: "excluded", label: t("Left out by your exclusion rules"), count: check.excluded, note: "A segment or group you chose to leave out." },
  ];
  return all.filter((x) => x.count > 0);
}

export type Level = "ok" | "info" | "warn" | "block";

export interface Finding {
  id: string;
  level: Level;
  title: string;
  detail?: string;
  /** Where to fix it. */
  tab?: Tab;
  /** Where to fix it outside the campaign: a settings page of this app or of Mail. */
  href?: string;
}

/** How many emails go out in an hour, and so how long a send takes. */
export function sendTime(mailable: number, perHour: number): string {
  return span(perHour > 0 ? mailable / perHour : 0);
}

const IMG = /<img\b[^>]*>/gi;
const ALT = /\balt\s*=/i;
const ANCHOR = /<a\b[^>]*?\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')/gi;

/** Pictures with no `alt` text at all: a screen reader says nothing about them, and a mail client that blocks pictures shows a blank. */
export function imagesWithoutAlt(html: string): number {
  return (html.match(IMG) ?? []).filter((tag) => !ALT.test(tag)).length;
}

const unescape = (s: string) => s.replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'");

export interface LinkInfo {
  url: string;
  tracked: boolean;
  /** Why a link is not tracked. */
  why?: string;
}

/** Every link in the content, in order, once each, and whether clicks on it are counted. */
export function linksIn(html: string, tracked: string[], trackClicks: boolean): LinkInfo[] {
  const out: LinkInfo[] = [];
  const seen = new Set<string>();
  for (const m of html.matchAll(ANCHOR)) {
    const raw = (m[1] ?? m[2] ?? "").trim();
    if (!raw || seen.has(raw)) continue;
    seen.add(raw);
    const url = unescape(raw);
    if (raw.includes("{{") || raw.includes("{%")) out.push({ url: raw, tracked: false, why: "it changes for each person" });
    else if (/^mailto:/i.test(url)) out.push({ url, tracked: false, why: "an email address" });
    else if (!/^https?:\/\//i.test(url)) out.push({ url, tracked: false, why: url.startsWith("#") ? "a link within the email" : "not a web address" });
    else if (!trackClicks) out.push({ url, tracked: false, why: "click tracking is off" });
    else if (tracked.includes(url)) out.push({ url, tracked: true });
    else out.push({ url, tracked: false, why: "past the limit of 100 tracked links" });
  }
  return out;
}

/** Gmail cuts an email off about here, and hides the rest behind "View entire message". */
export const CLIP_BYTES = 102 * 1024;
const LONG_SUBJECT = 70;

const label = (variant: string | null) => (variant ? t("Version {variant}", { variant }) : t("The email"));

export function findings(check: AudienceCheck, c: Pick<Campaign, "subject" | "content" | "track_clicks" | "ab_test" | "variants" | "winner_after_hours" | "winner_metric">): Finding[] {
  const out: Finding[] = [];
  if (check.mailable === 0) {
    out.push({
      id: "nobody",
      level: "block",
      title: t("Nobody would receive this"),
      detail: check.contacts > 0 ? "Everyone in the audience is left out (see why below)." : "The audience is empty. Add a segment or an email group.",
      tab: "setup",
    });
  }
  if (!check.ready.public_url) {
    out.push({
      id: "public_url",
      level: "block",
      title: t("Tracking and unsubscribe links cannot be built yet"),
      detail: "Set the Public URL in the Mail app's Communication Settings: links in the email must reach this server from the internet.",
      href: "/open/CommsSettings",
    });
  }
  if (check.ready.account) out.push({ id: "account", level: "block", title: t("The From account cannot send"), detail: check.ready.account, tab: "setup" });
  for (const e of check.emails) {
    if (e.error) out.push({ id: `render-${e.variant ?? "main"}`, level: "block", title: t("{value} cannot be built", { value: label(e.variant) }), detail: e.error, tab: "content" });
  }
  if (!c.subject.trim()) out.push({ id: "subject", level: "block", title: t("There is no subject"), tab: "setup" });
  else if (c.subject.trim().length > LONG_SUBJECT) {
    out.push({ id: "subject-long", level: "warn", title: t("A long subject"), detail: t("Inboxes show about {LONG_SUBJECT} characters on a computer and fewer on a phone.", { LONG_SUBJECT }), tab: "setup" });
  }
  if (!check.ready.postal_address) {
    out.push({
      id: "postal",
      level: "warn",
      title: t("Your postal address is not set"),
      detail: "Anti-spam laws such as CAN-SPAM ask for one in every marketing email. Add it in Campaign Settings.",
      href: "/open/CampaignSettings",
    });
  }
  const main = check.emails[0];
  if (main && !main.error) {
    out.push(
      main.unsubscribe === "own"
        ? { id: "unsub", level: "ok", title: t("Your content has its own unsubscribe link") }
        : { id: "unsub", level: "ok", title: t("An unsubscribe link is added at the bottom"), detail: "Your content has none of its own, so the sender adds a footer with the link and your postal address." },
    );
  }
  const html = [c.content, ...(c.ab_test ? c.variants.map((v) => v.content ?? "") : [])].join("\n");
  const noAlt = imagesWithoutAlt(html);
  if (noAlt > 0) {
    out.push({ id: "alt", level: "warn", title: t("{noAlt} without alternative text", { noAlt: (noAlt === 1 ? t("1 picture") : t("{n} pictures", { n: count(noAlt) })) }), detail: "Add alt=\"…\" so they make sense when pictures are blocked.", tab: "content" });
  }
  const big = check.emails.filter((e) => (e.bytes ?? 0) > CLIP_BYTES);
  if (big.length) {
    out.push({ id: "size", level: "warn", title: t("The email is {value}", { value: size(Math.max(...big.map((e) => e.bytes ?? 0))) }), detail: "Gmail cuts emails off at about 100 KB and hides the rest.", tab: "content" });
  }
  if (!c.track_clicks) out.push({ id: "clicks", level: "info", title: t("Clicks are not tracked"), detail: "Turn click tracking on in Setup to see which links people use.", tab: "setup" });
  if (check.mailable > 0) {
    out.push({
      id: "rate",
      level: "info",
      title: t("Goes out at up to {value} an hour: {value2}", { value: (check.rate.per_hour === 1 ? t("1 email") : t("{n} emails", { n: count(check.rate.per_hour) })), value2: sendTime(check.mailable, check.rate.per_hour) }),
      detail: check.rate.own > 0 && check.rate.own < check.rate.workspace ? "This campaign's own limit applies." : "The limit is shared with newsletters and journeys.",
    });
  }
  if (check.split) {
    out.push({
      id: "split",
      level: "info",
      title: t("{value} get the {variants} versions first", { value: (check.split.test === 1 ? t("1 person") : t("{n} people", { n: count(check.split.test) })), variants: check.split.variants }),
      detail: ((c.winner_metric ?? "Opens") === "Clicks" ? t("The version with the most clicks goes to the other {people} after {hours}.", { people: (check.split.rest === 1 ? t("1 person") : t("{n} people", { n: count(check.split.rest) })), hours: ((c.winner_after_hours ?? 4) === 1 ? t("1 hour") : t("{n} hours", { n: count(c.winner_after_hours ?? 4) })) }) : t("The version with the most opens goes to the other {people} after {hours}.", { people: (check.split.rest === 1 ? t("1 person") : t("{n} people", { n: count(check.split.rest) })), hours: ((c.winner_after_hours ?? 4) === 1 ? t("1 hour") : t("{n} hours", { n: count(c.winner_after_hours ?? 4) })) })),
      tab: "setup",
    });
  }
  return out;
}

export const canSend = (f: Finding[]): boolean => !f.some((x) => x.level === "block");

/** The count that matters changed between two checks: say so in words. */
export function changedSince(before: number, now: number): string | null {
  if (before === now) return null;
  const diff = Math.abs(now - before);
  return (now > before ? t("Up {diff} since you last looked ({before} before).", { diff: diff.toLocaleString(), before: before.toLocaleString() }) : t("Down {diff} since you last looked ({before} before).", { diff: diff.toLocaleString(), before: before.toLocaleString() }));
}
