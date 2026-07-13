/**
 * Tests for the harness-trace event handler (stage-run per-org progress).
 */

import { describe, expect, it, vi } from "vitest";
import { handleHarnessTrace, stageRunRequestIdFromAgentRequestId } from "./harness-handlers";
import type { EventHandlerContext } from "./types";

function mockCtx(
  upsert: ReturnType<typeof vi.fn>,
  setCandidateReviewHint: ReturnType<typeof vi.fn> = vi.fn(),
  setReportingReadModelHint: ReturnType<typeof vi.fn> = vi.fn(),
  candidateReviewHint?: {
    operationId: string;
    waveRunId: string;
    status: string;
    resumeVersion: number;
    refreshVersion: number;
  }
): EventHandlerContext {
  return {
    sessionId: "sess-1",
    getState: vi.fn(() => ({
      upsertStageRunRow: upsert,
      setCandidateReviewHint,
      setReportingReadModelHint,
      sessions: candidateReviewHint
        ? { "sess-1": { candidateReviewHint } }
        : {},
    })) as unknown as EventHandlerContext["getState"],
    flushTextDeltas: vi.fn(),
    flushSessionDeltas: vi.fn(),
    batchTextDelta: vi.fn(),
    batchThinkingContent: vi.fn(),
    batchSubAgentThinking: vi.fn(),
    batchToolOutputChunk: vi.fn(),
    convertToolSource: vi.fn(),
  };
}

type HarnessEvent = Parameters<typeof handleHarnessTrace>[0];

