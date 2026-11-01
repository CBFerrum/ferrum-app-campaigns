// One segment: who is in it, built rule by rule, with the count beside it following every change
// (the server counts rules that are not saved yet, and a sample says who they are and whether
// they would be emailed). Saving is explicit, against the version on screen, and the page says
// which campaigns and journeys use the segment, because changing it changes who they reach.

import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { api, ApiError, docPath, errorMessage, shell } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, EmptyState, ErrorState, Field, Icon, Menu, MenuItem, PageLoading, Spinner, toast, useDebounced, useLiveEntity, useShellTitle } from "@ferrum/kit";
import { can, LinkInput, useEntityMeta } from "@ferrum/records";
import { countSegment, keys, list, type Segment } from "./data";
import { count, noun, people } from "./format";
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
        <select className="select cm-rule-type" aria-label={`Rule ${n}: what to look at`} value={rule.rule_type} disabled={disabled} onChange={(e) => onChange({ rule_type: e.target.value as Rule["rule_type"] })}>
          {RULE_TYPES.map((t) => (
            <option key={t.id} value={t.id}>
              {t.label}
            </option>
          ))}
        </select>
        {rule.rule_type === "Contact Field" && (
          <select className="select" aria-label={`Rule ${n}: which detail`} aria-invalid={bad("field")} value={rule.field ?? ""} disabled={disabled} onChange={(e) => onChange({ field: e.target.value })}>
            {CONTACT_FIELDS.map((f) => (
              <option key={f}>{f}</option>
            ))}
          </select>
        )}
        {rule.rule_type === "Attribute" && (
          <input className="input" aria-label={`Rule ${n}: attribute name`} aria-invalid={bad("attribute")} placeholder="Attribute, e.g. city" value={rule.attribute ?? ""} maxLength={40} disabled={disabled} onChange={(e) => onChange({ attribute: e.target.value })} />
        )}
        {rule.rule_type === "Engagement" && (
          <select className="select" aria-label={`Rule ${n}: did or did not`} value={rule.operator} disabled={disabled} onChange={(e) => onChange({ operator: e.target.value })}>
            <option value="Did">Did</option>
            <option value="Did Not">Did not</option>
          </select>
        )}
        {rule.rule_type === "Engagement" ? (
          <select className="select" aria-label={`Rule ${n}: what they did`} aria-invalid={bad("activity")} value={rule.activity ?? ""} disabled={disabled} onChange={(e) => onChange({ activity: e.target.value })}>
            {ACTIVITIES.map((a) => (
              <option key={a} value={a}>
                {a === "Was Sent" ? "Get an email" : a === "Opened" ? "Open an email" : "Click a link"}
              </option>
            ))}
          </select>
        ) : (
          <select className="select" aria-label={`Rule ${n}: how to compare`} aria-invalid={bad("operator")} value={rule.operator} disabled={disabled} onChange={(e) => onChange({ operator: e.target.value })}>
            {operatorsFor(rule).map((o) => (
              <option key={o} value={o}>
                {OPERATOR_WORDS[o] ?? o}
              </option>
            ))}
          </select>
        )}
        {rule.rule_type === "Email Group" && (
          <div className="cm-rule-pick">
            <LinkInput to="EmailGroup" label={`Rule ${n}: email group`} placeholder="Choose an email group…" value={rule.email_group ?? null} title={rule._titles?.email_group} disabled={disabled} invalid={!!bad("email_group")} onChange={(id, title) => onChange({ email_group: id, _titles: title ? { email_group: title } : undefined })} />
          </div>
        )}
        {rule.rule_type === "Engagement" && (
          <>
            <div className="cm-rule-pick">
              <LinkInput to="EmailCampaign" label={`Rule ${n}: which campaign`} placeholder="Any campaign or journey" value={rule.campaign ?? null} title={rule._titles?.campaign} disabled={disabled} onChange={(id, title) => onChange({ campaign: id, _titles: title ? { ...rule._titles, campaign: title } : undefined })} />
            </div>
            <span className="cm-rule-in">in the last</span>
            <input className="input num cm-rule-days" type="number" min={1} max={3650} aria-label={`Rule ${n}: number of days`} aria-invalid={bad("within_days")} value={rule.within_days ?? ""} disabled={disabled} onChange={(e) => onChange({ within_days: e.target.value === "" ? null : Number(e.target.value) })} />
            <span className="cm-rule-in">days</span>
          </>
        )}
        {kind !== "none" && rule.rule_type !== "Email Group" && rule.rule_type !== "Engagement" && (
          <>
            <input
              className="input cm-rule-value"
              type={INPUT_TYPE[kind]}
              inputMode={kind === "number" || kind === "days" ? "decimal" : undefined}
              aria-label={`Rule ${n}: ${kind === "days" ? "number of days" : "value"}`}
              aria-invalid={bad("value")}
              placeholder={kind === "date" ? "YYYY-MM-DD" : kind === "days" ? "Days" : "Value"}
              value={rule.value ?? ""}
              disabled={disabled}
              onChange={(e) => onChange({ value: e.target.value })}
            />
            {kind === "days" && <span className="cm-rule-in">days</span>}
          </>
        )}
      </div>
      {!disabled && (
        <button type="button" className="icon-btn" aria-label={`Remove rule ${n}`} onClick={onRemove}>
          <Icon name="trash" size={15} />
        </button>
      )}
      {error && <p className="field-error cm-rule-error">{error.message}</p>}
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
  if (q.error) return <p className="muted">Where it is used could not be loaded.</p>;
  if (!q.data) return <Spinner label="Looking for where it is used" />;
  const { campaigns, journeys } = q.data;
  const live = campaigns.filter((c) => c.status === "Scheduled" || c.status === "Sending").length + journeys.filter((j) => j.status === "Active").length;
  if (!campaigns.length && !journeys.length) return <p className="muted">No campaign or journey uses it yet.</p>;
  return (
    <div className="stack" style={{ gap: 10 }}>
      {live > 0 && (
        <Alert tone="warning">
          {noun(live, "scheduled or running thing is", "scheduled or running things are")} using this segment. A segment is worked out when it is used, so a change here changes who they reach.
        </Alert>
      )}
      <ul className="cm-used">
        {campaigns.map((c) => (
          <li key={c.id}>
            <Link to={`/c/${c.id}`}>{c.campaign_name}</Link> <Badge>{c.status}</Badge>
          </li>
        ))}
        {journeys.map((j) => (
          <li key={j.id}>
            <Link to={`/e/MarketingJourney/${j.id}`}>{j.journey_name}</Link> <Badge tone="purple">Journey, {j.status.toLowerCase()}</Badge>
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

  if (q.error) return <ErrorState error={q.error} onRetry={() => void q.refetch()} title="This segment could not be opened" />;
  if (!saved) return <PageLoading label="Opening the segment" />;

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
      toast("Saved");
    } catch (e) {
      const stale = e instanceof ApiError && (e.status === 409 || e.status === 412);
      setFailure({ message: stale ? "Someone else changed this segment while you were editing it." : errorMessage(e), stale });
    } finally {
      setSaving(false);
    }
  };
  const remove = async () => {
    if (!(await confirmDialog({ title: `Delete “${saved.segment_name}”?`, message: "Campaigns and journeys that use it will not be able to send until they name another audience.", confirmLabel: "Delete", danger: true }))) return;
    try {
      await api.del(docPath("MarketingSegment", id), { ifMatch: saved.version });
      void qc.invalidateQueries({ queryKey: keys.segments });
      toast("Deleted");
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
          <Link to="/audiences">Audiences</Link>
          <Icon name="chevronRight" size={12} />
        </div>
        <div className="cm-camp-bar">
          <div className="cm-camp-title">
            <h1 className="page-title">{saved.segment_name}</h1>
            <p className="page-sub">{dirty ? "Changes not saved yet" : "Saved"}</p>
          </div>
          <div className="page-actions">
            <Link className="btn btn-default btn-md" to={`/new?segment=${id}`}>
              <Icon name="mail" />
              Send a campaign to it
            </Link>
            <Menu ariaLabel="More" icon="chevronDown" buttonClass="icon-btn">
              {(close) => (
                <>
                  <MenuItem onSelect={() => (close(), navigate(`/records/MarketingSegment/${id}`))}>Open as a record</MenuItem>
                  {canDelete && (
                    <MenuItem danger onSelect={() => (close(), void remove())}>
                      Delete…
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
                Use theirs
              </Button>
            ) : undefined
          }
        >
          {failure.message}
        </Alert>
      )}

      <div className="cm-seg-grid">
        <div className="cm-stack">
          <section className="card cm-section" aria-labelledby="cm-seg-about">
            <header className="card-head">
              <h2 id="cm-seg-about">About it</h2>
            </header>
            <div className="card-body stack" style={{ gap: 14 }}>
              <Field label="Name" htmlFor="cm-sname" required>
                <input id="cm-sname" className="input" value={current.name} maxLength={140} disabled={!canWrite} onChange={(e) => setName(e.target.value)} />
              </Field>
              <Field label="What it is for" htmlFor="cm-sdesc">
                <textarea id="cm-sdesc" className="textarea" rows={2} value={current.description} disabled={!canWrite} onChange={(e) => setDescription(e.target.value)} />
              </Field>
            </div>
          </section>

          <section className="card cm-section" aria-labelledby="cm-seg-rules">
            <header className="card-head">
              <h2 id="cm-seg-rules">Who is in it</h2>
              <div className="cm-aside">
                <div className="segmented" role="group" aria-label="Match">
                  <button type="button" aria-pressed={current.match === "All"} disabled={!canWrite} onClick={() => setMatch("All")}>
                    All of these
                  </button>
                  <button type="button" aria-pressed={current.match === "Any"} disabled={!canWrite} onClick={() => setMatch("Any")}>
                    Any of these
                  </button>
                </div>
              </div>
            </header>
            <div className="card-body">
              {current.rules.length === 0 ? (
                <p className="muted">No rules yet, so everyone is in. Add a rule to narrow it down.</p>
              ) : (
                <ul className="cm-rules">
                  {current.rules.map((r, i) => (
                    <RuleRow key={r.id ?? `new-${i}`} rule={r} index={i} disabled={!canWrite} error={shown || r._touched ? problems[i] : null} onChange={(p) => setRule(i, p)} onRemove={() => setRules(current.rules.filter((_, j) => j !== i))} />
                  ))}
                </ul>
              )}
              {canWrite && (
                <div className="row" style={{ marginTop: 12 }}>
                  <Menu label="Add a rule" icon="plus" buttonClass="btn btn-default btn-md">
                    {(close) =>
                      RULE_TYPES.map((t) => (
                        <MenuItem key={t.id} onSelect={() => (close(), setRules([...current.rules, blankRule(t.id)]))}>
                          {t.label}
                        </MenuItem>
                      ))
                    }
                  </Menu>
                </div>
              )}
              {words.length > 0 && (
                <p className="cm-words faint">
                  {current.match === "All" ? "People who match all of these:" : "People who match any of these:"} {words.join(current.match === "All" ? "; and " : "; or ")}.
                </p>
              )}
            </div>
          </section>

          {canWrite && (
            <div className="cm-savebar" role="group" aria-label="Save">
              <Button
                variant="primary"
                loading={saving}
                disabled={!dirty || !current.name.trim()}
                onClick={() => {
                  setShown(true);
                  if (sendable) void save();
                }}
              >
                Save changes
              </Button>
              <Button disabled={!dirty || saving} onClick={discard}>
                Discard changes
              </Button>
              {shown && !sendable && <span className="field-error">Fix the rules marked above first.</span>}
            </div>
          )}

          <section className="card cm-section" aria-labelledby="cm-seg-used">
            <header className="card-head">
              <h2 id="cm-seg-used">Where it is used</h2>
            </header>
            <div className="card-body">
              <UsedBy id={id} />
            </div>
          </section>
        </div>

        <aside className="cm-match card" aria-label="Who matches" data-stale={counted.isFetching || undefined}>
          <header className="card-head">
            <h2>Who matches</h2>
            {counted.isFetching && <Spinner small label="Counting" />}
          </header>
          <div className="card-body">
            {!sendable ? (
              <p className="muted">
                Rule {problems.findIndex(Boolean) + 1} is not finished: {problems.find(Boolean)?.message.replace(/^./, (c) => c.toLowerCase())}. Finish it to see who matches.
              </p>
            ) : counted.error && !c ? (
              <ErrorState error={counted.error} onRetry={() => void counted.refetch()} title="They could not be counted" />
            ) : !c ? (
              <PageLoading label="Counting" />
            ) : (
              <>
                <p className="cm-match-figure" aria-live="polite">
                  <span className="cm-reach-number">{count(c.contact_count)}</span> <span className="cm-reach-unit">{c.contact_count === 1 ? "person matches" : "people match"}</span>
                </p>
                <p className="muted">{count(c.mailable_count)} of them can be emailed.{c.mailable_count < c.contact_count ? ` ${people(c.contact_count - c.mailable_count)} cannot: they have unsubscribed, bounced, complained, not confirmed, or are on the suppression list.` : ""}</p>
                {c.sample.length === 0 ? (
                  <EmptyState title="Nobody matches" icon="user" />
                ) : (
                  <>
                    <h3 className="cm-sample-title">{c.contact_count > c.sample.length ? `The first ${c.sample.length}, by address` : "Everyone"}</h3>
                    <ul className="cm-sample">
                      {c.sample.map((s) => {
                        const st = standing(s.consent_status, s.mailable);
                        return (
                          <li key={s.id}>
                            <div className="cm-sample-who">
                              <Link to={`/e/MarketingContact/${s.id}`}>{s.email}</Link>
                              {s.full_name && <span className="muted">{s.full_name}</span>}
                            </div>
                            <Badge tone={st.tone === "green" ? "green" : st.tone === "red" ? "red" : st.tone === "amber" ? "amber" : "neutral"}>{st.label}</Badge>
                          </li>
                        );
                      })}
                    </ul>
                  </>
                )}
              </>
            )}
            {saved.counted_at && (
              <p className="faint cm-note">Saved counts, from the last time it was counted on the segment itself: {count(saved.contact_count)} people.</p>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}
