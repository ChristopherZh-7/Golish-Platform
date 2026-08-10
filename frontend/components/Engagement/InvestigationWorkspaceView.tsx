import { AlertTriangle, Bot, Circle, Loader2, OctagonX, Square, Workflow } from "lucide-react";
import { Fragment, type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "@/lib/utils";

export interface InvestigationStageIdentity {
  operationId: string;
  stageExecutionId: string | null;
  stageRunRequestId: string;
}

export type InvestigationActorKind =
  | "main"
  | "read_session"
  | "primary"
  | "worker"
  | "nested_worker"
  | "operator";

/**
 * Presentational projection only. The production IPC adapter must construct
 * this from ts-rs generated Task 8 DTOs; this type is never sent over IPC.
 */
export interface InvestigationActorNode {
  actorId: string;
  actorKind: InvestigationActorKind;
  label: string;
  organizationId: string;
  hypothesisRevisionId: string | null;
  taskId: string | null;
  subtaskId: string | null;
  workerRunId: string | null;
  owningStageRunRequestId: string;
  transcriptRequestId: string | null;
  parentActorTranscriptRequestId: string | null;
  parentDispatchToolRequestId: string | null;
  status: string;
  children: InvestigationActorNode[];
}

export interface InvestigationSubtaskNode {
  subtaskId: string;
  ordinal: number;
  label: string;
  status: string;
  workers: InvestigationActorNode[];
}

export interface InvestigationTaskNode {
  taskId: string;
  label: string;
  status: string;
  /** The shape permits one Primary by construction. */
  primary: InvestigationActorNode | null;
  subtasks: InvestigationSubtaskNode[];
  operator?: InvestigationActorNode | null;
}

export interface InvestigationOrganizationNode {
  organizationId: string;
  label: string;
  readSession: InvestigationActorNode | null;
  analysisTasks: InvestigationTaskNode[];
}

export interface InvestigationCampaignNode {
  campaignId: string;
  label: string;
  status: string;
}

export interface InvestigationHypothesisNode {
  revisionId: string;
  organizationId: string;
  claim: string;
  epistemicState: string;
  admissionDisposition: string;
  task: InvestigationTaskNode | null;
  campaigns: InvestigationCampaignNode[];
  evidenceRefs: string[];
  methodologyCitations: string[];
}

export interface InvestigationControlProjection {
  stageTopologyContract: string;
  investigationRunState: string;
  investigationRunStateHead: string;
  stopEpoch: number;
  stopAllowed: boolean;
  stopReason: string | null;
  resetAllowed: boolean;
  resetReason: string | null;
  forkAllowed: boolean;
  forkReason: string | null;
  adoptionContractVersion: number;
  controlPolicyVersion: number;
}

export interface InvestigationWorkspaceModel {
  identity: InvestigationStageIdentity & { stageExecutionId: string };
  projectionSchemaVersion: number;
  changeSeq: number;
  stale: boolean;
  stageStatus: string;
  allowPreparedActionJit: boolean;
  main: InvestigationActorNode | null;
  organizations: InvestigationOrganizationNode[];
  hypotheses: InvestigationHypothesisNode[];
  control: InvestigationControlProjection;
}

export interface InvestigationLiveActor {
  label: string;
  transcriptRequestId: string;
  owningStageRunRequestId: string;
  status: string;
}

export type InvestigationWorkspaceState =
  | { status: "loading"; message?: string }
  | { status: "error" | "unavailable"; message: string }
  | { status: "live-only"; actors: InvestigationLiveActor[]; message: string }
  | { status: "ready"; model: InvestigationWorkspaceModel }
  | { status: "stale"; model: InvestigationWorkspaceModel; message: string };

export type InvestigationWorkspaceSelection =
  | { kind: "agent"; actorId: string }
  | { kind: "hypothesis"; revisionId: string }
  | { kind: "campaign"; campaignId: string; revisionId: string };

export interface InvestigationWorkspaceViewProps {
  identity: InvestigationStageIdentity;
  state: InvestigationWorkspaceState;
  deepLinkTranscriptRequestId?: string | null;
  onBack?: () => void;
  onRetry?: () => void;
  onRequestStop?: (request: {
    identity: InvestigationStageIdentity & { stageExecutionId: string };
    expectedChangeSeq: number;
    expectedInvestigationRunStateHead: string;
  }) => void;
  onRequestReset?: () => void;
  onRequestSuccessorFork?: () => void;
  renderTranscript?: (identity: {
    transcriptRequestId: string;
    owningStageRunRequestId: string;
    actor: InvestigationActorNode;
  }) => ReactNode;
  renderPreparedActions?: (identity: { operationId: string; campaignId: string }) => ReactNode;
}

function sameIdentity(
  selected: InvestigationStageIdentity,
  projected: InvestigationWorkspaceModel["identity"]
): boolean {
  return (
    selected.operationId === projected.operationId &&
    selected.stageExecutionId === projected.stageExecutionId &&
    selected.stageRunRequestId === projected.stageRunRequestId
  );
}

function isUnifiedProjection(model: InvestigationWorkspaceModel): boolean {
  return model.control.stageTopologyContract === "unified_investigation_v1";
}

function statusTone(status: string): string {
  if (["completed", "verified", "refuted", "terminal"].includes(status)) {
    return "text-emerald-300";
  }
  if (status.includes("blocked") || status.includes("recovery") || status.includes("denied")) {
    return "text-amber-300";
  }
  if (status.includes("running") || status.includes("verifying")) return "text-cyan-300";
  return "text-muted-foreground";
}

function nestedActors(actor: InvestigationActorNode): InvestigationActorNode[] {
  return [actor, ...actor.children.flatMap(nestedActors)];
}

function taskActors(task: InvestigationTaskNode): InvestigationActorNode[] {
  return [
    ...(task.primary ? nestedActors(task.primary) : []),
    ...task.subtasks.flatMap((subtask) => subtask.workers.flatMap(nestedActors)),
  ];
}

function modelActors(model: InvestigationWorkspaceModel): InvestigationActorNode[] {
  return [
    ...(model.main ? nestedActors(model.main) : []),
    ...model.organizations.flatMap((organization) => [
      ...(organization.readSession ? nestedActors(organization.readSession) : []),
      ...organization.analysisTasks.flatMap(taskActors),
    ]),
    ...model.hypotheses.flatMap((hypothesis) =>
      hypothesis.task ? taskActors(hypothesis.task) : []
    ),
  ];
}

function initialSelection(
  model: InvestigationWorkspaceModel
): InvestigationWorkspaceSelection | null {
  if (model.main) return { kind: "agent", actorId: model.main.actorId };
  const firstHypothesis = model.hypotheses[0];
  if (firstHypothesis) return { kind: "hypothesis", revisionId: firstHypothesis.revisionId };
  const firstActor = modelActors(model)[0];
  return firstActor ? { kind: "agent", actorId: firstActor.actorId } : null;
}

function ActorTree({
  actor,
  depth,
  selection,
  onSelect,
}: {
  actor: InvestigationActorNode;
  depth: number;
  selection: InvestigationWorkspaceSelection | null;
  onSelect: (actor: InvestigationActorNode) => void;
}) {
  const selected = selection?.kind === "agent" && selection.actorId === actor.actorId;
  return (
    <Fragment>
      <button
        type="button"
        aria-current={selected ? "true" : undefined}
        aria-pressed={selected}
        aria-label={`View ${actor.label} transcript`}
        className={cn(
          "flex w-full items-center gap-1.5 rounded px-2 py-1.5 text-left text-[11px] outline-none transition-colors focus-visible:ring-1 focus-visible:ring-cyan-300",
          selected ? "bg-cyan-500/10 text-cyan-100" : "text-muted-foreground hover:bg-muted/30"
        )}
        style={{ paddingLeft: `${8 + depth * 12}px` }}
        onClick={() => onSelect(actor)}
      >
        {actor.actorKind === "operator" ? (
          <Square className="h-3 w-3 shrink-0 text-amber-300" />
        ) : (
          <Bot className="h-3 w-3 shrink-0" />
        )}
        <span className="min-w-0 flex-1 truncate">{actor.label}</span>
        <span className={cn("text-[9px]", statusTone(actor.status))}>{actor.status}</span>
      </button>
      {actor.children.map((child) => (
        <ActorTree
          key={child.actorId}
          actor={child}
          depth={depth + 1}
          selection={selection}
          onSelect={onSelect}
        />
      ))}
    </Fragment>
  );
}

function TaskTree({
  task,
  depth,
  selection,
  onSelectActor,
}: {
  task: InvestigationTaskNode;
  depth: number;
  selection: InvestigationWorkspaceSelection | null;
  onSelectActor: (actor: InvestigationActorNode) => void;
}) {
  return (
    <div className="space-y-0.5">
      <div
        className="flex items-center gap-1.5 px-2 py-1 text-[10px] font-medium text-foreground/75"
        style={{ paddingLeft: `${8 + depth * 12}px` }}
      >
        <Workflow className="h-3 w-3" />
        <span className="min-w-0 flex-1 truncate">{task.label}</span>
        <span className={statusTone(task.status)}>{task.status}</span>
      </div>
      {task.primary ? (
        <ActorTree
          actor={task.primary}
          depth={depth + 1}
          selection={selection}
          onSelect={onSelectActor}
        />
      ) : (
        <div
          role="status"
          className="px-2 py-1 text-[10px] text-muted-foreground"
          style={{ paddingLeft: `${8 + (depth + 1) * 12}px` }}
        >
          Primary starting — no Agent identity yet
        </div>
      )}
      {[...task.subtasks]
        .sort((left, right) => left.ordinal - right.ordinal)
        .map((subtask) => (
          <div key={subtask.subtaskId}>
            <div
              className="flex items-center gap-1.5 px-2 py-1 text-[10px] text-muted-foreground"
              style={{ paddingLeft: `${8 + (depth + 1) * 12}px` }}
            >
              <Circle className="h-2.5 w-2.5" />
              <span className="min-w-0 flex-1 truncate">
                {subtask.ordinal}. {subtask.label}
              </span>
              <span>{subtask.status}</span>
            </div>
            {subtask.workers.length === 0 ? (
              <div
                role="status"
                className="px-2 py-1 text-[10px] text-muted-foreground"
                style={{ paddingLeft: `${8 + (depth + 2) * 12}px` }}
              >
                Worker queued
              </div>
            ) : (
              subtask.workers.map((worker) => (
                <ActorTree
                  key={worker.actorId}
                  actor={worker}
                  depth={depth + 2}
                  selection={selection}
                  onSelect={onSelectActor}
                />
              ))
            )}
          </div>
        ))}
      {task.operator && (
        <div
          role="status"
          className="flex items-center gap-1.5 px-2 py-1.5 text-[11px] text-amber-200"
          style={{ paddingLeft: `${8 + (depth + 1) * 12}px` }}
        >
          <Square className="h-3 w-3 shrink-0" />
          <span className="min-w-0 flex-1 truncate">{task.operator.label}</span>
          <span className="text-[9px]">{task.operator.status}</span>
        </div>
      )}
    </div>
  );
}

function StatePanel({
  state,
  onRetry,
}: {
  state: Exclude<InvestigationWorkspaceState, { status: "ready" | "stale" }>;
  onRetry?: () => void;
}) {
  if (state.status === "loading") {
    return (
      <div
        role="status"
        aria-live="polite"
        className="flex flex-1 items-center justify-center gap-2 text-xs text-muted-foreground"
      >
        <Loader2 className="h-4 w-4 animate-spin" />
        {state.message ?? "Loading exact Investigation projection…"}
      </div>
    );
  }
  if (state.status === "live-only") {
    return (
      <div className="flex min-h-0 flex-1">
        <nav
          aria-label="Investigation agents and hypotheses"
          className="w-64 shrink-0 overflow-y-auto border-r border-border/25 p-2"
        >
          <div className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Live projection pending
          </div>
          {state.actors.map((actor) => (
            <div
              key={actor.transcriptRequestId}
              className="rounded px-2 py-1.5 text-[11px] text-foreground/80"
            >
              {actor.label} · {actor.status}
            </div>
          ))}
        </nav>
        <div
          role="status"
          aria-live="polite"
          className="flex flex-1 items-center justify-center p-6 text-center text-xs text-muted-foreground"
        >
          {state.message}
        </div>
      </div>
    );
  }
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <div
        role="alert"
        className="max-w-lg rounded border border-amber-500/30 bg-amber-500/[0.06] p-4 text-xs text-amber-100"
      >
        <div className="flex items-start gap-2">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <span>{state.message}</span>
        </div>
        {onRetry && (
          <button
            type="button"
            className="mt-3 rounded border border-amber-400/30 px-2 py-1"
            onClick={onRetry}
          >
            Retry exact read
          </button>
        )}
      </div>
    </div>
  );
}

