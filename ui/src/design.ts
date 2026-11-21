// An email's design as the server takes it (campaigns-5): rows of columns of blocks. Pure: every function returns a new
// tree and never changes its input; an index out of range returns the tree unchanged. `problems` checks the same rules
// the server does, so a person is told before the save is refused. The colours are the email's own data, not interface.

import { t } from "@ferrum/client";

export type Align = "left" | "center" | "right";

export type Block =
  | { type: "heading"; text: string; level: 1 | 2 | 3; align: Align; color: string | null }
  | { type: "text"; html: string; align: Align }
  | { type: "button"; label: string; href: string; background: string | null; color: string | null; align: Align }
  | { type: "image"; src: string; alt: string; href: string | null; width: number | null }
  | { type: "divider"; color: string | null }
  | { type: "spacer"; height: number };

export type BlockType = Block["type"];
export const BLOCK_TYPES: BlockType[] = ["heading", "text", "button", "image", "divider", "spacer"];

export interface Column {
  blocks: Block[];
}
export interface Row {
  background: string | null;
  padding: number;
  columns: Column[];
}
export interface Page {
  width: number;
  background: string;
  content_background: string;
  font: "sans" | "serif" | "mono";
  text_color: string;
  link_color: string;
}
export interface Design {
  version: 1;
  page: Page;
  rows: Row[];
}

/** A place in the tree: a row, a column of it, and a position in that column. */
export interface At {
  row: number;
  column: number;
  index: number;
}

export const LIMITS = { rows: 50, columns: 3, blocks: 20, text: 20000, heading: 500, label: 200, alt: 500, bytes: 256 * 1024 };

