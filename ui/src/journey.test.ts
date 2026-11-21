import { describe, expect, it } from "vitest";
import { blankStep, connect, END, insertOnElse, insertOnNext, layout, nextOf, otherwiseOf, problems, removeStep, type Step, type StepType } from "./journey";

const step = (tag: string, type: StepType = "Wait", over: Partial<Step> = {}): Step => ({ ...blankStep(type), label: tag, ...over });
const line = (...tags: string[]): Step[] => tags.map((t) => step(t));
// An Exit the editor added has no tag: it is the end.
const tagOf = (steps: Step[], i: number | null) => (i == null ? "END" : (steps[i].label ?? "END"));
const succ = (steps: Step[]) => Object.fromEntries(steps.map((s, i) => [s.label, [tagOf(steps, nextOf(steps, i)), steps[i].step_type === "Exit" ? undefined : tagOf(steps, otherwiseOf(steps, i) ?? null)]]));

describe("laying out a journey", () => {
  it("lays out a line", () => {
    const l = layout(line("a", "b", "c"));
    expect(l.nodes.map((n) => [n.step, n.lane, n.level])).toEqual([[0, 0, 0], [1, 0, 1], [2, 0, 2]]);
    expect(l.edges.map((e) => [e.from, e.to])).toEqual([[0, 1], [1, 2], [2, END]]);
  });

  it("a branch puts its other path in the next lane", () => {
    const steps = [step("a", "Branch", { next_step: 2, else_step: 3 }), step("yes", "Exit"), step("no", "Exit")];
    const l = layout(steps);
    expect(l.nodes.find((n) => n.step === 1)).toMatchObject({ lane: 0, level: 1 });
    expect(l.nodes.find((n) => n.step === 2)).toMatchObject({ lane: 1, level: 1 });
  });

  it("a loop back is an edge, not a second box", () => {
    const steps = [step("a"), step("b", "Wait", { next_step: 1 })];
    const l = layout(steps);
    expect(l.nodes).toHaveLength(2);
    expect(l.edges.find((e) => e.from === 1)).toMatchObject({ to: 0, back: true });
  });

  it("steps nothing leads to are listed apart", () => {
    const steps = [step("a", "Wait", { next_step: 3 }), step("lost"), step("c")];
    expect(layout(steps).unreached).toEqual([1]);
  });
});

