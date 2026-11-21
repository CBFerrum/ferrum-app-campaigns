import { describe, expect, it } from "vitest";
import { addBlock, addRow, blankBlock, blockNamed, blankRow, moveBlock, moveRow, problems, removeBlock, removeRow, setColumns, starterDesign, updateBlock, type Design } from "./design";

const two = (): Design => {
  const d = starterDesign();
  d.rows[0] = blankRow(2);
  d.rows[0].columns[0].blocks = [blankBlock("heading"), blankBlock("text"), blankBlock("divider")];
  d.rows[0].columns[1].blocks = [blankBlock("spacer")];
  return d;
};
const kinds = (d: Design, row = 0, column = 0) => d.rows[row].columns[column].blocks.map((b) => b.type);

describe("the design tree", () => {
  it("adds a block after the selected one", () => {
    const d = addBlock(two(), { row: 0, column: 0, index: 1 }, blankBlock("button"));
    expect(kinds(d)).toEqual(["heading", "button", "text", "divider"]);
  });

  it("moves a block within and across columns", () => {
    const within = moveBlock(two(), { row: 0, column: 0, index: 0 }, { row: 0, column: 0, index: 2 });
    expect(kinds(within)).toEqual(["text", "divider", "heading"]);
    const across = moveBlock(two(), { row: 0, column: 0, index: 1 }, { row: 0, column: 1, index: 0 });
    expect(kinds(across, 0, 0)).toEqual(["heading", "divider"]);
    expect(kinds(across, 0, 1)).toEqual(["text", "spacer"]);
  });

  it("removing a column keeps its blocks", () => {
    const d = setColumns(two(), 0, 1);
    expect(d.rows[0].columns).toHaveLength(1);
    expect(kinds(d)).toEqual(["heading", "text", "divider", "spacer"]);
    expect(setColumns(d, 0, 3).rows[0].columns).toHaveLength(3);
  });

  it("an index out of range changes nothing", () => {
    const d = two();
    expect(removeBlock(d, { row: 9, column: 0, index: 0 })).toBe(d);
    expect(removeBlock(d, { row: 0, column: 0, index: 9 })).toBe(d);
    expect(moveBlock(d, { row: 0, column: 0, index: 9 }, { row: 0, column: 0, index: 0 })).toBe(d);
    expect(updateBlock(d, { row: 0, column: 7, index: 0 }, { type: "divider" } as never)).toBe(d);
    expect(moveRow(d, 0, 5)).toBe(d);
    expect(removeRow(d, 3)).toBe(d);
    expect(addBlock(d, { row: 5, column: 0, index: 0 }, blankBlock("text"))).toBe(d);
  });

  it("functions never change their input", () => {
    const d = two();
    const before = JSON.stringify(d);
    addBlock(d, { row: 0, column: 0, index: 0 }, blankBlock("text"));
    moveBlock(d, { row: 0, column: 0, index: 0 }, { row: 0, column: 1, index: 0 });
    removeBlock(d, { row: 0, column: 0, index: 0 });
    setColumns(d, 0, 1);
    addRow(d, 0, 3);
    moveRow(addRow(d, 1, 1), 0, 1);
    removeRow(d, 0);
    expect(JSON.stringify(d)).toBe(before);
  });

  it("a column holds at most 20 blocks", () => {
    let d = two();
    for (let i = 0; i < 30; i++) d = addBlock(d, { row: 0, column: 1, index: 0 }, blankBlock("spacer"));
    expect(kinds(d, 0, 1)).toHaveLength(20);
  });
});

describe("problems", () => {
  const withBlock = (b: Parameters<typeof updateBlock>[2]) => updateBlock(starterDesign(), { row: 0, column: 0, index: 2 }, b);

  it("match the server's rules", () => {
    expect(problems(withBlock({ label: "" } as never)).map((p) => p.field)).toEqual(["label"]);
    expect(problems(withBlock({ href: "javascript:alert(1)" } as never)).map((p) => p.field)).toEqual(["href"]);
    expect(problems(withBlock({ href: "{{ unsubscribe_url }}" } as never))).toEqual([]);
    const image = updateBlock(addBlock(starterDesign(), { row: 0, column: 0, index: 3 }, blankBlock("image")), { row: 0, column: 0, index: 3 }, { src: "http://a.test/a.png" } as never);
    expect(problems(image).map((p) => p.field)).toEqual(["src"]);
    let many = starterDesign();
    for (let i = 0; i < 25; i++) many = { ...many, rows: [{ ...many.rows[0], columns: [{ blocks: [...many.rows[0].columns[0].blocks, blankBlock("spacer")] }] }] };
    expect(problems(many).some((p) => p.field === "blocks")).toBe(true);
    const long = updateBlock(starterDesign(), { row: 0, column: 0, index: 1 }, { html: "x".repeat(20001) } as never);
    expect(problems(long).map((p) => p.field)).toEqual(["html"]);
    expect(problems(withBlock({ background: "red" } as never)).map((p) => p.field)).toEqual(["background"]);
  });

  it("the starter design has no problems", () => {
    expect(problems(starterDesign())).toEqual([]);
  });

  it("an empty design is refused", () => {
    const d = starterDesign();
    d.rows[0].columns[0].blocks = [];
    expect(problems(d).map((p) => p.message)).toContain("Add at least one block.");
  });

  it("finds the block a server message names", () => {
    expect(blockNamed("row 2, column 1, block 3: the address must be https://")).toEqual({ row: 1, column: 0, index: 2 });
    expect(blockNamed("the design is limited to 256 KB")).toBeNull();
  });
});
