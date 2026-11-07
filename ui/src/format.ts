// Numbers and times as people say them. Pure, so the wording is tested without a browser. Times
// are on the workspace's clock (the kit's), named when the browser's is another.

import { formatMoment, fromZonedInput, shownTimeZone, toZonedInput, t } from "@ferrum/client";

/** 4812 → "4,812". */
export const count = (n: number | null | undefined): string => (n ?? 0).toLocaleString();

/** 1250000 → "1.3M": for axes and tight places. */
export const compact = (n: number): string => n.toLocaleString(undefined, { notation: "compact", maximumFractionDigits: 1 });

/** `part` as a percentage of `whole`; null when there is nothing to compare with. */
export function rate(part: number, whole: number): number | null {
  return whole > 0 ? (part / whole) * 100 : null;
}

/** 31.42 → "31.4%", 100 → "100%", 0.04 → "<0.1%", null → "–". */
export function percent(v: number | null | undefined): string {
  if (v == null || !Number.isFinite(v)) return "–";
  if (v > 0 && v < 0.1) return "<0.1%";
  const text = v >= 10 ? v.toFixed(v >= 99.95 ? 0 : 1) : v.toFixed(1);
  return `${text.replace(/\.0$/, "")}%`;
}

/** "1 person", "4,812 people". */
export const people = (n: number): string => `${count(n)} ${n === 1 ? "person" : "people"}`;

/** The first of `singular`/`plural` that fits `n`. */
export const noun = (n: number, singular: string, plural = `${singular}s`): string => `${count(n)} ${n === 1 ? singular : plural}`;

/** A moment on the workspace's clock: "Fri 7 Nov, 10:30". */
export function moment(iso: string | null | undefined, locale?: string): string {
  if (!iso || Number.isNaN(Date.parse(iso))) return "";
  return formatMoment(iso, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" }, locale);
}

/** The time of day of a moment, on the workspace's clock: "10:30". */
export const clock = (at: string | number): string => formatMoment(at, { hour: "numeric", minute: "2-digit" });

/** How long a number of hours is, roughly: "under an hour", "about 3 hours", "about 2 days". */
export function span(hours: number): string {
  if (!(hours > 0.75)) return "under an hour";
  if (hours < 36) {
    const h = Math.round(hours);
    return (h === 1 ? t("about 1 hour") : t("about {h} hours", { h }));
  }
  const d = Math.round(hours / 24);
  return (d === 1 ? t("about 1 day") : t("about {d} days", { d }));
}

/** Bytes as KB or MB. */
export function size(bytes: number): string {
  if (bytes < 1024) return t("{bytes} bytes", { bytes });
  if (bytes < 1024 * 1024) return t("{n} KB", { n: Math.round(bytes / 1024) });
  return t("{value} MB", { value: (bytes / 1024 / 1024).toFixed(1) });
}

/** A moment as a "datetime-local" box wants it, on the workspace's clock: "2026-11-07T10:30". */
export const toLocalInput = (iso: string | null | undefined): string => toZonedInput(iso);

/** What a "datetime-local" box holds, as the moment it means on the workspace's clock. */
export const fromLocalInput = (value: string): string | null => fromZonedInput(value);

/** The zone of the workspace's clock as people write it: "IST", "GMT+1". */
export function zoneName(locale?: string, at = new Date()): string {
  return new Intl.DateTimeFormat(locale, { timeZone: shownTimeZone(), timeZoneName: "short" }).formatToParts(at).find((p) => p.type === "timeZoneName")?.value ?? "";
}
