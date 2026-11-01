import { describe, expect, it } from "vitest";
import { areaPath, bucketName, columnPath, cumulative, labelled, linePath, localBucket, nearest, niceAxis, ratesOver, scaleLinear, tickLabel, wall } from "./chart";

describe("a chart's axis", () => {
  it("runs from zero to a round number above the data, in whole steps for counts", () => {
    expect(niceAxis(7)).toEqual({ max: 8, ticks: [0, 2, 4, 6, 8] });
    expect(niceAxis(130)).toEqual({ max: 150, ticks: [0, 50, 100, 150] });
    expect(niceAxis(1)).toEqual({ max: 1, ticks: [0, 1] });
    expect(niceAxis(4812).max).toBe(5000);
  });

  it("gives an empty chart a small axis of its own", () => {
    expect(niceAxis(0)).toEqual({ max: 4, ticks: [0, 1, 2, 3, 4] });
    expect(niceAxis(Number.NaN).max).toBe(4);
  });

  it("steps rates by quarters", () => {
    expect(niceAxis(100, { integers: false })).toEqual({ max: 100, ticks: [0, 25, 50, 75, 100] });
    expect(niceAxis(0.8, { integers: false }).max).toBeGreaterThanOrEqual(0.8);
  });

  it("never stops short of the data", () => {
    for (const v of [1, 3, 9, 10, 11, 99, 101, 999, 1234, 56789]) expect(niceAxis(v).max).toBeGreaterThanOrEqual(v);
  });
});

describe("paths and positions", () => {
  it("maps values onto pixels, and a flat domain onto the middle", () => {
    const y = scaleLinear(0, 10, 100, 0);
    expect([y(0), y(5), y(10)]).toEqual([100, 50, 0]);
    expect(scaleLinear(3, 3, 0, 10)(3)).toBe(5);
  });

  it("draws a line, and the area under it down to the baseline", () => {
    expect(linePath([0, 10, 20], [5, 1, 3])).toBe("M0,5 L10,1 L20,3");
    expect(areaPath([0, 10], [5, 1], 9)).toBe("M0,5 L10,1 L10,9 L0,9 Z");
    expect(areaPath([], [], 9)).toBe("");
  });

  it("rounds only the top of a column", () => {
    const d = columnPath(10, 8, 20, 60);
    expect(d.startsWith("M10,60 V24")).toBe(true);
    expect(d.endsWith("V60 Z")).toBe(true);
    expect(columnPath(10, 8, 60, 60)).toBe("");
    expect(columnPath(10, 2, 58, 60, 4)).toContain("Q");
  });

  it("finds the point nearest the pointer", () => {
    const xs = [0, 10, 20, 30];
    expect([nearest(xs, -5), nearest(xs, 4), nearest(xs, 6), nearest(xs, 24), nearest(xs, 99)]).toEqual([0, 0, 1, 2, 3]);
    expect(nearest([], 3)).toBe(-1);
    expect(nearest([7], 100)).toBe(0);
  });

  it("adds up, and divides without dividing by nothing", () => {
    expect(cumulative([1, 0, 4, 2])).toEqual([1, 1, 5, 7]);
    expect(ratesOver([1, 3, 0], [4, 0, 5])).toEqual([25, 0, 0]);
  });

  it("labels some points evenly, the first always", () => {
    expect(labelled(48, 6)).toEqual([0, 8, 16, 24, 32, 40]);
    expect(labelled(5, 8)).toEqual([0, 1, 2, 3, 4]);
    expect(labelled(0, 6)).toEqual([]);
  });
});

describe("points in time", () => {
  it("reads the workspace's wall clock as it is written", () => {
    const d = wall("2026-11-01T14:30:00");
    expect([d.getFullYear(), d.getMonth(), d.getDate(), d.getHours(), d.getMinutes()]).toEqual([2026, 10, 1, 14, 30]);
    expect(wall("2026-11-01").getHours()).toBe(0);
    expect(Number.isNaN(wall("soon").getTime())).toBe(true);
  });

  it("labels a tick with the time of day, and the date where a day begins", () => {
    expect(tickLabel("2026-11-01T14:00:00", "hour", false, "en-GB")).toBe("14:00");
    expect(tickLabel("2026-11-02T00:00:00", "hour", false, "en-GB")).toContain("2 Nov");
    expect(tickLabel("2026-11-01T14:00:00", "hour", true, "en-GB")).toContain("1 Nov");
    expect(tickLabel("2026-11-01T00:00:00", "day", false, "en-GB")).toBe("1 Nov");
    expect(tickLabel("nonsense", "day", false)).toBe("");
  });

  it("names a bucket in full", () => {
    expect(bucketName("2026-11-01T14:00:00", "hour", "en-GB")).toMatch(/1 Nov, 14:00/);
    expect(bucketName("2026-11-01T00:00:00", "day", "en-GB")).toMatch(/1 Nov/);
    expect(bucketName("nonsense", "day")).toBe("nonsense");
  });
});

describe("which bucket a moment is in", () => {
  it("is the workspace's wall clock, by the hour or by the day", () => {
    expect(localBucket("2026-11-01T10:20:00Z", "UTC", "hour")).toBe("2026-11-01T10:00:00");
    expect(localBucket("2026-11-01T10:20:00Z", "Asia/Kolkata", "hour")).toBe("2026-11-01T15:00:00");
    expect(localBucket("2026-11-01T21:20:00Z", "Asia/Kolkata", "day")).toBe("2026-11-02T00:00:00");
    expect(localBucket("2026-11-01T23:30:00Z", "UTC", "hour")).toBe("2026-11-01T23:00:00");
  });

  it("is empty for what it cannot place", () => {
    expect(localBucket("later", "UTC", "hour")).toBe("");
    expect(localBucket("2026-11-01T10:20:00Z", "Not/AZone", "hour")).toBe("");
  });
});
