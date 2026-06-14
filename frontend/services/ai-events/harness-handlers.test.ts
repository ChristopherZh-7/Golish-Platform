/**
 * Tests for the harness-trace event handler (stage-run per-org progress).
 */

import { describe, expect, it, vi } from "vitest";
import { handleHarnessTrace } from "./harness-handlers";
import type { EventHandlerContext } from "./types";

function mockCtx(upsert: ReturnType<typeof vi.fn>): EventHandlerContext {
  return {
    sessionId: "sess-1",
    getState: vi.fn(() => ({
      upsertStageRunRow: upsert,
    })) as unknown as EventHandlerContext["getState"],
    flushSessionDeltas: vi.fn(),
    batchTextDelta: vi.fn(),
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
