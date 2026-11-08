// One segment: who is in it, built rule by rule, with the count beside it following every change
// (the server counts rules that are not saved yet, and a sample says who they are and whether
// they would be emailed). Saving is explicit, against the version on screen, and the page says
// which campaigns and journeys use the segment, because changing it changes who they reach.

import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { api, ApiError, docPath, errorMessage, shell, t as tr, isolate } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, EmptyState, ErrorState, Field, Icon, Menu, MenuItem, PageLoading, Spinner, toast, useDebounced, useLiveEntity, useShellTitle, placed } from "@ferrum/kit";
import { can, LinkInput, useEntityMeta } from "@ferrum/records";
import { countSegment, keys, list, type Segment } from "./data";
import { count } from "./format";
import {
  ACTIVITIES,
  blankRule,
  change,
  CONTACT_FIELDS,
  describe,
  OPERATOR_WORDS,
  operatorsFor,
  problem as problemOf,
  RULE_TYPES,
  type Rule,
  rowsForSave,
  sameRules,
  standing,
  valueKind,
} from "./segment";

export function SegmentRoute() {
  const { id = "" } = useParams();
  const meta = useEntityMeta("MarketingSegment");
  if (meta.error) return <ErrorState error={meta.error} onRetry={() => void meta.refetch()} />;
  if (!meta.data) return <PageLoading />;
  return <SegmentPage key={id} id={id} canWrite={can(meta.data.permissions, "write")} canDelete={can(meta.data.permissions, "delete")} />;
}

const INPUT_TYPE = { text: "text", number: "number", date: "date", days: "number", none: "text" } as const;

function RuleRow({ rule, index, disabled, error, onChange, onRemove }: { rule: Rule; index: number; disabled: boolean; error: ReturnType<typeof problemOf>; onChange: (patch: Partial<Rule>) => void; onRemove: () => void }) {
  const kind = valueKind(rule);
  const n = index + 1;
  const bad = (f: string) => (error?.field === f ? true : undefined);
  return (
    <li className="cm-rule">
      <div className="cm-rule-fields">
        <select className="select cm-rule-type" aria-label={tr("Rule {n}: what to look at", { n })} value={rule.rule_type} disabled={disabled} onChange={(e) => onChange({ rule_type: e.target.value as Rule["rule_type"] })}>
          {RULE_TYPES.map((t) => (
            <option key={t.id} value={t.id}>
              {isolate(t.label)}
            </option>
          ))}
        </select>
        {rule.rule_type === "Contact Field" && (
          <select className="select" aria-label={tr("Rule {n}: which detail", { n })} aria-invalid={bad("field")} value={rule.field ?? ""} disabled={disabled} onChange={(e) => onChange({ field: e.target.value })}>
            {CONTACT_FIELDS.map((f) => (
              <option key={f}>{isolate(f)}</option>
            ))}
          </select>
        )}
        {rule.rule_type === "Attribute" && (
          <input className="input" aria-label={tr("Rule {n}: attribute name", { n })} aria-invalid={bad("attribute")} placeholder={tr("Attribute, e.g. city")} value={rule.attribute ?? ""} maxLength={40} disabled={disabled} onChange={(e) => onChange({ attribute: e.target.value })} />
        )}
        {rule.rule_type === "Engagement" && (
          <select className="select" aria-label={tr("Rule {n}: did or did not", { n })} value={rule.operator} disabled={disabled} onChange={(e) => onChange({ operator: e.target.value })}>
            <option value="Did">{tr("Did")}</option>
            <option value="Did Not">{tr("Did not")}</option>
          </select>
        )}
        {rule.rule_type === "Engagement" ? (
          <select className="select" aria-label={tr("Rule {n}: what they did", { n })} aria-invalid={bad("activity")} value={rule.activity ?? ""} disabled={disabled} onChange={(e) => onChange({ activity: e.target.value })}>
            {ACTIVITIES.map((a) => (
              <option key={a} value={a}>
                {a === "Was Sent" ? tr("Get an email") : a === "Opened" ? tr("Open an email") : tr("Click a link")}
              </option>
            ))}
          </select>
        ) : (
          <select className="select" aria-label={tr("Rule {n}: how to compare", { n })} aria-invalid={bad("operator")} value={rule.operator} disabled={disabled} onChange={(e) => onChange({ operator: e.target.value })}>
            {operatorsFor(rule).map((o) => (
              <option key={o} value={o}>
                {OPERATOR_WORDS[o] ?? o}
              </option>
            ))}
          </select>
        )}
        {rule.rule_type === "Email Group" && (
          <div className="cm-rule-pick">
            <LinkInput to="EmailGroup" label={tr("Rule {n}: email group", { n })} placeholder={tr("Choose an email group…")} value={rule.email_group ?? null} title={rule._titles?.email_group} disabled={disabled} invalid={!!bad("email_group")} onChange={(id, title) => onChange({ email_group: id, _titles: title ? { email_group: title } : undefined })} />
          </div>
        )}
        {rule.rule_type === "Engagement" && (
          <>
            <div className="cm-rule-pick">
              <LinkInput to="EmailCampaign" label={tr("Rule {n}: which campaign", { n })} placeholder={tr("Any campaign or journey")} value={rule.campaign ?? null} title={rule._titles?.campaign} disabled={disabled} onChange={(id, title) => onChange({ campaign: id, _titles: title ? { ...rule._titles, campaign: title } : undefined })} />
            </div>
            <span className="cm-rule-in">{tr("in the last")}</span>
            <input className="input num cm-rule-days" type="number" min={1} max={3650} aria-label={tr("Rule {n}: number of days", { n })} aria-invalid={bad("within_days")} value={rule.within_days ?? ""} disabled={disabled} onChange={(e) => onChange({ within_days: e.target.value === "" ? null : Number(e.target.value) })} />
            <span className="cm-rule-in">{tr("days")}</span>
          </>
        )}
        {kind !== "none" && rule.rule_type !== "Email Group" && rule.rule_type !== "Engagement" && (
          <>
            <input
              className="input cm-rule-value"
              type={INPUT_TYPE[kind]}
              inputMode={kind === "number" || kind === "days" ? "decimal" : undefined}
              aria-label={tr("Rule {n}: {value}", { n, value: kind === "days" ? tr("number of days") : "value" })}
              aria-invalid={bad("value")}
              placeholder={kind === "date" ? tr("YYYY-MM-DD") : kind === "days" ? tr("Days") : tr("Value")}
              value={rule.value ?? ""}
              disabled={disabled}
              onChange={(e) => onChange({ value: e.target.value })}
            />
            {kind === "days" && <span className="cm-rule-in">{tr("days")}</span>}
          </>
        )}
      </div>
      {!disabled && (
        <button type="button" className="icon-btn" aria-label={tr("Remove rule {n}", { n })} onClick={onRemove}>
          <Icon name="trash" size={15} />
        </button>
      )}
      {error && <p className="field-error cm-rule-error">{isolate(error.message)}</p>}
    </li>
  );
}

