// Bringing a contact file in, in five steps: the file, which column is which, what the people
// agreed to, a check of who is new against who you already have (and who is blocked), and the
// import itself, in small batches that can be stopped. The records API takes the contacts and
// stays the judge of each row; nothing already in the contact list is changed, except that an
// opt-out list unsubscribes the people on it.

import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router";
import { api, errorMessage, saveBlob, shell } from "@ferrum/client";
import { Alert, Badge, Button, EmptyState, ErrorState, Field, Icon, PageLoading, useShellTitle } from "@ferrum/kit";
import { can, useEntityMeta } from "@ferrum/records";
import { list } from "./data";
import { count, noun, people, percent, rate, size } from "./format";
import {
  BASIS_KEY,
  type Checked,
  chunk,
  classify,
  type Consent,
  guessMapping,
  keyFor,
  type Mapping,
  mappingProblems,
  MAX_BYTES,
  MAX_ROWS,
  needAttention,
  pool,
  readContacts,
  readTable,
  reason,
  recordFor,
  type Refusal,
  type Table,
  type Target,
  tally,
  toCsv,
} from "./importer";
import { Nav } from "./Nav";
import { Bar } from "./ui";

const STEPS = ["File", "Columns", "Consent", "Check", "Import"] as const;
type Step = 0 | 1 | 2 | 3 | 4;

/** Contacts are created this many to a request: small enough to show progress and to stop between. */
const BATCH = 250;
/** Addresses looked up this many to a request (they travel in the address of a GET). */
const LOOKUP = 80;

interface Known {
  id: string;
  version: number;
  consent_status: string;
}

const KINDS: { id: Target["kind"]; label: string }[] = [
  { id: "skip", label: "Do not import" },
  { id: "email", label: "Email address" },
  { id: "first_name", label: "First name" },
  { id: "last_name", label: "Last name" },
  { id: "full_name", label: "Full name (split in two)" },
  { id: "attribute", label: "Keep as an attribute" },
];

function Rail({ step, reached }: { step: Step; reached: Step }) {
  return (
    <ol className="cm-rail" aria-label="Steps">
      {STEPS.map((s, i) => (
        <li key={s} aria-current={i === step ? "step" : undefined} data-done={i < reached || undefined}>
          <span className="cm-rail-n">{i < reached ? <Icon name="check" size={12} /> : i + 1}</span>
          {s}
        </li>
      ))}
    </ol>
  );
}

