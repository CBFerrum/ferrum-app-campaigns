// A journey's steps as a flow (campaigns-7): the table `steps` drawn as boxes and lines, and the four edits a person
// makes on the canvas. Pure, and every function returns new rows and never changes its input. A step's `next_step` and
// `else_step` are 1-based row numbers; blank `next_step` falls through to the following row, blank `else_step` ends the
// journey. Every edit works on a graph whose targets are the steps themselves, not numbers, and writes the rows back
// with only the pointers a row needs, so an edit can never move a path it did not name.

import { t } from "@ferrum/client";

export type StepType = "Send Email" | "Send Message" | "Wait" | "Wait For Event" | "Branch" | "Update Attribute" | "Add To Group" | "Remove From Group" | "Exit";
export const STEP_TYPES: StepType[] = ["Send Email", "Send Message", "Wait", "Wait For Event", "Branch", "Update Attribute", "Add To Group", "Remove From Group", "Exit"];

export interface Step {
  id?: string;
  step_type: StepType;
  label?: string | null;
  subject?: string | null;
  preheader?: string | null;
  content?: string | null;
  email_template?: string | null;
  topic?: string | null;
  channel?: "Text Message" | "Chat App" | null;
  message_template?: string | null;
  wait_amount?: number | null;
  wait_unit?: "Minutes" | "Hours" | "Days" | null;
  branch_on?: "Opened Last Email" | "Clicked Last Email" | "Attribute Equals" | "In Segment" | "Event Happened" | "Record Condition" | null;
  event_rule?: number | null;
  condition?: string | null;
  attribute?: string | null;
  value?: string | null;
  segment?: string | null;
  email_group?: string | null;
  next_step?: number | null;
  else_step?: number | null;
  _titles?: Record<string, string>;
}

export const MAX_STEPS = 50;

/** The step a pointer names, as a 0-based index; `null` when the journey ends. */
export const END = null;
export type Target = number | null;

export const hasOtherwise = (s: Pick<Step, "step_type">): boolean => s.step_type === "Branch" || s.step_type === "Wait For Event";

/** Where step `i` (0-based) leads when its condition holds, or always. */
export function nextOf(steps: Step[], i: number): Target {
  const s = steps[i];
  if (!s || s.step_type === "Exit") return END;
  const p = s.next_step;
  if (p != null) return p >= 1 && p <= steps.length ? p - 1 : END;
  return i + 1 < steps.length ? i + 1 : END;
}

/** Where a Branch or Wait For Event leads otherwise (the journey ends when blank). */
export function otherwiseOf(steps: Step[], i: number): Target | undefined {
  const s = steps[i];
  if (!s || !hasOtherwise(s)) return undefined;
  const p = s.else_step;
  return p != null && p >= 1 && p <= steps.length ? p - 1 : END;
}

// ── the graph the edits work on ──

interface Node {
  step: Step;
  next: Node | null;
  other: Node | null;
}

const clean = (s: Step): Step => ({ ...s, next_step: null, else_step: null });

function toGraph(steps: Step[]): Node[] {
  const nodes: Node[] = steps.map((s) => ({ step: clean(s), next: null, other: null }));
  steps.forEach((_, i) => {
    const n = nextOf(steps, i);
    const o = otherwiseOf(steps, i);
    nodes[i].next = n == null ? null : nodes[n];
    nodes[i].other = o == null ? null : nodes[o];
  });
  return nodes;
}

/** The rows of a graph: where a path ends on a row that is not the last, an Exit is added (or the last Exit reused). */
function fromGraph(nodes: Node[]): Step[] {
  const list = [...nodes];
  const open = (n: Node) => n.step.step_type !== "Exit" && n.next === null;
  if (list.some((n) => open(n) && n !== list[list.length - 1])) {
    let exit = list[list.length - 1];
    if (exit.step.step_type !== "Exit") {
      exit = { step: { step_type: "Exit" }, next: null, other: null };
      list.push(exit);
    }
    for (const n of list) if (open(n)) n.next = exit;
  }
  return list.map((n, i) => {
    const s = { ...n.step };
    if (s.step_type === "Exit") return s;
    const to = n.next ? list.indexOf(n.next) : -1;
    s.next_step = to === -1 || to === i + 1 ? null : to + 1;
    if (hasOtherwise(s)) s.else_step = n.other ? list.indexOf(n.other) + 1 : null;
    return s;
  });
}

