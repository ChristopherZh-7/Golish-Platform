import { describe, expect, it } from "vitest";
import {
  appendLiveToolOutput,
  LIVE_TOOL_OUTPUT_TAIL_LIMIT,
  LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX,
} from "./live-output";

describe("appendLiveToolOutput", () => {
  it("keeps small live output unchanged", () => {
    expect(appendLiveToolOutput("hello", "\nworld")).toBe("hello\nworld");
  });

  it("keeps only a bounded tail after large live output", () => {
    const output = appendLiveToolOutput(undefined, "a".repeat(LIVE_TOOL_OUTPUT_TAIL_LIMIT + 100));

    expect(output.startsWith(LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX)).toBe(true);
    expect(output.length).toBe(LIVE_TOOL_OUTPUT_TAIL_LIMIT);
    expect(output.endsWith("a".repeat(50))).toBe(true);
  });

  it("does not duplicate the truncation marker on later chunks", () => {
    const first = appendLiveToolOutput(undefined, "a".repeat(LIVE_TOOL_OUTPUT_TAIL_LIMIT + 100));
    const second = appendLiveToolOutput(first, "\nb");

    expect(second.startsWith(LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX)).toBe(true);
    expect(second.indexOf(LIVE_TOOL_OUTPUT_TRUNCATED_PREFIX, 1)).toBe(-1);
    expect(second.endsWith("\nb")).toBe(true);
    expect(second.length).toBeLessThanOrEqual(LIVE_TOOL_OUTPUT_TAIL_LIMIT);
  });
});
