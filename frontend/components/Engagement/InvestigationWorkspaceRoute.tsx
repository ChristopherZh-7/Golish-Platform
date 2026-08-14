import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getInvestigationHypothesis,
  getInvestigationSummary,
  investigationGetCampaign,
  investigationListCampaigns,
  investigationListTimeline,
  investigationRequestStop,
  listInvestigationHypotheses,
} from "@/lib/api/investigation";
import type { InvestigationActorTopologyNodeView } from "@/lib/generated/InvestigationActorTopologyNodeView";
import type { InvestigationCampaignDetailResponse } from "@/lib/generated/InvestigationCampaignDetailResponse";
import type { InvestigationCampaignPageResponse } from "@/lib/generated/InvestigationCampaignPageResponse";
import type { InvestigationControlProjectionV1 } from "@/lib/generated/InvestigationControlProjectionV1";
import type { InvestigationHypothesisDetailView } from "@/lib/generated/InvestigationHypothesisDetailView";
import type { InvestigationHypothesisListView } from "@/lib/generated/InvestigationHypothesisListView";
import type { InvestigationProjectionEnvelope } from "@/lib/generated/InvestigationProjectionEnvelope";
import type { InvestigationRequestStopRequest } from "@/lib/generated/InvestigationRequestStopRequest";
import type { InvestigationRequestStopResponse } from "@/lib/generated/InvestigationRequestStopResponse";
import type { InvestigationScopeRequest } from "@/lib/generated/InvestigationScopeRequest";
import type { InvestigationSummaryView } from "@/lib/generated/InvestigationSummaryView";
import type { InvestigationTimelinePageResponse } from "@/lib/generated/InvestigationTimelinePageResponse";
import type { ActiveSubAgent } from "@/store";
import { useStore } from "@/store";
import { AgentPlanCard, resolveLatestVisibleAgentPlanRequest } from "./AgentPlanCard";
import { AgentTranscriptMessage, isLiveAgentThinkingEntry } from "./AgentTranscriptMessage";
import {
  type InvestigationActorKind,
  type InvestigationActorNode,
  type InvestigationCampaignNode,
  type InvestigationHypothesisNode,
  type InvestigationOrganizationNode,
  type InvestigationStageIdentity,
  type InvestigationSubtaskNode,
  type InvestigationTaskNode,
  type InvestigationWorkspaceModel,
  type InvestigationWorkspaceState,
  InvestigationWorkspaceView,
} from "./InvestigationWorkspaceView";
import type { StageRunRow } from "./StageRunOrgRows";
import { ToolActivityGroup } from "./ToolActivityDisclosure";

const PAGE_SIZE = 100;
const TRANSCRIPT_RENDER_LIMIT = 200;
const EMPTY_ACTIVE_SUB_AGENTS: ActiveSubAgent[] = [];
const STALE_CODES = new Set([
  "INVESTIGATION_PROJECTION_STALE",
  "INVESTIGATION_TEMPORAL_SNAPSHOT_STALE",
  "INVESTIGATION_CONTROL_STALE",
]);

export interface InvestigationWorkspaceRouteApi {
  getSummary: typeof getInvestigationSummary;
  listHypotheses: typeof listInvestigationHypotheses;
  getHypothesis: typeof getInvestigationHypothesis;
  listCampaigns: typeof investigationListCampaigns;
  getCampaign: typeof investigationGetCampaign;
  listTimeline: typeof investigationListTimeline;
  requestStop: typeof investigationRequestStop;
}

const productionApi: InvestigationWorkspaceRouteApi = {
  getSummary: getInvestigationSummary,
  listHypotheses: listInvestigationHypotheses,
  getHypothesis: getInvestigationHypothesis,
  listCampaigns: investigationListCampaigns,
  getCampaign: investigationGetCampaign,
  listTimeline: investigationListTimeline,
  requestStop: investigationRequestStop,
};