export function InvestigationWorkspaceView({
  identity,
  state,
  deepLinkTranscriptRequestId = null,
  onBack,
  onRetry,
  onRequestStop,
  onRequestReset,
  onRequestSuccessorFork,
  renderTranscript,
  renderPreparedActions,
}: InvestigationWorkspaceViewProps) {
  const model = state.status === "ready" || state.status === "stale" ? state.model : null;
  const [selection, setSelection] = useState<InvestigationWorkspaceSelection | null>(() =>
    model ? initialSelection(model) : null
  );
  const deepLinkApplied = useRef(false);
  const identityKey = `${identity.operationId}:${identity.stageExecutionId ?? "pending"}:${identity.stageRunRequestId}`;
  const previousIdentityKey = useRef(identityKey);
  const actors = useMemo(() => (model ? modelActors(model) : []), [model]);

  useEffect(() => {
    if (previousIdentityKey.current === identityKey) return;
    previousIdentityKey.current = identityKey;
    deepLinkApplied.current = false;
    setSelection(model ? initialSelection(model) : null);
  }, [identityKey, model]);

  useEffect(() => {
    if (!model || selection || deepLinkApplied.current) return;
    setSelection(initialSelection(model));
  }, [model, selection]);

  useEffect(() => {
    if (!model || deepLinkApplied.current || !deepLinkTranscriptRequestId) return;
    deepLinkApplied.current = true;
    const matches = actors.filter(
      (actor) => actor.transcriptRequestId === deepLinkTranscriptRequestId
    );
    if (matches.length === 1) setSelection({ kind: "agent", actorId: matches[0].actorId });
  }, [actors, deepLinkTranscriptRequestId, model]);

  const selectedActor =
    selection?.kind === "agent"
      ? (actors.find((actor) => actor.actorId === selection.actorId) ?? null)
      : null;
  const selectedTranscriptIdentityCount = selectedActor?.transcriptRequestId
    ? actors.filter((actor) => actor.transcriptRequestId === selectedActor.transcriptRequestId)
        .length
    : 0;
  const selectedParentIdentityCount = selectedActor?.parentActorTranscriptRequestId
    ? actors.filter(
        (actor) => actor.transcriptRequestId === selectedActor.parentActorTranscriptRequestId
      ).length
    : 0;
  const selectedHypothesis =
    selection?.kind === "hypothesis" || selection?.kind === "campaign"
      ? (model?.hypotheses.find(
          (hypothesis) =>
            hypothesis.revisionId ===
            (selection.kind === "hypothesis" ? selection.revisionId : selection.revisionId)
        ) ?? null)
      : null;
  const selectedCampaign =
    selection?.kind === "campaign"
      ? (selectedHypothesis?.campaigns.find(
          (campaign) => campaign.campaignId === selection.campaignId
        ) ?? null)
      : null;

  return (
    <section
      className="flex h-full min-h-0 w-full flex-col bg-card"
      data-testid="investigation-workspace-view"
    >
      <header className="flex shrink-0 items-center gap-2 border-b border-border/25 px-3 py-2">
        {onBack && (
          <button
            type="button"
            className="rounded px-2 py-1 text-xs text-muted-foreground hover:bg-muted/30"
            onClick={onBack}
          >
            Back to timeline
          </button>
        )}
        <Workflow className="h-4 w-4 text-cyan-300" />
        <div className="min-w-0">
          <h2 className="text-xs font-semibold">Investigation</h2>
          <p className="truncate font-mono text-[9px] text-muted-foreground">
            {identity.operationId} · {identity.stageExecutionId ?? "execution pending"} ·{" "}
            {identity.stageRunRequestId}
          </p>
        </div>
        {model && (
          <div className="ml-auto flex items-center gap-2 text-[10px] text-muted-foreground">
            <span>{model.stageStatus}</span>
            <span>projection #{model.changeSeq}</span>
            <button
              type="button"
              aria-label="Stop Investigation"
              disabled={!model.control.stopAllowed || !onRequestStop}
              title={
                model.control.stopAllowed ? undefined : (model.control.stopReason ?? undefined)
              }
              className="flex items-center gap-1 rounded border border-red-500/30 px-2 py-1 text-red-300 disabled:cursor-not-allowed disabled:opacity-40"
              onClick={() =>
                onRequestStop?.({
                  identity: model.identity,
                  expectedChangeSeq: model.changeSeq,
                  expectedInvestigationRunStateHead: model.control.investigationRunStateHead,
                })
              }
            >
              <OctagonX className="h-3 w-3" />
              Stop Investigation
            </button>
            <button
              type="button"
              aria-label="Reset Investigation"
              disabled={!model.control.resetAllowed || !onRequestReset}
              title={
                model.control.resetAllowed
                  ? "Reset action is not registered on this direct route"
                  : (model.control.resetReason ?? undefined)
              }
              className="rounded border border-border/30 px-2 py-1 disabled:cursor-not-allowed disabled:opacity-40"
              onClick={onRequestReset}
            >
              Reset
            </button>
            <button
              type="button"
              aria-label="Fork Investigation successor"
              disabled={!model.control.forkAllowed || !onRequestSuccessorFork}
              title={
                model.control.forkAllowed
                  ? "Successor fork action is not registered on this direct route"
                  : (model.control.forkReason ?? undefined)
              }
              className="rounded border border-border/30 px-2 py-1 disabled:cursor-not-allowed disabled:opacity-40"
              onClick={onRequestSuccessorFork}
            >
              Fork successor
            </button>
          </div>
        )}
      </header>

      {state.status !== "ready" && state.status !== "stale" ? (
        <StatePanel state={state} onRetry={onRetry} />
      ) : !sameIdentity(identity, state.model.identity) || !isUnifiedProjection(state.model) ? (
        <div className="flex flex-1 items-center justify-center p-6">
          <div
            role="alert"
            className="max-w-lg rounded border border-red-500/30 bg-red-500/[0.05] p-3 text-xs text-red-200"
          >
            Investigation identity or topology conflict. The projection does not belong to this
            unified stage_run.
          </div>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <nav
            aria-label="Investigation agents and hypotheses"
            className="w-72 shrink-0 overflow-y-auto border-r border-border/25 p-2"
          >
            {!state.model.main && (
              <div
                role="alert"
                className="mb-2 flex items-start gap-2 rounded border border-amber-500/30 bg-amber-500/[0.05] p-2 text-[10px] text-amber-200"
              >
                <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
                Main identity unavailable. No synthetic Main was created.
              </div>
            )}
            {state.model.main && (
              <ActorTree
                actor={state.model.main}
                depth={0}
                selection={selection}
                onSelect={(actor) => setSelection({ kind: "agent", actorId: actor.actorId })}
              />
            )}
            {state.model.organizations.map((organization) => (
              <div
                key={organization.organizationId}
                className="mt-2 border-t border-border/20 pt-2"
              >
                <div className="px-2 py-1 text-[10px] font-semibold text-foreground/75">
                  {organization.label}
                </div>
                {organization.readSession ? (
                  <ActorTree
                    actor={organization.readSession}
                    depth={1}
                    selection={selection}
                    onSelect={(actor) => setSelection({ kind: "agent", actorId: actor.actorId })}
                  />
                ) : (
                  <div role="alert" className="px-5 py-1 text-[10px] text-amber-300">
                    Read session identity unavailable
                  </div>
                )}
                {organization.analysisTasks.map((task) => (
                  <TaskTree
                    key={task.taskId}
                    task={task}
                    depth={1}
                    selection={selection}
                    onSelectActor={(actor) =>
                      setSelection({ kind: "agent", actorId: actor.actorId })
                    }
                  />
                ))}
                <div className="mt-1 px-5 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                  Hypotheses
                </div>
                {state.model.hypotheses
                  .filter((hypothesis) => hypothesis.organizationId === organization.organizationId)
                  .map((hypothesis) => {
                    const selected =
                      (selection?.kind === "hypothesis" || selection?.kind === "campaign") &&
                      selection.revisionId === hypothesis.revisionId;
                    return (
                      <div key={hypothesis.revisionId}>
                        <button
                          type="button"
                          aria-current={selected ? "true" : undefined}
                          aria-pressed={selected}
                          aria-label={`View hypothesis ${hypothesis.claim}`}
                          className={cn(
                            "w-full rounded px-5 py-1.5 text-left text-[11px] outline-none focus-visible:ring-1 focus-visible:ring-cyan-300",
                            selected
                              ? "bg-violet-500/10 text-violet-100"
                              : "text-muted-foreground hover:bg-muted/30"
                          )}
                          onClick={() =>
                            setSelection({ kind: "hypothesis", revisionId: hypothesis.revisionId })
                          }
                        >
                          <span className="line-clamp-2">{hypothesis.claim}</span>
                          <span className="mt-0.5 block text-[9px]">
                            {hypothesis.epistemicState} · {hypothesis.admissionDisposition}
                          </span>
                        </button>
                        {hypothesis.task && (
                          <TaskTree
                            task={hypothesis.task}
                            depth={2}
                            selection={selection}
                            onSelectActor={(actor) =>
                              setSelection({ kind: "agent", actorId: actor.actorId })
                            }
                          />
                        )}
                      </div>
                    );
                  })}
              </div>
            ))}
          </nav>

          <main className="min-w-0 flex-1 overflow-y-auto p-4">
            {state.status === "stale" && (
              <div
                role="alert"
                className="mb-3 flex items-start gap-2 rounded border border-amber-500/30 bg-amber-500/[0.06] p-2 text-xs text-amber-200"
              >
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>{state.message}</span>
                {onRetry && (
                  <button type="button" className="ml-auto underline" onClick={onRetry}>
                    Bootstrap exact snapshot
                  </button>
                )}
              </div>
            )}

            <div role="status" aria-live="polite" className="sr-only">
              {selection?.kind ?? "none"} selection active
            </div>

            {state.model.hypotheses.length === 0 && (
              <div
                role="status"
                className="mb-3 rounded border border-border/30 bg-muted/10 p-3 text-xs text-muted-foreground"
              >
                No hypotheses were generated. This is an explicit empty analysis result, not a
                safety claim.
              </div>
            )}

            {selectedActor && (
              <section aria-label={`${selectedActor.label} transcript`} className="space-y-3">
                <div>
                  <h3 className="text-sm font-semibold">{selectedActor.label}</h3>
                  <p className="mt-1 font-mono text-[10px] text-muted-foreground">
                    {selectedActor.transcriptRequestId ?? "transcript identity unavailable"}
                  </p>
                </div>
                {selectedTranscriptIdentityCount > 1 ? (
                  <div
                    role="alert"
                    className="rounded border border-red-500/30 bg-red-500/[0.05] p-3 text-xs text-red-200"
                  >
                    Conflicting transcript identity: more than one actor claims this transcript.
                  </div>
                ) : selectedActor.parentActorTranscriptRequestId &&
                  (selectedParentIdentityCount !== 1 ||
                    !selectedActor.parentDispatchToolRequestId) ? (
                  <div
                    role="alert"
                    className="rounded border border-red-500/30 bg-red-500/[0.05] p-3 text-xs text-red-200"
                  >
                    Conflicting transcript identity: nested actor parent ownership is incomplete.
                  </div>
                ) : selectedActor.owningStageRunRequestId !== identity.stageRunRequestId ? (
                  <div
                    role="alert"
                    className="rounded border border-red-500/30 bg-red-500/[0.05] p-3 text-xs text-red-200"
                  >
                    Conflicting transcript identity: actor belongs to a different stage_run.
                  </div>
                ) : !selectedActor.transcriptRequestId ? (
                  <div
                    role="alert"
                    className="rounded border border-amber-500/30 bg-amber-500/[0.05] p-3 text-xs text-amber-200"
                  >
                    Transcript unavailable: this actor has no host-verified transcript identity.
                  </div>
                ) : renderTranscript ? (
                  renderTranscript({
                    transcriptRequestId: selectedActor.transcriptRequestId,
                    owningStageRunRequestId: selectedActor.owningStageRunRequestId,
                    actor: selectedActor,
                  })
                ) : (
                  <div
                    role="status"
                    className="rounded border border-border/30 p-3 text-xs text-muted-foreground"
                  >
                    Exact transcript renderer unavailable in this projection adapter.
                  </div>
                )}
              </section>
            )}

            {selectedHypothesis && (
              <section aria-label="Selected hypothesis" className="space-y-4">
                <div>
                  <div className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                    Hypothesis · view only
                  </div>
                  <h3 className="mt-1 text-sm font-semibold">{selectedHypothesis.claim}</h3>
                  <p className="mt-1 text-[11px] text-muted-foreground">
                    {selectedHypothesis.epistemicState} · {selectedHypothesis.admissionDisposition}
                  </p>
                </div>
                {selectedHypothesis.campaigns.length > 0 && (
                  <div>
                    <h4 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                      Campaigns
                    </h4>
                    <div className="mt-2 flex flex-wrap gap-2">
                      {selectedHypothesis.campaigns.map((campaign) => (
                        <button
                          key={campaign.campaignId}
                          type="button"
                          aria-current={
                            selectedCampaign?.campaignId === campaign.campaignId
                              ? "true"
                              : undefined
                          }
                          aria-pressed={selectedCampaign?.campaignId === campaign.campaignId}
                          className="rounded border border-border/30 px-2 py-1 text-xs hover:bg-muted/30"
                          onClick={() =>
                            setSelection({
                              kind: "campaign",
                              campaignId: campaign.campaignId,
                              revisionId: selectedHypothesis.revisionId,
                            })
                          }
                        >
                          {campaign.label}
                        </button>
                      ))}
                    </div>
                  </div>
                )}
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="rounded border border-border/25 p-3">
                    <h4 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                      Evidence
                    </h4>
                    <div className="mt-2 text-xs text-foreground/80">
                      {selectedHypothesis.evidenceRefs.length > 0
                        ? selectedHypothesis.evidenceRefs.join(", ")
                        : "No evidence references yet."}
                    </div>
                  </div>
                  <div className="rounded border border-border/25 p-3">
                    <h4 className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                      Methodology signals
                    </h4>
                    <div className="mt-2 text-xs text-foreground/80">
                      {selectedHypothesis.methodologyCitations.length > 0
                        ? selectedHypothesis.methodologyCitations.join(", ")
                        : "No methodology signal citations."}
                    </div>
                  </div>
                </div>
                {selectedCampaign &&
                  renderPreparedActions?.({
                    operationId: identity.operationId,
                    campaignId: selectedCampaign.campaignId,
                  })}
              </section>
            )}

            {!selectedActor && !selectedHypothesis && (
              <div className="flex min-h-48 items-center justify-center text-center text-xs text-muted-foreground">
                {state.model.hypotheses.length === 0
                  ? "No hypotheses were generated. This is an explicit empty analysis result, not a safety claim."
                  : "The selected projection item is unavailable at this exact head."}
              </div>
            )}
          </main>
        </div>
      )}
    </section>
  );
}