/** Where the segment is used, counted from the records that name it. */
function UsedBy({ id }: { id: string }) {
  const q = useQuery({
    queryKey: ["campaigns", "segment-used", id],
    queryFn: async () => {
      const by = (entity: string, field: string, fields: string) => list<Record<string, string>>(entity, { filters: JSON.stringify([[field, "=", id]]), fields, limit: 50 }).then((p) => p.rows);
      const [campaigns, triggers, goals, steps] = await Promise.all([
        by("EmailCampaign", "audience.segment", "campaign_name,status"),
        by("MarketingJourney", "trigger_segment", "journey_name,status"),
        by("MarketingJourney", "goal_segment", "journey_name,status"),
        by("MarketingJourney", "steps.segment", "journey_name,status"),
      ]);
      const journeys = new Map([...triggers, ...goals, ...steps].map((j) => [j.id, j]));
      return { campaigns, journeys: [...journeys.values()] };
    },
  });
  if (q.error) return <p className="muted">{tr("Where it is used could not be loaded.")}</p>;
  if (!q.data) return <Spinner label={tr("Looking for where it is used")} />;
  const { campaigns, journeys } = q.data;
  const live = campaigns.filter((c) => c.status === "Scheduled" || c.status === "Sending").length + journeys.filter((j) => j.status === "Active").length;
  if (!campaigns.length && !journeys.length) return <p className="muted">{tr("No campaign or journey uses it yet.")}</p>;
  return (
    <div className="stack" style={{ gap: 10 }}>
      {live > 0 && (
        <Alert tone="warning">
          {isolate((live === 1 ? tr("1 scheduled or running thing is") : tr("{n} scheduled or running things are", { n: count(live) })))} {tr("using this segment. A segment is worked out when it is used, so a change here changes who they reach.")}
        </Alert>
      )}
      <ul className="cm-used">
        {campaigns.map((c) => (
          <li key={c.id}>
            <Link to={`/c/${c.id}`}>{isolate(c.campaign_name)}</Link> <Badge>{isolate(c.status)}</Badge>
          </li>
        ))}
        {journeys.map((j) => (
          <li key={j.id}>
            <Link to={`/e/MarketingJourney/${j.id}`}>{isolate(j.journey_name)}</Link> <Badge tone="purple">{tr("Journey,")} {isolate(j.status.toLowerCase())}</Badge>
          </li>
        ))}
      </ul>
    </div>
  );
}