describe("editing the flow", () => {
  it("inserting on a line keeps every other path, for explicit and for blank next steps", () => {
    for (const explicit of [false, true]) {
      const steps = [step("a", "Wait", explicit ? { next_step: 3 } : {}), step("b"), step("c")];
      const out = insertOnNext(steps, 0, step("new"));
      const s = succ(out);
      expect(s.a[0]).toBe("new");
      expect(s.new[0]).toBe(explicit ? "c" : "b");
      expect(s.b[0]).toBe("c");
      expect(s.c[0]).toBe("END");
    }
  });

  it("inserting on the other path ends the old last step explicitly", () => {
    const steps = [step("a", "Branch", { else_step: 2 }), step("b")];
    const out = insertOnElse(steps, 0, step("other"));
    const s = succ(out);
    expect(s.a[1]).toBe("other");
    expect(s.other[0]).toBe("b");
    expect(s.b[0]).toBe("END");
    expect(out[out.length - 1].step_type === "Exit" || nextOf(out, out.findIndex((x) => x.label === "b")) === END).toBe(true);
    expect(out.findIndex((x) => x.label === "b")).toBeLessThan(out.findIndex((x) => x.label === "other"));
  });

  it("removing a step joins its neighbours", () => {
    const out = removeStep(line("a", "b", "c"), 1).steps;
    expect(out.map((s) => s.label)).toEqual(["a", "c"]);
    expect(succ(out).a[0]).toBe("c");
    const withBranch = [step("a", "Branch", { next_step: 2, else_step: 3 }), step("b"), step("c")];
    expect(succ(removeStep(withBranch, 1).steps).a).toEqual(["c", "c"]);
  });

  it("removing the only step is refused", () => {
    const only = line("a");
    const r = removeStep(only, 0);
    expect(r.steps).toBe(only);
    expect(r.refused).toBeTruthy();
  });

  it("connects a line to another step or to the end", () => {
    const steps = line("a", "b", "c");
    expect(succ(connect(steps, 0, "next", 2)).a[0]).toBe("c");
    const ended = connect(steps, 0, "next", END);
    expect(succ(ended).a[0]).toBe("END");
    expect(succ(ended).b[0]).toBe("c");
  });

  it("functions never change their input", () => {
    const steps = [step("a", "Branch", { next_step: 2, else_step: 3 }), step("b"), step("c")];
    const before = JSON.stringify(steps);
    insertOnNext(steps, 0, step("n"));
    insertOnElse(steps, 0, step("o"));
    removeStep(steps, 1);
    connect(steps, 1, "next", 0);
    expect(JSON.stringify(steps)).toBe(before);
  });

  // The property: an edit never moves a path it did not name.
  it("edits never move an untouched path", () => {
    let seed = 7;
    const rnd = (n: number) => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff), seed % n);
    for (let round = 0; round < 200; round++) {
      const n = 1 + rnd(12);
      const steps: Step[] = Array.from({ length: n }, (_, i) => {
        const type = (["Wait", "Branch", "Wait For Event", "Exit"] as StepType[])[rnd(4)];
        const s = step(`s${i}`, type);
        if (type !== "Exit" && rnd(2)) s.next_step = 1 + rnd(n);
        if ((type === "Branch" || type === "Wait For Event") && rnd(2)) s.else_step = 1 + rnd(n);
        return s;
      });
      const before = succ(steps);
      const i = rnd(n);
      const edits: [string, Step[], string[]][] = [
        ["next", steps[i].step_type === "Exit" ? steps : insertOnNext(steps, i, step("new")), [`s${i}`, "new"]],
        ["else", insertOnElse(steps, i, step("new")), [`s${i}`, "new"]],
        ["remove", removeStep(steps, i).steps, [`s${i}`]],
        ["connect", connect(steps, i, "next", rnd(n)), [`s${i}`]],
      ];
      for (const [kind, out, named] of edits) {
        const after = succ(out);
        for (const tag of Object.keys(before)) {
          if (named.includes(tag) || !(tag in after)) continue;
          // a step that led to a removed or inserted-on step leads to where that one led: compare through them
          const ok = (idx: 0 | 1) => {
            const was = before[tag][idx], now = after[tag][idx];
            if (was === now) return true;
            if (kind === "remove" && was === `s${i}`) return true;
            if ((kind === "next" || kind === "else") && now === "new") return true;
            return false;
          };
          expect(ok(0) && ok(1), `${kind} on ${i} moved ${tag}: ${JSON.stringify(before[tag])} -> ${JSON.stringify(after[tag])}`).toBe(true);
        }
      }
    }
  });
});

describe("problems", () => {
  const one = (s: Partial<Step> & { step_type: StepType }) => problems([{ ...s } as Step]).map((p) => p.field);

  it("match the server", () => {
    expect(problems([])).toHaveLength(1);
    expect(one({ step_type: "Send Email" })).toEqual(["subject", "content"]);
    expect(one({ step_type: "Send Message", channel: null })).toEqual(["channel", "message_template"]);
    expect(one({ step_type: "Wait", wait_amount: 0 })).toEqual(["wait_amount"]);
    expect(one({ step_type: "Branch" })).toEqual(["branch_on"]);
    expect(one({ step_type: "Branch", branch_on: "Attribute Equals", attribute: "Bad Key" })).toEqual(["attribute"]);
    expect(one({ step_type: "Branch", branch_on: "In Segment" })).toEqual(["segment"]);
    expect(one({ step_type: "Update Attribute" })).toEqual(["attribute"]);
    expect(one({ step_type: "Add To Group" })).toEqual(["email_group"]);
    expect(one({ step_type: "Exit", next_step: 5 })).toEqual(["next_step"]);
    expect(one({ step_type: "Exit" })).toEqual([]);
  });
});