export interface InvestigationWorkspaceRouteProps {
  /** Canonical AI session used by every authorized backend read and mutation. */
  sessionId: string;
  /** Pane session that owns live transcript projections and refresh hints. */
  presentationSessionId?: string;
  identity: InvestigationStageIdentity & { stageExecutionId: string };
  displayStageRunRequestId?: string | null;
  liveRows: readonly StageRunRow[];
  deepLinkTranscriptRequestId?: string | null;
  onBack?: () => void;
  api?: InvestigationWorkspaceRouteApi;
}

interface ExactSnapshotExpectation {
  expectedChangeSeq: number;
  expectedTemporalCutoff: string;
  expectedAuthorityEpochSetHash: string;
  expectedEarliestEffectiveValidUntil: string;
}

interface LoadedProjection {
  summary: InvestigationSummaryView;
  hypotheses: InvestigationHypothesisDetailView[];
  campaigns: InvestigationCampaignDetailResponse[];
}

function errorCode(error: unknown): string | null {
  if (typeof error !== "object" || error === null || !("code" in error)) return null;
  return typeof error.code === "string" ? error.code : null;
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null && "message" in error) {
    const message = error.message;
    if (typeof message === "string") return message;
  }
  return String(error);
}

function bootstrapRequest(
  sessionId: string,
  identity: InvestigationWorkspaceRouteProps["identity"]
): InvestigationScopeRequest {
  return {
    sessionId,
    ...identity,
    expectedChangeSeq: null,
    expectedTemporalCutoff: null,
    expectedAuthorityEpochSetHash: null,
    expectedEarliestEffectiveValidUntil: null,
  };
}

function exactExpectation(envelope: InvestigationProjectionEnvelope): ExactSnapshotExpectation {
  return {
    expectedChangeSeq: envelope.changeSeq,
    expectedTemporalCutoff: envelope.temporalSnapshot.asOfTemporalCutoff,
    expectedAuthorityEpochSetHash: envelope.temporalSnapshot.authorityEpochSetHash,
    expectedEarliestEffectiveValidUntil: envelope.temporalSnapshot.earliestEffectiveValidUntil,
  };
}

function sameSnapshot(
  envelope: InvestigationProjectionEnvelope,
  expected: ExactSnapshotExpectation
): boolean {
  return (
    envelope.changeSeq === expected.expectedChangeSeq &&
    envelope.temporalSnapshot.asOfTemporalCutoff === expected.expectedTemporalCutoff &&
    envelope.temporalSnapshot.authorityEpochSetHash === expected.expectedAuthorityEpochSetHash &&
    envelope.temporalSnapshot.earliestEffectiveValidUntil ===
      expected.expectedEarliestEffectiveValidUntil
  );
}

function assertExactEnvelope(
  envelope: InvestigationProjectionEnvelope,
  expected: ExactSnapshotExpectation
): void {
  if (!sameSnapshot(envelope, expected)) {
    throw new Error("Investigation continuation returned a conflicting exact snapshot.");
  }
}

function assertSummaryIdentity(
  summary: InvestigationSummaryView,
  identity: InvestigationWorkspaceRouteProps["identity"]
): void {
  const control = summary.controlProjection;
  if (
    control.operationId !== identity.operationId ||
    control.stageExecutionId !== identity.stageExecutionId ||
    control.stageRunRequestId !== identity.stageRunRequestId
  ) {
    throw new Error("Investigation summary returned a conflicting exact stage identity.");
  }
}

function actorKind(value: string): InvestigationActorKind {
  if (value === "main" || value === "primary" || value === "worker" || value === "nested_worker") {
    return value;
  }
  throw new Error(`Unsupported Investigation actor kind: ${value}`);
}

function actorLabel(actor: InvestigationActorTopologyNodeView): string {
  if (actor.actorKind === "main") return "Main";
  if (actor.actorKind === "primary") {
    return actor.hypothesisRevisionId ? "Verification Primary" : "Analysis Primary";
  }
  if (actor.actorKind === "nested_worker") return "Nested worker";
  return actor.hypothesisRevisionId ? "Verification worker" : "Dynamic worker";
}