describe("handleHarnessTrace", () => {
  it("maps a stage_run_org_progress trace into an upserted row + meta", () => {
    const upsert = vi.fn();
    handleHarnessTrace(
      {
        type: "harness_trace",
        operation_id: "op",
        stage: "target_intel",
        agent_path: "main",
        kind: "stage_run_org_progress",
        org_id: "org-1",
        org_name: "平安科技",
        agent_request_id: "op::org::org-1",
        ownership_percent: 100,
        status: "running",
        coverage: [
          ["DNS", "found"],
          ["CT", "weird"],
        ],
        evidence_count: 3,
        activity: "subfinder",
        stage_label: "Target Intel",
        role_label: "Recon",
        coverage_axis: ["DNS", "CT"],
      } as unknown as HarnessEvent,
      mockCtx(upsert)
    );

    expect(upsert).toHaveBeenCalledTimes(1);
    const [sid, row, meta] = upsert.mock.calls[0];
    expect(sid).toBe("sess-1");
    expect(row.id).toBe("org-1");
    expect(row.operationId).toBe("op");
    expect(row.name).toBe("平安科技");
    expect(row.agentRequestId).toBe("op::org::org-1");
    expect(row.ownershipPercent).toBe(100);
    expect(row.status).toBe("running");
    expect(row.evidenceCount).toBe(3);
    // Unknown technique state is clamped to "pending".
    expect(row.coverage).toEqual({ DNS: "found", CT: "pending" });
    expect(meta.stageLabel).toBe("Target Intel");
    expect(meta.roleLabel).toBe("Recon");
    expect(meta.coverageAxis).toEqual(["DNS", "CT"]);
    expect(meta.requestId).toBe("op");
  });

  it("extracts the stage_run tool request id from an org agent request id", () => {
    expect(stageRunRequestIdFromAgentRequestId("tool-13::org::org-1")).toBe("tool-13");
    expect(stageRunRequestIdFromAgentRequestId("tool-13")).toBeNull();
    expect(stageRunRequestIdFromAgentRequestId(null)).toBeNull();
  });

  it("ignores non stage_run_org_progress harness traces", () => {
    const upsert = vi.fn();
    handleHarnessTrace(
      {
        type: "harness_trace",
        operation_id: "op",
        stage: "target_intel",
        agent_path: "main",
        kind: "gate_decision",
        gate: "PASS",
        findings: 0,
      } as unknown as HarnessEvent,
      mockCtx(upsert)
    );
    expect(upsert).not.toHaveBeenCalled();
  });

  it("stores Candidate review traces only as DB refresh hints", () => {
    const upsert = vi.fn();
    const setCandidateReviewHint = vi.fn();
    handleHarnessTrace(
      {
        type: "harness_trace",
        operation_id: "operation-1",
        stage: "attack_candidate",
        agent_path: "main",
        kind: "candidate_review_required",
        wave_run_id: "wave-1",
        status: "resume_pending",
        resume_version: 4,
        candidate_count: 3,
        proposed_candidate_count: 0,
      } as HarnessEvent,
      mockCtx(upsert, setCandidateReviewHint)
    );

    expect(upsert).not.toHaveBeenCalled();
    expect(setCandidateReviewHint).toHaveBeenCalledWith("sess-1", {
      operationId: "operation-1",
      waveRunId: "wave-1",
      status: "resume_pending",
      resumeVersion: 4,
    });
  });

  it.each([
    {
      name: "a terminal CandidateAttempt",
      event: {
        type: "harness_trace",
        operation_id: "operation-1",
        stage: "verification",
        agent_path: "main",
        kind: "candidate_attempt_terminalized",
        scope_snapshot_id: "scope-1",
        wave_run_id: "wave-1",
        wave_unit_id: "unit-1",
        organization_id: "org-1",
        candidate_id: "candidate-1",
        attempt_id: "attempt-1",
        finding_id: "finding-1",
        status: "verified",
        evidence_count: 3,
        fact_delta_count: 1,
        replayed: false,
      },
      status: "verified",
    },
    {
      name: "an AttackWave consolidation",
      event: {
        type: "harness_trace",
        operation_id: "operation-1",
        stage: "verification",
        agent_path: "main",
        kind: "attack_wave_consolidated",
        scope_snapshot_id: "scope-1",
        consolidation_id: "consolidation-1",
        source_wave_run_id: "wave-1",
        target_wave_run_id: "wave-2",
        decision_kind: "opened_next_wave",
        accepted_fact_delta_count: 1,
        rejected_fact_delta_count: 2,
        residual_risk_count: 0,
        replayed: true,
      },
      status: "opened_next_wave",
    },
  ])("uses $name only to refresh an existing DB-backed Candidate view", ({ event, status }) => {
    const upsert = vi.fn();
    const setCandidateReviewHint = vi.fn();
    handleHarnessTrace(
      event as unknown as HarnessEvent,
      mockCtx(upsert, setCandidateReviewHint, vi.fn(), {
        operationId: "operation-1",
        waveRunId: "wave-1",
        status: "resumed",
        resumeVersion: 4,
        refreshVersion: 7,
      })
    );

    expect(upsert).not.toHaveBeenCalled();
    expect(setCandidateReviewHint).toHaveBeenCalledWith("sess-1", {
      operationId: "operation-1",
      waveRunId: "wave-1",
      status,
      resumeVersion: 4,
    });
  });

  it("does not fabricate a review cursor from a terminal trace", () => {
    const setCandidateReviewHint = vi.fn();
    handleHarnessTrace(
      {
        type: "harness_trace",
        operation_id: "operation-1",
        stage: "verification",
        agent_path: "main",
        kind: "candidate_attempt_terminalized",
        scope_snapshot_id: "scope-1",
        wave_run_id: "wave-1",
        wave_unit_id: "unit-1",
        organization_id: "org-1",
        candidate_id: "candidate-1",
        attempt_id: "attempt-1",
        status: "blocked",
        evidence_count: 1,
        fact_delta_count: 0,
        replayed: false,
      } as unknown as HarnessEvent,
      mockCtx(vi.fn(), setCandidateReviewHint)
    );

    expect(setCandidateReviewHint).not.toHaveBeenCalled();
  });

  it.each(["gate_decision", "deliverable_submitted"] as const)(
    "stores a Reporting %s trace only as a read-model refresh hint",
    (kind) => {
      const upsert = vi.fn();
      const setReportingReadModelHint = vi.fn();
      handleHarnessTrace(
        {
          type: "harness_trace",
          operation_id: "operation-reporting-1",
          stage: "reporting",
          agent_path: "main",
          kind,
          ...(kind === "gate_decision"
            ? { gate: "PASS", findings: 0 }
            : {
                status: "accepted",
                cited_evidence_refs: [],
                available_real_ids: [],
              }),
        } as HarnessEvent,
        mockCtx(upsert, vi.fn(), setReportingReadModelHint)
      );

      expect(upsert).not.toHaveBeenCalled();
      expect(setReportingReadModelHint).toHaveBeenCalledWith("sess-1", {
        operationId: "operation-reporting-1",
      });
    }
  );

  it("does not create a Reporting hint from another stage's gate trace", () => {
    const setReportingReadModelHint = vi.fn();
    handleHarnessTrace(
      {
        type: "harness_trace",
        operation_id: "operation-target-intel",
        stage: "target_intel",
        agent_path: "main",
        kind: "gate_decision",
        gate: "PASS",
        findings: 0,
      } as HarnessEvent,
      mockCtx(vi.fn(), vi.fn(), setReportingReadModelHint)
    );

    expect(setReportingReadModelHint).not.toHaveBeenCalled();
  });

  it("clamps an unknown row status to pending", () => {
    const upsert = vi.fn();
    handleHarnessTrace(
      {
        type: "harness_trace",
        operation_id: "op",
        stage: "target_intel",
        agent_path: "main",
        kind: "stage_run_org_progress",
        org_id: "o",
        org_name: "n",
        status: "bogus",
        coverage: [],
        evidence_count: 0,
        stage_label: "S",
        role_label: "R",
        coverage_axis: [],
      } as unknown as HarnessEvent,
      mockCtx(upsert)
    );
    expect(upsert.mock.calls[0][1].status).toBe("pending");
  });
});
