import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  type AttackVerificationQueueState,
  listVerificationQueue,
  resolveCandidateRecovery,
} from "@/lib/api/attack";
import { CandidateVerificationProtocol } from "./CandidateVerificationProtocol";

vi.mock("@/lib/api/attack", () => ({
  listVerificationQueue: vi.fn(),
  resolveCandidateRecovery: vi.fn(),
}));

const listQueue = vi.mocked(listVerificationQueue);
const resolveRecovery = vi.mocked(resolveCandidateRecovery);

function queue(): AttackVerificationQueueState {
  return {
    operationId: "operation-1",
    scopeSnapshotId: "snapshot-1",
    waveRunId: "wave-1",
    generation: 2,
    waveStatus: "verification",
    waveRowVersion: 7,
    waveUnits: [
      {
        waveUnitId: "wave-unit-1",
        organizationId: "organization-1",
        ordinal: 0,
        status: "verification",
        reviewClosed: true,
        verificationClosed: false,
        consolidationStatus: "pending",
        rowVersion: 4,
      },
    ],
    consolidation: null,
    pendingEnrichmentCount: 1,
    pendingEnrichments: [
      {
        enrichmentId: "enrichment-1",
        factDeltaId: "fact-delta-1",
        sourceAttemptId: "attempt-1",
        candidateId: "candidate-1",
        waveUnitId: "wave-unit-1",
        organizationId: "organization-1",
        subjectKind: "api_endpoint",
        subjectId: "endpoint-1",
        targetTypeAtTime: "url",
        targetValueAtTime: "https://example.test/api/current",
        deltaKind: "new_surface",
        observationKind: "surface_analysis_v2",
        allowedTechniques: ["GOLISH-NDAY", "WSTG-INPV-05"],
        enrichmentRequired: true,
        reasonCode: "typed_observation_required",
        status: "pending",
        createdAt: "2026-07-14T11:01:10Z",
      },
    ],
    items: [
      {
        attemptId: "attempt-1",
        candidateId: "candidate-1",
        approvalId: "approval-1",
        waveUnitId: "wave-unit-1",
        organizationId: "organization-1",
        targetLiveId: "target-1",
        targetTypeAtTime: "url",
        targetValueAtTime: "https://example.test/login",
        targetIdentityHash: "sha256:target",
        candidatePlanHash: "sha256:plan",
        hypothesis: "The login request may accept an unauthenticated replay.",
        technique: "WSTG-ATHN-04",
        planSchemaVersion: "candidate-plan-v2",
        recipeVersion: "candidate-recipe.anonymous-exact-replay-v2",
        executorContractVersion: "candidate-executor.anonymous-exact-replay-v2",
        budgetMaxActions: 1,
        budgetMaxRequests: 2,
        budgetMaxRuntimeMs: 5000,
        approvalStartBefore: "2026-07-14T12:00:00Z",
        approvalExpiresAt: "2026-07-14T12:05:00Z",
        ordinal: 1,
        status: "running",
        workerRunId: "worker-1",
        workerStatus: "recovery_required",
        rowVersion: 9,
        createdAt: "2026-07-14T11:00:00Z",
        updatedAt: "2026-07-14T11:01:00Z",
        terminalAt: null,
        observationEvidence: [{ evidenceId: 101, role: "observation" }],
        attemptEvidence: [{ evidenceId: 201, role: "proof" }],
        actions: [
          {
            actionId: "action-1",
            actionOrdinal: 0,
            capabilityId: "verification.anonymous_request_replay_v2",
            actionKind: "anonymous_request_replay",
            status: "outcome_unknown",
            outcomeHash: null,
            errorCode: "response_lost",
            startedAt: "2026-07-14T11:00:10Z",
            completedAt: null,
            authorizationReceiptId: "auth-receipt-1",
            authorizationRequestId: "auth-request-1",
            authorizationReceiptHash: "sha256:auth",
            authorizedAt: "2026-07-14T11:00:09Z",
            startBefore: "2026-07-14T12:00:00Z",
            executionDeadline: "2026-07-14T11:01:00Z",
          },
        ],
        terminalIntent: {
          intentId: "intent-1",
          requestId: "intent-request-1",
          toolCallRecordId: "tool-call-1",
          disposition: "verified",
          resultHash: "sha256:result",
          evidenceManifestHash: "sha256:evidence",
          evidenceCount: 1,
          intentHash: "sha256:intent",
          createdAt: "2026-07-14T11:01:00Z",
        },
        terminalBarrier: {
          barrierId: "barrier-1",
          intentId: "intent-1",
          requestId: "barrier-request-1",
          toolCallRecordId: "tool-call-1",
          createdAt: "2026-07-14T11:01:01Z",
        },
        terminalReceipt: null,
        recoveryCases: [
          {
            recoveryCaseId: "recovery-1",
            requestId: "case-request-1",
            attemptId: "attempt-1",
            actionId: "action-1",
            intentId: "intent-1",
            caseKind: "outcome_unknown",
            reasonCode: "provider_response_lost",
            attemptRowVersion: 9,
            status: "open",
            resolutionKind: null,
            resolutionRequestId: null,
            rowVersion: 3,
            evidenceIds: [301],
            decidedAt: null,
            completedAt: null,
            createdAt: "2026-07-14T11:01:10Z",
            updatedAt: "2026-07-14T11:01:10Z",
          },
        ],
      },
    ],
  };
}

