import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InvestigationCampaignDetailResponse } from "@/lib/api/investigation";
import type { InvestigationHypothesisDetailView } from "@/lib/generated/InvestigationHypothesisDetailView";
import type { InvestigationSummaryView } from "@/lib/generated/InvestigationSummaryView";
import { useStore } from "@/store";
import {
  InvestigationWorkspaceRoute,
  type InvestigationWorkspaceRouteApi,
} from "./InvestigationWorkspaceRoute";

const identity = {
  operationId: "operation-1",
  stageExecutionId: "execution-1",
  stageRunRequestId: "request-1",
};

const snapshot = {
  contractVersion: 1,
  asOfTemporalCutoff: "2026-08-08T00:00:00Z",
  authorityEpochSetHash: "epoch-10",
  earliestEffectiveValidUntil: "2026-08-08T01:00:00Z",
};

function envelope(changeSeq = 10) {
  return {
    projectionSchemaVersion: 2,
    changeSeq,
    readAt: "2026-08-08T00:00:01Z",
    temporalSnapshot: { ...snapshot, authorityEpochSetHash: `epoch-${changeSeq}` },
    toolTruthContract: "tool-truth-v1",
    investigationContractVersion: "investigation-v1",
    investigationRolloutMode: "unified",
    modePolicy: {
      canonicalWriter: "unified",
      gateAuthority: "unified",
      allowLegacyMutation: false,
      campaignWritePolicy: "jit",
      comparePolicy: "none",
      legacyProjectionPolicy: "read-only",
    },
    nextCursor: null,
  };
}

function actor(
  actorKind: string,
  transcriptRequestId: string,
  overrides: Record<string, unknown> = {}
) {
  return {
    actorKind,
    organizationId: "org-1",
    hypothesisRevisionId: null,
    taskId: null,
    subtaskId: null,
    workerRunId: `${actorKind}-run`,
    owningStageRunRequestId: identity.stageRunRequestId,
    transcriptRequestId,
    parentActorTranscriptRequestId: null,
    parentDispatchToolRequestId: null,
    status: "running",
    ...overrides,
  };
}

function retainedMainAliasSummary(): InvestigationSummaryView {
  const retained = summary();
  const main = retained.mainActor;
  const analysisPrimary = retained.actorTopology.find(
    (node) => node.actorKind === "primary" && node.hypothesisRevisionId === null
  );
  if (!analysisPrimary) throw new Error("fixture is missing Analysis Primary");
  Object.assign(analysisPrimary, {
    organizationId: main.organizationId,
    workerRunId: main.workerRunId,
    owningStageRunRequestId: main.owningStageRunRequestId,
    transcriptRequestId: main.transcriptRequestId,
    status: "passed",
  });
  for (const node of retained.actorTopology) {
    if (node.hypothesisRevisionId !== null) continue;
    node.status = "passed";
    if (node.parentActorTranscriptRequestId === "analysis-primary") {
      node.parentActorTranscriptRequestId = main.transcriptRequestId;
    }
  }
  retained.controlProjection = {
    ...retained.controlProjection,
    investigationRunState: "closed",
    stopAllowed: false,
    stopUnavailableReason: "run is closed",
  };
  return retained;
}