function SegmentPage({ id, canWrite, canDelete }: { id: string; canWrite: boolean; canDelete: boolean }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.segment(id), queryFn: ({ signal }) => api.get<Segment>(docPath("MarketingSegment", id), { titles: true }, signal).then((s) => ({ ...s, rules: s.rules ?? [] })) });
  const saved = q.data;
  useShellTitle(saved?.segment_name ?? null);
  const [name, setName] = useState<string | null>(null);
  const [description, setDescription] = useState<string | null>(null);
  const [match, setMatch] = useState<"All" | "Any" | null>(null);
  const [rules, setRules] = useState<Rule[] | null>(null);
  const [saving, setSaving] = useState(false);
  const [failure, setFailure] = useState<{ message: string; stale: boolean } | null>(null);
  const [shown, setShown] = useState(false);

  const current = {
    name: name ?? saved?.segment_name ?? "",
    description: description ?? saved?.description ?? "",
    match: match ?? saved?.match_type ?? "All",
    rules: rules ?? saved?.rules ?? [],
  };
  const dirty =
    !!saved &&
    (current.name !== saved.segment_name || current.description !== (saved.description ?? "") || current.match !== saved.match_type || (rules !== null && !sameRules(rules, saved.rules ?? [])));
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  useEffect(() => shell().leaveGuard(() => dirtyRef.current), []);
  // Someone else's change shows up unless there is something unsaved here, which they would overwrite.
  useLiveEntity(
    "MarketingSegment",
    useCallback((e) => void (e.id === id && !dirtyRef.current && qc.invalidateQueries({ queryKey: keys.segment(id) })), [id, qc]),
  );

  // The count follows the rules a moment after the last change, without saving them.
  const problems = current.rules.map(problemOf);
  const sendable = problems.every((p) => !p);
  const body = useDebounced(JSON.stringify({ match: current.match, rules: rowsForSave(current.rules) }), 500);
  const counted = useQuery({
    queryKey: ["campaigns", "segment-count", id, body],
    queryFn: () => {
      const b = JSON.parse(body) as { match: "All" | "Any"; rules: Record<string, unknown>[] };
      return countSegment(id, b.match, b.rules);
    },
    enabled: !!saved && sendable,
    placeholderData: keepPreviousData,
  });

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title={tr("This segment could not be opened")} />;
  if (!saved) return <PageLoading label={tr("Opening the segment")} />;

  const setRule = (i: number, patch: Partial<Rule>) => setRules(current.rules.map((r, j) => (j === i ? change(r, patch) : r)));
  const discard = () => {
    setName(null);
    setDescription(null);
    setMatch(null);
    setRules(null);
    setFailure(null);
  };
  const save = async () => {
    if (!sendable || !current.name.trim()) return;
    setSaving(true);
    setFailure(null);
    try {
      const next = await api.patch<Segment>(
        docPath("MarketingSegment", id),
        { segment_name: current.name.trim(), description: current.description.trim() || null, match_type: current.match, rules: rowsForSave(current.rules) },
        { ifMatch: saved.version },
      );
      qc.setQueryData(keys.segment(id), { ...next, rules: next.rules ?? [] });
      void qc.invalidateQueries({ queryKey: keys.segment(id) });
      void qc.invalidateQueries({ queryKey: keys.segments });
      discard();
      toast(tr("Saved"));
    } catch (e) {
      const stale = e instanceof ApiError && (e.status === 409 || e.status === 412);
      setFailure({ message: stale ? tr("Someone else changed this segment while you were editing it.") : errorMessage(e), stale });
    } finally {
      setSaving(false);
    }
  };
  const remove = async () => {
    if (!(await confirmDialog({ title: tr("Delete “{segment_name}”?", { segment_name: saved.segment_name }), message: tr("Campaigns and journeys that use it will not be able to send until they name another audience."), confirmLabel: tr("Delete"), danger: true }))) return;
    try {
      await api.del(docPath("MarketingSegment", id), { ifMatch: saved.version });
      void qc.invalidateQueries({ queryKey: keys.segments });
      toast(tr("Deleted"));
      navigate("/audiences");
    } catch (e) {
      toast(errorMessage(e), "error");
    }
  };

  const c = counted.data;
  const names = (r: Rule) => ({ group: r._titles?.email_group, campaign: r._titles?.campaign });
  const words = current.rules.map((r) => describe(r, names(r)));

  return (
    <div className="cm-page cm-seg-page">
      <header className="cm-camp-head">
        <div className="crumbs">
          <Link to="/audiences">{tr("Audiences")}</Link>
          <Icon name="chevronRight" size={12} />
        </div>
        <div className="cm-camp-bar">
          <div className="cm-camp-title">
            <h1 className="page-title">{isolate(saved.segment_name)}</h1>
            <p className="page-sub">{dirty ? tr("Changes not saved yet") : tr("Saved")}</p>
          </div>
          <div className="page-actions">
            <Link className="btn btn-default btn-md" to={`/new?segment=${id}`}>
              <Icon name="mail" />
              {tr("Send a campaign to it")}
            </Link>
            <Menu ariaLabel={tr("More")} icon="chevronDown" buttonClass="icon-btn">
              {(close) => (
                <>
                  <MenuItem onSelect={() => (close(), navigate(`/records/MarketingSegment/${id}`))}>{tr("Open as a record")}</MenuItem>
                  {canDelete && (
                    <MenuItem danger onSelect={() => (close(), void remove())}>
                      {tr("Delete…")}
                    </MenuItem>
                  )}
                </>
              )}
            </Menu>
          </div>
        </div>
      </header>

      {failure && (
        <Alert
          tone={failure.stale ? "warning" : "error"}
          action={
            failure.stale ? (
              <Button
                onClick={() => {
                  discard();
                  void q.refetch();
                }}
              >
                {tr("Use theirs")}
              </Button>
            ) : undefined
          }
        >
          {isolate(failure.message)}
        </Alert>
      )}

      <div className="cm-seg-grid">
        <div className="cm-stack">
          <section className="card cm-section" aria-labelledby="cm-seg-about">
            <header className="card-head">
              <h2 id="cm-seg-about">{tr("About it")}</h2>
            </header>
            <div className="card-body stack" style={{ gap: 14 }}>
              <Field label={tr("Name")} htmlFor="cm-sname" required>
                <input id="cm-sname" className="input" value={current.name} maxLength={140} disabled={!canWrite} onChange={(e) => setName(e.target.value)} />
              </Field>
              <Field label={tr("What it is for")} htmlFor="cm-sdesc">
                <textarea id="cm-sdesc" className="textarea" rows={2} value={current.description} disabled={!canWrite} onChange={(e) => setDescription(e.target.value)} />
              </Field>
            </div>
          </section>

          <section className="card cm-section" aria-labelledby="cm-seg-rules">
            <header className="card-head">
              <h2 id="cm-seg-rules">{tr("Who is in it")}</h2>
              <div className="cm-aside">
                <div className="segmented" role="group" aria-label={tr("Match")}>
                  <button type="button" aria-pressed={current.match === "All"} disabled={!canWrite} onClick={() => setMatch("All")}>
                    {tr("All of these")}
                  </button>
                  <button type="button" aria-pressed={current.match === "Any"} disabled={!canWrite} onClick={() => setMatch("Any")}>
                    {tr("Any of these")}
                  </button>
                </div>
              </div>
            </header>
            <div className="card-body">
              {current.rules.length === 0 ? (
                <p className="muted">{tr("No rules yet, so everyone is in. Add a rule to narrow it down.")}</p>
              ) : (
                <ul className="cm-rules">
                  {current.rules.map((r, i) => (
                    <RuleRow key={r.id ?? `new-${i}`} rule={r} index={i} disabled={!canWrite} error={shown || r._touched ? problems[i] : null} onChange={(p) => setRule(i, p)} onRemove={() => setRules(current.rules.filter((_, j) => j !== i))} />
                  ))}
                </ul>
              )}
              {canWrite && (
                <div className="row" style={{ marginTop: 12 }}>
                  <Menu label={tr("Add a rule")} icon="plus" buttonClass="btn btn-default btn-md">
                    {(close) =>
                      RULE_TYPES.map((t) => (
                        <MenuItem key={t.id} onSelect={() => (close(), setRules([...current.rules, blankRule(t.id)]))}>
                          {isolate(t.label)}
                        </MenuItem>
                      ))
                    }
                  </Menu>
                </div>
              )}
              {words.length > 0 && (
                <p className="cm-words faint">
                  {current.match === "All" ? tr("People who match all of these:") : tr("People who match any of these:")} {isolate(words.join(current.match === "All" ? "; and " : "; or "))}.
                </p>
              )}
            </div>
          </section>

          {canWrite && (
            <div className="cm-savebar" role="group" aria-label={tr("Save")}>
              <Button
                variant="primary"
                loading={saving}
                disabled={!dirty || !current.name.trim()}
                onClick={() => {
                  setShown(true);
                  if (sendable) void save();
                }}
              >
                {tr("Save changes")}
              </Button>
              <Button disabled={!dirty || saving} onClick={discard}>
                {tr("Discard changes")}
              </Button>
              {shown && !sendable && <span className="field-error">{tr("Fix the rules marked above first.")}</span>}
            </div>
          )}

          <section className="card cm-section" aria-labelledby="cm-seg-used">
            <header className="card-head">
              <h2 id="cm-seg-used">{tr("Where it is used")}</h2>
            </header>
            <div className="card-body">
              <UsedBy id={id} />
            </div>
          </section>
        </div>

        <aside className="cm-match card" aria-label={tr("Who matches")} data-stale={counted.isFetching || undefined}>
          <header className="card-head">
            <h2>{tr("Who matches")}</h2>
            {counted.isFetching && <Spinner small label={tr("Counting")} />}
          </header>
          <div className="card-body">
            {!sendable ? (
              <p className="muted">
                {placed(tr("Rule {p1} is not finished: {p2}. Finish it to see who matches."), { p1: problems.findIndex(Boolean) + 1, p2: isolate(problems.find(Boolean)?.message.replace(/^./, (c) => c.toLowerCase()) ?? "") })}
              </p>
            ) : counted.error && !c ? (
              <ErrorState error={counted.error} onRetry={() => void counted.refetch()} title={tr("They could not be counted")} />
            ) : !c ? (
              <PageLoading label={tr("Counting")} />
            ) : (
              <>
                <p className="cm-match-figure" aria-live="polite">
                  <span className="cm-reach-number">{isolate(count(c.contact_count))}</span> <span className="cm-reach-unit">{c.contact_count === 1 ? tr("person matches") : tr("people match")}</span>
                </p>
                <p className="muted">{isolate(count(c.mailable_count))} {(c.mailable_count < c.contact_count ? tr("of them can be emailed. {value} cannot: they have unsubscribed, bounced, complained, not confirmed, or are on the suppression list.", { value: ((c.contact_count - c.mailable_count) === 1 ? tr("1 person") : tr("{n} people", { n: count(c.contact_count - c.mailable_count) })) }) : tr("of them can be emailed."))}</p>
                {c.sample.length === 0 ? (
                  <EmptyState title={tr("Nobody matches")} icon="user" />
                ) : (
                  <>
                    <h3 className="cm-sample-title">{c.contact_count > c.sample.length ? tr("The first {length}, by address", { length: c.sample.length }) : tr("Everyone")}</h3>
                    <ul className="cm-sample">
                      {c.sample.map((s) => {
                        const st = standing(s.consent_status, s.mailable);
                        return (
                          <li key={s.id}>
                            <div className="cm-sample-who">
                              <Link to={`/e/MarketingContact/${s.id}`}>{isolate(s.email)}</Link>
                              {s.full_name && <span className="muted">{isolate(s.full_name)}</span>}
                            </div>
                            <Badge tone={st.tone === "green" ? "green" : st.tone === "red" ? "red" : st.tone === "amber" ? "amber" : "neutral"}>{isolate(st.label)}</Badge>
                          </li>
                        );
                      })}
                    </ul>
                  </>
                )}
              </>
            )}
            {saved.counted_at && (
              <p className="faint cm-note">{tr("Saved counts, from the last time it was counted on the segment itself:")} {tr("{value} people.", { value: count(saved.contact_count) })}</p>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}