describe("CandidateVerificationProtocol", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    listQueue.mockResolvedValue(queue());
    resolveRecovery.mockImplementation(async (request) => ({
      recoveryCaseId: request.recoveryCaseId,
      decisionRequestId: request.requestId,
      decision: request.decision,
      status: "decision_recorded",
      rowVersion: 4,
      replayed: false,
      pendingServerConvergence: true,
    }));
  });

  it("shows exact protocol ids and records a recovery CAS without claiming terminal state", async () => {
    render(
      <CandidateVerificationProtocol
        operationId="operation-1"
        waveRunId="wave-1"
        refreshVersion={0}
      />
    );

    expect(screen.getByText("Loading durable Verification queue…")).toBeInTheDocument();
    expect(await screen.findByText("Recovery outcome_unknown")).toBeInTheDocument();
    expect(screen.getByText("intent-request-1")).toBeInTheDocument();
    expect(screen.getByText("barrier-request-1")).toBeInTheDocument();
    expect(screen.getByText("auth-request-1")).toBeInTheDocument();
    expect(screen.getByText("evidence #301")).toHaveAttribute("href", "#evidence-301");
    expect(screen.getByText("row v3 · attempt v9")).toBeInTheDocument();

    fireEvent.change(
      screen.getByLabelText("Exact evidence IDs for external-result acceptance"),
      { target: { value: "401, 402" } }
    );
    fireEvent.click(screen.getByRole("button", { name: "Accept external result evidence" }));

    await waitFor(() => expect(resolveRecovery).toHaveBeenCalledTimes(1));
    expect(resolveRecovery).toHaveBeenCalledWith({
      operationId: "operation-1",
      waveRunId: "wave-1",
      recoveryCaseId: "recovery-1",
      requestId: expect.any(String),
      expectedRowVersion: 3,
      expectedAttemptRowVersion: 9,
      decision: "accept_external_result_with_exact_evidence",
      evidenceIds: [401, 402],
    });
    expect(
      await screen.findByText("Recovery decision recorded; pending server convergence.")
    ).toBeInTheDocument();
    expect(screen.queryByText("Recovery resolved.")).not.toBeInTheDocument();
  });

  it("shows a safe read-only pending enrichment without claiming a follow-on WorkItem", async () => {
    render(
      <CandidateVerificationProtocol
        operationId="operation-1"
        waveRunId="wave-1"
        refreshVersion={0}
      />
    );

    expect(await screen.findByText("FactDelta enrichment pending")).toBeInTheDocument();
    expect(screen.getByText("typed_observation_required")).toBeInTheDocument();
    expect(screen.getByText("api_endpoint · endpoint-1")).toBeInTheDocument();
    expect(screen.getByText("new_surface → surface_analysis_v2")).toBeInTheDocument();
    expect(screen.getByText("GOLISH-NDAY")).toBeInTheDocument();
    expect(screen.getByText("WSTG-INPV-05")).toBeInTheDocument();
    expect(
      screen.getByText("Source Wave remains open; no Candidate WorkItem has been created.")
    ).toBeInTheDocument();
    expect(
      screen.getByTestId("verification-pending-enrichment-enrichment-1").querySelector("button")
    ).toBeNull();
  });

  it("has explicit error and empty states", async () => {
    listQueue.mockRejectedValueOnce(new Error("queue DB unavailable"));
    const failed = render(
      <CandidateVerificationProtocol
        operationId="operation-1"
        waveRunId="wave-1"
        refreshVersion={0}
      />
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("queue DB unavailable");
    failed.unmount();

    listQueue.mockResolvedValueOnce({ ...queue(), items: [] });
    render(
      <CandidateVerificationProtocol
        operationId="operation-1"
        waveRunId="wave-1"
        refreshVersion={0}
      />
    );
    expect(
      await screen.findByText("No durable Candidate Verification queue items exist for this Wave.")
    ).toBeInTheDocument();
  });
});
