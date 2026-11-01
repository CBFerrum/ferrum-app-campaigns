// Bringing a contact file in: reading the CSV, guessing which column is which, turning rows into
// contacts, and sorting them into who is new, who is known, and who cannot be imported. The
// server still decides what it accepts (and says why it refuses a row); this keeps the person
// from finding out one row at a time. Pure, so it is tested without a browser.

import { validKey } from "./snippets";

export const MAX_ROWS = 20_000;
export const MAX_BYTES = 8 * 1024 * 1024;

// ── reading the file ──

/** The delimiter most lines agree on: a comma, a semicolon, a tab or a bar. */
export function delimiterOf(text: string): string {
  const lines = text.replace(/^﻿/, "").split(/\r?\n/).filter(Boolean).slice(0, 5);
  let best = ",";
  let bestScore = 0;
  for (const d of [",", ";", "\t", "|"]) {
    // Outside quotes, how many fields the first lines have, and that they agree.
    const counts = lines.map((l) => fields(l, d).length);
    const score = counts[0] > 1 && counts.every((n) => n === counts[0]) ? counts[0] : 0;
    if (score > bestScore) [best, bestScore] = [d, score];
  }
  return best;
}

function fields(line: string, d: string): string[] {
  return parseCsv(line, d)[0] ?? [];
}

/** RFC 4180 rows: quoted fields may hold the delimiter, quotes ("") and line breaks. A leading
 *  byte-order mark and a closing line break are ignored. */
export function parseCsv(text: string, delimiter = ","): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let quoted = false;
  const src = text.replace(/^﻿/, "");
  for (let i = 0; i < src.length; i++) {
    const ch = src[i];
    if (quoted) {
      if (ch === '"') {
        if (src[i + 1] === '"') {
          cell += '"';
          i++;
        } else quoted = false;
      } else cell += ch;
    } else if (ch === '"' && cell === "") quoted = true;
    else if (ch === delimiter) {
      row.push(cell);
      cell = "";
    } else if (ch === "\n" || ch === "\r") {
      if (ch === "\r" && src[i + 1] === "\n") i++;
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
    } else cell += ch;
  }
  if (cell !== "" || row.length) {
    row.push(cell);
    rows.push(row);
  }
  return rows;
}

export interface Table {
  delimiter: string;
  headers: string[];
  /** The data rows: cells trimmed, every row as wide as the header, empty rows gone. */
  rows: string[][];
  /** The line of the file each row came from (the header is line 1). */
  lines: number[];
  /** The file had no header row: its first row is data. */
  headerless: boolean;
}

const AT = /@/;

export function readTable(text: string): Table {
  const delimiter = delimiterOf(text);
  const all = parseCsv(text, delimiter);
  const width = Math.max(0, ...all.map((r) => r.length));
  const first = all[0] ?? [];
  // A first row with an address in it is data, not column names.
  const headerless = first.some((c) => AT.test(c) && !/\s/.test(c.trim()));
  const headers = headerless ? Array.from({ length: width }, (_, i) => `Column ${i + 1}`) : Array.from({ length: width }, (_, i) => (first[i] ?? "").trim() || `Column ${i + 1}`);
  const rows: string[][] = [];
  const lines: number[] = [];
  all.forEach((r, i) => {
    if (i === 0 && !headerless) return;
    const cells = Array.from({ length: width }, (_, c) => (r[c] ?? "").trim());
    if (cells.every((c) => c === "")) return;
    rows.push(cells);
    lines.push(i + 1);
  });
  return { delimiter, headers, rows, lines, headerless };
}

// ── mapping columns ──

export type Target = { kind: "skip" } | { kind: "email" } | { kind: "first_name" } | { kind: "last_name" } | { kind: "full_name" } | { kind: "attribute"; key: string };
export type Mapping = Target[];

const norm = (h: string) => h.toLowerCase().replace(/[^a-z0-9]/g, "");
const EMAIL = new Set(["email", "emailaddress", "emailid", "mail", "eaddress"]);
const FIRST = new Set(["firstname", "first", "givenname", "fname", "forename"]);
const LAST = new Set(["lastname", "last", "surname", "familyname", "lname"]);
const FULL = new Set(["name", "fullname", "contactname", "customername"]);

