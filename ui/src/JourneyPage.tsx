// A journey as a flow: its steps as boxes and lines (what is saved is still the steps table), a `+` on every line to add a
// step, a panel for the selected step, and, once it runs, how many people reached each step and are on it now.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router";
import { api, ApiError, docPath, errorMessage, isolate, t as tr } from "@ferrum/client";
import { Alert, Badge, Button, confirmDialog, FormSkeleton, Menu, MenuItem, SaveStatus, toast, useShellTitle } from "@ferrum/kit";
import { can, type EntityMeta, useEntityMeta } from "@ferrum/records";
import { list } from "./data";
import {
  blankStep,
  connect,
  forSave,
  hasOtherwise,
  insertOnElse,
  insertOnNext,
  layout,
  nextOf,
  otherwiseOf,
  problems,
  removeStep,
  STEP_TYPES,
  type Step,
  stepName,
  type StepType,
  typeName,
} from "./journey";
import { Problem } from "./ui";

interface Journey {
  id: string;
  version: number;
  journey_name: string;
  status: "Draft" | "Active" | "Paused" | "Stopped";
  trigger_type?: string | null;
  trigger_attribute?: string | null;
  trigger_value?: string | null;
  goal_segment?: string | null;
  goal_rule?: number | null;
  steps: Step[];
  _titles?: Record<string, string>;
}

interface FunnelRow {
  step: number;
  reached: number;
  here: number;
  waiting: number;
  emails_sent: number;
  unique_opens: number;
  unique_clicks: number;
}
interface Funnel {
  rows: FunnelRow[];
  totals: { enrolled: number; active: number; completed: number; goal_reached: number; exited: number };
}

const keys = { journey: (id: string) => ["campaigns", "journey", id] as const, funnel: (id: string) => ["campaigns", "funnel", id] as const };
const AUTOSAVE_MS = 1200;
const LANE_W = 240;
const LEVEL_H = 120;
const BOX_W = 200;
const BOX_H = 72;

export function JourneyRoute() {
  const { id = "" } = useParams();
  const meta = useEntityMeta("MarketingJourney");
  if (meta.error) return <Problem error={meta.error} onRetry={() => void meta.refetch()} />;
  if (!meta.data) return <FormSkeleton />;
  return <JourneyPage key={id} id={id} meta={meta.data} />;
}

/** The trigger in words. */
function triggerLine(j: Journey): string {
  switch (j.trigger_type) {
    case "Attribute Changed":
      return j.trigger_attribute ? tr("When the attribute {attribute} changes", { attribute: isolate(j.trigger_attribute) }) : tr("When an attribute changes");
    case "Joined Email Group":
      return tr("When someone joins an email group");
    case "Joined Segment":
      return tr("When someone joins a segment");
    case "Date Anniversary":
      return tr("On a date attribute's anniversary");
    case "Record Event":
      return tr("When something happens to a record");
    default:
      return j.trigger_type ? isolate(j.trigger_type) : tr("When someone enters");
  }
}