function actorNode(actor: InvestigationActorTopologyNodeView): InvestigationActorNode {
  return {
    actorId: actor.transcriptRequestId,
    actorKind: actorKind(actor.actorKind),
    label: actorLabel(actor),
    organizationId: actor.organizationId,
    hypothesisRevisionId: actor.hypothesisRevisionId,
    taskId: actor.taskId,
    subtaskId: actor.subtaskId,
    workerRunId: actor.workerRunId,
    owningStageRunRequestId: actor.owningStageRunRequestId,
    transcriptRequestId: actor.transcriptRequestId,
    parentActorTranscriptRequestId: actor.parentActorTranscriptRequestId,
    parentDispatchToolRequestId: actor.parentDispatchToolRequestId,
    status: actor.status,
    children: [],
  };
}

function actorForest(
  rows: readonly InvestigationActorTopologyNodeView[]
): InvestigationActorNode[] {
  const byTranscript = new Map<string, InvestigationActorNode>();
  for (const row of rows) {
    if (!row.transcriptRequestId.trim() || byTranscript.has(row.transcriptRequestId)) {
      throw new Error("Investigation actor topology has a conflicting transcript identity.");
    }
    byTranscript.set(row.transcriptRequestId, actorNode(row));
  }

  const roots: InvestigationActorNode[] = [];
  for (const node of byTranscript.values()) {
    if (!node.parentActorTranscriptRequestId) {
      roots.push(node);
      continue;
    }
    const parent = byTranscript.get(node.parentActorTranscriptRequestId);
    if (!parent || !node.parentDispatchToolRequestId) {
      throw new Error("Investigation nested actor parent ownership is incomplete.");
    }
    parent.children.push(node);
  }
  return roots;
}

function subtaskNodes(primary: InvestigationActorNode): InvestigationSubtaskNode[] {
  const grouped = new Map<string, InvestigationActorNode[]>();
  for (const worker of primary.children) {
    const subtaskId = worker.subtaskId;
    if (!subtaskId) {
      throw new Error("Investigation worker is missing its exact subtask identity.");
    }
    const workers = grouped.get(subtaskId) ?? [];
    workers.push(worker);
    grouped.set(subtaskId, workers);
  }
  return [...grouped.entries()].map(([subtaskId, workers], index) => {
    const statuses = workers.map((worker) => worker.status);
    const status = statuses.every((value) => value === "completed" || value === "passed")
      ? "passed"
      : statuses.some(
            (value) =>
              value.includes("blocked") ||
              value === "failed" ||
              value === "error" ||
              value === "interrupted" ||
              value === "exhausted"
          )
        ? "blocked"
        : "running";
    return {
      subtaskId,
      ordinal: index + 1,
      label: `Subtask ${index + 1}`,
      status,
      workers,
    };
  });
}

function taskFromPrimary(
  primary: InvestigationActorNode,
  taskLabel: "Analysis Task" | "Verification Task",
  primaryAliasLabel: string | null = null
): InvestigationTaskNode {
  const subtasks = subtaskNodes(primary);
  return {
    taskId: primary.taskId ?? primary.workerRunId ?? primary.transcriptRequestId ?? primary.actorId,
    label: taskLabel,
    status: primary.status,
    primary: primaryAliasLabel ? null : { ...primary, children: [] },
    primaryAliasLabel,
    subtasks,
  };
}

function exactMainAlias(main: InvestigationActorNode, primary: InvestigationActorNode): boolean {
  return (
    primary.actorKind === "primary" &&
    primary.hypothesisRevisionId === null &&
    primary.workerRunId === main.workerRunId &&
    primary.transcriptRequestId === main.transcriptRequestId &&
    primary.organizationId === main.organizationId &&
    primary.owningStageRunRequestId === main.owningStageRunRequestId
  );
}