/** The column that looks like each thing; a column that looks like none is left out until the person says. */
export function guessMapping(headers: string[], rows: string[][] = []): Mapping {
  const taken = new Set<string>();
  const once = (kind: Target["kind"], t: Target): Target => {
    if (taken.has(kind)) return { kind: "skip" };
    taken.add(kind);
    return t;
  };
  const guessed = headers.map((h): Target => {
    const n = norm(h);
    if (EMAIL.has(n)) return once("email", { kind: "email" });
    if (FIRST.has(n)) return once("first_name", { kind: "first_name" });
    if (LAST.has(n)) return once("last_name", { kind: "last_name" });
    if (FULL.has(n)) return once("full_name", { kind: "full_name" });
    return { kind: "skip" };
  });
  // No header says which is the address: the column full of them is.
  if (!taken.has("email") && rows.length) {
    const col = headers.findIndex((_, c) => rows.slice(0, 20).filter((r) => AT.test(r[c] ?? "")).length >= Math.min(rows.length, 20) / 2);
    if (col >= 0) guessed[col] = { kind: "email" };
  }
  // A full name beside a first or last name would only repeat them.
  if (guessed.some((t) => t.kind === "first_name" || t.kind === "last_name")) return guessed.map((t) => (t.kind === "full_name" ? { kind: "skip" } : t));
  return guessed;
}

/** A column header as an attribute key: lowercase letters, digits and _ , starting with a letter. */
export function keyFor(header: string): string {
  const k = header
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .replace(/^[0-9_]+/, "")
    .slice(0, 40);
  return validKey(k) ? k : "";
}

/** The attribute that holds why we may write to these people. */
export const BASIS_KEY = "consent_basis";

/** What is wrong with a mapping as a whole, in words. */
export function mappingProblems(mapping: Mapping, headers: string[]): string[] {
  const out: string[] = [];
  if (!mapping.some((t) => t.kind === "email")) out.push("Choose which column holds the email address.");
  const seen = new Map<string, string>();
  mapping.forEach((t, i) => {
    if (t.kind === "skip") return;
    const id = t.kind === "attribute" ? `attribute ${t.key}` : t.kind;
    const label = headers[i] ?? `Column ${i + 1}`;
    if (t.kind === "attribute") {
      if (!validKey(t.key)) out.push(`“${label}”: the attribute name needs lowercase letters, digits and _, starting with a letter.`);
      else if (t.key === BASIS_KEY) out.push(`“${label}”: ${BASIS_KEY} is kept for why you may email these people.`);
    }
    const before = seen.get(id);
    if (before) out.push(`“${before}” and “${label}” both go to ${t.kind === "attribute" ? `the attribute ${t.key}` : t.kind.replace("_", " ")}.`);
    else seen.set(id, label);
  });
  return out;
}

// ── contacts from rows ──

export interface Contact {
  email: string;
  first_name?: string;
  last_name?: string;
  attributes: Record<string, string>;
}

export interface Read {
  /** The line of the file. */
  line: number;
  contact: Contact | null;
  problem?: string;
}

/** The pattern the server uses for an address worth importing. */
const ADDRESS = /^[^@\s]{1,64}@[^@\s]+\.[^@\s]+$/;
export const validAddress = (email: string): boolean => email.length <= 254 && ADDRESS.test(email);

const NAME_MAX = 100;
const VALUE_MAX = 1000;

/** Splits "Asha Rao" into a first name and the rest, the way the Mail app's group members are. */
export function splitName(full: string): { first_name?: string; last_name?: string } {
  const t = full.trim().replace(/\s+/g, " ");
  if (!t) return {};
  const at = t.indexOf(" ");
  return at < 0 ? { first_name: t } : { first_name: t.slice(0, at), last_name: t.slice(at + 1) };
}

export function readContacts(table: Pick<Table, "rows" | "lines">, mapping: Mapping): Read[] {
  return table.rows.map((cells, r) => {
    const line = table.lines[r];
    const cell = (kind: Target["kind"]) => {
      const c = mapping.findIndex((t) => t.kind === kind);
      return c < 0 ? "" : (cells[c] ?? "").trim();
    };
    const email = cell("email").toLowerCase();
    if (!email) return { line, contact: null, problem: "No email address" };
    if (!validAddress(email)) return { line, contact: null, problem: "Not a valid email address" };
    const named = cell("first_name") || cell("last_name") ? { first_name: cell("first_name") || undefined, last_name: cell("last_name") || undefined } : splitName(cell("full_name"));
    for (const [what, v] of [["First name", named.first_name], ["Last name", named.last_name]] as const) {
      if (v && v.length > NAME_MAX) return { line, contact: null, problem: `${what} is longer than ${NAME_MAX} characters` };
    }
    const attributes: Record<string, string> = {};
    for (const [c, t] of mapping.entries()) {
      if (t.kind !== "attribute") continue;
      const v = (cells[c] ?? "").trim();
      if (!v) continue;
      if (v.length > VALUE_MAX) return { line, contact: null, problem: `A value is longer than ${VALUE_MAX} characters (${t.key})` };
      attributes[t.key] = v;
    }
    return { line, contact: { email, ...(named.first_name ? { first_name: named.first_name } : {}), ...(named.last_name ? { last_name: named.last_name } : {}), attributes } };
  });
}

