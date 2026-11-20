// The email builder: rows, columns and blocks on a canvas, a palette to add blocks and a panel for the selected block's
// fields. Every change is a new tree handed to `onChange`; the server makes the HTML from it. A block is moved by its
// grip (drag), by Alt+Up and Alt+Down, or from its menu; each move is announced.

import { useRef, useState } from "react";
import { t as tr, isolate } from "@ferrum/client";
import { announce, Button, confirmDialog, Menu, MenuItem, richTextToPlain, sanitizeRichText } from "@ferrum/kit";
import {
  type Align,
  type At,
  type Block,
  type BlockType,
  BLOCK_TYPES,
  addBlock,
  addRow,
  blankBlock,
  blockNamed,
  type Design,
  LIMITS,
  moveBlock,
  moveRow,
  NEUTRAL,
  type Problem,
  problems,
  removeBlock,
  removeRow,
  setColumns,
  updateBlock,
  updateRow,
} from "./design";

export function kindName(type: BlockType): string {
  switch (type) {
    case "heading":
      return tr("Heading");
    case "text":
      return tr("Text");
    case "button":
      return tr("Button");
    case "image":
      return tr("Picture");
    case "divider":
      return tr("Divider");
    case "spacer":
      return tr("Space");
  }
}

/** What a block is called to someone who cannot see it. */
export function blockName(b: Block): string {
  switch (b.type) {
    case "heading":
      return tr("Heading: {text}", { text: isolate(b.text) });
    case "text":
      return tr("Text: {text}", { text: isolate(richTextToPlain(b.html).slice(0, 60)) });
    case "button":
      return tr("Button: {label}", { label: isolate(b.label) });
    case "image":
      return tr("Picture: {alt}", { alt: isolate(b.alt || b.src) });
    case "divider":
      return tr("Divider");
    case "spacer":
      return tr("Space of {height} pixels", { height: b.height });
  }
}

const same = (a: At | null, b: At) => !!a && a.row === b.row && a.column === b.column && a.index === b.index;
const key = (at: At) => `${at.row}-${at.column}-${at.index}`;

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="field cm-bd-field">
      <span className="field-label">{label}</span>
      {children}
    </label>
  );
}

function Colour({ label, value, onChange }: { label: string; value: string | null; onChange: (v: string | null) => void }) {
  return (
    <div className="field cm-bd-field">
      <label className="field-label" htmlFor={`cm-bd-${label}`}>{label}</label>
      <div className="row">
        <input id={`cm-bd-${label}`} type="color" value={value ?? NEUTRAL} onChange={(e) => onChange(e.target.value)} />
        <Button size="sm" disabled={value == null} onClick={() => onChange(null)}>
          {tr("Use the page's")}
        </Button>
      </div>
    </div>
  );
}

const ALIGNS: Align[] = ["left", "center", "right"];

function AlignField({ value, onChange }: { value: Align; onChange: (v: Align) => void }) {
  const names: Record<Align, string> = { left: tr("Left"), center: tr("Centre"), right: tr("Right") };
  return (
    <Field label={tr("Align")}>
      <select className="select" value={value} onChange={(e) => onChange(e.target.value as Align)}>
        {ALIGNS.map((a) => (
          <option key={a} value={a}>{names[a]}</option>
        ))}
      </select>
    </Field>
  );
}

