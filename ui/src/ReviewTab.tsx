// The third tab, before anything goes out: how many people will get the email, who is left out and
// why (the server counts it the way sending counts it), what would stop the send or deserves a
// look, and then when to send it. The final count is taken again at the last moment, and a person
// is asked to confirm the number they will actually reach.

import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { api, docPath, errorMessage, t as tr, isolate } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, Icon, toast, placed, ListSkeleton } from "@ferrum/kit";
import { type Campaign, type Tab } from "./campaign";
import { checkAudience, keys, whole } from "./data";
import { clock, count, fromLocalInput, moment, toLocalInput, zoneName } from "./format";
import { type AudienceCheck, canSend, changedSince, type Finding, findings, leftOut, linksIn, type Level } from "./review";
import { checkKey, useAudienceCheck } from "./useCheck";
import type { Draft } from "./useDraft";
import { Problem, Section } from "./ui";

const ORDER: Level[] = ["block", "warn", "info", "ok"];

/** The audience as one bar: who will be emailed, and each reason someone is not. */
function Split({ check }: { check: AudienceCheck }) {
  const parts = [{ key: "mail", label: tr("Will be emailed"), n: check.mailable }, ...leftOut(check).map((l) => ({ key: l.key, label: l.label, n: l.count }))].filter((p) => p.n > 0);
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
      title={tr("Who will get it")}
      id="cm-reach"
      aside={
        <Button size="sm" icon="refresh" loading={fetching} onClick={onAgain}>
          {tr("Count again")}
        </Button>
      }
    >
      <div className="cm-reach">
        <p className="cm-reach-figure" aria-live="polite">
          <span className="cm-reach-number">{isolate(count(check.mailable))}</span> <span className="cm-reach-unit">{(check.mailable === 1 ? tr("person will get this email") : tr("people will get this email"))}</span>
        </p>
        <Split check={check} />
        {reasons.length > 0 ? (
          <ul className="cm-reasons" aria-label={tr("Left out")}>
            <li>
              <i data-part="mail" aria-hidden="true" />
              <span>{tr("Will be emailed")}</span>
              <strong>{isolate(count(check.mailable))}</strong>
            </li>
            {reasons.map((r) => (
              <li key={r.key}>
                <i data-part={r.key} aria-hidden="true" />
                <span>
                  {isolate(r.label)} <span className="faint">{isolate(r.note)}</span>
                </span>
                <strong>{isolate(count(r.count))}</strong>
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">{tr("Everyone in the audience can be emailed.")}</p>
        )}
      </div>
      {check.rows.length > 0 && (
        <div className="table-wrap cm-rows">
          <table className="table">
            <thead>
              <tr>
                <th scope="col">{tr("Part of the audience")}</th>
                <th scope="col" className="num">
                  {tr("People")}
                </th>
                <th scope="col" className="num">
                  {tr("Can be emailed")}
                </th>
              </tr>
            </thead>
            <tbody>
              {check.rows.map((r) => (
                <tr key={`${r.kind}-${r.id}-${r.exclude}`}>
                  <td>
                    <Link to={r.kind === "segment" ? `/s/${r.id}` : `/e/EmailGroup/${r.id}`}>{r.name || tr("(unnamed)")}</Link> <Badge tone={r.exclude ? "amber" : "neutral"}>{r.exclude ? tr("Left out") : r.kind === "segment" ? tr("Segment") : tr("Email group")}</Badge>
                  </td>
                  <td className="num">{isolate(count(r.contacts))}</td>
                  <td className="num">{r.exclude ? "–" : count(r.mailable)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {check.new_contacts > 0 && (
        <p className="cm-note muted">
          {placed(tr("{p1} in your email groups {p2} not marketing contacts yet. Starting the campaign adds them as subscribed."), { p1: isolate((check.new_contacts === 1 ? tr("1 person") : tr("{n} people", { n: count(check.new_contacts) }))), p2: check.new_contacts === 1 ? tr("is") : tr("are") })}
        </p>
      )}
      {check.split && (
        <p className="cm-note muted">
          {placed(tr("A/B test: {p1} get the {variants} versions first, {p2} get the winner later."), { p1: isolate((check.split.test === 1 ? tr("1 person") : tr("{n} people", { n: count(check.split.test) }))), variants: check.split.variants, p2: isolate((check.split.rest === 1 ? tr("1 person") : tr("{n} people", { n: count(check.split.rest) }))) })}
        </p>
      )}
      <p className="cm-note faint">
        {placed(tr("Counted {p1}, the way sending counts. The segments are worked out again when the campaign starts: people who join them by then are added, and anyone who has unsubscribed or asked us to restrict how their data is used is skipped. So the final number can differ a little."), { p1: isolate(clock(check.checked_at)) })}
      </p>
    </Section>
  );
}

const ICON: Record<Level, string> = { ok: "check", info: "alert", warn: "alert", block: "x" };

function Findings({ items, go }: { items: Finding[]; go: (t: Tab) => void }) {
  const sorted = [...items].sort((a, b) => ORDER.indexOf(a.level) - ORDER.indexOf(b.level));
  const stops = items.filter((i) => i.level === "block").length;
  return (
    <Section title={tr("Before it goes out")} id="cm-findings" aside={stops > 0 ? <Badge tone="red">{isolate((stops === 1 ? tr("1 thing") : tr("{n} things", { n: count(stops) })))} {tr("to fix")}</Badge> : <Badge tone="green">{tr("Ready")}</Badge>}>
      <ul className="cm-findings">
        {sorted.map((f) => (
          <li key={f.id} data-level={f.level}>
            <span className="cm-finding-icon" aria-hidden="true">
              <Icon name={ICON[f.level]} size={14} />
            </span>
            <div className="cm-finding-text">
              <strong>{isolate(f.title)}</strong>
              {f.detail && <p className="muted">{isolate(f.detail)}</p>}
            </div>
            {f.tab && (
              <Button size="sm" onClick={() => go(f.tab!)}>
                {f.level === "ok" || f.level === "info" ? tr("Open") : tr("Fix")}
              </Button>
            )}
            {f.href && (
              <a className="btn btn-default btn-sm" href={f.href}>
                {tr("Fix")}
              </a>
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
        {tr("{tracked} of {value}", { tracked, value: (found.length === 1 ? tr("1 link") : tr("{n} links", { n: count(found.length) })) })} {tr("counted when clicked")}
      </summary>
      <ul>
        {found.map((l) => (
          <li key={l.url}>
            <span className="cm-link-url">{isolate(l.url)}</span> <span className="faint">{l.tracked ? tr("counted") : tr("not counted: {why}", { why: String(l.why) })}</span>
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
      <Alert tone="info" action={<Button onClick={() => go("results")}>{tr("See the results")}</Button>}>
        {(doc.status === "Cancelled" ? tr("This campaign was cancelled. Who it reached is on the results tab.") : tr("This campaign has been sent. Who it reached is on the results tab."))}
      </Alert>
    );
  }

  const future = later && doc.schedule_at && new Date(doc.schedule_at) > new Date() ? doc.schedule_at : null;

  const send = async () => {
    setBusy(true);
    setProblem(null);
    try {
      const saved = await draft.save();
      if (!saved) throw new Error(tr("The campaign could not be saved yet. Fix what is marked first."));
      // The numbers a person confirms are the ones of this moment.
      const fresh = await checkAudience(saved.id);
      qc.setQueryData(checkKey(saved.id, saved.version), fresh);
      const found = findings(fresh, saved);
      if (!canSend(found)) {
        setProblem(tr("Something has changed since you looked, and it stops the send. The list above shows what."));
        return;
      }
      const moved = changedSince(check?.mailable ?? fresh.mailable, fresh.mailable);
      const at = saved.schedule_at && new Date(saved.schedule_at) > new Date() ? saved.schedule_at : null;
      const ok = await confirmDialog({
        title: at ? tr("Schedule for {value}?", { value: (fresh.mailable === 1 ? tr("1 person") : tr("{n} people", { n: count(fresh.mailable) })) }) : tr("Send to {value} now?", { value: (fresh.mailable === 1 ? tr("1 person") : tr("{n} people", { n: count(fresh.mailable) })) }),
        message: (
          <div className="stack" style={{ gap: 8 }}>
            {moved && <p>{isolate(moved)}</p>}
            <p>
              <strong>{isolate(saved.subject)}</strong>
              <br />
              <span className="muted">
                {((emails: string) => (at ? tr("Goes out from {at}. It leaves at up to {emails} an hour.", { at: moment(at), emails }) : tr("Starts within a minute. It leaves at up to {emails} an hour.", { emails })))(fresh.rate.per_hour === 1 ? tr("1 email") : tr("{n} emails", { n: count(fresh.rate.per_hour) }))}
              </span>
            </p>
            <p className="muted">
              {tr("Once it is scheduled the content and the audience are fixed. You can stop it while it is sending, but emails that have gone out cannot be taken back.")}
            </p>
          </div>
        ),
        confirmLabel: at ? tr("Schedule it") : tr("Send it"),
      });
      if (!ok) return;
      const done = await api.post<Campaign>(docPath("EmailCampaign", saved.id, "submit"), undefined, { ifMatch: saved.version });
      qc.setQueryData<Campaign>(keys.campaign(saved.id), whole({ ...done, _titles: { ...saved._titles, ...done._titles } }));
      void qc.invalidateQueries({ queryKey: keys.campaign(saved.id) });
      void qc.invalidateQueries({ queryKey: keys.list });
      toast(at ? tr("Scheduled") : tr("On its way: it starts within a minute"));
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
                {tr("Cancel the send")}
              </Button>
            ) : undefined
          }
        >
          <strong>{doc.schedule_at && new Date(doc.schedule_at) > new Date() ? tr("Scheduled for {value}.", { value: moment(doc.schedule_at) }) : tr("Scheduled: it starts within a minute.")}</strong> {tr("The content and the audience are fixed now. Who is mailed is decided when it starts: people who join the segments by then are added, and anyone who unsubscribes is left out.")}
        </Alert>
      )}
      {q.error && !check ? (
        <Problem error={q.error} onRetry={() => void q.refetch()} title={tr("The audience could not be counted")} />
      ) : !check ? (
        <ListSkeleton rows={2} />
      ) : (
        <>
          <Reach check={check} fetching={q.isFetching} onAgain={() => void q.refetch()} />
          <Findings items={items} go={go} />
          <Links check={check} doc={doc} />
          {!scheduled && (
            <Section title={tr("Send")} id="cm-send">
              {!editable ? (
                <p className="muted">{tr("You cannot change this campaign.")}</p>
              ) : !canSubmit ? (
                <div className="stack" style={{ gap: 10 }}>
                  <p>
                    <strong>{tr("A Marketing Manager sends campaigns.")}</strong> {tr("Your draft is ready to hand over: send them the link, and test it yourself first.")}
                  </p>
                  <div className="row">
                    <Button icon="copy" onClick={() => void navigator.clipboard?.writeText(window.location.href).then(() => toast(tr("Link copied")), () => toast(tr("Copy the address from the bar"), "info"))}>
                      {tr("Copy the link")}
                    </Button>
                    <Button icon="mail" onClick={onTest}>
                      {tr("Send a test")}
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="stack" style={{ gap: 14 }}>
                  <fieldset className="cm-when">
                    <legend className="field-label">{tr("When")}</legend>
                    <label className="check-field">
                      <input type="radio" name="cm-when" checked={!later} onChange={() => (setLater(false), draft.set("schedule_at", null))} />
                      {tr("As soon as I confirm")}
                    </label>
                    <label className="check-field">
                      <input type="radio" name="cm-when" checked={later} onChange={() => setLater(true)} />
                      {tr("At a set time")}
                    </label>
                    {later && (
                      <div className="cm-when-at">
                        <input
                          className="input"
                          type="datetime-local"
                          aria-label={tr("Send at ({value})", { value: zoneName() })}
                          min={toLocalInput(new Date().toISOString())}
                          value={toLocalInput(doc.schedule_at)}
                          onChange={(e) => draft.set("schedule_at", fromLocalInput(e.target.value))}
                        />
                        <span className="faint">{isolate(zoneName())}</span>
                      </div>
                    )}
                    {later && !future && <p className="field-hint">{tr("Pick a time to come. A time that has passed starts it right away.")}</p>}
                  </fieldset>
                  {problem && <Alert tone="error">{isolate(problem)}</Alert>}
                  {!canSend(items) && <p className="field-error">{tr("Fix what is marked above first.")}</p>}
                  <div className="row">
                    <Button variant="primary" loading={busy} disabled={!canSend(items) || (later && !doc.schedule_at)} onClick={() => void send()}>
                      {later && future ? tr("Schedule for {value}", { value: (check.mailable === 1 ? tr("1 person") : tr("{n} people", { n: count(check.mailable) })) }) : tr("Send to {value}", { value: (check.mailable === 1 ? tr("1 person") : tr("{n} people", { n: count(check.mailable) })) })}
                    </Button>
                    <Button icon="mail" onClick={onTest}>
                      {tr("Send a test first")}
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
