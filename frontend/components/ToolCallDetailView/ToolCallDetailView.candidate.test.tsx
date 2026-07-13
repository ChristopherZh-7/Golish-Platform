import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { type AiToolExecution, useStore } from "@/store";
import { ToolCallDetailView } from "./ToolCallDetailView";

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

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const SESSION_ID = "candidate-detail-session";
const REQUEST_ID = "candidate-stage-run-request";

function setSelectedCandidateStageRun() {
  const execution: AiToolExecution = {
    requestId: REQUEST_ID,
    toolName: "stage_run",
    args: { stage: "attack_candidate" },
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
});