const without = (nodes: Node[], gone: Node, to: Node | null): void => {
  for (const n of nodes) {
    if (n.next === gone) n.next = to === n ? null : to;
    if (n.other === gone) n.other = to === n ? null : to;
  }
};

/** A new step on the line after step `i`: the line's old target follows it. */
export function insertOnNext(steps: Step[], i: number, s: Step): Step[] {
  if (!steps[i] || steps[i].step_type === "Exit" || steps.length >= MAX_STEPS) return steps;
  const nodes = toGraph(steps);
  const made: Node = { step: clean(s), next: nodes[i].next, other: null };
  nodes[i].next = made;
  nodes.splice(i + 1, 0, made);
  return fromGraph(nodes);
}

/** A new step on the otherwise line of step `i` (a Branch or Wait For Event): it takes over what that line led to. */
export function insertOnElse(steps: Step[], i: number, s: Step): Step[] {
  if (!steps[i] || !hasOtherwise(steps[i]) || steps.length >= MAX_STEPS) return steps;
  const nodes = toGraph(steps);
  const made: Node = { step: clean(s), next: nodes[i].other, other: null };
  nodes[i].other = made;
  nodes.push(made);
  return fromGraph(nodes);
}

/** Takes step `i` out: whatever led to it leads to where it led. Refused when it is the only step. */
export function removeStep(steps: Step[], i: number): { steps: Step[]; refused?: string } {
  if (!steps[i]) return { steps };
  if (steps.length === 1) return { steps, refused: t("A journey needs at least one step.") };
  const nodes = toGraph(steps);
  const [gone] = nodes.splice(i, 1);
  without(nodes, gone, gone.next);
  return { steps: fromGraph(nodes) };
}

/** Points a step's line (`next` or `otherwise`) at step `target` (0-based), or at the end. */
export function connect(steps: Step[], i: number, side: "next" | "otherwise", target: Target): Step[] {
  const s = steps[i];
  if (!s || s.step_type === "Exit" || (side === "otherwise" && !hasOtherwise(s)) || (target != null && !steps[target])) return steps;
  const nodes = toGraph(steps);
  const to = target == null ? null : nodes[target];
  if (side === "next") nodes[i].next = to;
  else nodes[i].other = to;
  return fromGraph(nodes);
}

export function blankStep(type: StepType): Step {
  switch (type) {
    case "Send Email":
      return { step_type: type, subject: "", content: "" };
    case "Send Message":
      return { step_type: type, channel: "Text Message", message_template: "" };
    case "Wait":
    case "Wait For Event":
      return { step_type: type, wait_amount: 1, wait_unit: "Days" };
    case "Branch":
      return { step_type: type, branch_on: "Opened Last Email" };
    default:
      return { step_type: type };
  }
}

// ── where things are drawn ──

export interface Placed {
  step: number;
  lane: number;
  level: number;
}
export interface Edge {
  from: number;
  to: Target;
  kind: "next" | "otherwise";
  back: boolean;
}

/** Lanes and levels: step 1 at the top left, each step's line below it, an otherwise line in the next free lane. */
export function layout(steps: Step[]): { nodes: Placed[]; edges: Edge[]; unreached: number[] } {
  if (steps.length === 0) return { nodes: [], edges: [], unreached: [] };
  const placed = new Map<number, Placed>();
  const put = (step: number, lane: number, level: number) => placed.set(step, { step, lane, level });
  const taken = (lane: number, level: number) => [...placed.values()].some((p) => p.lane === lane && p.level >= level);
  put(0, 0, 0);
  const queue = [0];
  const edges: Edge[] = [];
  while (queue.length) {
    const i = queue.shift()!;
    const here = placed.get(i)!;
    const n = nextOf(steps, i);
    edges.push({ from: i, to: n, kind: "next", back: false });
    if (n != null && !placed.has(n)) (put(n, here.lane, here.level + 1), queue.push(n));
    const o = otherwiseOf(steps, i);
    if (o !== undefined) {
      edges.push({ from: i, to: o, kind: "otherwise", back: false });
      if (o != null && !placed.has(o)) {
        let lane = here.lane + 1;
        while (taken(lane, here.level + 1)) lane++;
        put(o, lane, here.level + 1);
        queue.push(o);
      }
    }
  }
  for (const e of edges) e.back = e.to != null && placed.has(e.to) && placed.get(e.to)!.level <= placed.get(e.from)!.level;
  const unreached = steps.map((_, i) => i).filter((i) => !placed.has(i));
  const nodes = [...placed.values()].sort((a, b) => a.level - b.level || a.lane - b.lane);
  return { nodes, edges, unreached };
}

