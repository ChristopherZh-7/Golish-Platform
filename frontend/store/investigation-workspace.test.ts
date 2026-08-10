import { beforeEach, describe, expect, it } from "vitest";
import { type DetailViewMode, useStore } from "./index";

const SESSION_ID = "investigation-refresh-session";

function installExactRoute() {
  useStore.setState({
    sessions: {
      [SESSION_ID]: {
        id: SESSION_ID,
        name: "Investigation",
        workingDirectory: "/tmp/investigation",
        createdAt: "2026-08-08T00:00:00Z",
        mode: "agent",
        detailViewMode: "tool-detail",
        toolDetailRequestIds: ["request-1"],
        stageRuns: {
          "request-1": {
            requestId: "request-1",
            stageLabel: "Investigation",
            roleLabel: "Actor",
            coverageAxis: [],
            summary: { total: 1, covered: 0, active: 1, queued: 0, blocked: 0 },
            rows: [
              {
                id: "org-1",
                operationId: "operation-1",
                stageExecutionId: "execution-1",
                stageRunUnitId: "unit-1",
                name: "Acme",
                ownershipPercent: null,
                status: "running",
                evidenceCount: 0,
                coverage: {},
                stage: "investigation",
              },
            ],
          },
        },
      },
    },
    timelines: {},
  });
}

describe("Investigation has no independent Pane route", () => {
  beforeEach(installExactRoute);

  it("keeps the detail router limited to timeline, tool and sub-agent views", () => {
    const supportedModes: DetailViewMode[] = ["timeline", "tool-detail", "sub-agent-detail"];

    expect(supportedModes).not.toContain("investigation-workspace");
    expect(supportedModes).toContain("tool-detail");
  });

  it("retains only monotonic refresh hints for the exact selected stage_run", () => {
    const store = useStore.getState();
    const exact = {
      operationId: "operation-1",
      stageExecutionId: "execution-1",
      stageRunRequestId: "request-1",
      changeSeq: 8,
    };

    store.setInvestigationRefreshHint(SESSION_ID, exact);
    store.setInvestigationRefreshHint(SESSION_ID, { ...exact, changeSeq: 8 });
    store.setInvestigationRefreshHint(SESSION_ID, { ...exact, changeSeq: 7 });
    expect(useStore.getState().sessions[SESSION_ID].investigationRefreshHint).toEqual(exact);

    store.setInvestigationRefreshHint(SESSION_ID, {
      ...exact,
      operationId: "foreign-operation",
      changeSeq: 9,
    });
    store.setInvestigationRefreshHint(SESSION_ID, {
      ...exact,
      stageExecutionId: "foreign-execution",
      changeSeq: 9,
    });
    store.setInvestigationRefreshHint(SESSION_ID, {
      ...exact,
      stageRunRequestId: "foreign-request",
      changeSeq: 9,
    });
    expect(useStore.getState().sessions[SESSION_ID].investigationRefreshHint).toEqual(exact);
  });

  it("clears the refresh hint without changing the direct tool-detail route", () => {
    const store = useStore.getState();
    store.setInvestigationRefreshHint(SESSION_ID, {
      operationId: "operation-1",
      stageExecutionId: "execution-1",
      stageRunRequestId: "request-1",
      changeSeq: 1,
    });
    store.setInvestigationRefreshHint(SESSION_ID, null);

    expect(useStore.getState().sessions[SESSION_ID].investigationRefreshHint).toBeUndefined();
    expect(useStore.getState().sessions[SESSION_ID].detailViewMode).toBe("tool-detail");
  });

  it("accepts a restored exact tool execution when live stage rows are absent", () => {
    const session = useStore.getState().sessions[SESSION_ID];
    useStore.setState({
      sessions: {
        [SESSION_ID]: {
          ...session,
          stageRuns: {
            "request-1": {
              ...session.stageRuns!["request-1"],
              rows: [],
            },
          },
        },
      },
      timelines: {
        [SESSION_ID]: [
          {
            id: "tool-request-1",
            type: "ai_tool_execution",
            timestamp: "2026-08-08T00:00:00Z",
            data: {
              requestId: "request-1",
              toolName: "stage_run",
              args: {
                stage: "investigation",
                operation_id: "operation-1",
                stage_execution_id: "execution-1",
              },
              status: "running",
              startedAt: "2026-08-08T00:00:00Z",
            },
          },
        ],
      },
    });

    useStore.getState().setInvestigationRefreshHint(SESSION_ID, {
      operationId: "operation-1",
      stageExecutionId: "execution-1",
      stageRunRequestId: "request-1",
      changeSeq: 3,
    });
    expect(useStore.getState().sessions[SESSION_ID].investigationRefreshHint?.changeSeq).toBe(3);
  });
});
