import { beforeEach, describe, expect, it } from "vitest";
import type { StageRunRow } from "@/components/Engagement/StageRunOrgRows";
import { useStore } from "./index";

const SID = "stage-run-session";

function row(id: string, status: StageRunRow["status"]): StageRunRow {
  return {
    id,
    name: id,
    ownershipPercent: null,
    status,
    evidenceCount: 0,
    coverage: {},
  };
}

function meta(requestId: string) {
  return {
    stageLabel: "Target Intel",
    roleLabel: "Recon",
    coverageAxis: ["DNS"],
    requestId,
  };
}

describe("stage_run session state", () => {
  beforeEach(() => {
    useStore.setState({
      sessions: {
        [SID]: {
          id: SID,
          name: "Stage Run",
          workingDirectory: "/tmp",
          createdAt: new Date().toISOString(),
          mode: "agent",
          stageRun: null,
          stageRuns: {},
        },
      },
      timelines: { [SID]: [] },
      processedToolRequests: {},
      pendingToolIntentObservations: {},
    });
  });

  it("keeps interrupted and continued stage_run progress keyed by request id", () => {
    const store = useStore.getState();

    store.addToolExecutionBlock(SID, { requestId: "T9", toolName: "stage_run", args: {} });
    store.upsertStageRunRow(SID, row("old-org", "running"), meta("T9"));

    store.addToolExecutionBlock(SID, { requestId: "T13", toolName: "stage_run", args: {} });
    store.setSessionStageRun(SID, {
      rows: [],
      summary: { total: 0, covered: 0, active: 0, queued: 0, blocked: 0 },
      stageLabel: "Stage Run",
      roleLabel: "",
      coverageAxis: [],
      requestId: "T13",
    });
    store.upsertStageRunRow(SID, row("new-org", "queued"), meta("T13"));

    // A late frame from T9 must update T9's snapshot, not steal T13's current view.
    store.upsertStageRunRow(SID, row("old-org", "passed"), meta("T9"));

    const session = useStore.getState().sessions[SID];
    expect(session.stageRuns?.T9.rows).toEqual([expect.objectContaining({ status: "passed" })]);
    expect(session.stageRuns?.T13.rows).toEqual([expect.objectContaining({ id: "new-org" })]);
    expect(session.stageRun?.requestId).toBe("T13");
  });

  it("marks a stale running tool execution as interrupted without changing terminal blocks", () => {
    const store = useStore.getState();

    store.addToolExecutionBlock(SID, { requestId: "T5", toolName: "stage_run", args: {} });
    store.interruptToolExecutionBlock(SID, "T5", { reason: "expired" });

    const interrupted = useStore
      .getState()
      .timelines[SID].find((b) => b.type === "ai_tool_execution" && b.data.requestId === "T5");
    expect(interrupted?.type).toBe("ai_tool_execution");
    if (interrupted?.type === "ai_tool_execution") {
      expect(interrupted.data.status).toBe("interrupted");
      expect(interrupted.data.result).toEqual({ reason: "expired" });
      expect(interrupted.data.completedAt).toBeTruthy();
      expect(interrupted.data.durationMs).toEqual(expect.any(Number));
    }

    store.addToolExecutionBlock(SID, { requestId: "T6", toolName: "stage_run", args: {} });
    store.completeToolExecutionBlock(SID, "T6", true, "ok");
    store.interruptToolExecutionBlock(SID, "T6", { reason: "late expired view" });

    const completed = useStore
      .getState()
      .timelines[SID].find((b) => b.type === "ai_tool_execution" && b.data.requestId === "T6");
    expect(completed?.type).toBe("ai_tool_execution");
    if (completed?.type === "ai_tool_execution") {
      expect(completed.data.status).toBe("completed");
      expect(completed.data.result).toBe("ok");
    }
  });
});