// ── who is new ──

export type Fate = "new" | "known" | "suppressed" | "duplicate" | "invalid";

export interface Checked {
  line: number;
  email: string;
  fate: Fate;
  why?: string;
  contact?: Contact;
}

/** Sorts what was read: invalid ones, repeats within the file (the first stays), addresses already
 *  contacts, addresses on the suppression list (which are never added: a record that says they agreed
 *  would contradict the list), and new ones. */
export function classify(read: Read[], known: Set<string>, suppressed: Set<string>, rawEmail: (line: number) => string = () => ""): Checked[] {
  const first = new Map<string, number>();
  return read.map((r) => {
    if (!r.contact) return { line: r.line, email: rawEmail(r.line), fate: "invalid", why: r.problem };
    const { email } = r.contact;
    const before = first.get(email);
    if (before !== undefined) return { line: r.line, email, fate: "duplicate", why: `Same address as line ${before}`, contact: r.contact };
    first.set(email, r.line);
    if (known.has(email)) return { line: r.line, email, fate: "known", why: "Already a contact", contact: r.contact };
    if (suppressed.has(email)) return { line: r.line, email, fate: "suppressed", why: "On the suppression list: not added", contact: r.contact };
    return { line: r.line, email, fate: "new", contact: r.contact };
  });
}

export function tally(checked: Checked[]): Record<Fate, number> {
  const t: Record<Fate, number> = { new: 0, known: 0, suppressed: 0, duplicate: 0, invalid: 0 };
  for (const c of checked) t[c.fate]++;
  return t;
}

/** The first few rows that need a look. */
export const needAttention = (checked: Checked[], n = 20): Checked[] => checked.filter((c) => c.fate !== "new").slice(0, n);

// ── what is sent ──

export type Consent = { mode: "agreed"; basis: string } | { mode: "optout" };

/** A contact as the records API takes it. People who agreed are Subscribed, with the basis saved on
 *  them as an attribute (the server records who added them and when); an opt-out list is Unsubscribed,
 *  which also puts them on the suppression list. */
export function recordFor(c: Contact, consent: Consent): Record<string, unknown> {
  const attributes = consent.mode === "agreed" ? { ...c.attributes, [BASIS_KEY]: consent.basis.trim() } : c.attributes;
  return {
    email: c.email,
    ...(c.first_name ? { first_name: c.first_name } : {}),
    ...(c.last_name ? { last_name: c.last_name } : {}),
    attributes,
    source: "Import",
    consent_status: consent.mode === "agreed" ? "Subscribed" : "Unsubscribed",
  };
}

export function chunk<T>(items: T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
  return out;
}

export interface Refusal {
  row: number;
  error: string;
  message?: string;
  fields?: { field: string; message: string }[];
}

/** What the server said about a row it refused, in a sentence. */
export function reason(r: Refusal): string {
  if (r.fields?.length) return r.fields.map((f) => f.message).join("; ");
  if (r.error === "conflict") return "Already a contact";
  return r.message ?? r.error;
}

function quote(cell: string): string {
  return /[",\r\n]/.test(cell) ? `"${cell.replace(/"/g, '""')}"` : cell;
}

/** Rows as a CSV file's text, headers first. */
export function toCsv(headers: string[], rows: string[][]): string {
  return [headers, ...rows].map((r) => r.map(quote).join(",")).join("\r\n") + "\r\n";
}

/** The addresses in a box where people type or paste them, separated by commas, semicolons, spaces or line breaks. */
export function splitAddresses(text: string): { valid: string[]; invalid: string[] } {
  const seen = new Set<string>();
  const valid: string[] = [];
  const invalid: string[] = [];
  for (const raw of text.split(/[,;\s]+/)) {
    const a = raw.trim().toLowerCase();
    if (!a || seen.has(a)) continue;
    seen.add(a);
    (validAddress(a) ? valid : invalid).push(a);
  }
  return { valid, invalid };
}

/** `fn` over `items`, at most `size` at a time; the results in the order of the items. Stops starting new ones once `stop` says so. */
export async function pool<T, R>(items: T[], size: number, fn: (item: T, index: number) => Promise<R>, stop: () => boolean = () => false): Promise<(R | undefined)[]> {
  const out: (R | undefined)[] = new Array(items.length).fill(undefined);
  let next = 0;
  const worker = async () => {
    while (next < items.length && !stop()) {
      const i = next++;
      out[i] = await fn(items[i], i);
    }
  };
  await Promise.all(Array.from({ length: Math.min(size, items.length) }, worker));
  return out;
}
