import { describe, expect, it } from "vitest";
import { canSend, changedSince, findings, imagesWithoutAlt, leftOut, linksIn, sendTime, type AudienceCheck } from "./review";

const check = (over: Partial<AudienceCheck> = {}): AudienceCheck => ({
  checked_at: "2026-11-01T10:00:00.000Z",
  contacts: 120,
  mailable: 100,
  left_out: { pending: 2, unsubscribed: 10, bounced: 3, complained: 0, suppressed: 5 },
  excluded: 7,
  new_contacts: 0,
  rows: [],
  split: null,
  rate: { per_hour: 500, workspace: 500, own: 0 },
  ready: { public_url: true, account: null, postal_address: true },
  emails: [{ variant: null, error: null, bytes: 4000, links: ["https://shop.test/sale"], unsubscribe: "footer" }],
  ...over,
});

const content = { subject: "Spring sale", content: "<p>Hi</p>", track_clicks: true, ab_test: false, variants: [] as { subject?: string; content?: string | null }[] };

describe("who is left out, and why", () => {
  it("lists each reason that has people in it, the opt-outs first", () => {
    expect(leftOut(check()).map((l) => [l.key, l.count])).toEqual([
      ["unsubscribed", 10],
      ["bounced", 3],
      ["suppressed", 5],
      ["pending", 2],
      ["excluded", 7],
    ]);
    expect(leftOut(check({ left_out: { pending: 0, unsubscribed: 0, bounced: 0, complained: 0, suppressed: 0 }, excluded: 0 }))).toEqual([]);
  });
});

describe("what would stop a send, and what is worth a look", () => {
  const level = (f: ReturnType<typeof findings>, id: string) => f.find((x) => x.id === id)?.level;

  it("is clear to send when everything is in place", () => {
    const f = findings(check(), content);
    expect(canSend(f)).toBe(true);
    expect(level(f, "unsub")).toBe("ok");
    expect(f.find((x) => x.id === "rate")?.title).toBe("Goes out at up to 500 emails an hour: under an hour");
  });

  it("is stopped by an audience nobody is in, whatever else is right", () => {
    expect(level(findings(check({ mailable: 0 }), content), "nobody")).toBe("block");
    expect(findings(check({ mailable: 0, contacts: 0 }), content).find((x) => x.id === "nobody")?.detail).toMatch(/empty/);
    expect(canSend(findings(check({ mailable: 0 }), content))).toBe(false);
  });

  it("is stopped by what the server would refuse", () => {
    const f = findings(
      check({
        ready: { public_url: false, account: "Support does not send mail", postal_address: false },
        emails: [{ variant: "B", error: "template: unexpected end", bytes: null, links: [], unsubscribe: "footer" }],
      }),
      { ...content, subject: "  " },
    );
    expect(["public_url", "account", "render-B", "subject"].map((id) => level(f, id))).toEqual(["block", "block", "block", "block"]);
    expect(level(f, "postal")).toBe("warn");
    expect(f.find((x) => x.id === "render-B")?.title).toBe("Version B cannot be built");
    expect(canSend(f)).toBe(false);
  });

  it("warns, without stopping, about a long subject, bare pictures and a big email", () => {
    const f = findings(check({ emails: [{ variant: null, error: null, bytes: 150_000, links: [], unsubscribe: "own" }] }), {
      ...content,
      subject: "x".repeat(80),
      content: '<img src="a.png"><img src="b.png" alt=""><img src="c.png" alt="Logo">',
    });
    expect(["subject-long", "alt", "size"].map((id) => level(f, id))).toEqual(["warn", "warn", "warn"]);
    expect(f.find((x) => x.id === "alt")?.title).toBe("1 picture without alternative text");
    expect(canSend(f)).toBe(true);
  });

  it("explains an A/B split and how long the send takes", () => {
    const f = findings(check({ split: { variants: 2, test: 20, rest: 80 }, rate: { per_hour: 4, workspace: 500, own: 4 } }), {
      ...content,
      ab_test: true,
      winner_metric: "Clicks",
      winner_after_hours: 1,
    });
    expect(f.find((x) => x.id === "split")?.detail).toBe("The version with the most clicks goes to the other 80 people after 1 hour.");
    expect(f.find((x) => x.id === "rate")?.detail).toBe("This campaign's own limit applies.");
    expect(sendTime(100, 4)).toBe("about 25 hours");
  });
});

describe("what is in the content", () => {
  it("counts pictures that have no alt attribute at all", () => {
    expect(imagesWithoutAlt('<img src="a"> <IMG SRC="b" ALT="x"> <img alt="" src="c">')).toBe(1);
    expect(imagesWithoutAlt("<p>none</p>")).toBe(0);
  });

  it("lists links once, saying which are tracked and why the others are not", () => {
    const html = [
      '<a href="https://shop.test/sale?a=1&amp;b=2">Sale</a>',
      "<a href='https://shop.test/about'>About</a>",
      '<a href="https://shop.test/about">About again</a>',
      '<a href="{{ contact.attributes.url }}">Yours</a>',
      '<a href="mailto:hi@shop.test">Write</a>',
      '<a href="#top">Top</a>',
      '<a href="ftp://files.test/x">Files</a>',
    ].join("\n");
    const links = linksIn(html, ["https://shop.test/sale?a=1&b=2"], true);
    expect(links.map((l) => [l.url, l.tracked, l.why])).toEqual([
      ["https://shop.test/sale?a=1&b=2", true, undefined],
      ["https://shop.test/about", false, "past the limit of 100 tracked links"],
      ["{{ contact.attributes.url }}", false, "it changes for each person"],
      ["mailto:hi@shop.test", false, "an email address"],
      ["#top", false, "a link within the email"],
      ["ftp://files.test/x", false, "not a web address"],
    ]);
    expect(linksIn('<a href="https://shop.test/sale">x</a>', [], false)[0].why).toBe("click tracking is off");
  });
});

describe("a count that moved", () => {
  it("says so, and says nothing when it did not", () => {
    expect(changedSince(100, 100)).toBeNull();
    expect(changedSince(100, 92)).toBe("Down 8 since you last looked (100 before).");
    expect(changedSince(1000, 1500)).toBe(`Up 500 since you last looked (${(1000).toLocaleString()} before).`);
  });
});
