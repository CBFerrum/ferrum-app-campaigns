import { describe, expect, it } from "vitest";
import { clip, preheaderOf, previewDoc } from "./preview";
import { attributeTag, BLOCKS, insertAt, MERGE_TAGS, starter, UNSUBSCRIBE, validKey } from "./snippets";

describe("a preview", () => {
  it("is a light page that loads pictures and styles only, whose links open elsewhere, with the email after it", () => {
    const doc = previewDoc("<html><body><p>Hi</p></body></html>");
    expect(doc.startsWith("<!doctype html>")).toBe(true);
    expect(doc).toContain('<meta name="color-scheme" content="light">');
    expect(doc).toContain('<base target="_blank">');
    expect(doc).toContain("default-src 'none'");
    expect(doc).not.toContain("script-src");
    expect(doc.endsWith("<p>Hi</p></body></html>")).toBe(true);
  });

  it("reads back the preview text the sender hid in the email", () => {
    const hidden =
      '<div style="display:none;font-size:1px;line-height:1px;max-height:0;max-width:0;opacity:0;overflow:hidden;mso-hide:all">Up to 50% off &amp; free delivery' +
      "&#847;&zwnj;&nbsp;".repeat(30) +
      "</div>";
    expect(preheaderOf(`<body>${hidden}<p>Hi</p>`)).toBe("Up to 50% off & free delivery");
    expect(preheaderOf("<body><p>No preheader</p></body>")).toBe("");
  });

  it("cuts a line the way an inbox does", () => {
    expect(clip("Short", 10)).toBe("Short");
    expect(clip("A subject that is far too long to fit", 12)).toBe("A subject t…");
    expect(clip("  padded  ", 20)).toBe("padded");
  });
});

describe("what a person drops into an email", () => {
  it("replaces the selection and leaves the caret after what went in", () => {
    expect(insertAt("Hi , welcome", 3, 3, "{{ x }}")).toEqual({ text: "Hi {{ x }}, welcome", caret: 10 });
    expect(insertAt("Hi NAME!", 3, 7, "Ana")).toEqual({ text: "Hi Ana!", caret: 6 });
    expect(insertAt("abc", 99, 120, "!")).toEqual({ text: "abc!", caret: 4 });
    expect(insertAt("abc", 2, 1, "!")).toEqual({ text: "ab!c", caret: 3 });
  });

  it("offers only attribute keys the server accepts", () => {
    expect(["city", "plan_2", "a"].map(validKey)).toEqual([true, true, true]);
    expect(["City", "2fa", "has space", "", "x".repeat(41)].map(validKey)).toEqual([false, false, false, false, false]);
    expect(attributeTag("city")).toBe("{{ contact.attributes.city }}");
  });

  it("keeps blocks whole, with no script and no template code but the person's name", () => {
    for (const b of BLOCKS) {
      expect(b.code).not.toMatch(/<script/i);
      expect((b.code.match(/<(table|a|p|h1)\b/g) ?? []).length).toBe((b.code.match(/<\/(table|a|p|h1)>/g) ?? []).length);
    }
    expect(MERGE_TAGS.every((t) => t.code.startsWith("{{") && t.code.endsWith("}}"))).toBe(true);
    expect(UNSUBSCRIBE.code).toContain("{{ unsubscribe_url }}");
  });

  it("starts an email from a whole page the sender can build", () => {
    expect(starter()).toContain("<!doctype html>");
    expect(starter()).toContain("{{ contact.first_name");
    expect(starter()).toContain("Read more");
    expect(starter()).not.toContain("unsubscribe_url");
  });
});
