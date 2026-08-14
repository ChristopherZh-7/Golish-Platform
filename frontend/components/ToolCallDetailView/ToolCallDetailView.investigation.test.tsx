import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type AiToolExecution, useStore } from "@/store";
import { resolveInvestigationStageRun, ToolCallDetailView } from "./ToolCallDetailView";

const REQUEST_ID = "stage-run-investigation-1";
const SESSION_ID = "investigation-tool-detail-session";
const CONVERSATION_ID = "investigation-conversation";
const AI_SESSION_ID = "investigation-ai-session";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@/components/Engagement/InvestigationWorkspaceRoute", () => ({
  InvestigationWorkspaceRoute: ({
    sessionId,
    presentationSessionId,
    identity,
    displayStageRunRequestId,
  }: {
    sessionId: string;
    presentationSessionId?: string;
    identity: { stageExecutionId: string; stageRunRequestId: string };
    displayStageRunRequestId?: string | null;
  }) => (
    <div data-testid="investigation-workspace-view">
      Exact Investigation route · {identity.stageExecutionId} · {sessionId} ·{" "}
      {identity.stageRunRequestId} · pane:{presentationSessionId} · selected:
      {displayStageRunRequestId}
    </div>
  ),
}));

describe("resolveInvestigationStageRun", () => {
  it("resolves persisted Investigation only from one exact operation execution and request", () => {
    expect(
      resolveInvestigationStageRun({
        toolName: "stage_run",
        args: {
          stage: "investigation",
          operation_id: "operation-1",
          stage_execution_id: "execution-1",
          stage_run_request_id: REQUEST_ID,
        },
        result: {
          stage: "investigation",
          operationId: "operation-1",
          stageExecutionId: "execution-1",
        },
        selectedRequestId: REQUEST_ID,
        rows: [
          {
            stage: "investigation",
            operationId: "operation-1",
            stageExecutionId: "execution-1",
          },
        ],
      })
    ).toEqual({
      kind: "exact",
      identity: {
        operationId: "operation-1",
        stageExecutionId: "execution-1",
        stageRunRequestId: "stage_run:execution-1",
      },
      selectedStageRunRequestId: REQUEST_ID,
    });
  });

  it("routes a live-only actor without guessing a latest stage execution", () => {
    expect(
      resolveInvestigationStageRun({
        toolName: null,
        args: null,
        result: null,
        selectedRequestId: REQUEST_ID,
        rows: [
          {
            stage: "investigation",
            operationId: "operation-1",
            stageExecutionId: null,
            agentRequestId: "investigation-main-transcript",
          },
        ],
      })
    ).toEqual({
      kind: "live-only",
      identity: {
        operationId: "operation-1",
        stageExecutionId: null,
        stageRunRequestId: REQUEST_ID,
      },
      selectedStageRunRequestId: REQUEST_ID,
    });
  });

  it.each([
    {
      label: "operation",
      args: { stage: "investigation", operation_id: "operation-a" },
      result: { stage: "investigation", operation_id: "operation-b" },
      rows: [],
    },
    {
      label: "execution",
      args: {
        stage: "investigation",
        operation_id: "operation-a",
        stage_execution_id: "execution-a",
      },
      result: {
        stage: "investigation",
        operation_id: "operation-a",
        stage_execution_id: "execution-b",
      },
      rows: [],
    },
    {
      label: "request",
      args: {
        stage: "investigation",
        operation_id: "operation-a",
        stage_run_request_id: "foreign-request",
      },
      result: null,
      rows: [],
    },
    {
      label: "stage",
      args: { stage: "investigation", operation_id: "operation-a" },
      result: { stage: "verification", operation_id: "operation-a" },
      rows: [],
    },
    {
      label: "row request owner",
      args: {
        stage: "investigation",
        operation_id: "operation-a",
        stage_execution_id: "execution-a",
      },
      result: null,
      rows: [
        {
          stage: "investigation",
          operationId: "operation-a",
          stageExecutionId: "execution-a",
          agentRequestId: "foreign-stage-run::org::org-a",
        },
      ],
    },
  ])("fails closed on conflicting $label identity", ({ args, result, rows }) => {
    const resolution = resolveInvestigationStageRun({
      toolName: "stage_run",
      args,
      result,
      selectedRequestId: REQUEST_ID,
      rows,
    });

    expect(resolution.kind).toBe("invalid");
  });

  it("leaves legacy Candidate and Verification on their existing adapter", () => {
    expect(
      resolveInvestigationStageRun({
        toolName: "stage_run",
        args: { stage: "attack_candidate", operation_id: "operation-legacy" },
        result: null,
        selectedRequestId: REQUEST_ID,
        rows: [],
      })
    ).toEqual({ kind: "not-investigation" });
  });
});

