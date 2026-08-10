import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type AiToolExecution, useStore } from "@/store";
import {
  getCandidateStageRunOperationId,
  ToolCallDetailView,
} from "./ToolCallDetailView";

vi.mock("@/components/Engagement/AttackCandidateReview", () => ({
  AttackCandidateReview: (props: Record<string, unknown>) => (
    <div data-testid="candidate-review-production-entry">{JSON.stringify(props)}</div>
  ),
}));

vi.mock("@/components/Engagement/CandidateAttemptRows", () => ({
  CandidateAttemptRows: (props: Record<string, unknown>) => (
    <div data-testid="candidate-attempts-production-entry">{JSON.stringify(props)}</div>
  ),
}));

vi.mock("@/components/Engagement/PendingPreparedActionPanel", () => ({
  PendingPreparedActionPanel: (props: Record<string, unknown>) => (
    <div data-testid="prepared-action-production-entry">{JSON.stringify(props)}</div>
  ),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const SESSION_ID = "candidate-detail-session";
const REQUEST_ID = "candidate-stage-run-request";

function setSelectedCandidateStageRun() {
  const execution: AiToolExecution = {
    requestId: REQUEST_ID,
    toolName: "stage_run",
    args: { orgs: [] },
    result: { stage: "attack_candidate", passed: true },
    status: "completed",
    startedAt: "2026-07-13T00:00:00Z",
    completedAt: "2026-07-13T00:00:01Z",
    durationMs: 1000,
  };
  useStore.setState({
    sessions: {
      [SESSION_ID]: {
        id: SESSION_ID,
        name: "Candidate detail",
        workingDirectory: "/tmp/candidate-detail",
        createdAt: "2026-07-13T00:00:00Z",
        mode: "agent",
        toolDetailRequestIds: [REQUEST_ID],
        candidateReviewHint: {
          operationId: "operation-candidate-1",
          waveRunId: "wave-candidate-1",
          status: "resume_pending",
          resumeVersion: 2,
          refreshVersion: 7,
        },
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

function setSelectedRunningCandidateStageRun() {
  const execution: AiToolExecution = {
    requestId: REQUEST_ID,
    toolName: "stage_run",
    args: { orgs: [] },
    status: "running",
    startedAt: "2026-07-17T05:18:37Z",
  };
  useStore.setState({
    sessions: {
      [SESSION_ID]: {
        id: SESSION_ID,
        name: "Candidate detail",
        workingDirectory: "/tmp/candidate-detail",
        createdAt: "2026-07-17T05:18:37Z",
        mode: "agent",
        detailViewMode: "tool-detail",
        toolDetailRequestIds: [REQUEST_ID],
        stageRuns: {
          [REQUEST_ID]: {
            requestId: REQUEST_ID,
            stageLabel: "Attack Candidate",
            roleLabel: "Attack Analyst",
            coverageAxis: ["CANDIDATES"],
            rows: [
              {
                id: "org-candidate-1",
                name: "Acme Root",
                ownershipPercent: null,
                status: "running",
                agentRequestId: `${REQUEST_ID}::org::org-candidate-1`,
                activity: "classifying 26 frozen work items",
                evidenceCount: 0,
                coverage: {},
                stage: "attack_candidate",
              },
            ],
            summary: { total: 1, covered: 0, active: 1, queued: 0, blocked: 0 },
          },
        },
      },
    },
    timelines: {
      [SESSION_ID]: [
        {
          id: `tool-exec-${REQUEST_ID}`,
          type: "ai_tool_execution",
          timestamp: "2026-07-17T05:18:37Z",
          data: execution,
        },
      ],
    },
    activeSubAgents: {},
    backgroundJobs: {},
  });
}

function setSelectedCandidateStageRunWithOperationIds(operationIds: Array<string | undefined>) {
  const execution: AiToolExecution = {
    requestId: REQUEST_ID,
    toolName: "stage_run",
    args: { orgs: [] },
    result: { stage: "attack_candidate", passed: true },
    status: "completed",
    startedAt: "2026-07-30T00:00:00Z",
    completedAt: "2026-07-30T00:00:01Z",
    durationMs: 1000,
  };
  useStore.setState({
    sessions: {
      [SESSION_ID]: {
        id: SESSION_ID,
        name: "Candidate registry detail",
        workingDirectory: "/tmp/candidate-detail",
        createdAt: "2026-07-30T00:00:00Z",
        mode: "agent",
        detailViewMode: "tool-detail",
        toolDetailRequestIds: [REQUEST_ID],
        candidateReviewHint: {
          operationId: "session-global-hint-must-not-own-audit",
          waveRunId: "wave-candidate-1",
          status: "resume_pending",
          resumeVersion: 2,
          refreshVersion: 7,
        },
        stageRuns: {
          [REQUEST_ID]: {
            requestId: REQUEST_ID,
            stageLabel: "Attack Candidate",
            roleLabel: "Attack Analyst",
            coverageAxis: ["CANDIDATES"],
            rows: operationIds.map((operationId, index) => ({
              id: `org-candidate-${index + 1}`,
              operationId,
              name: `Candidate org ${index + 1}`,
              ownershipPercent: null,
              status: "passed" as const,
              evidenceCount: 1,
              coverage: {},
              stage: "attack_candidate",
            })),
            summary: {
              total: operationIds.length,
              covered: operationIds.length,
              active: 0,
              queued: 0,
              blocked: 0,
            },
          },
        },
      },
    },
    timelines: {
      [SESSION_ID]: [
        {
          id: `tool-exec-${REQUEST_ID}`,
          type: "ai_tool_execution",
          timestamp: "2026-07-30T00:00:00Z",
          data: execution,
        },
      ],
    },
    backgroundJobs: {},
  });
}

describe("ToolCallDetailView Candidate production entry", () => {
  beforeEach(() => {
    useStore.setState({ sessions: {}, timelines: {}, backgroundJobs: {} });
  });

  it("mounts review and Attempt DB views with the same exact refresh scope", () => {
    setSelectedCandidateStageRun();

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    const expectedProps = {
      operationId: "operation-candidate-1",
      waveRunId: "wave-candidate-1",
      refreshVersion: 7,
    };
    expect(screen.getByTestId("candidate-review-production-entry")).toHaveTextContent(
      JSON.stringify(expectedProps)
    );
    expect(screen.getByTestId("candidate-attempts-production-entry")).toHaveTextContent(
      JSON.stringify(expectedProps)
    );
  });

  it("shows the live Attack Analyst from progress when stage_run args contain only orgs", () => {
    setSelectedRunningCandidateStageRun();

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(screen.getByTestId("attack-candidate-stage-run")).toBeInTheDocument();
    expect(screen.getByText("Attack Analyst Agent")).toBeInTheDocument();
    expect(screen.getByText("Acme Root")).toBeInTheDocument();
    expect(screen.getByText("classifying 26 frozen work items")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /查看 Acme Root 的 Attack Analyst Agent 运行流/ }));

    expect(useStore.getState().sessions[SESSION_ID]?.detailViewMode).toBe("sub-agent-detail");
    expect(useStore.getState().sessions[SESSION_ID]?.toolDetailRequestIds).toEqual([
      REQUEST_ID,
      `${REQUEST_ID}::org::org-candidate-1`,
    ]);
  });

  it("does not mount the retired operation-only Registry Audit on Candidate rows", () => {
    setSelectedCandidateStageRunWithOperationIds(["operation-registry-1", "operation-registry-1"]);

    render(<ToolCallDetailView sessionId={SESSION_ID} />);

    expect(
      screen.queryByTestId("hypothesis-registry-audit-production-entry")
    ).not.toBeInTheDocument();
  });

  it.each([
    { operationIds: ["operation-registry-1", undefined], label: "missing" },
    {
      operationIds: ["operation-registry-1", "operation-registry-2"],
      label: "conflicting",
    },
  ] as const)(
    "does not mount Registry Audit for $label Candidate row identity",
    ({ operationIds }) => {
      setSelectedCandidateStageRunWithOperationIds([...operationIds]);

      render(<ToolCallDetailView sessionId={SESSION_ID} />);

      expect(
        screen.queryByTestId("hypothesis-registry-audit-production-entry")
      ).not.toBeInTheDocument();
    }
  );

  it("fails the pure Candidate audit helper closed without exact row authority", () => {
    expect(
      getCandidateStageRunOperationId(
        "stage_run",
        { orgs: [] },
        { stage: "attack_candidate" },
        [
          { stage: "attack_candidate", operationId: "operation-1" },
          { stage: "attack_candidate", operationId: " operation-1 " },
        ]
      )
    ).toBe("operation-1");
    expect(
      getCandidateStageRunOperationId(
        "stage_run",
        { stage: "verification" },
        { stage: "attack_candidate" },
        [{ stage: "attack_candidate", operationId: "operation-1" }]
      )
    ).toBeNull();
  });
});