function Sample({ table, n = 5 }: { table: Table; n?: number }) {
  return (
    <div className="table-wrap">
      <table className="table">
        <thead>
          <tr>
            {table.headers.map((h, i) => (
              <th key={i} scope="col">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {table.rows.slice(0, n).map((r, i) => (
            <tr key={i}>
              {r.map((c, j) => (
                <td key={j}>{c}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Step 1: the file, or addresses pasted. */
function Source({ onTable }: { onTable: (t: Table, name: string) => void }) {
  const [problem, setProblem] = useState<string | null>(null);
  const [over, setOver] = useState(false);
  const [pasted, setPasted] = useState("");
  const read = (text: string, name: string) => {
    const t = readTable(text);
    if (!t.rows.length) return setProblem("There is nothing to import in it: no rows below the first line.");
    if (t.rows.length > MAX_ROWS) return setProblem(`It has ${count(t.rows.length)} rows. Import at most ${count(MAX_ROWS)} at a time: split the file and import the parts one after the other.`);
    setProblem(null);
    onTable(t, name);
  };
  const file = async (f: File | undefined) => {
    if (!f) return;
    if (f.size > MAX_BYTES) return setProblem(`That file is ${size(f.size)}. The most is ${size(MAX_BYTES)}.`);
    try {
      read(await f.text(), f.name);
    } catch {
      setProblem("The file could not be read.");
    }
  };
  return (
    <div className="stack" style={{ gap: 16 }}>
      <label
        className="cm-drop"
        data-over={over || undefined}
        onDragOver={(e) => (e.preventDefault(), setOver(true))}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          void file(e.dataTransfer.files[0]);
        }}
      >
        <Icon name="upload" size={22} />
        <strong>Choose a CSV file</strong>
        <span className="muted">or drop it here. The first row names the columns. Up to {count(MAX_ROWS)} rows.</span>
        <input className="sr-only" type="file" accept=".csv,.tsv,.txt,text/csv,text/plain" onChange={(e) => void file(e.target.files?.[0])} />
      </label>
      <Field label="Or paste them" htmlFor="cm-paste" hint="One address on each line, or rows copied from a spreadsheet with a header row.">
        <textarea id="cm-paste" className="textarea mono" rows={5} value={pasted} onChange={(e) => setPasted(e.target.value)} />
      </Field>
      <div>
        <Button disabled={!pasted.trim()} onClick={() => read(/\n/.test(pasted.trim()) || /[,;\t]/.test(pasted) ? pasted : `email\n${pasted.trim()}`, "pasted rows")}>
          Use what I pasted
        </Button>
      </div>
      {problem && <Alert tone="error">{problem}</Alert>}
    </div>
  );
}

/** Step 2: which column is which. */
function Columns({ table, mapping, onChange }: { table: Table; mapping: Mapping; onChange: (m: Mapping) => void }) {
  const problems = mappingProblems(mapping, table.headers);
  const set = (i: number, t: Target) => onChange(mapping.map((x, j) => (j === i ? t : x)));
  const read = useMemo(() => readContacts({ rows: table.rows.slice(0, 3), lines: table.lines.slice(0, 3) }, mapping), [table, mapping]);
  return (
    <div className="stack" style={{ gap: 16 }}>
      <p className="muted">
        {table.headerless ? "The file has no row of column names. " : ""}
        {noun(table.rows.length, "row")} found. Say what each column holds. Columns you do not choose are left out, so nothing is kept that you did not ask for.
      </p>
      <ul className="cm-cols">
        {table.headers.map((h, i) => {
          const t = mapping[i] ?? { kind: "skip" };
          const examples = table.rows.slice(0, 3).map((r) => r[i]).filter(Boolean);
          return (
            <li key={i}>
              <div className="cm-col-name">
                <strong>{h}</strong>
                <span className="faint cm-col-example">{examples.join(" · ") || "(empty)"}</span>
              </div>
              <select
                className="select"
                aria-label={`What “${h}” holds`}
                value={t.kind}
                onChange={(e) => {
                  const kind = e.target.value as Target["kind"];
                  set(i, kind === "attribute" ? { kind, key: keyFor(h) } : ({ kind } as Target));
                }}
              >
                {KINDS.map((k) => (
                  <option key={k.id} value={k.id}>
                    {k.label}
                  </option>
                ))}
              </select>
              {t.kind === "attribute" && (
                <input className="input cm-col-key" aria-label={`Attribute name for “${h}”`} placeholder="attribute_name" value={t.key} maxLength={40} onChange={(e) => set(i, { kind: "attribute", key: e.target.value })} />
              )}
            </li>
          );
        })}
      </ul>
      {problems.length > 0 && (
        <Alert tone="warning">
          <ul>
            {problems.map((p) => (
              <li key={p}>{p}</li>
            ))}
          </ul>
        </Alert>
      )}
      {problems.length === 0 && (
        <div>
          <h3 className="cm-sample-title">The first rows, as contacts</h3>
          <ul className="cm-sample">
            {read.map((r) => (
              <li key={r.line}>
                <div className="cm-sample-who">
                  {r.contact ? (
                    <>
                      <strong>{r.contact.email}</strong>
                      <span className="muted">{[r.contact.first_name, r.contact.last_name].filter(Boolean).join(" ") || "no name"}</span>
                      {Object.keys(r.contact.attributes).length > 0 && <span className="faint">{Object.entries(r.contact.attributes).map(([k, v]) => `${k}: ${v}`).join(" · ")}</span>}
                    </>
                  ) : (
                    <span className="muted">Line {r.line}: {r.problem}</span>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

/** Step 3: why these people may be written to, or that they asked not to be. */
function ConsentStep({ consent, onChange }: { consent: Consent; onChange: (c: Consent) => void }) {
  const agreed = consent.mode === "agreed";
  return (
    <div className="stack" style={{ gap: 16 }}>
      <fieldset className="cm-choices">
        <legend className="field-label">These people…</legend>
        <label className="cm-choice" data-on={agreed || undefined}>
          <input type="radio" name="cm-consent" checked={agreed} onChange={() => onChange({ mode: "agreed", basis: consent.mode === "agreed" ? consent.basis : "" })} />
          <span>
            <strong>agreed to get marketing email from us</strong>
            <span className="muted">They become subscribed. You say how you know, and it is kept on each of them.</span>
          </span>
        </label>
        <label className="cm-choice" data-on={!agreed || undefined}>
          <input type="radio" name="cm-consent" checked={!agreed} onChange={() => onChange({ mode: "optout" })} />
          <span>
            <strong>asked not to be emailed (an opt-out list)</strong>
            <span className="muted">They are added as unsubscribed and put on the suppression list. People you already have on it are unsubscribed too.</span>
          </span>
        </label>
      </fieldset>
      {agreed && (
        <Field label="How do you know they agreed?" htmlFor="cm-basis" required hint={`For example: signed up at the spring fair, 3 May 2026. It is saved on each contact as the attribute ${BASIS_KEY}, next to who added them and when.`}>
          <textarea id="cm-basis" className="textarea" rows={3} value={consent.basis} maxLength={500} onChange={(e) => onChange({ mode: "agreed", basis: e.target.value })} />
        </Field>
      )}
      <Alert tone="info">
        People who have not agreed yet cannot be imported as subscribers, and nobody can be made subscribed after they unsubscribed. Send them to a signup form: they confirm for themselves, and that is the record that counts.
      </Alert>
    </div>
  );
}

const FATE: Record<Checked["fate"], { label: string; tone: "green" | "neutral" | "amber" | "red" }> = {
  new: { label: "New", tone: "green" },
  known: { label: "Already a contact", tone: "neutral" },
  suppressed: { label: "Blocked", tone: "amber" },
  duplicate: { label: "Repeated", tone: "neutral" },
  invalid: { label: "Cannot import", tone: "red" },
};

/** Step 4: who is new, and who is not, against the contacts and the suppression list as they are now. */
function CheckStep({ checked, error, progress, consent, onRetry }: { checked: Checked[] | null; error: unknown; progress: { done: number; total: number }; consent: Consent; onRetry: () => void }) {
  if (error) return <ErrorState error={error} onRetry={onRetry} title="The addresses could not be checked" />;
  if (!checked) {
    return (
      <div className="stack" style={{ gap: 10 }} role="status">
        <p>Checking {count(progress.total)} addresses against your contacts and the suppression list…</p>
        <Bar value={rate(progress.done, progress.total)} label="Checked so far" />
      </div>
    );
  }
  const t = tally(checked);
  const attention = needAttention(checked, 20);
  const adds = t.new;
  return (
    <div className="stack" style={{ gap: 16 }}>
      <dl className="cm-tiles">
        <div className="cm-tile">
          <dt>Will be added</dt>
          <dd className="cm-tile-value">{count(adds)}</dd>
          <dd className="cm-tile-note faint">{consent.mode === "agreed" ? "As subscribed contacts." : "As unsubscribed, on the suppression list."}</dd>
        </div>
        <div className="cm-tile">
          <dt>Already contacts</dt>
          <dd className="cm-tile-value">{count(t.known)}</dd>
          <dd className="cm-tile-note faint">{consent.mode === "optout" ? "Unsubscribed if they are not already." : "Left exactly as they are."}</dd>
        </div>
        <div className="cm-tile">
          <dt>Blocked</dt>
          <dd className="cm-tile-value">{count(t.suppressed)}</dd>
          <dd className="cm-tile-note faint">On the suppression list, so not added.</dd>
        </div>
        <div className="cm-tile">
          <dt>Left out</dt>
          <dd className="cm-tile-value">{count(t.invalid + t.duplicate)}</dd>
          <dd className="cm-tile-note faint">
            {count(t.invalid)} not valid, {count(t.duplicate)} repeated in the file.
          </dd>
        </div>
      </dl>
      {attention.length > 0 && (
        <div>
          <h3 className="cm-sample-title">{checked.filter((c) => c.fate !== "new").length > attention.length ? "The first 20 that need a look" : "Rows that need a look"}</h3>
          <div className="table-wrap">
            <table className="table">
              <thead>
                <tr>
                  <th scope="col">Line</th>
                  <th scope="col">Address</th>
                  <th scope="col">What happens</th>
                </tr>
              </thead>
              <tbody>
                {attention.map((c) => (
                  <tr key={c.line}>
                    <td className="num">{c.line}</td>
                    <td>{c.email || <span className="faint">(none)</span>}</td>
                    <td>
                      <Badge tone={FATE[c.fate].tone}>{FATE[c.fate].label}</Badge> <span className="muted">{c.why}</span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
      {adds === 0 && !(consent.mode === "optout" && t.known > 0) && <Alert tone="warning">Nobody would be added. Go back to change the columns or the file.</Alert>}
    </div>
  );
}

interface Outcome {
  added: number;
  unsubscribed: number;
  refused: { line: number; email: string; why: string }[];
  done: number;
  total: number;
  stopped: boolean;
  finished: boolean;
  error: string | null;
}

export function Import() {
  useShellTitle("Import contacts");
  const meta = useEntityMeta("MarketingContact");
  const [step, setStep] = useState<Step>(0);
  const [reached, setReached] = useState<Step>(0);
  const [table, setTable] = useState<Table | null>(null);
  const [name, setName] = useState("");
  const [mapping, setMapping] = useState<Mapping>([]);
  const [consent, setConsent] = useState<Consent>({ mode: "agreed", basis: "" });
  const [checked, setChecked] = useState<Checked[] | null>(null);
  const [known, setKnown] = useState<Map<string, Known>>(new Map());
  const [checkError, setCheckError] = useState<unknown>(null);
  const [looked, setLooked] = useState({ done: 0, total: 0 });
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const stop = useRef(false);
  const running = !!outcome && !outcome.finished;
  useEffect(() => shell().leaveGuard(() => running), [running]);

  const go = (s: Step) => {
    setStep(s);
    setReached((r) => (s > r ? s : r) as Step);
  };
  const problems = table ? mappingProblems(mapping, table.headers) : [];
  const basisOk = consent.mode === "optout" || consent.basis.trim().length >= 5;

  // The check runs when its step opens, and again if the file, the columns or the consent changed.
  const checkKey = useMemo(() => JSON.stringify([name, table?.rows.length, mapping]), [name, table, mapping]);
  const ranFor = useRef("");
  const check = async () => {
    if (!table) return;
    ranFor.current = checkKey;
    setChecked(null);
    setCheckError(null);
    const read = readContacts(table, mapping);
    const emails = [...new Set(read.flatMap((r) => (r.contact ? [r.contact.email] : [])))];
    const groups = chunk(emails, LOOKUP);
    setLooked({ done: 0, total: emails.length });
    let done = 0;
    try {
      const find = (entity: "MarketingContact" | "EmailSuppression", fields: string) =>
        pool(groups, 4, async (g) => {
          const rows = (await list<Record<string, unknown> & { email: string }>(entity, { filters: JSON.stringify([["email", "in", g]]), fields, limit: 500 })).rows;
          if (entity === "MarketingContact") setLooked({ done: (done += g.length), total: emails.length });
          return rows;
        }).then((r) => r.flat().filter((x): x is NonNullable<typeof x> => !!x));
      const [contacts, blocked] = await Promise.all([find("MarketingContact", "email,consent_status,version"), find("EmailSuppression", "email")]);
      const map = new Map(contacts.map((c) => [String(c.email).toLowerCase(), { id: String(c.id), version: Number(c.version), consent_status: String(c.consent_status) }]));
      setKnown(map);
      // A row that is not a contact still shows the address it had, to be found in the file.
      const col = mapping.findIndex((t) => t.kind === "email");
      const raw = new Map(table.lines.map((line, r) => [line, col < 0 ? "" : (table.rows[r][col] ?? "")]));
      setChecked(classify(read, new Set(map.keys()), new Set(blocked.map((b) => String(b.email).toLowerCase())), (line) => raw.get(line) ?? ""));
    } catch (e) {
      setCheckError(e);
    }
  };
  useEffect(() => {
    if (step === 3 && table && problems.length === 0 && ranFor.current !== checkKey) void check();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, checkKey]);

  const run = async () => {
    if (!checked) return;
    stop.current = false;
    const creates = checked.filter((c) => c.fate === "new");
    const batches = chunk(creates, BATCH);
    // An opt-out list also unsubscribes the people you already have on it.
    const ids = consent.mode === "optout" ? checked.filter((c) => c.fate === "known").flatMap((c) => (["Subscribed", "Pending"].includes(known.get(c.email)?.consent_status ?? "") ? [known.get(c.email)!.id] : [])) : [];
    const updates = chunk(ids, 500);
    const total = creates.length + ids.length;
    const o: Outcome = { added: 0, unsubscribed: 0, refused: [], done: 0, total, stopped: false, finished: false, error: null };
    setOutcome({ ...o });
    go(4);
    try {
      for (const batch of batches) {
        if (stop.current) break;
        const r = await api.post<{ created: number; errors: Refusal[] }>("/api/v1/MarketingContact/_import", batch.map((c) => recordFor(c.contact!, consent)));
        o.added += r.created;
        for (const e of r.errors ?? []) o.refused.push({ line: batch[e.row]?.line ?? 0, email: batch[e.row]?.email ?? "", why: reason(e) });
        o.done += batch.length;
        setOutcome({ ...o });
      }
      for (const group of updates) {
        if (stop.current) break;
        const r = await api.post<{ succeeded: number; results: { id: string; ok: boolean; error?: { message?: string } }[] }>("/api/v1/MarketingContact/_bulk", { ids: group, op: "update", values: { consent_status: "Unsubscribed" } });
        o.unsubscribed += r.succeeded;
        for (const f of r.results.filter((x) => !x.ok)) o.refused.push({ line: 0, email: [...known.entries()].find(([, k]) => k.id === f.id)?.[0] ?? "", why: f.error?.message ?? "Could not be unsubscribed" });
        o.done += group.length;
        setOutcome({ ...o });
      }
      o.stopped = stop.current;
    } catch (e) {
      o.error = errorMessage(e);
    }
    o.finished = true;
    setOutcome({ ...o });
  };

  const notImported = () => {
    const rows = [
      ...(checked ?? []).filter((c) => c.fate !== "new").map((c) => [String(c.line), c.email, c.why ?? ""]),
      ...(outcome?.refused ?? []).map((r) => [r.line ? String(r.line) : "", r.email, r.why]),
    ];
    saveBlob(new Blob(["﻿" + toCsv(["Line", "Email", "Why it was not imported"], rows)], { type: "text/csv;charset=utf-8" }), "not-imported.csv");
  };

  if (meta.error) return <ErrorState error={meta.error} onRetry={() => void meta.refetch()} />;
  if (!meta.data) return <PageLoading />;
  if (!can(meta.data.permissions, "import") || !can(meta.data.permissions, "create")) {
    return (
      <EmptyState title="You cannot import contacts" icon="upload">
        Ask a Marketing Manager for the Marketing User role.
      </EmptyState>
    );
  }

  const next = (s: Step, ok: boolean) => (
    <div className="row cm-wizard-foot">
      {s > 0 && s < 4 && (
        <Button onClick={() => go((s - 1) as Step)}>Back</Button>
      )}
      {s < 3 && (
        <Button variant="primary" disabled={!ok} onClick={() => go((s + 1) as Step)}>
          Continue
        </Button>
      )}
    </div>
  );

  return (
    <div className="cm-page">
      <header className="cm-head">
        <div>
          <h1 className="sr-only">Import contacts</h1>
          <Nav />
          <p className="page-sub">Bring people in from a file, with a record of why you may email them.</p>
        </div>
      </header>
      <div className="cm-wizard">
        <Rail step={step} reached={reached} />
        <section className="card card-pad cm-wizard-body" aria-label={STEPS[step]}>
          {step === 0 && (
            <>
              <h2 className="cm-wizard-title">Choose the file</h2>
              {table ? (
                <div className="stack" style={{ gap: 14 }}>
                  <p>
                    <strong>{name}</strong>: {noun(table.rows.length, "row")}, {noun(table.headers.length, "column")}.{" "}
                    <button type="button" className="link-btn" onClick={() => (setTable(null), setMapping([]), setChecked(null))}>
                      Choose another
                    </button>
                  </p>
                  <Sample table={table} />
                </div>
              ) : (
                <Source
                  onTable={(t, n) => {
                    setTable(t);
                    setName(n);
                    setMapping(guessMapping(t.headers, t.rows));
                    setChecked(null);
                  }}
                />
              )}
              {next(0, !!table)}
            </>
          )}
          {step === 1 && table && (
            <>
              <h2 className="cm-wizard-title">Which column is which?</h2>
              <Columns table={table} mapping={mapping} onChange={(m) => (setMapping(m), setChecked(null))} />
              {next(1, problems.length === 0)}
            </>
          )}
          {step === 2 && (
            <>
              <h2 className="cm-wizard-title">What did they agree to?</h2>
              <ConsentStep consent={consent} onChange={setConsent} />
              {next(2, basisOk)}
            </>
          )}
          {step === 3 && (
            <>
              <h2 className="cm-wizard-title">Check before importing</h2>
              <CheckStep checked={checked} error={checkError} progress={looked} consent={consent} onRetry={() => void check()} />
              <div className="row cm-wizard-foot">
                <Button onClick={() => go(2)}>Back</Button>
                <Button
                  variant="primary"
                  disabled={!checked || (tally(checked).new === 0 && !(consent.mode === "optout" && tally(checked).known > 0))}
                  onClick={() => void run()}
                >
                  {checked ? `Import ${people(tally(checked).new + (consent.mode === "optout" ? tally(checked).known : 0))}` : "Import"}
                </Button>
              </div>
            </>
          )}
          {step === 4 && outcome && (
            <>
              <h2 className="cm-wizard-title">{outcome.finished ? (outcome.stopped ? "Stopped" : outcome.error ? "Stopped by a problem" : "Done") : "Importing…"}</h2>
              <div className="stack" style={{ gap: 14 }}>
                <Bar value={rate(outcome.done, outcome.total)} label="Imported so far" />
                <p role="status" aria-live="polite">
                  {count(outcome.done)} of {count(outcome.total)} handled: <strong>{count(outcome.added)}</strong> added
                  {outcome.unsubscribed > 0 && (
                    <>
                      , <strong>{count(outcome.unsubscribed)}</strong> unsubscribed
                    </>
                  )}
                  {outcome.refused.length > 0 && (
                    <>
                      , <strong>{count(outcome.refused.length)}</strong> refused
                    </>
                  )}
                  . {outcome.total > 0 && !outcome.finished ? `${percent(rate(outcome.done, outcome.total))}` : ""}
                </p>
                {outcome.error && <Alert tone="error">{outcome.error} What was added before the problem stays added; running the file again skips those people.</Alert>}
                {outcome.stopped && <Alert tone="warning">You stopped it. What was added stays added; running the file again skips those people.</Alert>}
                {outcome.refused.length > 0 && (
                  <div className="table-wrap">
                    <table className="table">
                      <thead>
                        <tr>
                          <th scope="col">Line</th>
                          <th scope="col">Address</th>
                          <th scope="col">Why</th>
                        </tr>
                      </thead>
                      <tbody>
                        {outcome.refused.slice(0, 20).map((r, i) => (
                          <tr key={i}>
                            <td className="num">{r.line || ""}</td>
                            <td>{r.email}</td>
                            <td className="muted">{r.why}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
                <div className="row">
                  {!outcome.finished ? (
                    <Button variant="danger" onClick={() => (stop.current = true)}>
                      Stop
                    </Button>
                  ) : (
                    <>
                      <Link className="btn btn-primary btn-md" to="/e/MarketingContact?f.source=Import">
                        See the contacts
                      </Link>
                      <Button icon="download" onClick={notImported}>
                        Download what was not imported
                      </Button>
                      <Button
                        onClick={() => {
                          setTable(null);
                          setMapping([]);
                          setChecked(null);
                          setOutcome(null);
                          setReached(0);
                          go(0);
                        }}
                      >
                        Import another file
                      </Button>
                    </>
                  )}
                </div>
              </div>
            </>
          )}
        </section>
      </div>
    </div>
  );
}
