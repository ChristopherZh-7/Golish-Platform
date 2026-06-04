import { afterEach, describe, expect, it } from "vitest";
import type { TaskPlan } from "@/store/store-types";
import {
  clearStagePlans,
  readStagePlans,
  STAGE_PLAN_STORAGE_KEY,
  writeStagePlans,
} from "./stagePlanPersistence";

function plan(version: number): TaskPlan {
  return {
    version,
    explanation: null,
    updated_at: "2026-06-04T00:00:00.000Z",
    steps: [{ id: "s1", step: "do thing", status: "completed" }],
    summary: { total: 1, completed: 1, in_progress: 0, pending: 0 },
  };
}

const SNAPSHOT = {
  stageOrder: ["scoping", "target_intel"],
  plansByStage: { scoping: plan(1), target_intel: plan(2) },
  passedStages: ["scoping"],
};

describe("stagePlanPersistence", () => {
  afterEach(() => {
    localStorage.clear();
  });

  it("round-trips a per-stage roadmap snapshot for a conversation", () => {
    writeStagePlans("conv-1", SNAPSHOT);
    const restored = readStagePlans("conv-1");
    expect(restored?.stageOrder).toEqual(["scoping", "target_intel"]);
    expect(restored?.passedStages).toEqual(["scoping"]);
    expect(restored?.plansByStage.target_intel?.version).toBe(2);
  });

  it("returns null for an unknown conversation", () => {
    expect(readStagePlans("nope")).toBeNull();
  });

  it("does NOT persist (clobber) when stageOrder is empty", () => {
    // The write guard prevents an uninitialized store from wiping a saved
    // snapshot before the restore effect has run.
    writeStagePlans("conv-1", SNAPSHOT);
    writeStagePlans("conv-1", { stageOrder: [], plansByStage: {}, passedStages: [] });
    expect(readStagePlans("conv-1")?.stageOrder).toEqual(["scoping", "target_intel"]);
  });

  it("keeps conversations independent", () => {
    writeStagePlans("a", { ...SNAPSHOT, passedStages: ["scoping"] });
    writeStagePlans("b", { stageOrder: ["recon"], plansByStage: { recon: plan(1) }, passedStages: [] });
    expect(readStagePlans("a")?.passedStages).toEqual(["scoping"]);
    expect(readStagePlans("b")?.stageOrder).toEqual(["recon"]);
  });

  it("clears a conversation's snapshot", () => {
    writeStagePlans("conv-1", SNAPSHOT);
    clearStagePlans("conv-1");
    expect(readStagePlans("conv-1")).toBeNull();
  });

  it("returns null when the stored payload is corrupt", () => {
    localStorage.setItem(STAGE_PLAN_STORAGE_KEY, "{not json");
    expect(readStagePlans("conv-1")).toBeNull();
  });

  it("ignores malformed snapshots (missing arrays)", () => {
    localStorage.setItem(
      STAGE_PLAN_STORAGE_KEY,
      JSON.stringify({ "conv-1": { stageOrder: "x", plansByStage: {} } })
    );
    expect(readStagePlans("conv-1")).toBeNull();
  });

  it("returns null for an empty conversation id", () => {
    expect(readStagePlans("")).toBeNull();
  });
});
