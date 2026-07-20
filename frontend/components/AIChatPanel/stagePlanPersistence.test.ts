import { afterEach, describe, expect, it } from "vitest";
import type { TaskPlan } from "@/store/store-types";
import {
  clearStagePlans,
  readStagePlans,
  rewindPersistedStagePlans,
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

  it("atomically rewinds affected stages in the persisted roadmap", () => {
    writeStagePlans("conv-1", SNAPSHOT);
    const affected = rewindPersistedStagePlans(
      "conv-1",
      ["target_intel", "reporting"],
      "target_intel",
      "reset-epoch"
    );

    expect(affected).toEqual(["target_intel", "reporting"]);
    expect(readStagePlans("conv-1")).toEqual({
      stageOrder: ["scoping", "target_intel"],
      plansByStage: {
        scoping: SNAPSHOT.plansByStage.scoping,
        target_intel: {
          version: 0,
          explanation: null,
          updated_at: "reset-epoch",
          steps: [{ step: "target_intel", status: "in_progress" }],
          summary: { total: 1, completed: 0, in_progress: 1, pending: 0 },
        },
      },
      passedStages: ["scoping"],
    });
  });

  it("also removes descendants known only by the durable snapshot", () => {
    writeStagePlans("conv-1", {
      stageOrder: ["scoping", "external_attack_surface", "enumeration", "vuln_triage"],
      plansByStage: {
        scoping: plan(1),
        external_attack_surface: plan(2),
        enumeration: plan(3),
        vuln_triage: plan(4),
      },
      passedStages: ["scoping", "external_attack_surface", "enumeration"],
    });

    const affected = rewindPersistedStagePlans(
      "conv-1",
      ["external_attack_surface"],
      "external_attack_surface",
      "reset-epoch"
    );

    expect(affected).toEqual(["external_attack_surface", "enumeration", "vuln_triage"]);
    expect(readStagePlans("conv-1")?.stageOrder).toEqual([
      "scoping",
      "external_attack_surface",
    ]);
    expect(readStagePlans("conv-1")?.plansByStage.enumeration).toBeUndefined();
    expect(readStagePlans("conv-1")?.plansByStage.vuln_triage).toBeUndefined();
    expect(readStagePlans("conv-1")?.passedStages).toEqual(["scoping"]);
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