function JourneyPage({ id, meta }: { id: string; meta: EntityMeta }) {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: keys.journey(id), queryFn: ({ signal }) => api.get<Journey>(docPath("MarketingJourney", id), { titles: true }, signal) });
  const doc = q.data;
  const funnel = useQuery({
    queryKey: keys.funnel(id),
    queryFn: ({ signal }) => api.get<Funnel>("/api/reports/journey_funnel", { journey: id }, signal),
    enabled: !!doc && doc.status !== "Draft",
    refetchInterval: doc?.status === "Active" ? 60_000 : false,
  });
  const [edits, setEdits] = useState<Step[] | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [state, setState] = useState<"saved" | "saving" | "draft" | "failed" | "conflict">("saved");
  const [refusal, setRefusal] = useState<{ step: number; field: string; message: string }[]>([]);
  const [said, setSaid] = useState<string | null>(null);
  const sent = useRef<Step[] | null>(null);
  useShellTitle(doc?.journey_name ?? null);

  const steps = edits ?? doc?.steps ?? [];
  const editable = doc?.status === "Draft" && can(meta.permissions, "write");
  const found = useMemo(() => problems(steps), [steps]);
  const flow = useMemo(() => layout(steps), [steps]);

  // Each change is saved a moment after the last one, against the version on screen; nothing is merged.
  useEffect(() => {
    if (!edits || !doc || !editable || found.length > 0) return;
    setState("draft");
    const timer = setTimeout(async () => {
      sent.current = edits;
      setState("saving");
      try {
        const next = await api.patch<Journey>(docPath("MarketingJourney", id), { steps: forSave(edits) }, { ifMatch: doc.version });
        qc.setQueryData(keys.journey(id), next);
        setRefusal([]);
        setSaid(null);
        setState("saved");
        setEdits((now) => (now === sent.current ? null : now));
      } catch (e) {
        if (e instanceof ApiError && (e.status === 409 || e.status === 412)) {
          setSaid(tr("Changed by someone else: reload to see their version."));
          setState("conflict");
          return;
        }
        if (e instanceof ApiError && e.fields.length) setRefusal(e.fields.filter((f) => f.row?.[0] === "steps").map((f) => ({ step: f.row![1], field: f.field, message: f.message })));
        setSaid(errorMessage(e, false));
        setState("failed");
      }
    }, AUTOSAVE_MS);
    return () => clearTimeout(timer);
  }, [edits, doc, editable, found.length, id, qc]);

  if (q.error && !doc) return <Problem error={q.error} onRetry={() => void q.refetch()} title={tr("This journey could not be opened")} />;
  if (!doc) return <FormSkeleton fields={4} />;

  const change = (next: Step[], select?: number | null) => {
    if (next === steps) return;
    setEdits(next);
    if (select !== undefined) setSelected(select);
  };
  const add = (type: StepType, i: number, side: "next" | "otherwise") => {
    const s = blankStep(type);
    const next = side === "next" ? insertOnNext(steps, i, s) : insertOnElse(steps, i, s);
    // the new step is the one that is not in the old table (by identity of the rows' order): found by position
    const at = side === "next" ? i + 1 : steps.length;
    change(next, next.length > steps.length ? at : null);
  };
  const remove = async (i: number) => {
    if (!(await confirmDialog({ title: tr("Remove this step?"), message: tr("What led to it leads to where it led."), confirmLabel: tr("Remove"), danger: true }))) return;
    const r = removeStep(steps, i);
    if (r.refused) return void toast(r.refused, "error");
    change(r.steps, null);
  };
  const act = async (action: "activate" | "pause" | "stop") => {
    if (action === "stop" && !(await confirmDialog({ title: tr("Stop this journey?"), message: tr("Contacts still in the journey leave it and it cannot be restarted."), confirmLabel: tr("Stop"), danger: true }))) return;
    try {
      await api.post(docPath("MarketingJourney", id, "actions", action), {});
      void qc.invalidateQueries({ queryKey: keys.journey(id) });
      void qc.invalidateQueries({ queryKey: keys.funnel(id) });
    } catch (e) {
      toast(errorMessage(e), "error");
      setSaid(errorMessage(e, false));
    }
  };

  const rows = new Map((funnel.data?.rows ?? []).map((r) => [r.step, r]));
  const placed = new Map(flow.nodes.map((n) => [n.step, n]));
  const width = Math.max(1, ...flow.nodes.map((n) => n.lane + 1)) * LANE_W;
  const height = (Math.max(0, ...flow.nodes.map((n) => n.level)) + 1) * LEVEL_H + 60;
  const at = (i: number) => ({ x: (placed.get(i)?.lane ?? 0) * LANE_W, y: (placed.get(i)?.level ?? 0) * LEVEL_H + 70 });
  const sel = selected != null ? steps[selected] : undefined;
  const flowOrder = flow.nodes.map((n) => n.step);

  return (
    <div className="cm-jc">
      <header className="page-header">
        <div className="cm-camp-name">
          <h1 className="page-title">{isolate(doc.journey_name)}</h1>
          <Badge tone={doc.status === "Active" ? "green" : doc.status === "Paused" ? "amber" : doc.status === "Stopped" ? "red" : "neutral"}>{isolate(doc.status)}</Badge>
          {editable && <SaveStatus state={state} at={null} />}
        </div>
        <div className="page-actions">
          <Link className="btn btn-default btn-md" to={`/records/MarketingJourney/${id}`}>
            {tr("Trigger, events and settings")}
          </Link>
          {can(meta.permissions, "write") && doc.status === "Draft" && (
            <Button variant="primary" disabled={found.length > 0 || edits != null} onClick={() => void act("activate")}>
              {tr("Activate")}
            </Button>
          )}
          {can(meta.permissions, "write") && doc.status === "Active" && <Button onClick={() => void act("pause")}>{tr("Pause")}</Button>}
          {can(meta.permissions, "write") && doc.status === "Paused" && <Button variant="primary" onClick={() => void act("activate")}>{tr("Activate")}</Button>}
          {can(meta.permissions, "write") && (doc.status === "Active" || doc.status === "Paused") && (
            <Button variant="danger" onClick={() => void act("stop")}>
              {tr("Stop")}
            </Button>
          )}
        </div>
      </header>
      {said && <Alert tone={state === "conflict" ? "warning" : "error"}>{isolate(said)}</Alert>}
      {!editable && doc.status === "Draft" && <Alert tone="info">{tr("You can look at this journey but not change it.")}</Alert>}
      {doc.status !== "Draft" && funnel.data && (
        <p className="cm-hint muted">
          {tr("{enrolled} entered, {active} on it now, {completed} finished, {goal} reached the goal.", {
            enrolled: funnel.data.totals.enrolled,
            active: funnel.data.totals.active,
            completed: funnel.data.totals.completed,
            goal: funnel.data.totals.goal_reached,
          })}
        </p>
      )}
      {doc.status !== "Draft" && funnel.error && (
        <p className="field-error">
          {tr("The counts could not be read.")}{" "}
          <button type="button" className="link-btn" onClick={() => void funnel.refetch()}>
            {tr("Try again")}
          </button>
        </p>
      )}
      <div className="cm-jc-main">
        <div className="cm-jc-scroll">
          <div className="cm-jc-canvas" style={{ width, height }}>
            <div className="cm-jc-box cm-jc-trigger" style={{ insetInlineStart: 0, top: 0 }}>
              <strong>{tr("Trigger")}</strong>
              <span>{triggerLine(doc)}</span>
            </div>
            <svg className="cm-jc-lines" width={width} height={height} aria-hidden="true">
              {flow.edges.map((e, k) => {
                const from = at(e.from);
                const x1 = from.x + BOX_W / 2;
                const y1 = from.y + BOX_H;
                if (e.to == null) return <path key={k} d={`M${x1} ${y1} v24`} className="cm-jc-line" data-kind={e.kind} />;
                const to = at(e.to);
                const x2 = to.x + BOX_W / 2;
                const y2 = to.y;
                return e.back ? (
                  <path key={k} d={`M${x1} ${y1} C${x1 + 90} ${y1 + 40}, ${x2 + 110} ${y2 - 40}, ${x2} ${y2}`} className="cm-jc-line" data-kind={e.kind} data-back />
                ) : (
                  <path key={k} d={`M${x1} ${y1} C${x1} ${y1 + 36}, ${x2} ${y2 - 36}, ${x2} ${y2}`} className="cm-jc-line" data-kind={e.kind} />
                );
              })}
            </svg>
            {flowOrder.map((i) => {
              const s = steps[i];
              const p = at(i);
              const r = rows.get(i + 1);
              const flagged = found.some((f) => f.step === i) || refusal.some((f) => f.step === i);
              return (
                <button
                  key={i}
                  type="button"
                  className="cm-jc-box cm-jc-step"
                  style={{ insetInlineStart: p.x, top: p.y }}
                  aria-pressed={selected === i}
                  data-flagged={flagged || undefined}
                  onClick={() => setSelected(i)}
                >
                  <span className="cm-jc-kind">{typeName(s.step_type)}</span>
                  <span className="cm-jc-name">{isolate(stepName(s))}</span>
                  {doc.status !== "Draft" && r && (
                    <span className="cm-jc-counts">
                      {tr("{n} reached", { n: r.reached })} · {tr("{n} here now", { n: r.here })}
                      {s.step_type === "Send Email" && ` · ${tr("{n} opened", { n: r.unique_opens })} · ${tr("{n} clicked", { n: r.unique_clicks })}`}
                    </span>
                  )}
                </button>
              );
            })}
            {editable &&
              flow.edges.map((e, k) => {
                const s = steps[e.from];
                if (s.step_type === "Exit" && e.kind === "next") return null;
                const from = at(e.from);
                const toY = e.to == null ? from.y + BOX_H + 24 : at(e.to).y;
                const toX = e.to == null ? from.x + BOX_W / 2 : at(e.to).x + BOX_W / 2;
                const x = (from.x + BOX_W / 2 + toX) / 2;
                const y = (from.y + BOX_H + toY) / 2 - 14;
                const label = e.kind === "next" ? tr("Add a step after {step}", { step: stepName(s) }) : tr("Add a step on the other path of {step}", { step: stepName(s) });
                return (
                  <span key={k} className="cm-jc-plus" style={{ insetInlineStart: x - 14, top: y }}>
                    <Menu ariaLabel={label} icon="plus" buttonClass="icon-btn">
                      {(close) => STEP_TYPES.map((type) => <MenuItem key={type} onSelect={() => (close(), add(type, e.from, e.kind))}>{typeName(type)}</MenuItem>)}
                    </Menu>
                  </span>
                );
              })}
          </div>
          {flow.unreached.length > 0 && (
            <div className="cm-jc-unreached">
              <strong>{tr("Not reached")}</strong>
              <ul>
                {flow.unreached.map((i) => (
                  <li key={i}>
                    <button type="button" className="link-btn" onClick={() => setSelected(i)}>
                      {isolate(stepName(steps[i]))}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {found.filter((f) => f.step == null).map((f) => (
            <p key={f.message} className="field-error">{isolate(f.message)}</p>
          ))}
        </div>
        {sel && selected != null && (
          <aside className="cm-jc-panel card" aria-label={tr("The selected step")}>
            <h2 className="cm-bd-title">{typeName(sel.step_type)}</h2>
            <StepPanel steps={steps} index={selected} editable={editable} errors={[...found.filter((f) => f.step === selected), ...refusal.filter((f) => f.step === selected)]} onSteps={(s) => change(s)} />
            {editable && (
              <Button variant="danger" onClick={() => void remove(selected)}>
                {tr("Remove this step")}
              </Button>
            )}
          </aside>
        )}
      </div>
    </div>
  );
}

function Field({ label, error, children }: { label: string; error?: string; children: ReactNode }) {
  return (
    <label className="field cm-bd-field">
      <span className="field-label">{label}</span>
      {children}
      {error && <span className="field-error">{isolate(error)}</span>}
    </label>
  );
}

const TITLE: Record<string, string> = { MarketingTopic: "topic_name", MarketingSegment: "segment_name", EmailGroup: "group_name" };

function LinkSelect({ entity, value, onChange, disabled }: { entity: string; value: string | null | undefined; onChange: (v: string | null) => void; disabled: boolean }) {
  const q = useQuery({ queryKey: ["campaigns", "choices", entity], queryFn: ({ signal }) => list<Record<string, string>>(entity, { fields: TITLE[entity], limit: 200, sort: `${TITLE[entity]} asc` }, signal) });
  return (
    <select className="select" value={value ?? ""} disabled={disabled} onChange={(e) => onChange(e.target.value || null)}>
      <option value="">{q.isLoading ? tr("Loading…") : tr("Choose")}</option>
      {(q.data?.rows ?? []).map((r) => (
        <option key={r.id} value={r.id}>{r[TITLE[entity]]}</option>
      ))}
    </select>
  );
}

/** The selected step's fields: only those its type uses, with the server's words beside the field after a refusal. */
function StepPanel({ steps, index, editable, errors, onSteps }: { steps: Step[]; index: number; editable: boolean; errors: { field: string; message: string }[]; onSteps: (s: Step[]) => void }) {
  const s = steps[index];
  const set = (patch: Partial<Step>) => onSteps(steps.map((x, i) => (i === index ? { ...x, ...patch } : x)));
  const err = (f: string) => errors.find((e) => e.field === f)?.message;
  const off = !editable;
  const text = (f: keyof Step, label: string, multi = false) => (
    <Field label={label} error={err(f)}>
      {multi ? (
        <textarea className="textarea mono" rows={6} spellCheck={false} disabled={off} value={(s[f] as string) ?? ""} onChange={(e) => set({ [f]: e.target.value } as Partial<Step>)} />
      ) : (
        <input className="input" disabled={off} value={(s[f] as string) ?? ""} onChange={(e) => set({ [f]: e.target.value } as Partial<Step>)} />
      )}
    </Field>
  );
  const goes = (side: "next" | "otherwise", label: string) => {
    const now = side === "next" ? nextOf(steps, index) : (otherwiseOf(steps, index) ?? null);
    return (
      <Field label={label}>
        <select className="select" disabled={off} value={now == null ? "" : String(now)} onChange={(e) => onSteps(connect(steps, index, side, e.target.value === "" ? null : Number(e.target.value)))}>
          <option value="">{tr("The journey ends")}</option>
          {steps.map((o, i) => (i === index ? null : <option key={i} value={i}>{isolate(`${i + 1}. ${stepName(o)}`)}</option>))}
        </select>
      </Field>
    );
  };
  return (
    <div className="stack">
      {text("label", tr("Name on the canvas"))}
      {s.step_type === "Send Email" && (
        <>
          {text("subject", tr("Subject"))}
          {text("preheader", tr("Preview text"))}
          {text("content", tr("Email (HTML)"), true)}
          <Field label={tr("Only people in this topic")}>
            <LinkSelect entity="MarketingTopic" value={s.topic} disabled={off} onChange={(v) => set({ topic: v })} />
          </Field>
        </>
      )}
      {s.step_type === "Send Message" && (
        <>
          <Field label={tr("Channel")} error={err("channel")}>
            <select className="select" disabled={off} value={s.channel ?? ""} onChange={(e) => set({ channel: (e.target.value || null) as Step["channel"] })}>
              <option value="Text Message">{tr("Text message")}</option>
              <option value="Chat App">{tr("Chat app")}</option>
            </select>
          </Field>
          {text("message_template", tr("Message template key"))}
          <Field label={tr("Only people in this topic")}>
            <LinkSelect entity="MarketingTopic" value={s.topic} disabled={off} onChange={(v) => set({ topic: v })} />
          </Field>
        </>
      )}
      {(s.step_type === "Wait" || s.step_type === "Wait For Event") && (
        <>
          <Field label={tr("How long")} error={err("wait_amount")}>
            <input className="input" type="number" min={1} disabled={off} value={s.wait_amount ?? ""} onChange={(e) => set({ wait_amount: e.target.value ? Number(e.target.value) : null })} />
          </Field>
          <Field label={tr("Unit")}>
            <select className="select" disabled={off} value={s.wait_unit ?? "Days"} onChange={(e) => set({ wait_unit: e.target.value as Step["wait_unit"] })}>
              <option value="Minutes">{tr("Minutes")}</option>
              <option value="Hours">{tr("Hours")}</option>
              <option value="Days">{tr("Days")}</option>
            </select>
          </Field>
        </>
      )}
      {(s.step_type === "Wait For Event" || (s.step_type === "Branch" && s.branch_on === "Event Happened")) && (
        <Field label={tr("Event rule number")}>
          <input className="input" type="number" min={1} max={20} disabled={off} value={s.event_rule ?? ""} onChange={(e) => set({ event_rule: e.target.value ? Number(e.target.value) : null })} />
        </Field>
      )}
      {s.step_type === "Branch" && (
        <>
          <Field label={tr("Branch on")} error={err("branch_on")}>
            <select className="select" disabled={off} value={s.branch_on ?? ""} onChange={(e) => set({ branch_on: (e.target.value || null) as Step["branch_on"] })}>
              {["Opened Last Email", "Clicked Last Email", "Attribute Equals", "In Segment", "Event Happened", "Record Condition"].map((b) => (
                <option key={b} value={b}>{branchName(b)}</option>
              ))}
            </select>
          </Field>
          {s.branch_on === "Attribute Equals" && (
            <>
              {text("attribute", tr("Attribute"))}
              {text("value", tr("Equals"))}
            </>
          )}
          {s.branch_on === "In Segment" && (
            <Field label={tr("Segment")} error={err("segment")}>
              <LinkSelect entity="MarketingSegment" value={s.segment} disabled={off} onChange={(v) => set({ segment: v })} />
            </Field>
          )}
          {s.branch_on === "Record Condition" && text("condition", tr("Condition"), true)}
        </>
      )}
      {s.step_type === "Update Attribute" && (
        <>
          {text("attribute", tr("Attribute"))}
          {text("value", tr("Value (empty removes the attribute)"))}
        </>
      )}
      {(s.step_type === "Add To Group" || s.step_type === "Remove From Group") && (
        <Field label={tr("Email group")} error={err("email_group")}>
          <LinkSelect entity="EmailGroup" value={s.email_group} disabled={off} onChange={(v) => set({ email_group: v })} />
        </Field>
      )}
      {s.step_type !== "Exit" && goes("next", hasOtherwise(s) ? tr("Goes to, when it holds") : tr("Goes to"))}
      {hasOtherwise(s) && goes("otherwise", tr("Otherwise goes to"))}
    </div>
  );
}

function branchName(b: string): string {
  switch (b) {
    case "Opened Last Email":
      return tr("Opened the last email");
    case "Clicked Last Email":
      return tr("Clicked the last email");
    case "Attribute Equals":
      return tr("An attribute equals");
    case "In Segment":
      return tr("Is in a segment");
    case "Event Happened":
      return tr("An event happened");
    default:
      return tr("A condition on the record holds");
  }
}
