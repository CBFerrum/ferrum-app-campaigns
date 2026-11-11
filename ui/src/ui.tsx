// Small pieces the screens share.

import type { ReactNode } from "react";
import { SaveStatus, StatePanel, type SaveState } from "@ferrum/kit";
import { insertAt } from "./snippets";
import type { Draft } from "./useDraft";
import { ApiError, t, isolate } from "@ferrum/client";

/** A thin bar for a share of something (a send's progress, a link's clicks). `value` is 0 to 100. */
export function Bar({ value, label, tone }: { value: number | null; label: string; tone?: "primary" | "muted" }) {
  const v = Math.max(0, Math.min(100, value ?? 0));
  return (
    <div className={`cm-bar${tone === "muted" ? " cm-bar-muted" : ""}`} role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(v)}>
      <span style={{ width: `${v}%` }} />
    </div>
  );
}

/** What a draft's saving is up to, in words, where it can be seen: the kit's SaveStatus, plus the server's reason where there is one. */
export function SaveLine({ draft, editable }: { draft: Draft; editable: boolean }) {
  if (!editable) return null;
  const s = draft.status;
  const state: SaveState = s === "saving" ? "saving" : s === "unsaved" || s === "waiting" ? "draft" : s === "error" ? "failed" : s === "conflict" ? "conflict" : "saved";
  const reason = s === "waiting" || s === "error" ? draft.message : null;
  return (
    <span className="cm-save">
      <SaveStatus state={state} at={state === "saved" ? draft.savedAt : null} />
      {reason && <span className={s === "error" ? "cm-save-bad" : undefined}>{isolate(reason)}</span>}
      {s === "error" && (
        <button type="button" className="link-btn" onClick={() => void draft.save()}>
          {t("Try again")}
        </button>
      )}
    </span>
  );
}

/** A heading with something beside it, above a block of a page. */
export function Section({ title, aside, children, id }: { title: string; aside?: ReactNode; children: ReactNode; id?: string }) {
  return (
    <section className="card cm-section" aria-labelledby={id}>
      <header className="card-head">
        <h2 id={id}>{isolate(title)}</h2>
        {aside && <div className="cm-aside">{aside}</div>}
      </header>
      <div className="card-body">{children}</div>
    </section>
  );
}

/** Puts `code` where the caret is in a text box (replacing a selection) and leaves the caret after it. */
export function insertInto(el: HTMLInputElement | HTMLTextAreaElement | null, value: string, code: string, set: (v: string) => void): void {
  const next = insertAt(value, el?.selectionStart ?? value.length, el?.selectionEnd ?? value.length, code);
  set(next.text);
  requestAnimationFrame(() => {
    el?.focus();
    el?.setSelectionRange(next.caret, next.caret);
  });
}

/** Why something could not be shown: not allowed (never shown as empty), no connection, or failed. */
export function Problem({ error, onRetry, title }: { error: unknown; onRetry: () => void; title?: string }) {
  if (error instanceof ApiError && error.status === 403) return <StatePanel kind="denied" title={title} />;
  if (error instanceof ApiError && error.status === 0) return <StatePanel kind="offline" title={title} onRetry={onRetry} />;
  return <StatePanel kind="failed" title={title} error={error} onRetry={onRetry} />;
}
