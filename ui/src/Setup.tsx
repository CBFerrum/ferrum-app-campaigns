// The first tab of a campaign: what it is called, who it comes from, its subject and the line an
// inbox shows, who it goes to (with the people each part adds, as the server counts them), whether
// opens and clicks are counted, how fast it goes out, and an A/B test of its subject or content.

import { type ReactNode, useRef, useState } from "react";
import { Alert, Button, Field, Icon, Menu, MenuItem, placed } from "@ferrum/kit";
import { LinkInput } from "@ferrum/records";
import type { AudienceRow, Campaign, VariantRow } from "./campaign";
import { count } from "./format";
import { DeviceSwitch, InboxLine, type Device, usePreview, useSender } from "./PreviewPane";
import { preheaderOf } from "./preview";
import type { AudienceCheck, CheckRow } from "./review";
import { MERGE_TAGS } from "./snippets";
import type { Draft, RowError } from "./useDraft";
import { insertInto, Section } from "./ui";
import { t as tr, isolate } from "@ferrum/client";

const LETTERS = ["A", "B", "C", "D", "E"];

/** Rows of an audience: each a segment or an email group, to send to or to leave out. */
export function AudienceRows({
  rows,
  onChange,
  disabled,
  error,
  counts,
  rowErrors = [],
}: {
  rows: AudienceRow[];
  onChange: (rows: AudienceRow[]) => void;
  disabled?: boolean;
  error?: string;
  /** What the server counted for each complete row, in order. */
  counts?: CheckRow[];
  rowErrors?: RowError[];
}) {
  const kindOf = (r: AudienceRow) => (r.email_group ? "group" : r.segment ? "segment" : (r.kind ?? "segment"));
  const update = (i: number, patch: Partial<AudienceRow>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  let complete = 0;
  return (
    <div className="cm-audience">
      {rows.length > 0 && (
        <ul className="cm-arows">
          {rows.map((r, i) => {
            const kind = kindOf(r);
            const done = !!(r.segment || r.email_group);
            const counted = done ? counts?.[complete++] : undefined;
            const problems = rowErrors.filter((e) => e.row[0] === "audience" && e.row[1] === i);
            return (
              <li key={r.id ?? `new-${i}`} className="cm-arow">
                <select
                  className="select cm-arow-kind"
                  aria-label={tr("Audience {value}: what to add", { value: i + 1 })}
                  value={kind}
                  disabled={disabled}
                  onChange={(e) => update(i, { kind: e.target.value as "segment" | "group", segment: null, email_group: null, _titles: undefined })}
                >
                  <option value="segment">{tr("Segment")}</option>
                  <option value="group">{tr("Email group")}</option>
                </select>
                <div className="cm-arow-pick">
                  <LinkInput
                    key={kind}
                    to={kind === "group" ? "EmailGroup" : "MarketingSegment"}
                    label={kind === "group" ? tr("Email group") : tr("Segment")}
                    placeholder={kind === "group" ? tr("Choose an email group…") : tr("Choose a segment…")}
                    value={(kind === "group" ? r.email_group : r.segment) ?? null}
                    title={r._titles?.[kind === "group" ? "email_group" : "segment"]}
                    disabled={disabled}
                    onChange={(id, title) => update(i, kind === "group" ? { email_group: id, segment: null, kind, _titles: title ? { email_group: title } : undefined } : { segment: id, email_group: null, kind, _titles: title ? { segment: title } : undefined })}
                  />
                </div>
                <label className="check-field cm-arow-leave">
                  <input type="checkbox" checked={!!r.exclude} disabled={disabled} onChange={(e) => update(i, { exclude: e.target.checked })} />
                  {tr("Leave out")}
                </label>
                {counted && (
                  <span className="cm-arow-count faint" title={r.exclude ? tr("People in it, who are left out") : tr("People in it, and how many of them can be emailed")}>
                    {r.exclude ? (counted.contacts === 1 ? tr("1 person") : tr("{n} people", { n: count(counted.contacts) })) : tr("{value} of {value2}", { value: count(counted.mailable), value2: count(counted.contacts) })}
                  </span>
                )}
                {!disabled && (
                  <button type="button" className="icon-btn" aria-label={tr("Remove audience {value}", { value: i + 1 })} onClick={() => onChange(rows.filter((_, j) => j !== i))}>
                    <Icon name="trash" size={15} />
                  </button>
                )}
                {problems.map((p) => (
                  <p key={p.message} className="field-error cm-arow-error">
                    {isolate(p.message)}
                  </p>
                ))}
              </li>
            );
          })}
        </ul>
      )}
      {!disabled && (
        <div className="row">
          <Button size="sm" icon="plus" onClick={() => onChange([...rows, { kind: "segment", segment: null, email_group: null, exclude: false }])}>
            {tr("Add a segment")}
          </Button>
          <Button size="sm" icon="plus" onClick={() => onChange([...rows, { kind: "group", segment: null, email_group: null, exclude: false }])}>
            {tr("Add an email group")}
          </Button>
        </div>
      )}
      {error && <p className="field-error">{isolate(error)}</p>}
    </div>
  );
}

/** A text box with a menu that puts what changes with each person into it. */
function WithMerge({ label, children, onInsert }: { label: string; children: ReactNode; onInsert: (code: string) => void }) {
  return (
    <div className="cm-merge">
      {children}
      <Menu ariaLabel={tr("Insert a person's detail into the {label}", { label })} label={tr("Insert")} icon="plus" buttonClass="btn btn-default btn-md">
        {(close) =>
          MERGE_TAGS.slice(0, 3).map((t) => (
            <MenuItem
              key={t.id}
              onSelect={() => {
                close();
                onInsert(t.code);
              }}
            >
              {isolate(t.label)}
            </MenuItem>
          ))
        }
      </Menu>
    </div>
  );
}

export function Setup({ draft, editable, check }: { draft: Draft; editable: boolean; check?: AudienceCheck }) {
  const doc = draft.doc!;
  const { set, fieldErrors: errors } = draft;
  const subjectRef = useRef<HTMLInputElement>(null);
  const preheaderRef = useRef<HTMLInputElement>(null);
  const [device, setDevice] = useState<Device>("desktop");
  const sender = useSender(doc.email_account);
  const preview = usePreview(doc.id, doc.version, { variant: doc.ab_test ? (doc.variants[0]?.variant ?? null) : null, contact: null });
  const subject = doc.subject ?? "";
  const rowErrors = draft.rowErrors;

  const setVariants = (variants: VariantRow[]) => set("variants", variants);
  const turnTest = (on: boolean) => {
    set("ab_test", on);
    if (on && doc.variants.length < 2) setVariants([{ subject: doc.subject }, { subject: "" }]);
    if (!on && doc.variants.length) setVariants([]);
  };

  return (
    <div className="cm-stack">
      <Section title={tr("The email")} id="cm-basics">
        <div className="form-grid" style={{ ["--cols" as string]: 2 }}>
          <Field label={tr("Name")} htmlFor="cm-name" required error={errors.campaign_name} hint={tr("For you and your team. Recipients never see it.")}>
            <input id="cm-name" className="input" value={doc.campaign_name ?? ""} maxLength={140} disabled={!editable} onChange={(e) => set("campaign_name", e.target.value)} />
          </Field>
          <Field label={tr("From")} htmlFor="cm-from" error={errors.email_account} hint={tr("The account that sends it. Blank: the workspace's default account.")}>
            <LinkInput to="EmailAccount" id="cm-from" value={doc.email_account ?? null} title={doc._titles?.email_account} disabled={!editable} placeholder={tr("Default sending account")} onChange={(id) => set("email_account", id)} />
          </Field>
          <Field
            label={tr("Subject")}
            htmlFor="cm-subject"
            required
            error={errors.subject}
            className="span-all"
            hint={
              <>
                {tr("{length} characters. Inboxes show about 70 on a computer and fewer on a phone.", { length: subject.trim().length })} {isolate("{{ contact.first_name }}")} {tr("and the other details of each person work here too.")}
              </>
            }
          >
            <WithMerge label="subject" onInsert={(code) => insertInto(subjectRef.current, subject, code, (v) => set("subject", v))}>
              <input id="cm-subject" ref={subjectRef} className="input" value={subject} maxLength={998} disabled={!editable} onChange={(e) => set("subject", e.target.value)} />
            </WithMerge>
          </Field>
          <Field label={tr("Preview text")} htmlFor="cm-preheader" error={errors.preheader} className="span-all" hint={tr("The first words after the subject in most inboxes. Optional.")}>
            <WithMerge label={tr("preview text")} onInsert={(code) => insertInto(preheaderRef.current, doc.preheader ?? "", code, (v) => set("preheader", v))}>
              <input id="cm-preheader" ref={preheaderRef} className="input" value={doc.preheader ?? ""} maxLength={250} disabled={!editable} onChange={(e) => set("preheader", e.target.value)} />
            </WithMerge>
          </Field>
        </div>
        <div className="cm-inbox-wrap">
          <div className="row">
            <span className="field-label">{tr("In an inbox")}</span>
            <span className="spacer" />
            <DeviceSwitch value={device} onChange={setDevice} />
          </div>
          {/* What is being typed shows as typed; once it is saved, as the server builds it. */}
          <InboxLine
            sender={sender.data}
            subject={draft.dirty || !preview.data ? subject : preview.data.subject}
            preheader={draft.dirty || !preview.data ? (doc.preheader ?? "") : preheaderOf(preview.data.html)}
            device={device}
          />
        </div>
      </Section>

      <Section
        title={tr("Who gets it")}
        id="cm-audience"
        aside={check ? <strong className="cm-reach-inline">{isolate(count(check.mailable))} {tr("can be emailed")}</strong> : undefined}
      >
        <AudienceRows rows={doc.audience ?? []} disabled={!editable} onChange={(rows) => set("audience", rows)} error={errors.audience} counts={check?.rows} rowErrors={rowErrors} />
        <p className="faint cm-note">
          {tr("People who unsubscribed, bounced, complained or are on the suppression list are never emailed, whatever the audience says. The Review tab counts exactly who is left out.")}
        </p>
      </Section>

      <Section title={tr("Sending")} id="cm-sending">
        <div className="form-grid" style={{ ["--cols" as string]: 2 }}>
          <div className="stack" style={{ gap: 8 }}>
            <label className="check-field">
              <input type="checkbox" checked={!!doc.track_opens} disabled={!editable} onChange={(e) => set("track_opens", e.target.checked)} />
              {tr("Count opens")}
            </label>
            <label className="check-field">
              <input type="checkbox" checked={!!doc.track_clicks} disabled={!editable} onChange={(e) => set("track_clicks", e.target.checked)} />
              {tr("Count clicks on links")}
            </label>
            <p className="field-hint">{tr("Opens are counted when the email's picture loads; automatic opens by privacy proxies and scanners are kept apart.")}</p>
          </div>
          <Field label={tr("Emails per hour")} htmlFor="cm-rate" error={errors.hourly_limit} hint={tr("This campaign's own pace. 0: only the workspace's limit, shared with newsletters and journeys.")}>
            <input
              id="cm-rate"
              className="input num"
              type="number"
              min={0}
              max={1000000}
              inputMode="numeric"
              value={doc.hourly_limit ?? 0}
              disabled={!editable}
              onChange={(e) => set("hourly_limit", Math.max(0, Math.floor(Number(e.target.value) || 0)))}
            />
          </Field>
        </div>
      </Section>

      <Section
        title={tr("A/B test")}
        id="cm-ab"
        aside={
          <label className="check-field">
            <input type="checkbox" checked={!!doc.ab_test} disabled={!editable} onChange={(e) => turnTest(e.target.checked)} />
            {tr("Test versions")}
          </label>
        }
      >
        {!doc.ab_test ? (
          <p className="muted">{tr("Try two to five subjects (or whole emails) on a share of the audience. The one that does best goes to everyone else.")}</p>
        ) : (
          <div className="stack" style={{ gap: 14 }}>
            <ul className="cm-variants">
              {doc.variants.map((v, i) => {
                const problems = rowErrors.filter((e) => e.row[0] === "variants" && e.row[1] === i);
                return (
                  <li key={v.id ?? `v-${i}`}>
                    <span className="cm-letter" aria-hidden="true">
                      {isolate(LETTERS[i])}
                    </span>
                    <input
                      className="input"
                      aria-label={tr("Subject of version {value}", { value: LETTERS[i] })}
                      placeholder={tr("Subject of version {value}", { value: LETTERS[i] })}
                      value={v.subject ?? ""}
                      maxLength={998}
                      disabled={!editable}
                      onChange={(e) => setVariants(doc.variants.map((x, j) => (j === i ? { ...x, subject: e.target.value } : x)))}
                    />
                    {editable && doc.variants.length > 2 && (
                      <button type="button" className="icon-btn" aria-label={tr("Remove version {value}", { value: LETTERS[i] })} onClick={() => setVariants(doc.variants.filter((_, j) => j !== i))}>
                        <Icon name="trash" size={15} />
                      </button>
                    )}
                    {problems.map((p) => (
                      <p key={p.message} className="field-error">
                        {isolate(p.message)}
                      </p>
                    ))}
                  </li>
                );
              })}
            </ul>
            {errors.variants && <p className="field-error">{isolate(errors.variants)}</p>}
            {editable && doc.variants.length < 5 && (
              <div>
                <Button size="sm" icon="plus" onClick={() => setVariants([...doc.variants, { subject: doc.subject }])}>
                  {tr("Add a version")}
                </Button>
              </div>
            )}
            <div className="form-grid" style={{ ["--cols" as string]: 3 }}>
              <Field label={tr("Share that gets the test")} htmlFor="cm-share" error={errors.test_percent} hint={tr("Split evenly between the versions.")}>
                <div className="cm-unit">
                  <input id="cm-share" className="input num" type="number" min={1} max={90} value={doc.test_percent ?? 20} disabled={!editable} onChange={(e) => set("test_percent", Math.min(90, Math.max(1, Math.floor(Number(e.target.value) || 1))))} />
                  <span className="faint">%</span>
                </div>
              </Field>
              <Field label={tr("The winner has the most")} htmlFor="cm-metric" error={errors.winner_metric}>
                <select id="cm-metric" className="select" value={doc.winner_metric ?? "Opens"} disabled={!editable} onChange={(e) => set("winner_metric", e.target.value as Campaign["winner_metric"])}>
                  <option value="Opens">{tr("Opens")}</option>
                  <option value="Clicks">{tr("Clicks")}</option>
                </select>
              </Field>
              <Field label={tr("Decided after")} htmlFor="cm-hours" error={errors.winner_after_hours} hint={tr("Counted from when the last test email went out.")}>
                <div className="cm-unit">
                  <input id="cm-hours" className="input num" type="number" min={1} max={720} value={doc.winner_after_hours ?? 4} disabled={!editable} onChange={(e) => set("winner_after_hours", Math.min(720, Math.max(1, Math.floor(Number(e.target.value) || 1))))} />
                  <span className="faint">{tr("hours")}</span>
                </div>
              </Field>
            </div>
            {((doc.winner_metric ?? "Opens") === "Opens" && !doc.track_opens) || (doc.winner_metric === "Clicks" && !doc.track_clicks) ? (
              <Alert tone="warning">{placed(tr("Choosing the winner by {p1} needs that to be counted: turn it on under Sending."), { p1: isolate((doc.winner_metric ?? "Opens").toLowerCase()) })}</Alert>
            ) : null}
            <p className="faint cm-note">{tr("To give a version its own content, write it on the Content tab.")}</p>
          </div>
        )}
      </Section>
    </div>
  );
}
