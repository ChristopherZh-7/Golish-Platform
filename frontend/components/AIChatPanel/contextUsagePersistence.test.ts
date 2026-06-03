import { afterEach, describe, expect, it } from "vitest";
import {
  CONTEXT_USAGE_STORAGE_KEY,
  readContextUsage,
  writeContextUsage,
} from "./contextUsagePersistence";

describe("contextUsagePersistence", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("round-trips a snapshot for a conversation", () => {
    writeContextUsage("conv-1", { utilization: 0.42, totalTokens: 4200, maxTokens: 10_000 });
    expect(readContextUsage("conv-1")).toEqual({
      utilization: 0.42,
      totalTokens: 4200,
      maxTokens: 10_000,
    });
  });

  it("returns null for an unknown conversation", () => {
    expect(readContextUsage("nope")).toBeNull();
  });

  it("keeps conversations independent", () => {
    writeContextUsage("a", { utilization: 0.1, totalTokens: 1, maxTokens: 10 });
    writeContextUsage("b", { utilization: 0.9, totalTokens: 9, maxTokens: 10 });
    expect(readContextUsage("a")?.utilization).toBe(0.1);
    expect(readContextUsage("b")?.utilization).toBe(0.9);
  });

  it("returns null when the stored payload is corrupt", () => {
    localStorage.setItem(CONTEXT_USAGE_STORAGE_KEY, "{not json");
    expect(readContextUsage("conv-1")).toBeNull();
  });

  it("ignores malformed snapshots", () => {
    localStorage.setItem(
      CONTEXT_USAGE_STORAGE_KEY,
      JSON.stringify({ "conv-1": { utilization: "x" } })
    );
    expect(readContextUsage("conv-1")).toBeNull();
  });

  it("returns null for an empty conversation id", () => {
    expect(readContextUsage("")).toBeNull();
  });
});
