import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type CandidateAttemptRow,
  listCandidateAttempts,
} from "@/lib/api/attack";
import { CandidateAttemptRows } from "./CandidateAttemptRows";

vi.mock("@/lib/api/attack", () => ({
  listCandidateAttempts: vi.fn(),
}));

const listAttempts = vi.mocked(listCandidateAttempts);

function attempt(overrides: Partial<CandidateAttemptRow> = {}): CandidateAttemptRow {
  return {
    attemptId: "attempt-1",
    candidateId: "candidate-1",
    approvalId: "approval-1",
    organizationId: "organization-1",
    targetLiveId: "target-1",
    targetTypeAtTime: "url",
    targetValueAtTime: "https://api.example.test/login",
    targetIdentityHash: "sha256:target-1",
    candidatePlanHash: "sha256:plan-1",
    ordinal: 1,
    status: "queued",
    stageWorkerRunId: null,
    result: null,
    resultHash: null,
    rowVersion: 0,
    createdAt: "2026-07-13T08:00:00Z",
    updatedAt: "2026-07-13T08:00:00Z",
    terminalAt: null,
    ...overrides,
  };
}

function renderAttempts(refreshVersion = 0) {
  return render(
    <CandidateAttemptRows
      operationId="operation-1"
      waveRunId="wave-1"
      refreshVersion={refreshVersion}
    />
  );
}

describe("CandidateAttemptRows", () => {
  beforeEach(() => {
    vi.resetAllMocks();
  });

  it("renders the exact ordinal, terminal status, evidence roles, and Finding lineage", async () => {
    listAttempts.mockResolvedValue([
      attempt({
        ordinal: 2,
        status: "verified",
        rowVersion: 4,
        terminalAt: "2026-07-13T08:05:00Z",
        resultHash: "sha256:result-1",
        result: {
          attempt_id: "attempt-1",
          candidate_plan_hash: "sha256:plan-1",
          disposition: "verified",
          proof_evidence_ids: [41, 42],
          refutation_evidence_ids: [],
          blocker_evidence_ids: [],
          blocker_reason_code: null,
          finding: {
            title: "Verified SQL injection",
            severity: "high",
          },
          fact_deltas: [],
        },
      }),
    ]);

    renderAttempts();

    expect(screen.getByText("Loading Candidate attempts…")).toBeInTheDocument();
    expect(await screen.findByText("Attempt 2")).toBeInTheDocument();
    expect(screen.getByText("Verified")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Proof evidence #41" })).toHaveAttribute(
      "href",
      "#evidence-41"
    );
    expect(screen.getByRole("link", { name: "Proof evidence #42" })).toBeInTheDocument();
    expect(screen.getByText("Finding lineage")).toBeInTheDocument();
    expect(screen.getByText("Verified SQL injection")).toBeInTheDocument();
    expect(screen.getByText(/candidate-1.*attempt-1.*Finding/)).toBeInTheDocument();
    expect(listAttempts).toHaveBeenCalledWith({
      operationId: "operation-1",
      waveRunId: "wave-1",
    });
  });

  it("shows a blocked residual without presenting the Candidate as verified", async () => {
    listAttempts.mockResolvedValue([
      attempt({
        status: "blocked",
        terminalAt: "2026-07-13T08:05:00Z",
        result: {
          disposition: "blocked",
          proof_evidence_ids: [],
          refutation_evidence_ids: [],
          blocker_evidence_ids: [55],
          blocker_reason_code: "approval_expired",
          finding: null,
          fact_deltas: [],
        },
      }),
    ]);

    renderAttempts();

    expect(await screen.findByText("Blocked")).toBeInTheDocument();
    expect(screen.getByText("Residual risk remains")).toBeInTheDocument();
    expect(screen.getByText("approval_expired")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Blocker evidence #55" })).toBeInTheDocument();
    expect(screen.queryByText("Finding lineage")).not.toBeInTheDocument();
    expect(screen.queryByText("Verified Finding")).not.toBeInTheDocument();
  });

  it("shows one active exploit lane and a queued next Candidate", async () => {
    listAttempts.mockResolvedValue([
      attempt({ attemptId: "attempt-active", candidateId: "candidate-active", status: "running" }),
      attempt({
        attemptId: "attempt-queued",
        candidateId: "candidate-queued",
        targetValueAtTime: "https://api.example.test/admin",
        status: "queued",
      }),
    ]);

    renderAttempts();

    expect(await screen.findByText("1 active · 1 queued")).toBeInTheDocument();
    expect(screen.getByTestId("attempt-attempt-active")).toHaveTextContent("Exploit lane active");
    expect(screen.getByTestId("attempt-attempt-queued")).toHaveTextContent("Queued");
  });

  it("reloads DB truth after a missed terminal trace is represented by a refresh hint", async () => {
    listAttempts
      .mockResolvedValueOnce([attempt({ status: "running" })])
      .mockResolvedValueOnce([
        attempt({
          status: "refuted",
          terminalAt: "2026-07-13T08:05:00Z",
          result: {
            disposition: "refuted",
            proof_evidence_ids: [],
            refutation_evidence_ids: [77],
            blocker_evidence_ids: [],
            blocker_reason_code: null,
            finding: null,
            fact_deltas: [],
          },
        }),
      ]);
    const view = renderAttempts(1);
    expect(await screen.findByText("Running")).toBeInTheDocument();

    view.rerender(
      <CandidateAttemptRows
        operationId="operation-1"
        waveRunId="wave-1"
        refreshVersion={2}
      />
    );

    await waitFor(() => expect(listAttempts).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("Refuted")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Refutation evidence #77" })).toBeInTheDocument();
  });

  it("renders explicit error and empty states", async () => {
    listAttempts.mockRejectedValueOnce(new Error("DB unavailable"));
    const first = renderAttempts();
    expect(await screen.findByRole("alert")).toHaveTextContent("DB unavailable");
    first.unmount();

    listAttempts.mockResolvedValueOnce([]);
    renderAttempts();
    expect(
      await screen.findByText("No Candidate verification attempts were recorded for this wave.")
    ).toBeInTheDocument();
  });
});
