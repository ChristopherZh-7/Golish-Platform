import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearTerminalAutoFocusSuppression,
  isTerminalAutoFocusSuppressed,
  suppressTerminalAutoFocus,
} from "./terminalAutoFocus";

describe("terminalAutoFocus", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    clearTerminalAutoFocusSuppression("s1");
    vi.useRealTimers();
  });

  it("suppresses within the window and auto-expires after it", () => {
    suppressTerminalAutoFocus("s1", 1000);
    expect(isTerminalAutoFocusSuppressed("s1")).toBe(true);

    vi.advanceTimersByTime(500);
    expect(isTerminalAutoFocusSuppressed("s1")).toBe(true);

    vi.advanceTimersByTime(600); // now past the 1000ms window
    expect(isTerminalAutoFocusSuppressed("s1")).toBe(false);
  });

  it("reports false for sessions that were never suppressed", () => {
    expect(isTerminalAutoFocusSuppressed("never")).toBe(false);
  });

  it("can be cleared early (e.g. user focuses the terminal)", () => {
    suppressTerminalAutoFocus("s1", 5000);
    expect(isTerminalAutoFocusSuppressed("s1")).toBe(true);

    clearTerminalAutoFocusSuppression("s1");
    expect(isTerminalAutoFocusSuppressed("s1")).toBe(false);
  });

  it("ignores empty session ids", () => {
    suppressTerminalAutoFocus("", 5000);
    expect(isTerminalAutoFocusSuppressed("")).toBe(false);
  });
});
