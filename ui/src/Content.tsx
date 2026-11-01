// The second tab: writing the email. HTML in a box with what a person needs to hand (details that
// change with each person, a few blocks that mail programs understand, a Mail template to start
// from) and, beside it, the email as the server builds it. Plain text, and a version of an A/B
// test with content of its own, are the same box.

import { useQuery } from "@tanstack/react-query";
import { type FormEvent, useRef, useState } from "react";
import { confirmDialog, Dialog, Button, EmptyState, ErrorState, Field, Menu, MenuItem, PageLoading } from "@ferrum/kit";
import { list } from "./data";
import { PreviewPane } from "./PreviewPane";
import { size } from "./format";
import { attributeTag, BLOCKS, MERGE_TAGS, STARTER, UNSUBSCRIBE, validKey } from "./snippets";
import type { Draft } from "./useDraft";
import { insertInto } from "./ui";

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
    <Dialog open onClose={onClose} title="Start from a template">
      {q.error ? (
        <ErrorState error={q.error} onRetry={() => void q.refetch()} />
      ) : !q.data ? (
        <PageLoading label="Loading templates" />
      ) : q.data.rows.length === 0 ? (
        <EmptyState title="No templates yet" icon="file">
          <p className="muted">Templates are kept in the Mail app. Anything you write here can be saved there for next time.</p>
        </EmptyState>
      ) : (
        <ul className="cm-templates">
          {q.data.rows.map((t) => (
            <li key={t.id}>
              <div>
                <strong>{t.template_name}</strong>
                <div className="muted">{t.subject}</div>
              </div>
              <Button size="sm" onClick={() => onPick(t)}>
                Use
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
    <Dialog open onClose={onClose} title="Insert an attribute">
      <form className="stack" onSubmit={submit}>
        <Field label="Attribute name" htmlFor="cm-attr" error={key && !ok ? "Use lowercase letters, digits and _, starting with a letter." : undefined} hint="The name a contact's attribute has, such as city or plan. A person without it gets nothing in its place.">
          <input id="cm-attr" className="input" value={key} autoComplete="off" onChange={(e) => setKey(e.target.value)} />
        </Field>
        <div className="row" style={{ justifyContent: "flex-end" }}>
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={!ok}>
            Insert
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
    const filled = (doc.content ?? "").trim() && doc.content !== STARTER;
    if (filled && !(await confirmDialog({ title: "Replace the content?", message: `“${t.template_name}” replaces what you have written for the email. The subject stays unless it is empty.`, confirmLabel: "Replace" }))) return;
    set("content", t.body);
    if (!(doc.subject ?? "").trim()) set("subject", t.subject);
    setVersion("main");
    setFormat("html");
    setDialog(null);
  };

  return (
    <div className="cm-compose" data-pane={pane}>
      <div className="cm-pane-switch segmented" role="group" aria-label="Show">
        <button type="button" aria-pressed={pane === "write"} onClick={() => setPane("write")}>
          Write
        </button>
        <button type="button" aria-pressed={pane === "preview"} onClick={() => setPane("preview")}>
          Preview
        </button>
      </div>
      <section className="cm-write card" aria-label="Write the email">
        <div className="cm-toolbar">
          {editable && (
            <>
              <Menu label="Insert" icon="plus" ariaLabel="Insert a person's detail" buttonClass="btn btn-default btn-sm">
                {(close) => (
                  <>
                    {MERGE_TAGS.map((t) => (
                      <MenuItem key={t.id} onSelect={() => (close(), insert(t.code))}>
                        {t.label}
                      </MenuItem>
                    ))}
                    <MenuItem onSelect={() => (close(), setDialog("attribute"))}>An attribute…</MenuItem>
                    {!text && <MenuItem onSelect={() => (close(), insert(UNSUBSCRIBE.code))}>{UNSUBSCRIBE.label}</MenuItem>}
                  </>
                )}
              </Menu>
              {!text && (
                <Menu label="Blocks" icon="plus" ariaLabel="Insert a block of HTML" buttonClass="btn btn-default btn-sm">
                  {(close) => BLOCKS.map((b) => <MenuItem key={b.id} onSelect={() => (close(), insert(b.code))}>{b.label}</MenuItem>)}
                </Menu>
              )}
              <Button size="sm" icon="file" onClick={() => setDialog("templates")}>
                Templates
              </Button>
            </>
          )}
          <span className="spacer" />
          {variants.length > 1 && (
            <select className="select cm-version" aria-label="Which version you are writing" value={onVariant ? version : "main"} onChange={(e) => (setVersion(e.target.value), setFormat("html"))}>
              <option value="main">All versions</option>
              {variants.map((v) => (
                <option key={v.id ?? v.variant} value={v.variant ?? ""}>{`Version ${v.variant ?? "?"} only`}</option>
              ))}
            </select>
          )}
          <div className="segmented" role="group" aria-label="Format">
            <button type="button" aria-pressed={!text} onClick={() => setFormat("html")}>
              HTML
            </button>
            <button type="button" aria-pressed={text} disabled={onVariant} title={onVariant ? "A version with its own content makes its plain text from its HTML" : undefined} onClick={() => setFormat("text")}>
              Plain text
            </button>
          </div>
        </div>
        {onVariant && <p className="cm-hint muted">Only version {version} gets this content. Leave it empty and it uses the content of all versions.</p>}
        {text && <p className="cm-hint muted">The plain text some mail programs show instead. Leave it empty and it is made from the HTML, with the links kept.</p>}
        <label className="sr-only" htmlFor="cm-editor">
          {text ? "Plain text of the email" : onVariant ? `HTML of version ${version}` : "HTML of the email"}
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
          placeholder={onVariant ? "Empty: this version uses the content of all versions" : text ? "Empty: made from the HTML" : "<p>Write your email here…</p>"}
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
            {m}
          </p>
        ))}
        <p className="cm-hint faint" id="cm-editor-help">
          {size(bytes)}. Use {"{{ contact.first_name }}"} for what changes with each person. Unless the content has {"{{ unsubscribe_url }}"}, an unsubscribe link and your postal address are added at the bottom.
        </p>
      </section>
      <section className="cm-see" aria-label="Preview">
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