/** The selected block's fields. Text is cleaned of anything a mail may not hold when the box is left. */
function Panel({ block, onPatch }: { block: Block; onPatch: (p: Partial<Block>) => void }) {
  const [html, setHtml] = useState(block.type === "text" ? block.html : "");
  switch (block.type) {
    case "heading":
      return (
        <div className="stack">
          <Field label={tr("Text")}>
            <input className="input" value={block.text} onChange={(e) => onPatch({ text: e.target.value })} />
          </Field>
          <Field label={tr("Size")}>
            <select className="select" value={block.level} onChange={(e) => onPatch({ level: Number(e.target.value) as 1 | 2 | 3 })}>
              <option value={1}>{tr("Large")}</option>
              <option value={2}>{tr("Medium")}</option>
              <option value={3}>{tr("Small")}</option>
            </select>
          </Field>
          <AlignField value={block.align} onChange={(align) => onPatch({ align })} />
          <Colour label={tr("Colour")} value={block.color} onChange={(color) => onPatch({ color })} />
        </div>
      );
    case "text":
      return (
        <div className="stack">
          <Field label={tr("Text (HTML)")}>
            <textarea
              className="textarea mono"
              rows={8}
              spellCheck={false}
              value={html}
              onChange={(e) => setHtml(e.target.value)}
              onBlur={() => {
                const clean = sanitizeRichText(html, { mail: true });
                setHtml(clean);
                if (clean !== block.html) onPatch({ html: clean });
              }}
            />
          </Field>
          <AlignField value={block.align} onChange={(align) => onPatch({ align })} />
        </div>
      );
    case "button":
      return (
        <div className="stack">
          <Field label={tr("Label")}>
            <input className="input" value={block.label} onChange={(e) => onPatch({ label: e.target.value })} />
          </Field>
          <Field label={tr("Address")}>
            <input className="input" value={block.href} inputMode="url" onChange={(e) => onPatch({ href: e.target.value })} />
          </Field>
          <AlignField value={block.align} onChange={(align) => onPatch({ align })} />
          <Colour label={tr("Button colour")} value={block.background} onChange={(background) => onPatch({ background })} />
          <Colour label={tr("Label colour")} value={block.color} onChange={(color) => onPatch({ color })} />
        </div>
      );
    case "image":
      return (
        <div className="stack">
          <Field label={tr("Picture address (https://)")}>
            <input className="input" value={block.src} inputMode="url" onChange={(e) => onPatch({ src: e.target.value })} />
          </Field>
          <Field label={tr("Describe the picture")}>
            <input className="input" value={block.alt} onChange={(e) => onPatch({ alt: e.target.value })} />
          </Field>
          <Field label={tr("Link (optional)")}>
            <input className="input" value={block.href ?? ""} inputMode="url" onChange={(e) => onPatch({ href: e.target.value || null })} />
          </Field>
          <Field label={tr("Width in pixels (optional)")}>
            <input className="input" type="number" min={16} value={block.width ?? ""} onChange={(e) => onPatch({ width: e.target.value ? Number(e.target.value) : null })} />
          </Field>
        </div>
      );
    case "divider":
      return <Colour label={tr("Colour")} value={block.color} onChange={(color) => onPatch({ color })} />;
    case "spacer":
      return (
        <Field label={tr("Height in pixels")}>
          <input className="input" type="number" min={4} max={120} value={block.height} onChange={(e) => onPatch({ height: Number(e.target.value) })} />
        </Field>
      );
  }
}