// ── words ──

/** What a step is called on the canvas. */
export function stepName(s: Step): string {
  if (s.label?.trim()) return s.label.trim();
  switch (s.step_type) {
    case "Send Email":
      return s.subject?.trim() ? t("Send email: {subject}", { subject: s.subject.trim() }) : t("Send an email");
    case "Send Message":
      return s.channel === "Chat App" ? t("Send a chat message") : t("Send a text message");
    case "Wait":
      return t("Wait {n} {unit}", { n: s.wait_amount ?? 1, unit: unitName(s.wait_unit ?? "Days", s.wait_amount ?? 1) });
    case "Wait For Event":
      return t("Wait for an event, up to {n} {unit}", { n: s.wait_amount ?? 1, unit: unitName(s.wait_unit ?? "Days", s.wait_amount ?? 1) });
    case "Branch":
      return t("Branch");
    case "Update Attribute":
      return s.attribute ? t("Set {attribute}", { attribute: s.attribute }) : t("Update an attribute");
    case "Add To Group":
      return t("Add to an email group");
    case "Remove From Group":
      return t("Remove from an email group");
    case "Exit":
      return t("Leave the journey");
  }
}

function unitName(unit: "Minutes" | "Hours" | "Days", n: number): string {
  if (unit === "Minutes") return n === 1 ? t("minute") : t("minutes");
  if (unit === "Hours") return n === 1 ? t("hour") : t("hours");
  return n === 1 ? t("day") : t("days");
}

export const typeName = (type: StepType): string => {
  switch (type) {
    case "Send Email":
      return t("Send an email");
    case "Send Message":
      return t("Send a text or chat message");
    case "Wait":
      return t("Wait");
    case "Wait For Event":
      return t("Wait for an event");
    case "Branch":
      return t("Branch");
    case "Update Attribute":
      return t("Set an attribute");
    case "Add To Group":
      return t("Add to an email group");
    case "Remove From Group":
      return t("Remove from an email group");
    case "Exit":
      return t("Leave the journey");
  }
};

// ── what the server refuses ──

export interface Problem {
  step: number | null;
  field: string;
  message: string;
}

const KEY = /^[a-z][a-z0-9_]{0,39}$/;

/** The server's rules for the steps (src/journey.rs), so a save is never sent that would be refused. */
export function problems(steps: Step[]): Problem[] {
  const out: Problem[] = [];
  const bad = (step: number | null, field: string, message: string) => out.push({ step, field, message });
  if (steps.length < 1 || steps.length > MAX_STEPS) bad(null, "steps", t("A journey has 1 to {max} steps.", { max: MAX_STEPS }));
  steps.forEach((s, i) => {
    for (const f of ["next_step", "else_step"] as const) {
      const p = s[f];
      if (p != null && p > steps.length) bad(i, f, t("There is no such step."));
    }
    switch (s.step_type) {
      case "Send Email":
        if (!s.subject?.trim()) bad(i, "subject", t("Enter the subject."));
        if (!s.content?.trim()) bad(i, "content", t("Write the email."));
        break;
      case "Send Message":
        if (!s.channel) bad(i, "channel", t("Choose text message or chat app."));
        if (!s.message_template?.trim()) bad(i, "message_template", t("Enter the key of the message template."));
        break;
      case "Wait":
      case "Wait For Event":
        if ((s.wait_amount ?? 0) < 1) bad(i, "wait_amount", t("Enter how long to wait."));
        break;
      case "Branch":
        if (!s.branch_on) bad(i, "branch_on", t("Choose what to branch on."));
        else if (s.branch_on === "Attribute Equals" && !KEY.test(s.attribute ?? "")) bad(i, "attribute", t("Enter an attribute key."));
        else if (s.branch_on === "In Segment" && !s.segment) bad(i, "segment", t("Choose the segment."));
        break;
      case "Update Attribute":
        if (!KEY.test(s.attribute ?? "")) bad(i, "attribute", t("Enter an attribute key."));
        break;
      case "Add To Group":
      case "Remove From Group":
        if (!s.email_group) bad(i, "email_group", t("Choose the email group."));
        break;
      case "Exit":
        break;
    }
  });
  return out;
}

/** The rows as the records API takes them: no client-only keys. */
export const forSave = (steps: Step[]): Record<string, unknown>[] =>
  steps.map((s) => Object.fromEntries(Object.entries(s).filter(([k, v]) => !k.startsWith("_") && v !== undefined)));
