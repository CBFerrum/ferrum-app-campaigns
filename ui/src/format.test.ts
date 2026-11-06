import { describe, expect, it } from "vitest";
import { setTimeZone } from "@ferrum/client";
import { fromLocalInput, moment, noun, people, percent, rate, size, span, toLocalInput, zoneName } from "./format";

describe("numbers as people say them", () => {
  it("writes a share of a whole, and nothing when there is no whole", () => {
    expect(rate(1, 4)).toBe(25);
    expect(rate(3, 0)).toBeNull();
    expect(percent(31.42)).toBe("31.4%");
    expect(percent(100)).toBe("100%");
    expect(percent(5)).toBe("5%");
    expect(percent(0)).toBe("0%");
    expect(percent(0.04)).toBe("<0.1%");
    expect(percent(null)).toBe("–");
  });

  it("counts with the right noun", () => {
    expect(people(1)).toBe("1 person");
    expect(people(4812)).toBe(`${(4812).toLocaleString()} people`);
    expect(noun(1, "link")).toBe("1 link");
    expect(noun(2, "address", "addresses")).toBe("2 addresses");
  });

  it("says how long something takes", () => {
    expect(span(0.2)).toBe("under an hour");
    expect(span(1)).toBe("about 1 hour");
    expect(span(10.4)).toBe("about 10 hours");
    expect(span(72)).toBe("about 3 days");
    expect(span(Number.NaN)).toBe("under an hour");
  });

  it("writes sizes", () => {
    expect(size(900)).toBe("900 bytes");
    expect(size(102_400)).toBe("100 KB");
    expect(size(3 * 1024 * 1024)).toBe("3.0 MB");
  });
});

describe("a time in a box", () => {
  it("goes in and comes out as the same moment, in the viewer's own time", () => {
    const iso = new Date(2026, 10, 7, 10, 30).toISOString();
    expect(toLocalInput(iso)).toBe("2026-11-07T10:30");
    expect(fromLocalInput("2026-11-07T10:30")).toBe(iso);
  });

  it("is on the workspace's clock when it knows it", () => {
    setTimeZone("Asia/Kolkata");
    expect(toLocalInput("2026-11-07T05:00:00Z")).toBe("2026-11-07T10:30");
    expect(fromLocalInput("2026-11-07T10:30")).toBe("2026-11-07T05:00:00.000Z");
    expect(zoneName("en-GB", new Date("2026-11-07T10:30:00Z"))).toBe("GMT+5:30");
    expect(moment("2026-11-07T05:00:00Z", "en-GB").startsWith("Sat 7 Nov, 10:30")).toBe(true);
    setTimeZone(undefined);
  });

  it("is empty for nothing, and nothing for a mess", () => {
    expect([toLocalInput(null), toLocalInput("soon")]).toEqual(["", ""]);
    expect([fromLocalInput(""), fromLocalInput("soon")]).toEqual([null, null]);
  });

  it("names a time zone", () => {
    expect(zoneName("en-GB", new Date("2026-11-07T10:30:00Z")).length).toBeGreaterThan(0);
  });
});