function summary(changeSeq = 10, controlChangeSeq = changeSeq): InvestigationSummaryView {
  return {
    envelope: envelope(changeSeq),
    controlProjection: {
      ...identity,
      stageTopologyContract: "unified_investigation_v1",
      investigationRunState: "verifying",
      investigationRunStateHead: `head-${controlChangeSeq}`,
      changeSeq: controlChangeSeq,
      stopEpoch: 0,
      stopAllowed: true,
      stopUnavailableReason: null,
      resetAllowed: false,
      resetUnavailableReason: "active run",
      successorForkAllowed: false,
      successorForkUnavailableReason: "active run",
      adoptionContractVersion: 1,
      controlPolicyVersion: 1,
    },
    activeGenerationId: "generation-1",
    activeGenerationSealHash: "seal-1",
    currentHypothesisCount: 1,
    closedHypothesisCount: 0,
    contestedHypothesisCount: 0,
    residualCount: 0,
    generationCount: 1,
    waveCount: 1,
    campaignCount: 1,
    openObligationCount: 0,
    generations: [{ generationId: "generation-1", generationOrdinal: 1, state: "active" }],
    waves: [{ waveId: "wave-1", waveOrdinal: 1, state: "active" }],
    openObligations: [],
    sourceCensus: [
      {
        organizationId: "org-1",
        snapshotId: "snapshot-1",
        contextItemCount: 3,
        contextItemSetSha256: "context-hash",
        methodologyHitCount: 1,
        methodologyResultSetSha256: "method-hash",
        omissionCount: 0,
        omissionSetSha256: "omission-hash",
      },
    ],
    mainActor: actor("main", "main-transcript"),
    actorTopology: [
      actor("primary", "analysis-primary"),
      actor("worker", "analysis-worker", {
        subtaskId: "analysis-subtask-1",
        workerRunId: "analysis-worker-run",
        parentActorTranscriptRequestId: "analysis-primary",
        parentDispatchToolRequestId: "dispatch-analysis-worker",
      }),
      actor("nested_worker", "nested-worker", {
        subtaskId: "analysis-subtask-1",
        workerRunId: "nested-worker-run",
        parentActorTranscriptRequestId: "analysis-worker",
        parentDispatchToolRequestId: "dispatch-nested-worker",
      }),
      actor("primary", "verification-primary", {
        hypothesisRevisionId: "revision-1",
        taskId: "verification-task-1",
        workerRunId: "verification-primary-run",
      }),
      actor("worker", "verification-worker", {
        hypothesisRevisionId: "revision-1",
        taskId: "verification-task-1",
        subtaskId: "verification-subtask-1",
        workerRunId: "verification-worker-run",
        parentActorTranscriptRequestId: "verification-primary",
        parentDispatchToolRequestId: "dispatch-verification-worker",
      }),
    ],
    coverageDenominator: {
      planned: 1,
      testedComplete: 0,
      testedDegraded: 0,
      untested: 1,
      blocked: 0,
    },
    coverageSufficiency: "incomplete",
    authorityTimeMembers: [],
    controlDecision: "continue",
    coverageGrade: "partial",
  };
}

function hypothesisDetail(): InvestigationHypothesisDetailView {
  return {
    envelope: envelope(),
    hypothesis: {
      rootId: "root-1",
      revisionId: "revision-1",
      organizationId: "org-1",
      subjectKind: "endpoint",
      subjectIdentityHash: "subject-hash",
      targetTypeAtTime: "url",
      targetValueAtTime: "https://acme.test",
      predicateSchema: "predicate-v1",
      predicateSummary: "Acme endpoint may expose an authorization bypass",
      trustBoundary: "public",
      polarity: "positive",
      epistemicState: "supported",
      lifecycleState: "active",
      planningReadiness: "scheduled",
      supportCount: 1,
      contradictionCount: 0,
      gapCount: 0,
      legacyProjectionStatus: null,
      residualCodes: [],
    },
    predecessorRevisionId: null,
    lineageRevisionIds: [],
    supportRefIds: ["evidence-1"],
    contradictionRefIds: [],
    applicationContextRefIds: [],
    gapRefIds: [],
    verificationObjectiveSummaries: ["Attempt falsification"],
    actorTopology: summary().actorTopology.filter(
      (node) => node.hypothesisRevisionId === "revision-1"
    ),
    legacyUnavailableFields: [],
  };
}

function campaignDetail(): InvestigationCampaignDetailResponse {
  return {
    envelope: envelope(),
    campaign: {
      campaignId: "campaign-1",
      hypothesisRevisionId: "revision-1",
      waveOrdinal: 1,
      campaignOrdinal: 1,
      state: "awaiting_authorization",
      coverageStatus: "open",
      roundIds: [],
      preparedActionIds: ["prepared-action-1"],
      authorizedActionCount: 0,
      blockedActionCount: 0,
      openResidualIds: [],
      redactedRoundSummaries: [],
      authorityTime: {
        observedAsOf: "2026-08-08T00:00:00Z",
        effectiveValidUntil: null,
        authorityEpochHash: "epoch-10",
        temporalStatus: "current",
      },
    },
  };
}

