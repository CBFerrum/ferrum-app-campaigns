// A campaign being written. Changes show at once and are saved by themselves a moment after the
// last one, each against the version on screen; the server's refusal is shown where it belongs, and
// if someone else changed the same thing meanwhile, nothing is overwritten without asking.

import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, ApiError, docPath, errorMessage, shell } from "@ferrum/client";
import { useLiveEntity } from "@ferrum/kit";
import { patchOf, savable, type Campaign } from "./campaign";
import { getCampaign, keys, whole } from "./data";

export type SaveState = "saved" | "unsaved" | "saving" | "waiting" | "error" | "conflict";

export interface RowError {
  field: string;
  message: string;
  row: [string, number];
}

/** A moment after the last change. */
const AUTOSAVE_MS = 1200;

const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

const get = (c: Campaign | Partial<Campaign>, k: string): unknown => (c as Record<string, unknown>)[k];

/** What of `snapshot` is still waiting: a field changed again while it was being saved stays. */
function stillEdited(current: Partial<Campaign>, snapshot: Partial<Campaign>): Partial<Campaign> {
  const rest = { ...current } as Record<string, unknown>;
  for (const [k, v] of Object.entries(snapshot)) if (rest[k] === v) delete rest[k];
  return rest as Partial<Campaign>;
}

type Problem = { kind: "error" | "waiting" | "conflict"; text: string };

