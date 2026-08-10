import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type AiToolExecution, useStore } from "@/store";
import { ToolCallDetailView } from "./ToolCallDetailView";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const SESSION_ID = "stage-workspace-session";
const REQUEST_ID = "stage-workspace-request";

function installCompanyStageRun(): void {
  const execution: AiToolExecution = {
    requestId: REQUEST_ID,
    toolName: "stage_run",
    args: {
      orgs: [{ id: "org-1", name: "默安科技", ownership_percent: 100 }],
    },
    status: "running",
    startedAt: "2026-08-10T00:00:00Z",
    toolIntent: {
      modelWanted: "stage_run",
      source: "native_tool_call",
      decision: "allow",
    },
  };

  useStore.setState({
    sessions: {
      [SESSION_ID]: {
        id: SESSION_ID,
        toolDetailRequestIds: [REQUEST_ID],
        stageRuns: {
          [REQUEST_ID]: {
            requestId: REQUEST_ID,
            stageLabel: "Target Intel",
            roleLabel: "Company Controller",
            coverageAxis: [],
            rows: [
              {
                id: "org-1",
                name: "默安科技",
                ownershipPercent: 100,
                status: "running",
                evidenceCount: 0,
                coverage: {},
                stage: "target_intel",
              },
            ],
            summary: { total: 1, covered: 0, active: 1, queued: 0, blocked: 0 },
          },
        },
      } as any,
    },
    timelines: {
      [SESSION_ID]: [
        {
          id: `tool-${REQUEST_ID}`,
          type: "ai_tool_execution",
          timestamp: execution.startedAt,
          data: execution,
        },
      ],
    },
    activeSubAgents: {},
    backgroundJobs: {},
  });
}

describe("ToolCallDetailView Company Controller workspace", () => {
  beforeEach(() => {
    useStore.setState({ sessions: {}, timelines: {}, activeSubAgents: {}, backgroundJobs: {} });
  });

  it("replaces the generic tool envelope and gives the Agent workspace the fixed viewport", () => {
    installCompanyStageRun();
    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByTestId("stage-team-rerun-required")).toBeInTheDocument();
    expect(screen.getByTestId("tool-detail-content")).toHaveClass(
      "min-h-0",
      "flex-1",
      "overflow-hidden"
    );
    expect(screen.queryByText("Tool")).not.toBeInTheDocument();
    expect(screen.queryByText("Intent")).not.toBeInTheDocument();
    expect(screen.queryByText("Input")).not.toBeInTheDocument();
    expect(screen.queryByText("Output")).not.toBeInTheDocument();
    expect(screen.queryByText("Model wanted")).not.toBeInTheDocument();
    expect(screen.queryByText("Waiting for output...")).not.toBeInTheDocument();
    expect(screen.queryByText("stage_run")).not.toBeInTheDocument();
  });
});