function hypothesisLabel(predicateSummary: string, predicateSchema: string): string {
  const canonicalBoundary = predicateSummary.indexOf(":");
  if (
    canonicalBoundary < 0 ||
    !predicateSummary
      .slice(canonicalBoundary + 1)
      .trimStart()
      .startsWith("{")
  ) {
    return predicateSummary;
  }
  const stableSchema = predicateSchema.trim() || predicateSummary.slice(0, canonicalBoundary);
  return stableSchema
    .replace(/^typed(?:[._-]+verification)?[._-]*/i, "")
    .replace(/[._-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function readSessionForOrganization(
  organizationId: string,
  identity: InvestigationWorkspaceRouteProps["identity"],
  liveRows: readonly StageRunRow[]
): InvestigationActorNode | null {
  const matches = liveRows.filter(
    (row) => row.id === organizationId && Boolean(row.agentRequestId?.trim())
  );
  if (matches.length !== 1) return null;
  const row = matches[0];
  return {
    actorId: `read:${row.agentRequestId}`,
    actorKind: "read_session",
    label: "Bounded read session",
    organizationId,
    hypothesisRevisionId: null,
    taskId: null,
    subtaskId: null,
    workerRunId: row.stageRunUnitId ?? null,
    owningStageRunRequestId: identity.stageRunRequestId,
    transcriptRequestId: row.agentRequestId?.trim() ?? null,
    parentActorTranscriptRequestId: null,
    parentDispatchToolRequestId: null,
    status: row.status,
    children: [],
  };
}

function controlProjection(control: InvestigationControlProjectionV1) {
  return {
    stageTopologyContract: control.stageTopologyContract,
    investigationRunState: control.investigationRunState,
    investigationRunStateHead: control.investigationRunStateHead,
    changeSeq: control.changeSeq,
    stopEpoch: control.stopEpoch,
    stopAllowed: control.stopAllowed,
    stopReason: control.stopUnavailableReason,
    resetAllowed: control.resetAllowed,
    resetReason: control.resetUnavailableReason,
    forkAllowed: control.successorForkAllowed,
    forkReason: control.successorForkUnavailableReason,
    adoptionContractVersion: control.adoptionContractVersion,
    controlPolicyVersion: control.controlPolicyVersion,
  };
}

function workspaceModel(
  identity: InvestigationWorkspaceRouteProps["identity"],
  liveRows: readonly StageRunRow[],
  loaded: LoadedProjection
): InvestigationWorkspaceModel {
  const roots = actorForest(loaded.summary.actorTopology);
  const main = actorNode(loaded.summary.mainActor);
  const organizations: InvestigationOrganizationNode[] = loaded.summary.sourceCensus.map(
    (source) => {
      const analysisPrimaries = roots.filter(
        (root) =>
          root.actorKind === "primary" &&
          root.organizationId === source.organizationId &&
          root.hypothesisRevisionId === null
      );
      return {
        organizationId: source.organizationId,
        label:
          liveRows.find((row) => row.id === source.organizationId)?.name ??
          `Organization ${source.organizationId}`,
        readSession: readSessionForOrganization(source.organizationId, identity, liveRows),
        analysisTasks: analysisPrimaries.map((primary) =>
          taskFromPrimary(
            primary,
            "Analysis Task",
            exactMainAlias(main, primary) ? "Handled by Main" : null
          )
        ),
      };
    }
  );

  const campaignByHypothesis = new Map<string, InvestigationCampaignNode[]>();
  for (const detail of loaded.campaigns) {
    const campaign = detail.campaign;
    const items = campaignByHypothesis.get(campaign.hypothesisRevisionId) ?? [];
    items.push({
      campaignId: campaign.campaignId,
      label: `Campaign ${campaign.campaignOrdinal}`,
      status: campaign.state,
    });
    campaignByHypothesis.set(campaign.hypothesisRevisionId, items);
  }

  const hypotheses: InvestigationHypothesisNode[] = loaded.hypotheses.map((detail) => {
    const hypothesis = detail.hypothesis;
    const detailRoots = actorForest(detail.actorTopology);
    const primary = detailRoots.find(
      (root) => root.actorKind === "primary" && root.hypothesisRevisionId === hypothesis.revisionId
    );
    const task = primary ? taskFromPrimary(primary, "Verification Task") : null;
    if (task) {
      task.operator = {
        actorId: `operator:${hypothesis.revisionId}`,
        actorKind: "operator",
        label: "Typed Operator",
        organizationId: hypothesis.organizationId,
        hypothesisRevisionId: hypothesis.revisionId,
        taskId: task.taskId,
        subtaskId: null,
        workerRunId: null,
        owningStageRunRequestId: identity.stageRunRequestId,
        transcriptRequestId: null,
        parentActorTranscriptRequestId: null,
        parentDispatchToolRequestId: null,
        status: "view-only control boundary",
        children: [],
      };
    }
    return {
      revisionId: hypothesis.revisionId,
      organizationId: hypothesis.organizationId,
      claim: hypothesisLabel(hypothesis.predicateSummary, hypothesis.predicateSchema),
      exactPredicate: hypothesis.predicateSummary,
      epistemicState: hypothesis.epistemicState,
      admissionDisposition: hypothesis.planningReadiness,
      task,
      campaigns: campaignByHypothesis.get(hypothesis.revisionId) ?? [],
      evidenceRefs: [
        ...detail.supportRefIds,
        ...detail.contradictionRefIds,
        ...detail.applicationContextRefIds,
        ...detail.gapRefIds,
      ],
      methodologyCitations: [],
    };
  });

  return {
    identity,
    projectionSchemaVersion: loaded.summary.envelope.projectionSchemaVersion,
    changeSeq: loaded.summary.envelope.changeSeq,
    stale: false,
    stageStatus: loaded.summary.controlProjection.investigationRunState,
    main,
    organizations,
    hypotheses,
    control: controlProjection(loaded.summary.controlProjection),
  };
}

function TranscriptProjection({ agents }: { agents: readonly ActiveSubAgent[] }) {
  if (agents.length !== 1) {
    return (
      <div role="alert" className="rounded border border-amber-500/30 p-3 text-xs">
        {agents.length === 0
          ? "Transcript unavailable: the exact actor transcript was not restored."
          : "Transcript unavailable: the actor transcript identity is ambiguous."}
      </div>
    );
  }
  const agent = agents[0];
  const currentPlanId = resolveLatestVisibleAgentPlanRequest(agent.toolCalls, {
    entries: agent.entries,
  })?.id;
  const toolsById = new Map(agent.toolCalls.map((tool) => [tool.id, tool]));
  const omittedEntryCount = Math.max(0, agent.entries.length - TRANSCRIPT_RENDER_LIMIT);
  const visibleEntries = agent.entries.slice(-TRANSCRIPT_RENDER_LIMIT);
  const visibleResponse = agent.response?.trim();
  const hasTerminalSubmit = agent.toolCalls.some(
    (tool) =>
      tool.name === "submit_result" &&
      (tool.status === "completed" || tool.status === "error" || tool.status === "interrupted")
  );
  const responseIsMachinePayload =
    hasTerminalSubmit && (visibleResponse?.startsWith("{") || visibleResponse?.startsWith("["));
  return (
    <div className="overflow-hidden rounded border border-border/30 bg-card/35 text-xs">
      {omittedEntryCount > 0 && (
        <div role="status" className="border-b border-border/20 px-4 py-2 text-muted-foreground">
          {omittedEntryCount} older transcript entries are hidden for performance.
        </div>
      )}
      {visibleEntries.map((entry, index) => {
        if (entry.kind === "tool_call") {
          if (!entry.toolCallId) return null;
          const tool = toolsById.get(entry.toolCallId);
          if (!tool) return null;
          if (tool.name === "update_plan") {
            return tool.id === currentPlanId ? (
              <AgentPlanCard
                key={tool.id}
                tool={tool}
                parentStagePassed={agent.status === "completed"}
              />
            ) : null;
          }
          return <ToolActivityGroup key={tool.id} tools={[tool]} actorLabel={agent.agentName} />;
        }
        if (!entry.text?.trim()) return null;
        return (
          <AgentTranscriptMessage
            key={`${entry.kind}-${index}`}
            kind={entry.kind}
            actorLabel={agent.agentName}
            text={entry.text}
            startedAt={entry.startedAt}
            endedAt={entry.endedAt}
            thinkingActive={isLiveAgentThinkingEntry(agent, entry)}
          />
        );
      })}
      {visibleResponse &&
        !responseIsMachinePayload &&
        !agent.entries.some(
          (entry) => entry.kind === "text" && entry.text?.trim() === visibleResponse
        ) && (
          <AgentTranscriptMessage kind="text" actorLabel={agent.agentName} text={visibleResponse} />
        )}
    </div>
  );
}

async function loadExactProjection(
  api: InvestigationWorkspaceRouteApi,
  sessionId: string,
  identity: InvestigationWorkspaceRouteProps["identity"]
): Promise<LoadedProjection> {
  const summary = await api.getSummary(bootstrapRequest(sessionId, identity));
  assertSummaryIdentity(summary, identity);
  const expected = exactExpectation(summary.envelope);
  const common = { sessionId, ...identity, ...expected };
  const [hypothesisPage, campaignPage, timelinePage]: [
    InvestigationHypothesisListView,
    InvestigationCampaignPageResponse,
    InvestigationTimelinePageResponse,
  ] = await Promise.all([
    api.listHypotheses({
      ...common,
      organizationIds: [],
      epistemicStates: [],
      readinessStates: [],
      capabilityStates: [],
      sourceKinds: [],
      cursor: null,
      pageSize: PAGE_SIZE,
    }),
    api.listCampaigns({
      ...common,
      waveIds: [],
      campaignStates: [],
      cursor: null,
      pageSize: PAGE_SIZE,
    }),
    api.listTimeline({
      ...common,
      eventKinds: [],
      cursor: null,
      pageSize: PAGE_SIZE,
    }),
  ]);
  assertExactEnvelope(hypothesisPage.envelope, expected);
  assertExactEnvelope(campaignPage.envelope, expected);
  assertExactEnvelope(timelinePage.envelope, expected);

  const [hypotheses, campaigns] = await Promise.all([
    Promise.all(
      hypothesisPage.hypotheses.map((hypothesis) =>
        api.getHypothesis({ ...common, revisionId: hypothesis.revisionId })
      )
    ),
    Promise.all(
      campaignPage.campaigns.map((campaign) =>
        api.getCampaign({ ...common, campaignId: campaign.campaignId })
      )
    ),
  ]);
  for (const detail of hypotheses) assertExactEnvelope(detail.envelope, expected);
  for (const detail of campaigns) assertExactEnvelope(detail.envelope, expected);
  return { summary, hypotheses, campaigns };
}

export function InvestigationWorkspaceRoute({
  sessionId,
  presentationSessionId = sessionId,
  identity,
  displayStageRunRequestId = null,
  liveRows,
  deepLinkTranscriptRequestId = null,
  onBack,
  api = productionApi,
}: InvestigationWorkspaceRouteProps) {
  const [state, setState] = useState<InvestigationWorkspaceState>({ status: "loading" });
  const generation = useRef(0);
  const stopKeys = useRef(new Map<string, string>());
  const consumedRefreshHint = useRef<string | null>(null);
  const refreshHint = useStore(
    (store) => store.sessions[presentationSessionId]?.investigationRefreshHint
  );
  const activeSubAgents = useStore(
    (store) => store.activeSubAgents[presentationSessionId] ?? EMPTY_ACTIVE_SUB_AGENTS
  );

  const bootstrap = useCallback(async () => {
    const requestGeneration = ++generation.current;
    setState((current) =>
      current.status === "ready" || current.status === "stale"
        ? {
            status: "stale",
            model: current.model,
            message: "Recovering the exact Investigation projection from a fresh bootstrap…",
          }
        : { status: "loading" }
    );
    try {
      const loaded = await loadExactProjection(api, sessionId, identity);
      if (generation.current !== requestGeneration) return;
      setState({ status: "ready", model: workspaceModel(identity, liveRows, loaded) });
    } catch (error) {
      if (generation.current !== requestGeneration) return;
      const message = errorMessage(error);
      setState((current) =>
        current.status === "ready" || current.status === "stale"
          ? { status: "stale", model: current.model, message }
          : { status: "error", message }
      );
    }
  }, [api, identity, liveRows, sessionId]);

  useEffect(() => {
    void bootstrap();
    return () => {
      generation.current += 1;
    };
  }, [bootstrap]);

  useEffect(() => {
    if (!refreshHint) return;
    if (
      refreshHint.operationId !== identity.operationId ||
      refreshHint.stageExecutionId !== identity.stageExecutionId ||
      refreshHint.stageRunRequestId !== identity.stageRunRequestId
    ) {
      return;
    }
    const loadedSeq =
      state.status === "ready" || state.status === "stale" ? state.model.changeSeq : null;
    const hintKey = `${refreshHint.operationId}:${refreshHint.stageExecutionId}:${refreshHint.stageRunRequestId}:${refreshHint.changeSeq}`;
    if (
      loadedSeq !== null &&
      refreshHint.changeSeq > loadedSeq &&
      consumedRefreshHint.current !== hintKey
    ) {
      consumedRefreshHint.current = hintKey;
      void bootstrap();
    }
  }, [bootstrap, identity, refreshHint, state]);

  const requestStop = useCallback(
    async ({
      identity: projectedIdentity,
      expectedChangeSeq,
      expectedInvestigationRunStateHead,
    }: {
      identity: InvestigationWorkspaceRouteProps["identity"];
      expectedChangeSeq: number;
      expectedInvestigationRunStateHead: string;
    }) => {
      const keyId = `${projectedIdentity.stageRunRequestId}:${expectedInvestigationRunStateHead}`;
      const idempotencyKey =
        stopKeys.current.get(keyId) ??
        globalThis.crypto?.randomUUID?.() ??
        `investigation-stop-${expectedChangeSeq}`;
      stopKeys.current.set(keyId, idempotencyKey);
      const request: InvestigationRequestStopRequest = {
        sessionId,
        ...projectedIdentity,
        expectedInvestigationRunStateHead,
        expectedChangeSeq,
        idempotencyKey,
      };
      try {
        const response: InvestigationRequestStopResponse = await api.requestStop(request);
        stopKeys.current.delete(keyId);
        setState((current) => {
          if (current.status !== "ready" && current.status !== "stale") return current;
          const control = response.controlProjection;
          if (
            control.operationId !== projectedIdentity.operationId ||
            control.stageExecutionId !== projectedIdentity.stageExecutionId ||
            control.stageRunRequestId !== projectedIdentity.stageRunRequestId
          ) {
            return {
              status: "stale",
              model: current.model,
              message: "Stop response returned a conflicting exact control projection.",
            };
          }
          return {
            status: "ready",
            model: {
              ...current.model,
              stageStatus: control.investigationRunState,
              control: controlProjection(control),
            },
          };
        });
      } catch (error) {
        if (STALE_CODES.has(errorCode(error) ?? "")) {
          void bootstrap();
          return;
        }
        setState((current) =>
          current.status === "ready" || current.status === "stale"
            ? { status: "stale", model: current.model, message: errorMessage(error) }
            : current
        );
      }
    },
    [api, bootstrap, sessionId]
  );

  const transcriptByRequest = useMemo(() => {
    const byRequest = new Map<string, ActiveSubAgent[]>();
    for (const agent of activeSubAgents) {
      const matches = byRequest.get(agent.parentRequestId) ?? [];
      matches.push(agent);
      byRequest.set(agent.parentRequestId, matches);
    }
    return byRequest;
  }, [activeSubAgents]);

  return (
    <InvestigationWorkspaceView
      identity={identity}
      displayStageRunRequestId={displayStageRunRequestId}
      state={state}
      deepLinkTranscriptRequestId={deepLinkTranscriptRequestId}
      onBack={onBack}
      onRetry={() => void bootstrap()}
      onRequestStop={(request) => void requestStop(request)}
      renderTranscript={({ transcriptRequestId }) => (
        <TranscriptProjection agents={transcriptByRequest.get(transcriptRequestId) ?? []} />
      )}
    />
  );
}
