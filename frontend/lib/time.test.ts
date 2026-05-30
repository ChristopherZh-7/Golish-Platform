import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  formatDurationClock,
  formatDurationCompact,
  formatRelativeAgo,
} from "./time";

describe("formatDurationClock", () => {
  it("formats whole minutes:seconds with zero-padding", () => {
    expect(formatDurationClock(0)).toBe("0:00");
    expect(formatDurationClock(5_000)).toBe("0:05");
    expect(formatDurationClock(65_000)).toBe("1:05");
    expect(formatDurationClock(3_661_000)).toBe("61:01");
  });

  it("floors sub-second remainders", () => {
    expect(formatDurationClock(500)).toBe("0:00");
    expect(formatDurationClock(1_999)).toBe("0:01");
  });
});

describe("formatDurationCompact", () => {
  it("omits the minute component below 60s and uses no decimals", () => {
    expect(formatDurationCompact(0)).toBe("0s");
    expect(formatDurationCompact(500)).toBe("0s");
    expect(formatDurationCompact(45_000)).toBe("45s");
  });

  it("shows minutes and seconds above 60s", () => {
    expect(formatDurationCompact(60_000)).toBe("1m 0s");
    expect(formatDurationCompact(150_000)).toBe("2m 30s");
  });
});

describe("formatRelativeAgo", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  const ago = (ms: number) => Date.now() - ms;

  // Mirrors DispatchInflightSection: second granularity, hours never roll up.
  describe("second/hour policy", () => {
    const opts = { minUnit: "second", maxUnit: "hour" } as const;

    it("shows seconds, minutes, then unbounded hours", () => {
      expect(formatRelativeAgo(ago(5_000), opts)).toBe("5s ago");
      expect(formatRelativeAgo(ago(90_000), opts)).toBe("1m ago");
      expect(formatRelativeAgo(ago(3_700_000), opts)).toBe("1h ago");
      expect(formatRelativeAgo(ago(100 * 3_600_000), opts)).toBe("100h ago");
    });

    it("clamps the future to 0s and echoes unparseable input", () => {
      expect(formatRelativeAgo(Date.now() + 10_000, opts)).toBe("0s ago");
      expect(formatRelativeAgo("not-a-date", opts)).toBe("not-a-date");
    });
  });

  // Mirrors buildTopologyModel: minute floor of 1, days after 24h, "recently" fallbacks.
  describe("minute/day policy with recently fallbacks", () => {
    const opts = { invalidLabel: "recently", futureLabel: "recently" } as const;

    it("clamps sub-minute to 1m and rolls hours into days", () => {
      expect(formatRelativeAgo(ago(30_000), opts)).toBe("1m ago");
      expect(formatRelativeAgo(ago(5 * 60_000), opts)).toBe("5m ago");
      expect(formatRelativeAgo(ago(90 * 60_000), opts)).toBe("1h ago");
      expect(formatRelativeAgo(ago(25 * 3_600_000), opts)).toBe("1d ago");
    });

    it("returns recently for future and non-finite input", () => {
      expect(formatRelativeAgo(Date.now() + 60_000, opts)).toBe("recently");
      expect(formatRelativeAgo(Number.NaN, opts)).toBe("recently");
    });
  });

  it("defaults to minute/day granularity", () => {
    expect(formatRelativeAgo(ago(30_000))).toBe("1m ago");
    expect(formatRelativeAgo(ago(2 * 3_600_000))).toBe("2h ago");
  });
});