describe("ToolCallDetailView Investigation production route", () => {
  beforeEach(() => {
    useStore.setState({
      sessions: {},
      timelines: {},
      activeSubAgents: {},
      backgroundJobs: {},
      conversations: {},
      conversationOrder: [],
      conversationTerminals: {},
      activeConversationId: null,
    });
  });

  function installRun({
    execution,
    stageExecutionId,
    operationId = "operation-1",
  }: {
    execution?: AiToolExecution;
    stageExecutionId: string | null;
    operationId?: string;
  }) {
    useStore.setState({
      sessions: {
        [SESSION_ID]: {
          id: SESSION_ID,
          name: "Investigation",
          workingDirectory: "/tmp/investigation",
          createdAt: "2026-08-02T00:00:00Z",
          mode: "agent",
          detailViewMode: "tool-detail",
          toolDetailRequestIds: [REQUEST_ID],
          stageRuns: {
            [REQUEST_ID]: {
              requestId: REQUEST_ID,
              stageLabel: "Investigation",
              roleLabel: "Investigation actor",
              coverageAxis: [],
              rows: [
                {
                  id: "org-1",
                  operationId,
                  stageExecutionId,
                  stageRunUnitId: "unit-1",
                  name: "Acme",
                  ownershipPercent: null,
                  status: "running",
                  agentRequestId: "exact-live-transcript",
                  evidenceCount: 0,
                  coverage: {},
                  stage: "investigation",
                },
              ],
              summary: { total: 1, covered: 0, active: 1, queued: 0, blocked: 0 },
            },
          },
        },
      },
      timelines: execution
        ? {
            [SESSION_ID]: [
              {
                id: `tool-${REQUEST_ID}`,
                type: "ai_tool_execution",
                timestamp: execution.startedAt,
                data: execution,
              },
            ],
          }
        : {},
      activeSubAgents: {},
      backgroundJobs: {},
      conversations: {
        [CONVERSATION_ID]: {
          id: CONVERSATION_ID,
          title: "Investigation",
          messages: [],
          createdAt: 0,
          aiSessionId: AI_SESSION_ID,
          aiInitialized: true,
          isStreaming: false,
        },
      },
      conversationOrder: [CONVERSATION_ID],
      conversationTerminals: { [CONVERSATION_ID]: [SESSION_ID] },
      activeConversationId: CONVERSATION_ID,
    });
  }

  it("routes a persisted unified run directly inside tool-detail", () => {
    installRun({
      stageExecutionId: "execution-1",
      execution: {
        requestId: REQUEST_ID,
        toolName: "stage_run",
        args: {
          stage: "investigation",
          operation_id: "operation-1",
          stage_execution_id: "execution-1",
        },
        status: "running",
        startedAt: "2026-08-02T00:00:00Z",
      },
    });

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByTestId("investigation-workspace-view")).toBeInTheDocument();
    expect(screen.getByTestId("investigation-workspace-view")).toHaveTextContent(
      "Exact Investigation route · execution-1"
    );
    expect(useStore.getState().sessions[SESSION_ID].detailViewMode).toBe("tool-detail");
  });

  it("uses the owning AI session and stable execution authority for exact reads", () => {
    installRun({
      stageExecutionId: "execution-1",
      execution: {
        requestId: REQUEST_ID,
        toolName: "stage_run",
        args: {
          stage: "investigation",
          operation_id: "operation-1",
          stage_execution_id: "execution-1",
        },
        status: "running",
        startedAt: "2026-08-02T00:00:00Z",
      },
    });

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByTestId("investigation-workspace-view")).toHaveTextContent(
      `Exact Investigation route · execution-1 · ${AI_SESSION_ID} · stage_run:execution-1 · pane:${SESSION_ID} · selected:${REQUEST_ID}`
    );
  });

  it("fails closed when a terminal has no owning conversation AI session", () => {
    installRun({
      stageExecutionId: "execution-1",
      execution: {
        requestId: REQUEST_ID,
        toolName: "stage_run",
        args: {
          stage: "investigation",
          operation_id: "operation-1",
          stage_execution_id: "execution-1",
        },
        status: "running",
        startedAt: "2026-08-02T00:00:00Z",
      },
    });
    useStore.setState({ conversationTerminals: {} });

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Investigation AI session identity is unavailable"
    );
    expect(screen.queryByTestId("investigation-workspace-view")).not.toBeInTheDocument();
  });

  it("routes a running outer call from its exact first progress row when args only contain orgs", () => {
    installRun({
      stageExecutionId: "execution-1",
      execution: {
        requestId: REQUEST_ID,
        toolName: "stage_run",
        args: { orgs: [{ organization_id: "org-1" }] },
        status: "running",
        startedAt: "2026-08-02T00:00:00Z",
      },
    });

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByTestId("investigation-workspace-view")).toBeInTheDocument();
    expect(screen.getByTestId("investigation-workspace-view")).toHaveTextContent(
      "Exact Investigation route · execution-1"
    );
  });

  it("uses the same workspace for a live-only actor and never guesses execution", () => {
    installRun({ stageExecutionId: null });

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByTestId("investigation-workspace-view")).toBeInTheDocument();
    expect(screen.getByText("Acme live actor · running")).toBeInTheDocument();
    expect(screen.getByText(/No latest DB execution is inferred/)).toBeInTheDocument();
  });

  it("ignores an unkeyed stage-run singleton instead of treating it as the selected request", () => {
    installRun({ stageExecutionId: null });
    const session = useStore.getState().sessions[SESSION_ID];
    const keyedRun = session.stageRuns?.[REQUEST_ID];
    useStore.setState({
      sessions: {
        [SESSION_ID]: {
          ...session,
          stageRuns: undefined,
          stageRun: keyedRun ? { ...keyedRun, requestId: undefined } : undefined,
        },
      },
    });

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.queryByTestId("investigation-workspace-view")).not.toBeInTheDocument();
    expect(screen.queryByText(/Acme live actor/)).not.toBeInTheDocument();
  });

  it("shows unavailable instead of routing conflicting identities", () => {
    installRun({
      operationId: "operation-row",
      stageExecutionId: "execution-1",
      execution: {
        requestId: REQUEST_ID,
        toolName: "stage_run",
        args: {
          stage: "investigation",
          operation_id: "operation-args",
          stage_execution_id: "execution-1",
        },
        status: "running",
        startedAt: "2026-08-02T00:00:00Z",
      },
    });

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByRole("alert")).toHaveTextContent("Conflicting Investigation operation");
    expect(screen.queryByTestId("investigation-workspace-view")).not.toBeInTheDocument();
  });
});