const HEX = /^#[0-9a-fA-F]{6}$/;
const TEMPLATE = /^\{\{ ?[A-Za-z_][A-Za-z0-9_.]* ?\}\}$/;
const BAD_CHARS = /[\s"'<>`\\]/;

export const NEUTRAL = "#000000"; // ui-lint-allow raw-color: the email's own colours are data

export const blankPage = (): Page => ({ width: 600, background: "#f3f4f6", content_background: "#ffffff", font: "sans", text_color: "#374151", link_color: "#2f5bea" }); // ui-lint-allow raw-color: the email's own colours are data

export function blankBlock(type: BlockType): Block {
  switch (type) {
    case "heading":
      return { type, text: t("A clear headline"), level: 1, align: "left", color: null };
    case "text":
      return { type, html: `<p>${t("Say something worth reading.")}</p>`, align: "left" };
    case "button":
      return { type, label: t("Read more"), href: "https://example.com", background: null, color: null, align: "left" };
    case "image":
      return { type, src: "https://", alt: "", href: null, width: null };
    case "divider":
      return { type, color: null };
    case "spacer":
      return { type, height: 24 };
  }
}

export const blankRow = (columns = 1): Row => ({ background: null, padding: 24, columns: Array.from({ length: Math.min(LIMITS.columns, Math.max(1, columns)) }, () => ({ blocks: [] })) });

/** The look of `snippets.ts`'s starter: a greeting, a sentence, a button. */
export function starterDesign(): Design {
  const row = blankRow(1);
  row.columns[0].blocks = [
    { type: "heading", text: t("Hello {name},", { name: '{{ contact.first_name or "there" }}' }), level: 1, align: "left", color: null },
    { type: "text", html: `<p>${t("Tell people what is new, in a sentence or two.")}</p>`, align: "left" },
    blankBlock("button"),
  ];
  return { version: 1, page: blankPage(), rows: [row] };
}

const inRange = (tree: Design, at: At): boolean => !!tree.rows[at.row]?.columns[at.column] && at.index >= 0;
const withRows = (tree: Design, rows: Row[]): Design => ({ ...tree, rows });
const mapColumn = (tree: Design, row: number, column: number, f: (blocks: Block[]) => Block[]): Design =>
  withRows(tree, tree.rows.map((r, i) => (i !== row ? r : { ...r, columns: r.columns.map((c, j) => (j !== column ? c : { blocks: f(c.blocks) })) })));

export function addBlock(tree: Design, at: At, block: Block): Design {
  if (!inRange(tree, at)) return tree;
  const here = tree.rows[at.row].columns[at.column].blocks;
  if (here.length >= LIMITS.blocks) return tree;
  const index = Math.min(at.index, here.length);
  return mapColumn(tree, at.row, at.column, (b) => [...b.slice(0, index), block, ...b.slice(index)]);
}

export function removeBlock(tree: Design, at: At): Design {
  if (!inRange(tree, at) || at.index >= tree.rows[at.row].columns[at.column].blocks.length) return tree;
  return mapColumn(tree, at.row, at.column, (b) => b.filter((_, i) => i !== at.index));
}

export function updateBlock(tree: Design, at: At, patch: Partial<Block>): Design {
  if (!inRange(tree, at) || at.index >= tree.rows[at.row].columns[at.column].blocks.length) return tree;
  return mapColumn(tree, at.row, at.column, (b) => b.map((x, i) => (i === at.index ? ({ ...x, ...patch } as Block) : x)));
}

/** Moves a block within its column or into another; `to.index` is where it stands afterwards. */
export function moveBlock(tree: Design, from: At, to: At): Design {
  if (!inRange(tree, from) || !inRange(tree, to)) return tree;
  const block = tree.rows[from.row].columns[from.column].blocks[from.index];
  if (!block) return tree;
  const sameColumn = from.row === to.row && from.column === to.column;
  const target = tree.rows[to.row].columns[to.column].blocks;
  if (!sameColumn && target.length >= LIMITS.blocks) return tree;
  const removed = removeBlock(tree, from);
  const length = removed.rows[to.row].columns[to.column].blocks.length;
  return addBlock(removed, { ...to, index: Math.min(to.index, length) }, block);
}

export function addRow(tree: Design, index: number, columns: number): Design {
  if (tree.rows.length >= LIMITS.rows || index < 0) return tree;
  const at = Math.min(index, tree.rows.length);
  return withRows(tree, [...tree.rows.slice(0, at), blankRow(columns), ...tree.rows.slice(at)]);
}

export function moveRow(tree: Design, from: number, to: number): Design {
  if (!tree.rows[from] || to < 0 || to >= tree.rows.length || from === to) return tree;
  const rows = tree.rows.filter((_, i) => i !== from);
  rows.splice(to, 0, tree.rows[from]);
  return withRows(tree, rows);
}

export function removeRow(tree: Design, index: number): Design {
  if (!tree.rows[index]) return tree;
  return withRows(tree, tree.rows.filter((_, i) => i !== index));
}

export function updateRow(tree: Design, index: number, patch: Partial<Pick<Row, "background" | "padding">>): Design {
  if (!tree.rows[index]) return tree;
  return withRows(tree, tree.rows.map((r, i) => (i === index ? { ...r, ...patch } : r)));
}

/** A row with `n` columns (1 to 3): the blocks of columns that go are added to the last one kept. */
export function setColumns(tree: Design, row: number, n: number): Design {
  const r = tree.rows[row];
  if (!r || n < 1 || n > LIMITS.columns || n === r.columns.length) return tree;
  const kept = r.columns.slice(0, n).map((c) => ({ blocks: [...c.blocks] }));
  const dropped = r.columns.slice(n).flatMap((c) => c.blocks);
  while (kept.length < n) kept.push({ blocks: [] });
  kept[kept.length - 1].blocks.push(...dropped);
  return withRows(tree, tree.rows.map((x, i) => (i === row ? { ...x, columns: kept } : x)));
}

export const blocksIn = (tree: Design): number => tree.rows.reduce((n, r) => n + r.columns.reduce((m, c) => m + c.blocks.length, 0), 0);

export interface Problem {
  at: { row: number; column: number; index: number } | null;
  field: string;
  message: string;
}

const address = (h: string): boolean => h.length <= 2000 && (TEMPLATE.test(h) || (/^(https?:\/\/|mailto:|tel:)/.test(h) && !BAD_CHARS.test(h)));

/** The server's rules (campaigns-5), so a person is told before a save is refused. */
export function problems(tree: Design): Problem[] {
  const out: Problem[] = [];
  const bad = (at: Problem["at"], field: string, message: string) => out.push({ at, field, message });
  if (tree.rows.length === 0 || tree.rows.length > LIMITS.rows) bad(null, "rows", t("A design has 1 to {max} rows.", { max: LIMITS.rows }));
  if (blocksIn(tree) === 0) bad(null, "rows", t("Add at least one block."));
  tree.rows.forEach((r, row) => {
    if (r.columns.length < 1 || r.columns.length > LIMITS.columns) bad({ row, column: 0, index: 0 }, "columns", t("A row has 1 to {max} columns.", { max: LIMITS.columns }));
    r.columns.forEach((c, column) => {
      if (c.blocks.length > LIMITS.blocks) bad({ row, column, index: 0 }, "blocks", t("A column holds at most {max} blocks.", { max: LIMITS.blocks }));
      c.blocks.forEach((b, index) => {
        const at = { row, column, index };
        switch (b.type) {
          case "heading":
            if (!b.text.trim() || b.text.length > LIMITS.heading) bad(at, "text", t("A heading has 1 to {max} characters.", { max: LIMITS.heading }));
            break;
          case "text":
            if (b.html.length > LIMITS.text) bad(at, "html", t("A text block holds at most {max} characters.", { max: LIMITS.text }));
            if (/<\/?mj-/i.test(b.html)) bad(at, "html", t("A text block cannot hold MJML."));
            break;
          case "button":
            if (!b.label.trim() || b.label.length > LIMITS.label) bad(at, "label", t("A button's label has 1 to {max} characters.", { max: LIMITS.label }));
            if (!address(b.href)) bad(at, "href", t("The address must be https://, http://, mailto:, tel: or one template value."));
            break;
          case "image":
            if (!b.src.startsWith("https://") || b.src.length <= 8 || b.src.length > 2000 || BAD_CHARS.test(b.src)) bad(at, "src", t("The picture's address must start with https:// and have no spaces or quotes."));
            if (b.alt.length > LIMITS.alt) bad(at, "alt", t("The alternative text holds at most {max} characters.", { max: LIMITS.alt }));
            if (b.href && !address(b.href)) bad(at, "href", t("The picture's link must be https://, http://, mailto:, tel: or one template value."));
            if (b.width != null && (b.width < 16 || b.width > tree.page.width)) bad(at, "width", t("The picture's width is 16 up to the page's width."));
            break;
          case "spacer":
            if (b.height < 4 || b.height > 120) bad(at, "height", t("A spacer's height is 4 to 120."));
            break;
          case "divider":
            break;
        }
        for (const colour of ["color", "background"] as const) {
          const v = (b as unknown as Record<string, unknown>)[colour];
          if (typeof v === "string" && !HEX.test(v)) bad(at, colour, t("A colour looks like #2f5bea."));
        }
      });
    });
  });
  if (JSON.stringify(tree).length > LIMITS.bytes) bad(null, "rows", t("The design is limited to 256 KB."));
  return out;
}

/** The block a server message names ("row 1, column 2, block 3: …"), if it names one. */
export function blockNamed(message: string): At | null {
  const m = /row (\d+), column (\d+), block (\d+)/.exec(message);
  return m ? { row: Number(m[1]) - 1, column: Number(m[2]) - 1, index: Number(m[3]) - 1 } : null;
}