function api(): InvestigationWorkspaceRouteApi {
  return {
    getSummary: vi.fn(async (request) => summary(request.expectedChangeSeq ?? 10)),
    listHypotheses: vi.fn(async () => ({
      envelope: envelope(),
      hypotheses: [hypothesisDetail().hypothesis],
    })),
    getHypothesis: vi.fn(async () => hypothesisDetail()),
    listCampaigns: vi.fn(async () => ({
      envelope: envelope(),
      campaigns: [
        {
          campaignId: "campaign-1",
          waveOrdinal: 1,
          campaignOrdinal: 1,
          label: "Campaign 1",
          state: "awaiting_authorization",
          coverageStatus: "open",
          authorityTime: campaignDetail().campaign.authorityTime,
        },
      ],
    })),
    getCampaign: vi.fn(async () => campaignDetail()),
    listTimeline: vi.fn(async () => ({ envelope: envelope(), events: [] })),
    requestStop: vi.fn(async (request) => ({
      stopIntentId: "stop-1",
      idempotencyKey: request.idempotencyKey,
      stopEpoch: 1,
      frozenWorkCount: 1,
      frozenWorkSetSha256: "frozen-hash",
      receiptSha256: "receipt-hash",
      controlProjection: { ...summary().controlProjection, stopAllowed: false, stopEpoch: 1 },
    })),
  };
}

function installStore() {
  useStore.setState({
    sessions: {
      "session-1": {
        id: "session-1",
        name: "Investigation",
        workingDirectory: "/tmp/investigation",
        createdAt: "2026-08-08T00:00:00Z",
        mode: "agent",
        detailViewMode: "tool-detail",
        toolDetailRequestIds: [identity.stageRunRequestId],
        stageRuns: {
          [identity.stageRunRequestId]: {
            requestId: identity.stageRunRequestId,
            stageLabel: "Investigation",
            roleLabel: "Actor",
            coverageAxis: [],
            summary: { total: 1, covered: 0, active: 1, queued: 0, blocked: 0 },
            rows: [
              {
                id: "org-1",
                operationId: identity.operationId,
                stageExecutionId: identity.stageExecutionId,
                stageRunUnitId: "unit-1",
                name: "Acme",
                ownershipPercent: null,
                status: "running",
                evidenceCount: 0,
                coverage: {},
                stage: "investigation",
                agentRequestId: "org-read-transcript",
              },
            ],
          },
        },
      },
    },
    activeSubAgents: {
      "session-1": [
        {
          agentId: "agent-1",
          agentName: "Nested worker",
          parentRequestId: "nested-worker",
          task: "Inspect endpoint",
          depth: 2,
          status: "completed",
          toolCalls: [],
          entries: [{ kind: "text", text: "Exact transcript evidence" }],
          startedAt: "2026-08-08T00:00:00Z",
        },
      ],
    },
  });
}

