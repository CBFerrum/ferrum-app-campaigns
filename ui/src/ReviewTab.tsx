// The third tab, before anything goes out: how many people will get the email, who is left out and
// why (the server counts it the way sending counts it), what would stop the send or deserves a
// look, and then when to send it. The final count is taken again at the last moment, and a person
// is asked to confirm the number they will actually reach.

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { api, docPath, errorMessage } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, ErrorState, Icon, PageLoading, toast } from "@ferrum/kit";
import { type Campaign, type Tab } from "./campaign";
import { checkAudience, keys, whole } from "./data";
import { count, fromLocalInput, moment, noun, people, toLocalInput, zoneName } from "./format";
import { type AudienceCheck, canSend, changedSince, type Finding, findings, leftOut, linksIn, type Level } from "./review";
import { checkKey, useAudienceCheck } from "./useCheck";
import type { Draft } from "./useDraft";
import { Section } from "./ui";

const ORDER: Level[] = ["block", "warn", "info", "ok"];

/** The audience as one bar: who will be emailed, and each reason someone is not. */
function Split({ check }: { check: AudienceCheck }) {
  const parts = [{ key: "mail", label: "Will be emailed", n: check.mailable }, ...leftOut(check).map((l) => ({ key: l.key, label: l.label, n: l.count }))].filter((p) => p.n > 0);
  if (!parts.length) return null;
  return (
    <div className="cm-split" role="img" aria-label={parts.map((p) => `${p.label}: ${count(p.n)}`).join("; ")}>
      {parts.map((p) => (
        <span key={p.key} data-part={p.key} style={{ flexGrow: p.n }} title={`${p.label}: ${count(p.n)}`} />
      ))}
    </div>
  );
}

