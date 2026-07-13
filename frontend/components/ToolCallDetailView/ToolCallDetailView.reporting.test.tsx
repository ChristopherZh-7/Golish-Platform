import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type AiToolExecution, useStore } from "@/store";
import { ToolCallDetailView } from "./ToolCallDetailView";

vi.mock("@/components/Engagement/ReportReadModelView", () => ({
  ReportReadModelView: ({ operationId }: { operationId: string }) => (
    <div data-testid="report-read-model-production-entry">{operationId}</div>
  ),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const SESSION_ID = "reporting-detail-session";
const REQUEST_ID = "reporting-stage-run-request";

function setSelectedStageRun(args: Record<string, unknown>, result: unknown) {
  const execution: AiToolExecution = {
    requestId: REQUEST_ID,
    toolName: "stage_run",
    args,
    result,
    status: "completed",
    startedAt: "2026-07-13T00:00:00Z",
    completedAt: "2026-07-13T00:00:01Z",
    durationMs: 1000,
  };
  useStore.setState({
    sessions: {
      [SESSION_ID]: {
        id: SESSION_ID,
        name: "Reporting detail",
        workingDirectory: "/tmp/reporting-detail",
        createdAt: "2026-07-13T00:00:00Z",
        mode: "agent",
        toolDetailRequestIds: [REQUEST_ID],
      },
    },
    timelines: {
      [SESSION_ID]: [
        {
          id: `tool-exec-${REQUEST_ID}`,
          type: "ai_tool_execution",
          timestamp: "2026-07-13T00:00:00Z",
          data: execution,
        },
      ],
    },
    backgroundJobs: {},
  });
}

describe("ToolCallDetailView Reporting production entry", () => {
  beforeEach(() => {
    useStore.setState({ sessions: {}, timelines: {}, backgroundJobs: {} });
  });

  it("mounts the DB-backed report view for a selected Reporting stage_run result", () => {
    setSelectedStageRun(
      { orgs: [] },
      { stage: "reporting", operation_id: "operation-reporting-1", passed: true }
    );

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByTestId("report-read-model-production-entry")).toHaveTextContent(
      "operation-reporting-1"
    );
  });

  it("does not mount the report view for another stage or conflicting identity", () => {
    setSelectedStageRun(
      { stage: "cleanup", operation_id: "operation-cleanup" },
      { stage: "cleanup", operation_id: "operation-cleanup", passed: true }
    );
    const view = render(<ToolCallDetailView sessionId={SESSION_ID} />);
    expect(screen.queryByTestId("report-read-model-production-entry")).not.toBeInTheDocument();

    act(() => {
      setSelectedStageRun(
        { stage: "reporting", operation_id: "operation-a" },
        { stage: "reporting", operation_id: "operation-b", passed: true }
      );
    });
    view.rerender(<ToolCallDetailView sessionId={SESSION_ID} />);
    expect(screen.queryByTestId("report-read-model-production-entry")).not.toBeInTheDocument();
  });
});