describe("InvestigationWorkspaceRoute", () => {
  beforeEach(installStore);

  it("bootstraps without a sequence then binds every continuation and detail to its exact snapshot", async () => {
    const routeApi = api();
    render(
      <InvestigationWorkspaceRoute
        sessionId="session-1"
        identity={identity}
        liveRows={useStore.getState().sessions["session-1"].stageRuns?.["request-1"]?.rows ?? []}
        api={routeApi}
      />
    );

    expect(screen.getByRole("status")).toHaveTextContent("Loading exact Investigation projection");
    expect(await screen.findByText("Analysis Primary")).toBeInTheDocument();
    expect(screen.getByText("Bounded read session")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Bounded read session/ })
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Verification Primary")).not.toBeInTheDocument();

    expect(routeApi.getSummary).toHaveBeenCalledWith({
      sessionId: "session-1",
      ...identity,
      expectedChangeSeq: null,
      expectedTemporalCutoff: null,
      expectedAuthorityEpochSetHash: null,
      expectedEarliestEffectiveValidUntil: null,
    });
    const exactSnapshot = {
      expectedChangeSeq: 10,
      expectedTemporalCutoff: snapshot.asOfTemporalCutoff,
      expectedAuthorityEpochSetHash: "epoch-10",
      expectedEarliestEffectiveValidUntil: snapshot.earliestEffectiveValidUntil,
    };
    expect(routeApi.listHypotheses).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "session-1", ...identity, ...exactSnapshot })
    );
    expect(routeApi.listCampaigns).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "session-1", ...identity, ...exactSnapshot })
    );
    expect(routeApi.getHypothesis).toHaveBeenCalledWith(
      expect.objectContaining({ revisionId: "revision-1", ...exactSnapshot })
    );
    expect(routeApi.getCampaign).toHaveBeenCalledWith(
      expect.objectContaining({ campaignId: "campaign-1", ...exactSnapshot })
    );
    expect(routeApi.listTimeline).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "session-1", ...identity, ...exactSnapshot })
    );

    fireEvent.click(screen.getByRole("button", { name: /Nested worker/ }));
    expect(screen.getByText("Exact transcript evidence")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /Acme endpoint may expose/ }));
    expect(screen.getByText("Verification Primary")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Campaign 1" }));
    expect(screen.getByRole("button", { name: "Campaign 1" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
  });

  it("keeps API authority on the AI session while reading live projections from the pane session", async () => {
    const routeApi = api();
    render(
      <InvestigationWorkspaceRoute
        sessionId="ai-session-1"
        presentationSessionId="session-1"
        identity={identity}
        liveRows={useStore.getState().sessions["session-1"].stageRuns?.["request-1"]?.rows ?? []}
        api={routeApi}
      />
    );

    expect(await screen.findByText("Analysis Primary")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Nested worker/ }));
    expect(screen.getByText("Exact transcript evidence")).toBeInTheDocument();
    expect(routeApi.getSummary).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "ai-session-1" })
    );

    act(() => {
      useStore.getState().setInvestigationRefreshHint("session-1", {
        ...identity,
        changeSeq: 12,
      });
    });
    await waitFor(() => expect(routeApi.getSummary).toHaveBeenCalledTimes(2));
  });

  it("canonicalizes the exact retained Main alias and treats passed workers as terminal", async () => {
    const routeApi = api();
    const canonicalHypothesis = hypothesisDetail();
    canonicalHypothesis.hypothesis.predicateSchema =
      "typed.verification.license_api_authz_probe_request";
    canonicalHypothesis.hypothesis.predicateSummary =
      'typed.verification.license_api_authz_probe_request@1:{"auth_states":["anonymous"]}';
    vi.mocked(routeApi.getSummary).mockResolvedValue(retainedMainAliasSummary());
    vi.mocked(routeApi.getHypothesis).mockResolvedValue(canonicalHypothesis);
    render(
      <InvestigationWorkspaceRoute
        sessionId="session-1"
        identity={identity}
        liveRows={useStore.getState().sessions["session-1"].stageRuns?.["request-1"]?.rows ?? []}
        api={routeApi}
      />
    );

    const main = await screen.findByRole("button", { name: /View Main transcript/ });
    expect(screen.queryByRole("button", { name: /View Analysis Primary transcript/ })).not.toBeInTheDocument();
    expect(screen.getByText("Handled by Main")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /license api authz probe request/i })
    ).toBeInTheDocument();
    expect(screen.getAllByText("passed").length).toBeGreaterThan(1);
    expect(screen.getByText("1. Subtask 1").parentElement).toHaveTextContent("passed");
    fireEvent.click(main);
    expect(screen.queryByText(/more than one actor claims this transcript/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop Investigation" })).not.toBeInTheDocument();
  });

  it("renders ordered compact Agent activity without repeating a machine submit response", async () => {
    const routeApi = api();
    useStore.setState((state) => ({
      activeSubAgents: {
        ...state.activeSubAgents,
        "session-1": [
          {
            agentId: "agent-activity",
            agentName: "Nested worker",
            parentRequestId: "nested-worker",
            task: "Inspect endpoint",
            depth: 2,
            status: "completed",
            toolCalls: [
              {
                id: "plan-1",
                name: "update_plan",
                args: { plan: [{ step: "Verify boundary", status: "completed" }] },
                status: "completed",
                result: { ok: true },
                startedAt: "2026-08-08T00:00:00Z",
              },
              {
                id: "submit-1",
                name: "submit_result",
                args: { result: { schema_version: 1 } },
                status: "completed",
                result: { accepted_output_sha256: "machine-only-secret" },
                startedAt: "2026-08-08T00:00:01Z",
              },
            ],
            entries: [
              { kind: "text", text: "Readable conclusion" },
              { kind: "tool_call", toolCallId: "plan-1" },
              { kind: "tool_call", toolCallId: "submit-1" },
            ],
            response: '{"accepted_output_sha256":"machine-only-secret"}',
            startedAt: "2026-08-08T00:00:00Z",
          },
        ],
      },
    }));
    render(
      <InvestigationWorkspaceRoute
        sessionId="session-1"
        identity={identity}
        liveRows={useStore.getState().sessions["session-1"].stageRuns?.["request-1"]?.rows ?? []}
        api={routeApi}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: /Nested worker/ }));
    expect(screen.getByText("Readable conclusion")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Controller plan" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /submit result/i })).toBeInTheDocument();
    expect(screen.queryByText(/machine-only-secret/)).not.toBeInTheDocument();
  });

  it("keeps projection and run-control sequences distinct and stops only at the server head", async () => {
    const routeApi = api();
    vi.mocked(routeApi.getSummary).mockResolvedValue(summary(10, 2));
    render(
      <InvestigationWorkspaceRoute
        sessionId="session-1"
        identity={identity}
        liveRows={useStore.getState().sessions["session-1"].stageRuns?.["request-1"]?.rows ?? []}
        api={routeApi}
      />
    );
    await screen.findByText("Analysis Primary");

    fireEvent.click(screen.getByRole("button", { name: "Stop Investigation" }));
    await waitFor(() => expect(routeApi.requestStop).toHaveBeenCalledTimes(1));
    expect(routeApi.requestStop).toHaveBeenCalledWith({
      sessionId: "session-1",
      ...identity,
      expectedInvestigationRunStateHead: "head-2",
      expectedChangeSeq: 2,
      idempotencyKey: expect.any(String),
    });

    act(() => {
      useStore.getState().setInvestigationRefreshHint("session-1", {
        ...identity,
        changeSeq: 12,
      });
    });
    await waitFor(() => expect(routeApi.getSummary).toHaveBeenCalledTimes(2));
    expect(routeApi.getSummary).toHaveBeenLastCalledWith({
      sessionId: "session-1",
      ...identity,
      expectedChangeSeq: null,
      expectedTemporalCutoff: null,
      expectedAuthorityEpochSetHash: null,
      expectedEarliestEffectiveValidUntil: null,
    });
  });

  it("keeps campaign selection view-only", async () => {
    const routeApi = api();
    render(
      <InvestigationWorkspaceRoute
        sessionId="session-1"
        identity={identity}
        liveRows={useStore.getState().sessions["session-1"].stageRuns?.["request-1"]?.rows ?? []}
        api={routeApi}
      />
    );
    await screen.findByText("Analysis Primary");
    fireEvent.click(screen.getByRole("button", { name: /Acme endpoint may expose/ }));
    fireEvent.click(screen.getByRole("button", { name: "Campaign 1" }));

    expect(screen.getByRole("button", { name: "Campaign 1" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
  });

  it("recovers a stale stop with one no-sequence bootstrap and never retries against latest", async () => {
    const routeApi = api();
    vi.mocked(routeApi.requestStop).mockRejectedValue({
      code: "INVESTIGATION_CONTROL_STALE",
      message: "run head changed",
    });
    render(
      <InvestigationWorkspaceRoute
        sessionId="session-1"
        identity={identity}
        liveRows={useStore.getState().sessions["session-1"].stageRuns?.["request-1"]?.rows ?? []}
        api={routeApi}
      />
    );
    await screen.findByText("Analysis Primary");
    fireEvent.click(screen.getByRole("button", { name: "Stop Investigation" }));

    await waitFor(() => expect(routeApi.getSummary).toHaveBeenCalledTimes(2));
    expect(routeApi.requestStop).toHaveBeenCalledTimes(1);
    expect(routeApi.getSummary).toHaveBeenLastCalledWith(
      expect.objectContaining({
        ...identity,
        expectedChangeSeq: null,
        expectedTemporalCutoff: null,
      })
    );
  });
});
