import { describe, expect, it } from "vitest";
import { abBars, abStanding, hasAny, host, linkShares, shape, tiles, variantRates, type Perf, type Timeline } from "./results";

const perf: Perf = {
  recipients: 120,
  sent: 100,
  delivered: 96,
  failed: 5,
  skipped: 15,
  unique_opens: 48,
  total_opens: 70,
  machine_opens: 9,
  unique_clicks: 12,
  total_clicks: 15,
  unsubscribes: 2,
  bounces: 4,
  hard_bounces: 1,
  soft_bounces: 3,
  complaints: 0,
};

describe("the headline numbers", () => {
  const byId = Object.fromEntries(tiles(perf).map((t) => [t.id, t]));

  it("each carry the share they are a share of", () => {
    expect([byId.sent.value, byId.sent.sub]).toEqual(["100", "of 120 recipients"]);
    expect(byId.sent.note).toBe("5 failed, 15 skipped");
    expect([byId.delivered.value, byId.delivered.sub]).toEqual(["96", "96% of sent"]);
    expect([byId.opened.value, byId.opened.sub]).toEqual(["48", "50% of delivered"]);
    expect([byId.clicked.value, byId.clicked.sub, byId.clicked.note]).toEqual(["12", "12.5% of delivered", "25% of those who opened"]);
    expect([byId.bounced.sub, byId.bounced.note]).toEqual(["1 hard, 3 soft", "4% of sent"]);
  });

  it("keep automatic opens apart and say they are not counted", () => {
    expect(byId.opened.note).toBe("9 more were automatic (privacy proxies, scanners) and are not counted");
  });

  it("say nothing odd about a send that has not begun", () => {
    const none = Object.fromEntries(
      tiles({ ...perf, recipients: 0, sent: 0, delivered: 0, failed: 0, skipped: 0, unique_opens: 0, machine_opens: 0, unique_clicks: 0, bounces: 0, hard_bounces: 0, soft_bounces: 0, unsubscribes: 0 }).map((t) => [t.id, t]),
    );
    expect([none.sent.note, none.opened.note, none.clicked.note, none.bounced.note]).toEqual([undefined, undefined, undefined, undefined]);
    expect(none.opened.sub).toBe("– of delivered");
  });
});

describe("a series", () => {
  it("is per bucket or a running total", () => {
    expect(shape([1, 0, 2], false)).toEqual([1, 0, 2]);
    expect(shape([1, 0, 2], true)).toEqual([1, 1, 3]);
    expect(hasAny([0, 0])).toBe(false);
    expect(hasAny([0, 3])).toBe(true);
  });

  it("gives each version's rate as the send went on", () => {
    const tl = {
      variants: { B: { sent: [10, 10, 0], opens: [1, 3, 0], clicks: [0, 0, 0] }, A: { sent: [10, 0, 0], opens: [0, 1, 1], clicks: [0, 0, 0] } },
    } as unknown as Timeline;
    expect(variantRates(tl, "opens")).toEqual([
      { variant: "A", values: [0, 10, 20] },
      { variant: "B", values: [10, 20, 20] },
    ]);
  });
});

describe("an A/B test", () => {
  const rows = [
    { variant: "A", subject: "One", recipients: 5, delivered: 5, unique_opens: 1, unique_clicks: 0, winner: false },
    { variant: "B", subject: "Two", recipients: 15, delivered: 15, unique_opens: 3, unique_clicks: 3, winner: true },
  ];

  it("compares versions by the metric it is decided by", () => {
    expect(abBars(rows, "Opens").map((b) => [b.variant, b.rate, b.winner])).toEqual([["A", 20, false], ["B", 20, true]]);
    expect(abBars(rows, "Clicks").map((b) => b.rate)).toEqual([0, 20]);
    expect(abBars([{ ...rows[0], delivered: 0 }], "Opens")[0].rate).toBeNull();
  });

  it("says where it stands", () => {
    const now = new Date("2026-11-01T12:00:00Z").getTime();
    expect(abStanding({ winner_variant: "B" }, now)).toBe("Version B won and went to everyone else.");
    expect(abStanding({}, now)).toMatch(/^Waiting for every test email/);
    expect(abStanding({ test_sent_at: "2026-11-01T10:00:00Z", winner_after_hours: 5 }, now)).toBe("The winner is chosen in about 3 hours.");
    expect(abStanding({ test_sent_at: "2026-11-01T01:00:00Z", winner_after_hours: 4 }, now)).toMatch(/next round/);
  });
});

describe("links", () => {
  it("show a link's share of those who got the email, and its bar against the busiest", () => {
    const shares = linkShares(
      [
        { link_no: 1, url: "https://shop.test/sale", unique_clicks: 8, total_clicks: 10 },
        { link_no: 2, url: "https://shop.test/about", unique_clicks: 2, total_clicks: 2 },
      ],
      100,
    );
    expect(shares.map((s) => [s.share, s.relative])).toEqual([[8, 1], [2, 0.25]]);
    expect(linkShares([{ link_no: 1, url: "x", unique_clicks: 0, total_clicks: 0 }], 0)[0]).toMatchObject({ share: null, relative: 0 });
  });

  it("name a link by its host", () => {
    expect(host("https://shop.test/sale?x=1")).toBe("shop.test");
    expect(host("not a url")).toBe("not a url");
  });
});
