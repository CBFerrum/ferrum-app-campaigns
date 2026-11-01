// The arithmetic of a chart: round axes, scales, paths, and the label for a point in time. Pure, so
// it is tested without a browser; Chart.tsx only draws what this works out.

export interface Axis {
  /** The top of the axis: a round number at or above the data. */
  max: number;
  ticks: number[];
}

const WHOLE = [1, 2, 5, 10];
const ANY = [1, 2, 2.5, 5, 10];

/** An axis from 0 to a round number above `max`, in about `want` steps: counts step by whole numbers. */
export function niceAxis(max: number, { want = 4, integers = true }: { want?: number; integers?: boolean } = {}): Axis {
  if (!(max > 0) || !Number.isFinite(max)) return { max: integers ? 4 : 1, ticks: integers ? [0, 1, 2, 3, 4] : [0, 1] };
  const pow = 10 ** Math.floor(Math.log10(max / want));
  const candidates: number[] = [];
  for (let p = pow / 10; p <= pow * 10; p *= 10) for (const s of integers ? WHOLE : ANY) candidates.push(s * p);
  candidates.sort((a, b) => a - b);
  // The smallest step that needs few enough lines (counts may take one more, to hug the data).
  const limit = integers ? want + 1 : want;
  const step = candidates.find((c) => (!integers || c >= 1) && Math.ceil(max / c - 1e-9) <= limit) ?? pow * 10;
  const top = Math.ceil(max / step - 1e-9) * step;
  const ticks: number[] = [];
  for (let v = 0; v <= top + step / 2; v += step) ticks.push(Math.round(v * 1e6) / 1e6);
  return { max: Math.round(top * 1e6) / 1e6, ticks };
}

/** Maps a value in [d0, d1] onto [r0, r1]. */
export const scaleLinear = (d0: number, d1: number, r0: number, r1: number) => (v: number) =>
  d1 === d0 ? (r0 + r1) / 2 : r0 + ((v - d0) / (d1 - d0)) * (r1 - r0);

const r2 = (n: number) => Math.round(n * 100) / 100;

/** "M x,y L x,y …" through the points. */
export function linePath(xs: number[], ys: number[]): string {
  return xs.map((x, i) => `${i ? "L" : "M"}${r2(x)},${r2(ys[i])}`).join(" ");
}

/** The area between a line and the baseline. */
export function areaPath(xs: number[], ys: number[], base: number): string {
  if (!xs.length) return "";
  return `${linePath(xs, ys)} L${r2(xs[xs.length - 1])},${r2(base)} L${r2(xs[0])},${r2(base)} Z`;
}

/** A column square at the baseline and rounded at the top. */
export function columnPath(x: number, w: number, top: number, base: number, radius = 4): string {
  const h = base - top;
  if (!(h > 0) || !(w > 0)) return "";
  const r = Math.min(radius, w / 2, h);
  return `M${r2(x)},${r2(base)} V${r2(top + r)} Q${r2(x)},${r2(top)} ${r2(x + r)},${r2(top)} H${r2(x + w - r)} Q${r2(x + w)},${r2(top)} ${r2(x + w)},${r2(top + r)} V${r2(base)} Z`;
}

/** The index of the point closest to `px`; `xs` ascends. */
export function nearest(xs: number[], px: number): number {
  if (!xs.length) return -1;
  let lo = 0;
  let hi = xs.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (xs[mid] <= px) lo = mid;
    else hi = mid;
  }
  return Math.abs(xs[lo] - px) <= Math.abs(xs[hi] - px) ? lo : hi;
}

/** Running totals. */
export function cumulative(values: number[]): number[] {
  let sum = 0;
  return values.map((v) => (sum += v));
}

/** Which of `n` points get an axis label when at most `max` fit: evenly spaced, the first always. */
export function labelled(n: number, max: number): number[] {
  if (n <= 0 || max <= 0) return [];
  const stride = Math.max(1, Math.ceil(n / max));
  const out: number[] = [];
  for (let i = 0; i < n; i += stride) out.push(i);
  return out;
}

export type Interval = "hour" | "day";

/** "2026-11-01T14:00:00" (the workspace's wall clock) as a Date holding the same wall-clock parts. */
export function wall(at: string): Date {
  const m = /^(\d{4})-(\d{2})-(\d{2})(?:T(\d{2}):(\d{2}))?/.exec(at);
  if (!m) return new Date(NaN);
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]), Number(m[4] ?? 0), Number(m[5] ?? 0));
}

/** The label under an axis: a time of day, and the date where a day begins. */
export function tickLabel(at: string, interval: Interval, first: boolean, locale?: string): string {
  const d = wall(at);
  if (Number.isNaN(d.getTime())) return "";
  if (interval === "day") return d.toLocaleDateString(locale, { day: "numeric", month: "short" });
  if (first || d.getHours() === 0) return d.toLocaleDateString(locale, { weekday: "short", day: "numeric", month: "short" });
  return d.toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" });
}

/** The full name of a bucket, for a tooltip or a table row. */
export function bucketName(at: string, interval: Interval, locale?: string): string {
  const d = wall(at);
  if (Number.isNaN(d.getTime())) return at;
  const day = d.toLocaleDateString(locale, { weekday: "short", day: "numeric", month: "short" });
  return interval === "day" ? day : `${day}, ${d.toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" })}`;
}

/** Per-bucket counts of one kind, as the timeline answers them. */
export type Counts = number[];

/** `part[i] / whole[i]` as a percentage, 0 where nothing was counted yet. */
export function ratesOver(part: Counts, whole: Counts): number[] {
  return part.map((p, i) => (whole[i] > 0 ? (p / whole[i]) * 100 : 0));
}

/** The bucket a moment falls in, written like the timeline writes them: the workspace's wall clock. */
export function localBucket(iso: string, timezone: string, interval: Interval): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  let parts: Intl.DateTimeFormatPart[];
  try {
    parts = new Intl.DateTimeFormat("en-US", { timeZone: timezone, hourCycle: "h23", year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit" }).formatToParts(at);
  } catch {
    return "";
  }
  const p = (t: string) => parts.find((x) => x.type === t)?.value ?? "00";
  const hour = interval === "hour" ? p("hour").padStart(2, "0").replace("24", "00") : "00";
  return `${p("year")}-${p("month")}-${p("day")}T${hour}:00:00`;
}
