// Small pieces the screens share.

import type { ReactNode } from "react";
import { Icon } from "@ferrum/kit";
import { clock } from "./format";
import { insertAt } from "./snippets";
import type { Draft } from "./useDraft";
import { t, isolate } from "@ferrum/client";

/** A thin bar for a share of something (a send's progress, a link's clicks). `value` is 0 to 100. */
export function Bar({ value, label, tone }: { value: number | null; label: string; tone?: "primary" | "muted" }) {
  const v = Math.max(0, Math.min(100, value ?? 0));
  return (
    <div className={`cm-bar${tone === "muted" ? " cm-bar-muted" : ""}`} role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(v)}>
      <span style={{ width: `${v}%` }} />
    </div>
  );
}

/** What a draft's saving is up to, in words, where it can be seen. */
export function SaveLine({ draft, editable }: { draft: Draft; editable: boolean }) {
  if (!editable) return null;
  const s = draft.status;
  const text =
    s === "saving"
      ? t("Saving…")
      : s === "unsaved"
        ? t("Saving in a moment…")
        : s === "waiting"
          ? (draft.message ?? t("Not saved yet"))
          : s === "error"
            ? t("Not saved: {value}", { value: draft.message ?? t("something went wrong") })
            : s === "conflict"
              ? t("Not saved: someone else changed this")
              : draft.savedAt
                ? t("Saved {value}", { value: clock(draft.savedAt) })
                : t("All changes saved");
  const bad = s === "error" || s === "conflict";
  return (
    <span className={`cm-save${bad ? " cm-save-bad" : ""}`} role="status" aria-live="polite">
      {s === "saved" && <Icon name="check" size={13} />}
      {isolate(text)}
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