function Reach({ check, fetching, onAgain }: { check: AudienceCheck; fetching: boolean; onAgain: () => void }) {
  const reasons = leftOut(check);
  return (
    <Section
      title="Who will get it"
      id="cm-reach"
      aside={
        <Button size="sm" icon="refresh" loading={fetching} onClick={onAgain}>
          Count again
        </Button>
      }
    >
      <div className="cm-reach">
        <p className="cm-reach-figure" aria-live="polite">
          <span className="cm-reach-number">{count(check.mailable)}</span> <span className="cm-reach-unit">{check.mailable === 1 ? "person" : "people"} will get this email</span>
        </p>
        <Split check={check} />
        {reasons.length > 0 ? (
          <ul className="cm-reasons" aria-label="Left out">
            <li>
              <i data-part="mail" aria-hidden="true" />
              <span>Will be emailed</span>
              <strong>{count(check.mailable)}</strong>
            </li>
            {reasons.map((r) => (
              <li key={r.key}>
                <i data-part={r.key} aria-hidden="true" />
                <span>
                  {r.label} <span className="faint">{r.note}</span>
                </span>
                <strong>{count(r.count)}</strong>
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">Everyone in the audience can be emailed.</p>
        )}
      </div>
      {check.rows.length > 0 && (
        <div className="table-wrap cm-rows">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">Part of the audience</th>
                <th scope="col" className="num">
                  People
                </th>
                <th scope="col" className="num">
                  Can be emailed
                </th>
              </tr>
            </thead>
            <tbody>
              {check.rows.map((r) => (
                <tr key={`${r.kind}-${r.id}-${r.exclude}`}>
                  <td>
                    <Link to={r.kind === "segment" ? `/s/${r.id}` : `/e/EmailGroup/${r.id}`}>{r.name || "(unnamed)"}</Link> <Badge tone={r.exclude ? "amber" : "neutral"}>{r.exclude ? "Left out" : r.kind === "segment" ? "Segment" : "Email group"}</Badge>
                  </td>
                  <td className="num">{count(r.contacts)}</td>
                  <td className="num">{r.exclude ? "–" : count(r.mailable)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {check.new_contacts > 0 && (
        <p className="cm-note muted">
          {noun(check.new_contacts, "person", "people")} in your email groups {check.new_contacts === 1 ? "is" : "are"} not marketing contacts yet. Starting the campaign adds them as subscribed.
        </p>
      )}
      {check.split && (
        <p className="cm-note muted">
          A/B test: {people(check.split.test)} get the {check.split.variants} versions first, {people(check.split.rest)} get the winner later.
        </p>
      )}
      <p className="cm-note faint">
        Counted {new Date(check.checked_at).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}, the way sending counts. Anyone who unsubscribes before their email is sent is skipped, and so is anyone who has asked us to restrict how their data is used, so the final number can be a little lower.
      </p>
    </Section>
  );
}

const ICON: Record<Level, string> = { ok: "check", info: "alert", warn: "alert", block: "x" };

function Findings({ items, go }: { items: Finding[]; go: (t: Tab) => void }) {
  const sorted = [...items].sort((a, b) => ORDER.indexOf(a.level) - ORDER.indexOf(b.level));
  const stops = items.filter((i) => i.level === "block").length;
  return (
    <Section title="Before it goes out" id="cm-findings" aside={stops > 0 ? <Badge tone="red">{noun(stops, "thing")} to fix</Badge> : <Badge tone="green">Ready</Badge>}>
      <ul className="cm-findings">
        {sorted.map((f) => (
          <li key={f.id} data-level={f.level}>
            <span className="cm-finding-icon" aria-hidden="true">
              <Icon name={ICON[f.level]} size={14} />
            </span>
            <div className="cm-finding-text">
              <strong>{f.title}</strong>
              {f.detail && <p className="muted">{f.detail}</p>}
            </div>
            {f.tab && (
              <Button size="sm" onClick={() => go(f.tab!)}>
                {f.level === "ok" || f.level === "info" ? "Open" : "Fix"}
              </Button>
            )}
          </li>
        ))}
      </ul>
    </Section>
  );
}

/** Which links in the content are counted: a short list, because "5 of 7" is worth knowing. */
function Links({ check, doc }: { check: AudienceCheck; doc: Campaign }) {
  const found = linksIn(doc.content ?? "", check.emails[0]?.links ?? [], !!doc.track_clicks);
  if (!found.length) return null;
  const tracked = found.filter((l) => l.tracked).length;
  return (
    <details className="cm-links">
      <summary>
        {tracked} of {noun(found.length, "link")} counted when clicked
      </summary>
      <ul>
        {found.map((l) => (
          <li key={l.url}>
            <span className="cm-link-url">{l.url}</span> <span className="faint">{l.tracked ? "counted" : `not counted: ${l.why}`}</span>
          </li>
        ))}
      </ul>
    </details>
  );
}

export function ReviewTab({
  draft,
  editable,
  canSubmit,
  canCancel,
  go,
  onTest,
  onCancel,
}: {
  draft: Draft;
  editable: boolean;
  canSubmit: boolean;
  canCancel: boolean;
  go: (t: Tab) => void;
  onTest: () => void;
  onCancel: () => void;
}) {
  const doc = draft.doc!;
  const qc = useQueryClient();
  const q = useAudienceCheck(doc.id, doc.version, doc.status === "Draft" || doc.status === "Scheduled");
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [later, setLater] = useState(!!doc.schedule_at);
  const check = q.data;
  const items = check ? findings(check, doc) : [];

  if (doc.status === "Sending" || doc.status === "Sent" || doc.status === "Cancelled") {
    return (
      <Alert tone="info" action={<Button onClick={() => go("results")}>See the results</Button>}>
        {doc.status === "Cancelled" ? "This campaign was cancelled." : "This campaign has been sent."} Who it reached is on the results tab.
      </Alert>
    );
  }

  const future = later && doc.schedule_at && new Date(doc.schedule_at) > new Date() ? doc.schedule_at : null;

  const send = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const saved = await draft.save();
      if (!saved) throw new Error("The campaign could not be saved yet. Fix what is marked first.");
      // The numbers a person confirms are the ones of this moment.
      const fresh = await checkAudience(saved.id);
      qc.setQueryData(checkKey(saved.id, saved.version), fresh);
      const found = findings(fresh, saved);
      if (!canSend(found)) {
        setProblem("Something has changed since you looked, and it stops the send. The list above shows what.");
        return;
      }
      const moved = changedSince(check?.mailable ?? fresh.mailable, fresh.mailable);
      const at = saved.schedule_at && new Date(saved.schedule_at) > new Date() ? saved.schedule_at : null;
      const ok = await confirmDialog({
        title: at ? `Schedule for ${people(fresh.mailable)}?` : `Send to ${people(fresh.mailable)} now?`,
        message: (
          <div className="stack" style={{ gap: 8 }}>
            {moved && <p>{moved}</p>}
            <p>
              <strong>{saved.subject}</strong>
              <br />
              <span className="muted">
                {at ? `Goes out from ${moment(at)}.` : "Starts within a minute."} It leaves at up to {noun(fresh.rate.per_hour, "email")} an hour.
              </span>
            </p>
            <p className="muted">
              Once it is scheduled the content and the audience are fixed. You can stop it while it is sending, but emails that have gone out cannot be taken back.
            </p>
          </div>
        ),
        confirmLabel: at ? "Schedule it" : "Send it",
      });
      if (!ok) return;
      const done = await api.post<Campaign>(docPath("EmailCampaign", saved.id, "submit"), undefined, { ifMatch: saved.version });
      qc.setQueryData<Campaign>(keys.campaign(saved.id), whole({ ...done, _titles: { ...saved._titles, ...done._titles } }));
      void qc.invalidateQueries({ queryKey: keys.campaign(saved.id) });
      void qc.invalidateQueries({ queryKey: keys.list });
      toast(at ? "Scheduled" : "On its way: it starts within a minute");
    } catch (e) {
      setProblem(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const scheduled = doc.status === "Scheduled";
  return (
    <div className="cm-stack">
      {scheduled && (
        <Alert
          tone="success"
          action={
            canCancel ? (
              <Button variant="danger" onClick={onCancel}>
                Cancel the send
              </Button>
            ) : undefined
          }
        >
          <strong>{doc.schedule_at && new Date(doc.schedule_at) > new Date() ? `Scheduled for ${moment(doc.schedule_at)}.` : "Scheduled: it starts within a minute."}</strong> The content and the audience are fixed now. Who is mailed is decided when it starts, so anyone who unsubscribes before then is left out.
        </Alert>
      )}
      {q.error && !check ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} title="The audience could not be counted" />
      ) : !check ? (
        <PageLoading label="Counting who will get it" />
      ) : (
        <>
          <Reach check={check} fetching={q.isFetching} onAgain={() => void q.refetch()} />
          <Findings items={items} go={go} />
          <Links check={check} doc={doc} />
          {!scheduled && (
            <Section title="Send" id="cm-send">
              {!editable ? (
                <p className="muted">You cannot change this campaign.</p>
              ) : !canSubmit ? (
                <div className="stack" style={{ gap: 10 }}>
                  <p>
                    <strong>A Marketing Manager sends campaigns.</strong> Your draft is ready to hand over: send them the link, and test it yourself first.
                  </p>
                  <div className="row">
                    <Button icon="copy" onClick={() => void navigator.clipboard?.writeText(window.location.href).then(() => toast("Link copied"), () => toast("Copy the address from the bar", "info"))}>
                      Copy the link
                    </Button>
                    <Button icon="mail" onClick={onTest}>
                      Send a test
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="stack" style={{ gap: 14 }}>
                  <fieldset className="cm-when">
                    <legend className="field-label">When</legend>
                    <label className="check-field">
                      <input type="radio" name="cm-when" checked={!later} onChange={() => (setLater(false), draft.set("schedule_at", null))} />
                      As soon as I confirm
                    </label>
                    <label className="check-field">
                      <input type="radio" name="cm-when" checked={later} onChange={() => setLater(true)} />
                      At a set time
                    </label>
                    {later && (
                      <div className="cm-when-at">
                        <input
                          className="input"
                          type="datetime-local"
                          aria-label={`Send at (${zoneName()})`}
                          min={toLocalInput(new Date().toISOString())}
                          value={toLocalInput(doc.schedule_at)}
                          onChange={(e) => draft.set("schedule_at", fromLocalInput(e.target.value))}
                        />
                        <span className="faint">{zoneName()}</span>
                      </div>
                    )}
                    {later && !future && <p className="field-hint">Pick a time to come. A time that has passed starts it right away.</p>}
                  </fieldset>
                  {problem && <Alert tone="error">{problem}</Alert>}
                  {!canSend(items) && <p className="field-error">Fix what is marked above first.</p>}
                  <div className="row">
                    <Button variant="primary" loading={busy} disabled={!canSend(items) || (later && !doc.schedule_at)} onClick={() => void send()}>
                      {later && future ? `Schedule for ${people(check.mailable)}` : `Send to ${people(check.mailable)}`}
                    </Button>
                    <Button icon="mail" onClick={onTest}>
                      Send a test first
                    </Button>
                  </div>
                </div>
              )}
            </Section>
          )}
        </>
      )}
    </div>
  );
}
