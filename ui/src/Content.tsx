// The second tab: writing the email. HTML in a box with what a person needs to hand (details that
// change with each person, a few blocks that mail programs understand, a Mail template to start
// from) and, beside it, the email as the server builds it. Plain text, and a version of an A/B
// test with content of its own, are the same box.

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useRef, useState } from "react";
import { confirmDialog, Dialog, Button, EmptyState, ErrorState, Field, Menu, MenuItem, PageLoading, placed } from "@ferrum/kit";
import { list } from "./data";
import { PreviewPane } from "./PreviewPane";
import { size } from "./format";
import { attributeTag, BLOCKS, MERGE_TAGS, starter, UNSUBSCRIBE, validKey } from "./snippets";
import type { Draft } from "./useDraft";
import { insertInto } from "./ui";
import { t as tr, isolate } from "@ferrum/client";

interface Template {
  id: string;
  template_name: string;
  subject: string;
  body: string;
}

function Templates({ onPick, onClose }: { onPick: (t: Template) => void; onClose: () => void }) {
  const q = useQuery({
    queryKey: ["campaigns", "templates"],
    queryFn: ({ signal }) => list<Template>("EmailTemplate", { filters: JSON.stringify({ disabled: false }), fields: "template_name,subject,body", sort: "template_name asc", limit: 100 }, signal),
  });
  return (
    <Dialog open onClose={onClose} title={tr("Start from a template")}>
      {q.error ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} />
      ) : !q.data ? (
        <PageLoading label={tr("Loading templates")} />
      ) : q.data.rows.length === 0 ? (
        <EmptyState title={tr("No templates yet")} icon="file">
          <p className="muted">{tr("Templates are kept in the Mail app. Anything you write here can be saved there for next time.")}</p>
        </EmptyState>
      ) : (
        <ul className="cm-templates">
          {q.data.rows.map((t) => (
            <li key={t.id}>
              <div>
                <strong>{isolate(t.template_name)}</strong>
                <div className="muted">{isolate(t.subject)}</div>
              </div>
              <Button size="sm" onClick={() => onPick(t)}>
                {tr("Use")}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </Dialog>
  );
}

function AttributeDialog({ onInsert, onClose }: { onInsert: (code: string) => void; onClose: () => void }) {
  const [key, setKey] = useState("");
  const ok = validKey(key.trim());
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (ok) onInsert(attributeTag(key.trim()));
  };
  return (
    <Dialog open onClose={onClose} title={tr("Insert an attribute")}>
      <form className="stack" onSubmit={submit}>
        <Field label={tr("Attribute name")} htmlFor="cm-attr" error={key && !ok ? tr("Use lowercase letters, digits and _, starting with a letter.") : undefined} hint={tr("The name a contact's attribute has, such as city or plan. A person without it gets nothing in its place.")}>
          <input id="cm-attr" className="input" value={key} autoComplete="off" onChange={(e) => setKey(e.target.value)} />
        </Field>
        <div className="row" style={{ justifyContent: "flex-end" }}>
          <Button onClick={onClose}>{tr("Cancel")}</Button>
          <Button type="submit" variant="primary" disabled={!ok}>
            {tr("Insert")}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function Content({ draft, editable }: { draft: Draft; editable: boolean }) {
  const doc = draft.doc!;
  const { set } = draft;
  const box = useRef<HTMLTextAreaElement>(null);
  const [pane, setPane] = useState<"write" | "preview">("write");
  const [format, setFormat] = useState<"html" | "text">("html");
  const [version, setVersion] = useState<string>("main");
  const [dialog, setDialog] = useState<"templates" | "attribute" | null>(null);
  const variants = doc.ab_test ? doc.variants : [];
  const at = variants.findIndex((v) => v.variant === version);
  const onVariant = version !== "main" && at >= 0;
  const text = format === "text" && !onVariant;

  const value = onVariant ? (variants[at].content ?? "") : text ? (doc.text_content ?? "") : (doc.content ?? "");
  const write = (v: string) => {
    if (onVariant) set("variants", doc.variants.map((x, j) => (j === at ? { ...x, content: v } : x)));
    else set(text ? "text_content" : "content", v);
  };
  const insert = (code: string) => insertInto(box.current, value, code, write);
  const bytes = new TextEncoder().encode(value).length;
  const problems = [
    ...(onVariant ? draft.rowErrors.filter((e) => e.row[0] === "variants" && e.row[1] === at).map((e) => e.message) : []),
    ...(!onVariant ? [draft.fieldErrors[text ? "text_content" : "content"]].filter(Boolean) : []),
  ];

  const applyTemplate = async (t: Template) => {
    const filled = (doc.content ?? "").trim() && doc.content !== starter();
    if (filled && !(await confirmDialog({ title: tr("Replace the content?"), message: tr("“{template_name}” replaces what you have written for the email. The subject stays unless it is empty.", { template_name: t.template_name }), confirmLabel: tr("Replace") }))) return;
    set("content", t.body);
    if (!(doc.subject ?? "").trim()) set("subject", t.subject);
    setVersion("main");
    setFormat("html");
    setDialog(null);
  };

  return (
    <div className="cm-compose" data-pane={pane}>
      <div className="cm-pane-switch segmented" role="group" aria-label={tr("Show")}>
        <button type="button" aria-pressed={pane === "write"} onClick={() => setPane("write")}>
          {tr("Write")}
        </button>
        <button type="button" aria-pressed={pane === "preview"} onClick={() => setPane("preview")}>
          {tr("Preview")}
        </button>
      </div>
      <section className="cm-write card" aria-label={tr("Write the email")}>
        <div className="cm-toolbar">
          {editable && (
            <>
              <Menu label={tr("Insert")} icon="plus" ariaLabel={tr("Insert a person's detail")} buttonClass="btn btn-default btn-sm">
                {(close) => (
                  <>
                    {MERGE_TAGS.map((t) => (
                      <MenuItem key={t.id} onSelect={() => (close(), insert(t.code))}>
                        {isolate(t.label)}
                      </MenuItem>
                    ))}
                    <MenuItem onSelect={() => (close(), setDialog("attribute"))}>{tr("An attribute…")}</MenuItem>
                    {!text && <MenuItem onSelect={() => (close(), insert(UNSUBSCRIBE.code))}>{isolate(UNSUBSCRIBE.label)}</MenuItem>}
                  </>
                )}
              </Menu>
              {!text && (
                <Menu label={tr("Blocks")} icon="plus" ariaLabel={tr("Insert a block of HTML")} buttonClass="btn btn-default btn-sm">
                  {(close) => BLOCKS.map((b) => <MenuItem key={b.id} onSelect={() => (close(), insert(b.code))}>{isolate(b.label)}</MenuItem>)}
                </Menu>
              )}
              <Button size="sm" icon="file" onClick={() => setDialog("templates")}>
                {tr("Templates")}
              </Button>
            </>
          )}
          <span className="spacer" />
          {variants.length > 1 && (
            <select className="select cm-version" aria-label={tr("Which version you are writing")} value={onVariant ? version : "main"} onChange={(e) => (setVersion(e.target.value), setFormat("html"))}>
              <option value="main">{tr("All versions")}</option>
              {variants.map((v) => (
                <option key={v.id ?? v.variant} value={v.variant ?? ""}>{tr("Version {value} only", { value: v.variant ?? "?" })}</option>
              ))}
            </select>
          )}
          <div className="segmented" role="group" aria-label={tr("Format")}>
            <button type="button" aria-pressed={!text} onClick={() => setFormat("html")}>
              {tr("HTML")}
            </button>
            <button type="button" aria-pressed={text} disabled={onVariant} title={onVariant ? tr("A version with its own content makes its plain text from its HTML") : undefined} onClick={() => setFormat("text")}>
              {tr("Plain text")}
            </button>
          </div>
        </div>
        {onVariant && <p className="cm-hint muted">{tr("Only version {version} gets this content. Leave it empty and it uses the content of all versions.", { version })}</p>}
        {text && <p className="cm-hint muted">{tr("The plain text some mail programs show instead. Leave it empty and it is made from the HTML, with the links kept.")}</p>}
        <label className="sr-only" htmlFor="cm-editor">
          {text ? tr("Plain text of the email") : onVariant ? tr("HTML of version {version}", { version }) : tr("HTML of the email")}
        </label>
        <textarea
          id="cm-editor"
          ref={box}
          className="textarea mono cm-editor"
          spellCheck={false}
          value={value}
          disabled={!editable}
          aria-invalid={problems.length > 0 || undefined}
          aria-describedby="cm-editor-help"
          placeholder={onVariant ? tr("Empty: this version uses the content of all versions") : text ? tr("Empty: made from the HTML") : "<p>Write your email here…</p>"}
          onChange={(e) => write(e.target.value)}
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
              e.preventDefault();
              void draft.save();
            }
          }}
        />
        {problems.map((m) => (
          <p key={m} className="field-error cm-editor-error">
            {isolate(m)}
          </p>
        ))}
        <p className="cm-hint faint" id="cm-editor-help">
          {placed(tr("{p1}. Use {p2} for what changes with each person. Unless the content has {p3}, an unsubscribe link and your postal address are added at the bottom."), { p1: isolate(size(bytes)), p2: isolate("{{ contact.first_name }}"), p3: isolate("{{ unsubscribe_url }}") })}
        </p>
      </section>
      <section className="cm-see" aria-label={tr("Preview")}>
        <PreviewPane doc={doc} dirty={draft.dirty} editable={editable} />
      </section>
      {dialog === "templates" && <Templates onPick={(t) => void applyTemplate(t)} onClose={() => setDialog(null)} />}
      {dialog === "attribute" && (
        <AttributeDialog
          onClose={() => setDialog(null)}
          onInsert={(code) => {
            setDialog(null);
            insert(code);
          }}
        />
      )}
    </div>
  );
}