export function Builder({ design, onChange, editable, refused }: { design: Design; onChange: (d: Design) => void; editable: boolean; refused?: string | null }) {
  const [sel, setSel] = useState<At | null>(null);
  const drag = useRef<At | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const found = problems(design);
  const named = refused ? blockNamed(refused) : null;
  const block = sel ? design.rows[sel.row]?.columns[sel.column]?.blocks[sel.index] : undefined;
  const flagged = (at: At) => found.some((p: Problem) => p.at && same(p.at, at) && p.field !== "blocks" && p.field !== "columns") || (!!named && same(named, at));

  const focusLater = (at: At) => requestAnimationFrame(() => root.current?.querySelector<HTMLElement>(`[data-at="${key(at)}"]`)?.focus());
  const edit = (next: Design, at: At | null, say?: string) => {
    onChange(next);
    setSel(at);
    if (at) focusLater(at);
    if (say) announce(say);
  };
  const lastColumn = (): At => {
    const row = design.rows.length - 1;
    const column = Math.max(0, (design.rows[row]?.columns.length ?? 1) - 1);
    return { row, column, index: design.rows[row]?.columns[column]?.blocks.length ?? 0 };
  };
  const add = (type: BlockType) => {
    const target = sel && block ? { ...sel, index: sel.index + 1 } : lastColumn();
    if (design.rows.length === 0) return edit(addBlock(addRow(design, 0, 1), { row: 0, column: 0, index: 0 }, blankBlock(type)), { row: 0, column: 0, index: 0 });
    edit(addBlock(design, target, blankBlock(type)), target);
  };
  const move = (from: At, to: At) => {
    const next = moveBlock(design, from, to);
    if (next === design) return;
    const count = next.rows[to.row].columns[to.column].blocks.length;
    const at = { ...to, index: Math.min(to.index, count - 1) };
    edit(next, at, tr("{block} moved to position {n} of {count}", { block: kindName(next.rows[at.row].columns[at.column].blocks[at.index].type), n: at.index + 1, count }));
  };
  const remove = async (at: At) => {
    if (!(await confirmDialog({ title: tr("Remove this block?"), message: tr("It is taken out of the email."), confirmLabel: tr("Remove"), danger: true }))) return;
    const length = design.rows[at.row].columns[at.column].blocks.length;
    const next = removeBlock(design, at);
    edit(next, length > 1 ? { ...at, index: Math.min(at.index, length - 2) } : null);
  };
  const removeRowAt = async (i: number) => {
    const holds = design.rows[i].columns.some((c) => c.blocks.length > 0);
    if (holds && !(await confirmDialog({ title: tr("Remove this row?"), message: tr("The blocks in it are taken out of the email."), confirmLabel: tr("Remove"), danger: true }))) return;
    edit(removeRow(design, i), null);
  };

  const onKey = (e: React.KeyboardEvent, at: At) => {
    if (!editable) return;
    if (e.altKey && e.key === "ArrowUp" && at.index > 0) (e.preventDefault(), move(at, { ...at, index: at.index - 1 }));
    else if (e.altKey && e.key === "ArrowDown" && at.index < design.rows[at.row].columns[at.column].blocks.length - 1) (e.preventDefault(), move(at, { ...at, index: at.index + 1 }));
    else if (e.key === "Enter" || e.key === " ") (e.preventDefault(), setSel(at));
    else if (e.key === "Delete") (e.preventDefault(), void remove(at));
  };

  return (
    <div className="cm-bd" ref={root}>
      {editable && (
        <div className="cm-bd-palette" role="group" aria-label={tr("Blocks")}>
          <span className="field-label">{tr("Blocks")}</span>
          {BLOCK_TYPES.map((type) => (
            <Button key={type} size="sm" icon="plus" onClick={() => add(type)}>
              {kindName(type)}
            </Button>
          ))}
        </div>
      )}
      <div className="cm-bd-main">
        <div className="cm-bd-canvas">
          {design.rows.map((row, r) => (
            <section key={r} className="cm-bd-row card" aria-label={tr("Row {n}", { n: r + 1 })} style={{ ["--cm-bd-cols" as string]: row.columns.length }}>
              <div className="cm-bd-rowbar">
                <strong>{tr("Row {n}", { n: r + 1 })}</strong>
                {editable && (
                  <Menu ariaLabel={tr("Row {n} options", { n: r + 1 })} icon="chevronDown" buttonClass="icon-btn">
                    {(close) => (
                      <>
                        {r > 0 && <MenuItem onSelect={() => (close(), edit(moveRow(design, r, r - 1), null, tr("Row moved up")))}>{tr("Move up")}</MenuItem>}
                        {r < design.rows.length - 1 && <MenuItem onSelect={() => (close(), edit(moveRow(design, r, r + 1), null, tr("Row moved down")))}>{tr("Move down")}</MenuItem>}
                        {[1, 2, 3].filter((n) => n !== row.columns.length).map((n) => (
                          <MenuItem key={n} onSelect={() => (close(), edit(setColumns(design, r, n), null))}>
                            {n === 1 ? tr("One column") : n === 2 ? tr("Two columns") : tr("Three columns")}
                          </MenuItem>
                        ))}
                        <MenuItem onSelect={() => (close(), edit(updateRow(design, r, { padding: row.padding >= 48 ? 0 : row.padding + 8 }), sel))}>{tr("Padding: {n} pixels, change", { n: row.padding })}</MenuItem>
                        <MenuItem danger onSelect={() => (close(), void removeRowAt(r))}>{tr("Remove the row")}</MenuItem>
                      </>
                    )}
                  </Menu>
                )}
              </div>
              <div className="cm-bd-cols">
                {row.columns.map((col, c) => (
                  <ul
                    key={c}
                    className="cm-bd-col"
                    aria-label={tr("Row {n}, column {m}", { n: r + 1, m: c + 1 })}
                    onDragOver={(e) => editable && e.preventDefault()}
                    onDrop={(e) => {
                      e.preventDefault();
                      if (drag.current) move(drag.current, { row: r, column: c, index: col.blocks.length });
                      drag.current = null;
                    }}
                  >
                    {col.blocks.map((b, i) => {
                      const at = { row: r, column: c, index: i };
                      return (
                        <li
                          key={i}
                          role="listitem"
                          tabIndex={0}
                          data-at={key(at)}
                          className="cm-bd-block"
                          data-selected={same(sel, at) || undefined}
                          data-flagged={flagged(at) || undefined}
                          aria-current={same(sel, at) || undefined}
                          aria-label={blockName(b)}
                          draggable={editable}
                          onDragStart={() => (drag.current = at)}
                          onDragOver={(e) => editable && e.preventDefault()}
                          onDrop={(e) => {
                            e.stopPropagation();
                            e.preventDefault();
                            if (drag.current) move(drag.current, at);
                            drag.current = null;
                          }}
                          onClick={() => setSel(at)}
                          onKeyDown={(e) => onKey(e, at)}
                        >
                          <span className="cm-bd-kind">{kindName(b.type)}</span>
                          <span className="cm-bd-text">{isolate(summary(b))}</span>
                          {editable && (
                            <Menu ariaLabel={tr("Options of {block}", { block: blockName(b) })} icon="chevronDown" buttonClass="icon-btn">
                              {(close) => (
                                <>
                                  {i > 0 && <MenuItem onSelect={() => (close(), move(at, { ...at, index: i - 1 }))}>{tr("Move up")}</MenuItem>}
                                  {i < col.blocks.length - 1 && <MenuItem onSelect={() => (close(), move(at, { ...at, index: i + 1 }))}>{tr("Move down")}</MenuItem>}
                                  <MenuItem danger onSelect={() => (close(), void remove(at))}>{tr("Remove")}</MenuItem>
                                </>
                              )}
                            </Menu>
                          )}
                        </li>
                      );
                    })}
                    {col.blocks.length === 0 && <li className="cm-bd-empty faint">{tr("Empty: add a block")}</li>}
                  </ul>
                ))}
              </div>
            </section>
          ))}
          {editable && design.rows.length < LIMITS.rows && (
            <div className="row">
              {[1, 2, 3].map((n) => (
                <Button key={n} size="sm" icon="plus" onClick={() => edit(addRow(design, design.rows.length, n), null)}>
                  {n === 1 ? tr("Add a row") : n === 2 ? tr("Add a row of two columns") : tr("Add a row of three columns")}
                </Button>
              ))}
            </div>
          )}
          {found.filter((p) => !p.at).map((p) => (
            <p key={p.message} className="field-error">{isolate(p.message)}</p>
          ))}
          {refused && <p className="field-error" role="alert">{isolate(refused)}</p>}
        </div>
        {editable && block && sel && (
          <aside className="cm-bd-panel card" aria-label={tr("The selected block")}>
            <h3 className="cm-bd-title">{kindName(block.type)}</h3>
            <Panel key={key(sel)} block={block} onPatch={(p) => onChange(updateBlock(design, sel, p))} />
            {found.filter((p) => p.at && same(p.at, sel)).map((p) => (
              <p key={p.message} className="field-error">{isolate(p.message)}</p>
            ))}
          </aside>
        )}
      </div>
    </div>
  );
}

function summary(b: Block): string {
  switch (b.type) {
    case "heading":
      return b.text;
    case "text":
      return richTextToPlain(b.html).slice(0, 120);
    case "button":
      return b.label;
    case "image":
      return b.alt || b.src;
    case "divider":
      return "";
    case "spacer":
      return `${b.height}px`;
  }
}