export function useDraft(id: string, writable: boolean) {
  const qc = useQueryClient();
  const query = useQuery({ queryKey: keys.campaign(id), queryFn: ({ signal }) => getCampaign(id, signal) });
  const saved = query.data;
  const savedRef = useRef(saved);
  savedRef.current = saved;

  const [edits, setEdits] = useState<Partial<Campaign>>({});
  const editsRef = useRef(edits);
  editsRef.current = edits;
  const [saving, setSaving] = useState(false);
  const [problem, setProblem] = useState<Problem | null>(null);
  const problemRef = useRef(problem);
  problemRef.current = problem;
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [rowErrors, setRowErrors] = useState<RowError[]>([]);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [theirs, setTheirs] = useState<string[]>([]);
  const busy = useRef<Promise<Campaign | null> | null>(null);
  /** The version this screen last saved, so its own save is not taken for someone else's. */
  const ours = useRef<number | null>(null);

  const doc = useMemo(() => (saved ? ({ ...saved, ...edits } as Campaign) : undefined), [saved, edits]);

  const set = useCallback(<K extends keyof Campaign>(key: K, value: Campaign[K]) => {
    setEdits((e) => {
      const rest = { ...e } as Record<string, unknown>;
      // Back to what is saved: not an edit any more.
      if (savedRef.current && same(get(savedRef.current, key), value)) delete rest[key];
      else rest[key] = value;
      return rest as Partial<Campaign>;
    });
    setFieldErrors((f) => {
      if (!(key in f)) return f;
      const { [key]: _gone, ...rest } = f;
      return rest;
    });
    if (key === "audience" || key === "variants") setRowErrors((r) => r.filter((x) => x.row[0] !== key));
    if (problemRef.current?.kind === "waiting") setProblem(null);
  }, []);

  const attempt = useCallback(
    async (base: Campaign, snapshot: Partial<Campaign>, retried: boolean, waiting: string | null): Promise<Campaign | null> => {
      try {
        const next = await api.patch<Campaign>(docPath("EmailCampaign", id), patchOf(base, snapshot), { ifMatch: base.version });
        const merged = whole({ ...next, _titles: { ...savedRef.current?._titles, ...next._titles } });
        ours.current = next.version;
        savedRef.current = merged;
        qc.setQueryData<Campaign>(keys.campaign(id), merged);
        // The reply has no names for linked records: read them again.
        void qc.invalidateQueries({ queryKey: keys.campaign(id) });
        void qc.invalidateQueries({ queryKey: keys.list });
        setEdits((cur) => stillEdited(cur, snapshot));
        setProblem(waiting ? { kind: "waiting", text: waiting } : null);
        setFieldErrors({});
        setRowErrors([]);
        setSavedAt(Date.now());
        return merged;
      } catch (e) {
        if (e instanceof ApiError && (e.status === 409 || e.status === 412)) {
          // Someone saved first. If they changed nothing this screen has changed, carry on from theirs.
          const fresh = await qc.fetchQuery({ queryKey: keys.campaign(id), queryFn: ({ signal }) => getCampaign(id, signal), staleTime: 0 });
          savedRef.current = fresh;
          const overlap = Object.keys(snapshot).filter((k) => !same(get(base, k), get(fresh, k)));
          if (!overlap.length && !retried) return attempt(fresh, snapshot, true, waiting);
          setTheirs(overlap);
          setProblem({ kind: "conflict", text: "Someone else changed this campaign while you were editing it." });
          return null;
        }
        if (e instanceof ApiError && e.fields.length) {
          setFieldErrors(Object.fromEntries(e.fields.filter((f) => !f.row).map((f) => [f.field, f.message])));
          setRowErrors(e.fields.filter((f): f is typeof f & { row: [string, number] } => !!f.row).map((f) => ({ field: f.field, message: f.message, row: f.row })));
        }
        setProblem({ kind: "error", text: errorMessage(e, false) });
        return null;
      }
    },
    [id, qc],
  );

  /** Saves what can be saved of what is edited now; resolves to the saved campaign, or null if saving failed. */
  const save = useCallback(async (): Promise<Campaign | null> => {
    while (busy.current) await busy.current;
    const base = savedRef.current;
    if (!base) return null;
    const { ready, waiting } = savable(editsRef.current);
    // A row still waiting for its choice is said, and kept; everything else goes.
    if (waiting) setProblem({ kind: "waiting", text: waiting });
    if (!Object.keys(ready).length) return base;
    if (!Object.keys(patchOf(base, ready)).length) {
      setEdits((cur) => stillEdited(cur, ready));
      return base;
    }
    setSaving(true);
    const run = attempt(base, ready, false, waiting);
    busy.current = run;
    try {
      return await run;
    } finally {
      busy.current = null;
      setSaving(false);
    }
  }, [attempt]);

  // A moment after the last change.
  const changes = Object.keys(edits).length > 0;
  const editable = writable && saved?.docstatus === 0;
  useEffect(() => {
    if (!editable || !changes || problemRef.current?.kind === "conflict") return;
    const t = setTimeout(() => void save(), AUTOSAVE_MS);
    return () => clearTimeout(t);
  }, [edits, editable, changes, save]);

  // Someone else's change shows up; if it is to something this screen has changed too, say so.
  const previous = useRef<Campaign | undefined>(undefined);
  useEffect(() => {
    const was = previous.current;
    previous.current = saved;
    if (!was || !saved || was.version === saved.version || saved.version === ours.current) return;
    const overlap = Object.keys(editsRef.current).filter((k) => !same(get(was, k), get(saved, k)));
    if (overlap.length) {
      setTheirs(overlap);
      setProblem({ kind: "conflict", text: "Someone else changed this campaign while you were editing it." });
    }
  }, [saved]);
  useLiveEntity(
    "EmailCampaign",
    useCallback((e) => void (e.id === id && qc.invalidateQueries({ queryKey: keys.campaign(id) })), [qc, id]),
  );

  // Leaving with something unsaved: a moment's grace here, and the browser's question on a reload.
  useEffect(() => shell().leaveGuard(() => Object.keys(editsRef.current).length > 0), []);
  useEffect(
    () => () => {
      if (Object.keys(editsRef.current).length) void save();
    },
    [save],
  );

  /** Use what is saved now and drop what was edited here. */
  const reload = useCallback(async () => {
    setEdits({});
    setProblem(null);
    setTheirs([]);
    setFieldErrors({});
    setRowErrors([]);
    await qc.invalidateQueries({ queryKey: keys.campaign(id) });
  }, [qc, id]);

  /** Keep what was edited here on top of what they saved. */
  const keepMine = useCallback(() => {
    setTheirs([]);
    setProblem(null);
    void save();
  }, [save]);

  const status: SaveState = problem?.kind === "conflict" ? "conflict" : saving ? "saving" : problem ? problem.kind : changes ? "unsaved" : "saved";

  return { query, saved, doc, editable, set, save, reload, keepMine, status, message: problem?.text ?? null, fieldErrors, rowErrors, savedAt, theirs, dirty: changes };
}

export type Draft = ReturnType<typeof useDraft>;
