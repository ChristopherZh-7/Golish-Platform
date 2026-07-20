import { describe, expect, it } from "vitest";
import type { HarnessDevStageCheckpointResetResult } from "./generated/HarnessDevStageCheckpointResetResult";
import {
  inferCurrentResetStage,
  trustedResetAffectedStages,
  validateCommittedStageResetReceipt,
} from "./stage-reset";

const RECEIPT: HarnessDevStageCheckpointResetResult = {
  operationId: "op-1",
  stage: "external_attack_surface",
  mode: "restart_from_stage_purge",
  affectedStages: ["external_attack_surface", "enumeration", "vuln_triage", "reporting"],
  clearedAgentRunCheckpoints: 0,
  clearedStageRunWorkers: 0,
  resetGraphFlow: true,
  trimmedGraphFlowVisited: 2,
  trimmedGraphFlowApplied: 2,
  refreshedStageCursor: true,
  previousStage: "vuln_triage",
  currentStage: "external_attack_surface",
  message: "committed",
  purgedFacts: true,
  purgeScopeOrgCount: 2,
  purgeCounts: { targetAssets: 1 },
  purgeNote: null,
};

describe("committed stage reset receipt", () => {
  const localAffected = ["external_attack_surface", "enumeration", "vuln_triage", "reporting"];

  it("accepts the exact full-reset receipt contract", () => {
    expect(
      validateCommittedStageResetReceipt(RECEIPT, "external_attack_surface", localAffected)
    ).toBeNull();
  });

  it.each([
    [{ stage: "enumeration" }, "stage"],
    [{ mode: "restart_stage" }, "mode"],
    [{ currentStage: "enumeration" }, "currentStage"],
    [{ refreshedStageCursor: false }, "refreshedStageCursor"],
    [{ resetGraphFlow: false }, "resetGraphFlow"],
    [{ purgedFacts: false }, "purgedFacts"],
    [{ purgeScopeOrgCount: 0 }, "purgeScopeOrgCount"],
    [{ purgeCounts: null }, "purgeCounts"],
    [{ purgeNote: "skipped" }, "purgeNote"],
    [{ affectedStages: ["enumeration"] }, "affectedStages"],
    [{ affectedStages: ["external_attack_surface", "unknown_future"] }, "affectedStages"],
  ])("rejects malformed committed field %s", (overrides, field) => {
    expect(
      validateCommittedStageResetReceipt(
        { ...RECEIPT, ...overrides },
        "external_attack_surface",
        localAffected
      )
    ).toContain(field);
  });

  it("uses the local DAG suffix when a committed receipt is malformed or null", () => {
    expect(
      trustedResetAffectedStages(
        { ...RECEIPT, affectedStages: ["external_attack_surface", "unknown_future"] },
        "external_attack_surface",
        localAffected
      )
    ).toEqual(localAffected);
    expect(trustedResetAffectedStages(null, "external_attack_surface", localAffected)).toEqual(
      localAffected
    );
  });

  it("rejects a receipt that omits locally materialized descendants", () => {
    expect(
      validateCommittedStageResetReceipt(
        { ...RECEIPT, affectedStages: ["external_attack_surface"] },
        "external_attack_surface",
        localAffected
      )
    ).toContain("affectedStages");
    expect(
      validateCommittedStageResetReceipt(null, "external_attack_surface", localAffected)
    ).toContain("receipt");
  });
});

describe("reset current-stage inference", () => {
  const pending = (stage: string) => ({
    version: 0,
    steps: [{ step: stage, status: "pending" as const }],
  });

  it("fails closed when the roadmap only contains whole-DAG pending seeds", () => {
    expect(
      inferCurrentResetStage(
        ["external_attack_surface", "enumeration", "reporting"],
        {
          external_attack_surface: pending("external_attack_surface"),
          enumeration: pending("enumeration"),
          reporting: pending("reporting"),
        },
        []
      )
    ).toBeNull();
  });

  it("uses an actual non-pending stage seed instead of the first unpassed DAG node", () => {
    expect(
      inferCurrentResetStage(
        ["external_attack_surface", "enumeration", "reporting"],
        {
          external_attack_surface: pending("external_attack_surface"),
          enumeration: pending("enumeration"),
          reporting: {
            version: 0,
            steps: [{ step: "reporting", status: "in_progress" }],
          },
        },
        ["external_attack_surface"]
      )
    ).toBe("reporting");
  });
});
