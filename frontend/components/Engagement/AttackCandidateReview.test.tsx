import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  listCandidateReviews,
  resumeCandidateReview,
  reviewCandidates,
} from "@/lib/api/attack";
import { AttackCandidateReview } from "./AttackCandidateReview";

vi.mock("@/lib/api/attack", () => ({
  listCandidateReviews: vi.fn(),
  resumeCandidateReview: vi.fn(),
  reviewCandidates: vi.fn(),
}));

const mockedList = vi.mocked(listCandidateReviews);
const mockedReview = vi.mocked(reviewCandidates);
const mockedResume = vi.mocked(resumeCandidateReview);

function reviewState(overrides: Record<string, unknown> = {}) {
  return {
    operationId: "operation-1",
    projectScopeId: "project-1",
    scopeSnapshotId: "snapshot-1",
    waveRunId: "wave-1",
    profile: "active_authorized",
    reviewClosed: false,
    status: "open",
    resumeVersion: 0,
    lastError: null,
    waveUnitCount: 1,
    reviewClosedUnitCount: 0,
    candidateCount: 1,
    proposedCandidateCount: 1,
    candidates: [
      {
        candidateId: "candidate-alpha",
        waveUnitId: "unit-1",
        organizationId: "org-1",
        targetLiveId: null,
        liveTargetPresent: false,
        targetTypeAtTime: "domain",
        targetValueAtTime: "api.example.test",
        targetIdentityHash: "sha256:target-alpha",
        hypothesis: "The legacy endpoint accepts a risky action.",
        technique: "T1190",
        rationale: "Evidence-backed Candidate.",
        riskClass: "high",
        executionPlan: {
          actions: [
            {
              ordinal: 0,
              action_kind: "verify_legacy_endpoint",
              capability_id: "http-safe-check",
            },
          ],
          budget: { max_actions: 1, max_requests: 2, max_runtime_ms: 5000 },
        },
        candidatePlanHash: "sha256:alpha",
        disposition: "proposed",
        rowVersion: 4,
        latestApproval: null,
      },
    ],
    ...overrides,
  };
}

function renderReview(refreshVersion = 0) {
  return render(
    <AttackCandidateReview
      operationId="operation-1"
      waveRunId="wave-1"
      refreshVersion={refreshVersion}
    />
  );
}

describe("AttackCandidateReview", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("reloads an open review from DB without requiring a trace event", async () => {
    mockedList.mockResolvedValue(reviewState() as never);

    renderReview();

    expect(screen.getByText("Loading Candidate review…")).toBeInTheDocument();
    expect(await screen.findByText("api.example.test")).toBeInTheDocument();
    expect(mockedList).toHaveBeenCalledWith({
      operationId: "operation-1",
      waveRunId: "wave-1",
    });
    expect(screen.getByText("Live target removed · frozen identity")).toBeInTheDocument();
    expect(screen.getByText("verify_legacy_endpoint · http-safe-check")).toBeInTheDocument();
    expect(screen.getByText(/"max_requests":2/)).toBeInTheDocument();
  });

  it("submits only the exact Candidate plan hash and row version, then exposes resume", async () => {
    mockedList.mockResolvedValue(reviewState() as never);
    mockedReview.mockResolvedValue({
      state: reviewState({ reviewClosed: true, status: "resume_pending", resumeVersion: 1 }),
      replayed: false,
      approvalsWritten: 1,
    } as never);

    renderReview();
    await screen.findByText("api.example.test");
    fireEvent.click(screen.getByRole("radio", { name: "Approve api.example.test" }));
    fireEvent.click(screen.getByRole("button", { name: "Submit review" }));

    await waitFor(() => expect(mockedReview).toHaveBeenCalledTimes(1));
    expect(mockedReview.mock.calls[0][0]).toEqual({
      operationId: "operation-1",
      waveRunId: "wave-1",
      decisions: [
        {
          candidateId: "candidate-alpha",
          candidatePlanHash: "sha256:alpha",
          expectedRowVersion: 4,
          decision: "approve",
          expiresAt: expect.stringMatching(/^\d{4}-\d{2}-\d{2}T/),
        },
      ],
    });
    expect(await screen.findByRole("button", { name: "Resume verification" })).toBeInTheDocument();
  });

  it("keeps durable decisions visible when resume fails and permits an idempotent retry", async () => {
    const pending = reviewState({
      reviewClosed: true,
      status: "resume_pending",
      resumeVersion: 2,
      candidates: [
        {
          ...reviewState().candidates[0],
          disposition: "approved",
          rowVersion: 5,
          latestApproval: {
            approvalId: "approval-1",
            status: "approved",
            expiresAt: "2026-07-14T08:00:00Z",
            decisionVersion: 1,
            rowVersion: 0,
            decidedAt: "2026-07-13T08:00:00Z",
          },
        },
      ],
    });
    mockedList.mockResolvedValue(pending as never);
    mockedResume
      .mockRejectedValueOnce(new Error("resume dispatcher offline"))
      .mockResolvedValueOnce({ state: pending, replayed: true, approvalsWritten: 0 } as never);

    renderReview();
    await screen.findByText("Approved");
    fireEvent.click(screen.getByRole("button", { name: "Resume verification" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("resume dispatcher offline");
    expect(screen.getByText("Approved")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Resume verification" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Resume verification" }));
    await waitFor(() => expect(mockedResume).toHaveBeenCalledTimes(2));
    expect(mockedResume).toHaveBeenLastCalledWith({
      operationId: "operation-1",
      waveRunId: "wave-1",
      expectedResumeVersion: 2,
    });
  });

  it("renders explicit error and empty states", async () => {
    mockedList.mockRejectedValueOnce(new Error("DB unavailable"));
    const first = renderReview();
    expect(await screen.findByRole("alert")).toHaveTextContent("DB unavailable");
    first.unmount();

    mockedList.mockResolvedValueOnce(
      reviewState({ candidateCount: 0, proposedCandidateCount: 0, candidates: [] }) as never
    );
    renderReview();
    expect(await screen.findByText("No Candidates were recorded for this wave.")).toBeInTheDocument();
  });

  it("treats a trace hint as refresh-only and reads DB again", async () => {
    mockedList.mockResolvedValue(reviewState() as never);
    const view = renderReview(1);
    await screen.findByText("api.example.test");

    view.rerender(
      <AttackCandidateReview
        operationId="operation-1"
        waveRunId="wave-1"
        refreshVersion={2}
      />
    );
    await waitFor(() => expect(mockedList).toHaveBeenCalledTimes(2));
  });
});
