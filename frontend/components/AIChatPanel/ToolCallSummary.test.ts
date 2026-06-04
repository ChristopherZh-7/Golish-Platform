import { describe, expect, it } from "vitest";
import { toolResultIsFailure } from "./ToolCallSummary";

describe("toolResultIsFailure", () => {
  it("flags a rejected status body as a failure (shows ❌ not ✅)", () => {
    expect(toolResultIsFailure('{"status":"rejected","reason":"stage_id mismatch"}')).toBe(true);
  });

  it("flags needs_fix / error / failed statuses too", () => {
    expect(toolResultIsFailure('{"status": "needs_fix"}')).toBe(true);
    expect(toolResultIsFailure('{ "status" : "error" }')).toBe(true);
    expect(toolResultIsFailure('{"status":"failed"}')).toBe(true);
  });

  it("does NOT flag an accepted deliverable", () => {
    expect(
      toolResultIsFailure('{"status":"accepted","note":"structure OK; final gate at stage close."}')
    ).toBe(false);
  });

  it("ignores unrelated / non-status results and empty input", () => {
    expect(toolResultIsFailure('{"result":"done"}')).toBe(false);
    expect(toolResultIsFailure("plain text output")).toBe(false);
    expect(toolResultIsFailure(undefined)).toBe(false);
    expect(toolResultIsFailure("")).toBe(false);
  });
});
