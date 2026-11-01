// Numbers and times as people say them. Pure, so the wording is tested without a browser.

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

/** A moment in the viewer's own time: "Fri 7 Nov, 10:30". */
export function moment(iso: string | null | undefined, locale?: string): string {
  if (!iso) return "";
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  return at.toLocaleString(locale, { weekday: "short", day: "numeric", month: "short", hour: "numeric", minute: "2-digit" });
}

/** How long a number of hours is, roughly: "under an hour", "about 3 hours", "about 2 days". */
export function span(hours: number): string {
  if (!(hours > 0.75)) return "under an hour";
  if (hours < 36) {
    const h = Math.round(hours);
    return `about ${h} ${h === 1 ? "hour" : "hours"}`;
  }
  const d = Math.round(hours / 24);
  return `about ${d} ${d === 1 ? "day" : "days"}`;
}

/** Bytes as KB or MB. */
export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} bytes`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

const two = (n: number) => String(n).padStart(2, "0");

/** A moment as a "datetime-local" box wants it: the viewer's own time, "2026-11-07T10:30". */
export function toLocalInput(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}T${two(d.getHours())}:${two(d.getMinutes())}`;
}

/** What a "datetime-local" box holds, as the moment it means in the viewer's time zone. */
export function fromLocalInput(value: string): string | null {
  if (!value) return null;
  const d = new Date(value);
  return Number.isNaN(d.getTime()) ? null : d.toISOString();
}

/** The viewer's time zone as people write it: "IST", "GMT+1". */
export function zoneName(locale?: string, at = new Date()): string {
  return new Intl.DateTimeFormat(locale, { timeZoneName: "short" }).formatToParts(at).find((p) => p.type === "timeZoneName")?.value ?? "";
}
