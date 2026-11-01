import { describe, expect, it } from "vitest";
import {
  chunk,
  classify,
  delimiterOf,
  guessMapping,
  keyFor,
  mappingProblems,
  needAttention,
  parseCsv,
  pool,
  readContacts,
  readTable,
  reason,
  recordFor,
  splitAddresses,
  splitName,
  tally,
  toCsv,
  validAddress,
  type Mapping,
} from "./importer";

describe("reading a CSV file", () => {
  it("reads quoted fields with commas, quotes and line breaks, and ignores the closing line break", () => {
    expect(parseCsv('a,"b, c","say ""hi""","two\nlines"\r\nd,e,f,g\r\n')).toEqual([
      ["a", "b, c", 'say "hi"', "two\nlines"],
      ["d", "e", "f", "g"],
    ]);
    expect(parseCsv("")).toEqual([]);
    expect(parseCsv("a,,c")).toEqual([["a", "", "c"]]);
    expect(parseCsv("﻿email\nx@y.test")).toEqual([["email"], ["x@y.test"]]);
  });

  it("finds the delimiter the lines agree on, and ignores one inside quotes", () => {
    expect(delimiterOf("email;name\nx@y.test;Ana")).toBe(";");
    expect(delimiterOf("email\tname\nx@y.test\tAna")).toBe("\t");
    expect(delimiterOf('email,name\n"x@y.test","Rao, Ana"')).toBe(",");
    expect(delimiterOf("just one column")).toBe(",");
  });

  it("takes the first row for names and pads short rows, and skips empty ones", () => {
    const t = readTable("Email, First Name ,City\nx@y.test,Ana\n\n,,\nz@y.test,Raj,Pune\n");
    expect(t.headers).toEqual(["Email", "First Name", "City"]);
    expect(t.rows).toEqual([["x@y.test", "Ana", ""], ["z@y.test", "Raj", "Pune"]]);
    expect(t.lines).toEqual([2, 5]);
    expect(t.headerless).toBe(false);
  });

  it("knows a file with no header row, when its first row holds an address", () => {
    const t = readTable("x@y.test,Ana\nz@y.test,Raj\n");
    expect(t.headerless).toBe(true);
    expect(t.headers).toEqual(["Column 1", "Column 2"]);
    expect(t.rows.length).toBe(2);
    expect(t.lines).toEqual([1, 2]);
  });
});

describe("which column is which", () => {
  it("recognises the usual names and leaves the rest out until told", () => {
    expect(guessMapping(["E-mail Address", "First name", "Surname", "City", "Notes"]).map((t) => t.kind)).toEqual(["email", "first_name", "last_name", "skip", "skip"]);
    expect(guessMapping(["Name", "Email"]).map((t) => t.kind)).toEqual(["full_name", "email"]);
  });

  it("does not take a full name as well as a first and last name, nor one thing twice", () => {
    expect(guessMapping(["Name", "First", "Last", "Email"]).map((t) => t.kind)).toEqual(["skip", "first_name", "last_name", "email"]);
    expect(guessMapping(["Email", "Mail"]).map((t) => t.kind)).toEqual(["email", "skip"]);
  });

  it("finds the column of addresses when no header says so", () => {
    const rows = [["Ana", "ana@x.test"], ["Raj", "raj@x.test"], ["Mia", "none"]];
    expect(guessMapping(["Column 1", "Column 2"], rows).map((t) => t.kind)).toEqual(["skip", "email"]);
  });

  it("makes an attribute key out of a header, or nothing when it cannot", () => {
    expect(keyFor("Plan Type")).toBe("plan_type");
    expect(keyFor("  City (home) ")).toBe("city_home");
    expect(keyFor("2024 spend")).toBe("spend");
    expect(keyFor("###")).toBe("");
    expect(keyFor("x".repeat(60)).length).toBe(40);
  });

  it("says what is wrong with a mapping", () => {
    const h = ["A", "B", "C"];
    expect(mappingProblems([{ kind: "skip" }, { kind: "skip" }, { kind: "skip" }], h)).toEqual(["Choose which column holds the email address."]);
    expect(mappingProblems([{ kind: "email" }, { kind: "email" }, { kind: "skip" }], h)).toEqual(["“A” and “B” both go to email."]);
    expect(mappingProblems([{ kind: "email" }, { kind: "attribute", key: "city" }, { kind: "attribute", key: "city" }], h)[0]).toMatch(/both go to the attribute city/);
    expect(mappingProblems([{ kind: "email" }, { kind: "attribute", key: "Bad Key" }, { kind: "attribute", key: "consent_basis" }], h)).toHaveLength(2);
    expect(mappingProblems([{ kind: "email" }, { kind: "first_name" }, { kind: "attribute", key: "city" }], h)).toEqual([]);
  });
});

describe("contacts from rows", () => {
  const mapping: Mapping = [{ kind: "email" }, { kind: "full_name" }, { kind: "attribute", key: "city" }];
  const table = {
    rows: [[" ANA@X.test ", "Ana Maria Rao", "Pune"], ["bad address", "Raj", ""], ["", "No Mail", ""], ["raj@x.test", "Raj", ""]],
    lines: [2, 3, 4, 5],
  };

  it("lowercases the address, splits a full name and keeps the attributes that have values", () => {
    const [ana, , , raj] = readContacts(table, mapping);
    expect(ana.contact).toEqual({ email: "ana@x.test", first_name: "Ana", last_name: "Maria Rao", attributes: { city: "Pune" } });
    expect(raj.contact).toEqual({ email: "raj@x.test", first_name: "Raj", attributes: {} });
  });

  it("says why a row cannot be a contact", () => {
    const read = readContacts(table, mapping);
    expect([read[1], read[2]].map((r) => [r.line, r.contact, r.problem])).toEqual([
      [3, null, "Not a valid email address"],
      [4, null, "No email address"],
    ]);
    const long = readContacts({ rows: [["a@x.test", "x".repeat(101), ""]], lines: [2] }, [{ kind: "email" }, { kind: "first_name" }, { kind: "skip" }]);
    expect(long[0].problem).toMatch(/First name is longer than 100/);
  });

  it("uses a first and last name column over a full name", () => {
    const r = readContacts({ rows: [["a@x.test", "Ana", "Rao"]], lines: [2] }, [{ kind: "email" }, { kind: "first_name" }, { kind: "last_name" }]);
    expect(r[0].contact).toMatchObject({ first_name: "Ana", last_name: "Rao" });
  });

  it("splits names the way the Mail app does", () => {
    expect(splitName("  Ana   Rao ")).toEqual({ first_name: "Ana", last_name: "Rao" });
    expect(splitName("Ana")).toEqual({ first_name: "Ana" });
    expect(splitName("")).toEqual({});
  });

  it("accepts the addresses the server accepts", () => {
    expect(["a@b.co", "first.last+tag@sub.example.org"].every(validAddress)).toBe(true);
    expect(["a@b", "a b@c.co", "@c.co", "a@@c.co", `${"x".repeat(65)}@c.co`].some(validAddress)).toBe(false);
  });
});

describe("who is new", () => {
  const read = readContacts(
    { rows: [["a@x.test"], ["b@x.test"], ["A@x.test"], ["c@x.test"], ["d@x.test"], ["nope"]], lines: [2, 3, 4, 5, 6, 7] },
    [{ kind: "email" }],
  );

  it("sorts rows into new, known, suppressed, repeated and invalid, the first of a repeat staying", () => {
    const checked = classify(read, new Set(["b@x.test"]), new Set(["c@x.test"]), (line) => (line === 7 ? "nope" : ""));
    expect(checked.map((c) => [c.line, c.fate])).toEqual([[2, "new"], [3, "known"], [4, "duplicate"], [5, "suppressed"], [6, "new"], [7, "invalid"]]);
    expect(checked[2].why).toBe("Same address as line 2");
    expect(tally(checked)).toEqual({ new: 2, known: 1, suppressed: 1, duplicate: 1, invalid: 1 });
    expect(needAttention(checked, 2).map((c) => c.line)).toEqual([3, 4]);
  });
});

describe("what is sent", () => {
  const ana = { email: "ana@x.test", first_name: "Ana", attributes: { city: "Pune" } };

  it("makes people who agreed Subscribed, with the basis saved on them", () => {
    expect(recordFor(ana, { mode: "agreed", basis: " Signed up at the 2026 fair " })).toEqual({
      email: "ana@x.test",
      first_name: "Ana",
      attributes: { city: "Pune", consent_basis: "Signed up at the 2026 fair" },
      source: "Import",
      consent_status: "Subscribed",
    });
  });

  it("makes an opt-out list Unsubscribed, without a basis", () => {
    const r = recordFor(ana, { mode: "optout" });
    expect(r).toMatchObject({ consent_status: "Unsubscribed", source: "Import", attributes: { city: "Pune" } });
    expect(JSON.stringify(r)).not.toContain("consent_basis");
  });

  it("goes in chunks", () => {
    expect(chunk([1, 2, 3, 4, 5], 2)).toEqual([[1, 2], [3, 4], [5]]);
    expect(chunk([], 3)).toEqual([]);
  });

  it("says why the server refused a row", () => {
    expect(reason({ row: 0, error: "validation_failed", fields: [{ field: "email", message: "enter a valid email address" }] })).toBe("enter a valid email address");
    expect(reason({ row: 1, error: "conflict", message: "email already exists" })).toBe("Already a contact");
    expect(reason({ row: 2, error: "forbidden", message: "You do not have permission to do that." })).toBe("You do not have permission to do that.");
  });

  it("writes rejected rows back as a CSV", () => {
    expect(toCsv(["Email", "Why"], [["a@x.test", 'Said "no", twice']])).toBe('Email,Why\r\na@x.test,"Said ""no"", twice"\r\n');
  });
});

describe("addresses typed in a box", () => {
  it("are split on commas, semicolons, spaces and lines, once each, the wrong ones apart", () => {
    expect(splitAddresses("ana@x.test, RAJ@x.test;\nana@x.test  mia@x.test oops")).toEqual({ valid: ["ana@x.test", "raj@x.test", "mia@x.test"], invalid: ["oops"] });
    expect(splitAddresses("  ")).toEqual({ valid: [], invalid: [] });
  });
});

describe("doing a few things at a time", () => {
  it("keeps the order of the results and never runs more than asked at once", async () => {
    let live = 0;
    let most = 0;
    const out = await pool([5, 1, 3, 2, 4], 2, async (n) => {
      live++;
      most = Math.max(most, live);
      await new Promise((r) => setTimeout(r, n));
      live--;
      return n * 10;
    });
    expect(out).toEqual([50, 10, 30, 20, 40]);
    expect(most).toBe(2);
  });

  it("stops starting new ones when told to", async () => {
    const seen: number[] = [];
    const out = await pool([1, 2, 3, 4], 1, async (n) => (seen.push(n), n), () => seen.length >= 2);
    expect(seen).toEqual([1, 2]);
    expect(out).toEqual([1, 2, undefined, undefined]);
    expect(await pool([], 3, async (n: number) => n)).toEqual([]);
  });
});
