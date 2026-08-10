import {
  AlertTriangle,
  ArrowLeft,
  Bot,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  Clock3,
  FileCheck2,
  Loader2,
  Wrench,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import {
  AgentPlanCard,
  resolveLatestVisibleAgentPlanRequest,
} from "@/components/Engagement/AgentPlanCard";
import type { StageAssetCoverageSnapshot } from "@/lib/api/stage-coverage";
import type { StageTeamReadModel } from "@/lib/api/stage-team";
import { safeStringify } from "@/lib/text";
import { cn } from "@/lib/utils";
import type { ActiveSubAgent, SubAgentEntry, SubAgentToolCall } from "@/store";
import { AgentTranscriptMessage, isLiveAgentThinkingEntry } from "./AgentTranscriptMessage";
import { StageRunDetailShell } from "./StageRunDetailShell";
import { useTranscriptAutoScroll } from "./useTranscriptAutoScroll";

type StageTeamUnit = StageTeamReadModel["units"][number];
type StageTeamPlan = NonNullable<StageTeamUnit["plan"]>;
type StageTeamItem = StageTeamPlan["workItems"][number];
type StageTeamWorker = StageTeamItem["workers"][number];

type ActorState = "queued" | "running" | "waiting" | "completed" | "blocked";

interface StageActor {
  id: string;
  unit: StageTeamUnit;
  item: StageTeamItem | null;
  worker: StageTeamWorker | null;
  requestId: string | null;
  activity: ActiveSubAgent | null;
  label: string;
  scopeLabel: string | null;
  state: ActorState;
  stateLabel: string;
  controller: boolean;
}

export interface StageTeamWorkspaceViewProps {
  model: StageTeamReadModel;
  children?: ReactNode;
  agentActivities?: readonly ActiveSubAgent[];
  agentRequestIdsByWorker?: Readonly<Record<string, string>>;
  focusedAgentRequestId?: string | null;
  coverageByOrg?: Readonly<Record<string, StageAssetCoverageSnapshot>>;
}

const STATE_DOT: Record<ActorState, string> = {
  queued: "bg-indigo-300",
  running: "bg-sky-300 shadow-[0_0_0_3px_rgba(125,211,252,0.12)]",
  waiting: "bg-amber-300",
  completed: "bg-emerald-300",
  blocked: "bg-rose-300",
};

const STATE_BADGE: Record<ActorState, string> = {
  queued: "border-indigo-400/25 bg-indigo-400/10 text-indigo-200",
  running: "border-sky-400/25 bg-sky-400/10 text-sky-200",
  waiting: "border-amber-400/25 bg-amber-400/10 text-amber-200",
  completed: "border-emerald-400/25 bg-emerald-400/10 text-emerald-200",
  blocked: "border-rose-400/25 bg-rose-400/10 text-rose-200",
};

export const STAGE_TRANSCRIPT_RENDER_LIMIT = 200;

export function boundedTranscriptEntries(
  entries: readonly SubAgentEntry[],
  currentPlanToolId: string | null,
  limit = STAGE_TRANSCRIPT_RENDER_LIMIT
): { entries: SubAgentEntry[]; omitted: number } {
  if (entries.length <= limit) return { entries: [...entries], omitted: 0 };
  const tail = entries.slice(-limit);
  const planEntry = currentPlanToolId
    ? entries.find((entry) => entry.kind === "tool_call" && entry.toolCallId === currentPlanToolId)
    : undefined;
  if (planEntry && !tail.includes(planEntry)) tail.unshift(planEntry);
  return { entries: tail, omitted: entries.length - limit };
}

function isPrimaryLeader(item: StageTeamItem): boolean {
  return item.stableKey === "leader:primary";
}

function actorState(status: string): ActorState {
  if (status === "running" || status === "claimed") return "running";
  if (status === "waiting_dependency") return "waiting";
  if (status === "completed" || status === "passed") return "completed";
  if (
    status.includes("blocked") ||
    status === "recovery_required" ||
    status === "exhausted" ||
    status === "failed" ||
    status === "error" ||
    status === "interrupted"
  ) {
    return "blocked";
  }
  return "queued";
}

function actorStateLabel(item: StageTeamItem | null, state: ActorState, stageKind: string): string {
  if (!item) {
    return stageKind === "application_understanding"
      ? "正在准备 Application Model Controller…"
      : "旧版固定 Team 运行已不再支持，请重新运行本阶段以启动 Company Controller。";
  }
  const controller = isPrimaryLeader(item);
  if (controller && item.status === "waiting_dependency") {
    return stageKind === "application_understanding"
      ? "模型 Controller 正在等待并校验 Agent 产物"
      : "Controller 正在监控 SubAgent";
  }
  if (controller && state === "queued") return "Controller 排队中";
  if (controller && state === "running") return "Controller 运行中";
  if (controller && state === "completed") return "Controller 已完成";
  if (controller && state === "blocked") return "Controller 已阻塞";
  if (state === "running") return "Agent 运行中";
  if (state === "waiting") return "Agent 等待依赖";
  if (state === "completed") return "Agent 已完成";
  if (state === "blocked") return "Agent 已阻塞";
  return "Agent 排队中";
}

function humanizeToken(token: string): string {
  const upper = new Set(["api", "asn", "ct", "dns", "http", "https", "ip", "js", "osint", "url"]);
  return token
    .split(/[_\s-]+/)
    .filter(Boolean)
    .map((word) =>
      upper.has(word.toLowerCase())
        ? word.toUpperCase()
        : `${word[0]?.toUpperCase() ?? ""}${word.slice(1)}`
    )
    .join(" ");
}

function actorBaseLabel(
  stageKind: string,
  item: StageTeamItem | null,
  worker: StageTeamWorker | null
) {
  if (!item) {
    return stageKind === "application_understanding"
      ? "Application Model Controller"
      : "Company Controller";
  }
  if (isPrimaryLeader(item)) {
    if (stageKind === "vuln_triage") return "漏洞扫描调度器";
    if (stageKind === "application_understanding") {
      if (item.role === "application_model_synthesizer") {
        return "Application Model Synthesizer";
      }
      if (item.role === "application_model_worker") return "Application Modeler";
      return "Application Model Controller";
    }
    return "Company Controller";
  }
  if (item.role === "application_model_worker") return "Application Modeler";
  if (item.role === "application_model_synthesizer") return "Application Model Synthesizer";
  return humanizeToken(worker?.specialist || item.role || item.kind);
}

function workScopeLabel(stageKind: string, item: StageTeamItem | null): string | null {
  if (!item || isPrimaryLeader(item)) return null;
  const separator = item.stableKey.indexOf(":");
  const prefix = separator >= 0 ? item.stableKey.slice(0, separator) : item.kind;
  const rest = separator >= 0 ? item.stableKey.slice(separator + 1) : item.stableKey;
  const normalized = prefix.toLowerCase().replace(/_/g, "-");
  const label =
    normalized === "web-origin"
      ? "WEB ORIGIN"
      : normalized === "url"
        ? "URL"
        : normalized === "service-host"
          ? "SERVICE HOST"
          : normalized === "asset"
            ? "ASSET"
            : normalized === "vuln-worklist"
              ? "SCAN TARGET"
              : stageKind === "vuln_triage"
                ? "SCAN WORK"
                : "WORK ITEM";
  return rest && rest !== item.stableKey ? `${label} · ${rest}` : `${label} · ${item.stableKey}`;
}

function activityForRequest(
  requestId: string | null,
  activities: readonly ActiveSubAgent[]
): ActiveSubAgent | null {
  if (!requestId) return null;
  return activities.find((activity) => activity.parentRequestId === requestId) ?? null;
}

function actorsFromModel(
  model: StageTeamReadModel,
  activities: readonly ActiveSubAgent[],
  requestIds: Readonly<Record<string, string>>
): StageActor[] {
  return model.units.flatMap((unit): StageActor[] => {
    if (!unit.plan) {
      return [
        {
          id: `${unit.stageRunUnitId}:unseeded`,
          unit,
          item: null,
          worker: null,
          requestId: null,
          activity: null,
          label: actorBaseLabel(model.stageKind, null, null),
          scopeLabel: null,
          state: actorState(unit.status),
          stateLabel: actorStateLabel(null, actorState(unit.status), model.stageKind),
          controller: true,
        } satisfies StageActor,
      ];
    }
    if (!unit.plan.workItems.some(isPrimaryLeader)) {
      return [
        {
          id: `${unit.stageRunUnitId}:unsupported-team`,
          unit,
          item: null,
          worker: null,
          requestId: null,
          activity: null,
          label: "Company Controller",
          scopeLabel: null,
          state: "blocked",
          stateLabel: "旧版固定 Team 运行已不再支持，请重新运行本阶段以启动 Company Controller。",
          controller: true,
        } satisfies StageActor,
      ];
    }
    const items = unit.plan.workItems.filter((item) => isPrimaryLeader(item) || !item.isAggregator);
    return items.flatMap((item) => {
      const workers: Array<StageTeamWorker | null> =
        item.workers.length > 0 ? item.workers : [null];
      return workers.map((worker, index) => {
        const state =
          item.output?.businessDisposition === "blocked"
            ? "blocked"
            : actorState(worker?.status ?? item.status);
        const requestId = worker ? (requestIds[worker.workerRunId] ?? null) : null;
        const baseLabel = actorBaseLabel(model.stageKind, item, worker);
        return {
          id: `${unit.stageRunUnitId}:${item.workItemId}:${worker?.workerRunId ?? "unassigned"}`,
          unit,
          item,
          worker,
          requestId,
          activity: activityForRequest(requestId, activities),
          label: item.workers.length > 1 ? `${baseLabel} · Attempt ${index + 1}` : baseLabel,
          scopeLabel: workScopeLabel(model.stageKind, item),
          state,
          stateLabel: actorStateLabel(item, state, model.stageKind),
          controller: isPrimaryLeader(item),
        } satisfies StageActor;
      });
    });
  });
}

function defaultActor(actors: readonly StageActor[]): StageActor | null {
  return (
    actors.find((actor) => !actor.controller && actor.state === "running") ??
    actors.find((actor) => !actor.controller && actor.state === "blocked") ??
    actors.find((actor) => actor.controller && actor.state === "blocked") ??
    actors.find((actor) => actor.controller && actor.state === "running") ??
    actors.find((actor) => !actor.controller) ??
    actors[0] ??
    null
  );
}

function stageStatus(model: StageTeamReadModel): string {
  if (
    model.units.some((unit) => unit.plan !== null && !unit.plan.workItems.some(isPrimaryLeader))
  ) {
    return "旧版固定 Team 运行已不再支持，请重新运行本阶段以启动 Company Controller。";
  }
  const passed =
    model.units.length > 0 &&
    model.units.every((unit) => unit.status === "passed" && unit.gate.finalHandoffId);
  if (passed) return model.stageKind === "vuln_triage" ? "证据门禁已通过" : "阶段已通过";
  if (
    model.units.some(
      (unit) => unit.status.includes("blocked") || unit.gate.status.includes("blocked")
    )
  ) {
    return model.stageKind === "vuln_triage" ? "证据门禁已阻塞" : "阶段被 Gate 阻塞";
  }
  const live = model.units.reduce(
    (count, unit) => count + (unit.plan?.barrier.liveWorkers ?? 0),
    0
  );
  return live > 0 ? `${live} 个 Agent 运行中` : humanizeToken(model.executionStatus);
}

function MetricCard({
  label,
  value,
  caption,
  progress,
  active = false,
  onActivate,
}: {
  label: string;
  value: string;
  caption: ReactNode;
  progress: number;
  active?: boolean;
  onActivate?: () => void;
}) {
  const content = (
    <>
      <div className="truncate text-xs font-medium text-foreground">
        <span>{label}</span>
        <span> · {value}</span>
      </div>
      <div className="mt-1 min-h-4 truncate text-[11px] text-muted-foreground">{caption}</div>
      <div className="mt-3 h-1 overflow-hidden rounded-full bg-muted">
        <div
          className="h-full rounded-full bg-sky-300"
          style={{ width: `${Math.max(0, Math.min(100, progress))}%` }}
        />
      </div>
    </>
  );
  if (onActivate) {
    return (
      <button
        type="button"
        aria-label={`查看 ${label}`}
        aria-pressed={active}
        className={cn(
          "min-w-0 rounded-lg border bg-background/65 p-2.5 text-left transition-colors hover:border-sky-300/45 hover:bg-sky-300/[0.06]",
          active ? "border-sky-300/55 ring-1 ring-sky-300/20" : "border-border/60"
        )}
        onClick={onActivate}
      >
        {content}
      </button>
    );
  }
  return (
    <div className="min-w-0 rounded-lg border border-border/60 bg-background/65 p-2.5">
      {content}
    </div>
  );
}

function ActorRail({
  actors,
  selectedId,
  onSelect,
}: {
  actors: readonly StageActor[];
  selectedId: string;
  onSelect: (id: string) => void;
}) {
  const units = [
    ...new Map(actors.map((actor) => [actor.unit.stageRunUnitId, actor.unit])).values(),
  ];
  const live = actors.filter((actor) => actor.state === "running").length;
  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden">
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-border/50 bg-card/80 px-3">
        <Bot className="h-4 w-4 text-sky-300" aria-hidden="true" />
        <span className="text-sm font-medium">运行节点</span>
        <span className="text-xs text-muted-foreground">{live} active</span>
      </div>
      <div
        data-testid="stage-agent-list"
        className="stage-detail-scroll min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 py-2"
      >
        {units.map((unit) => {
          const unitActors = actors
            .filter((actor) => actor.unit.stageRunUnitId === unit.stageRunUnitId)
            .sort((left, right) => Number(right.controller) - Number(left.controller));
          const controller = unitActors.find((actor) => actor.controller) ?? null;
          const childActors = unitActors.filter((actor) => !actor.controller);
          return (
            <section key={unit.stageRunUnitId} className="mb-3 last:mb-0">
              <div className="px-2 py-1.5 text-[10px] font-medium tracking-[0.08em] text-muted-foreground/80">
                {unit.organizationName}
              </div>
              <div className="space-y-1">
                {unitActors
                  .filter((actor) => actor.controller)
                  .map((actor) => (
                    <ActorRailButton
                      key={actor.id}
                      actor={actor}
                      selected={selectedId === actor.id}
                      onSelect={onSelect}
                    />
                  ))}
                {childActors.length > 0 && (
                  <div
                    className="relative ml-3 space-y-1 border-l border-sky-300/25 pb-0.5 pl-3 pt-1"
                    data-testid={`stage-agent-children-${unit.stageRunUnitId}`}
                  >
                    {childActors.map((actor, index) => (
                      <div
                        key={actor.id}
                        className="relative"
                        data-parent-agent={controller?.label ?? "Company Controller"}
                      >
                        <span
                          aria-hidden="true"
                          className="absolute -left-3 top-4 h-px w-3 bg-sky-300/25"
                        />
                        {actor.scopeLabel &&
                          (index === 0 ||
                            childActors[index - 1]?.scopeLabel !== actor.scopeLabel) && (
                            <div className="px-2 pb-1 pt-2 text-[9px] font-medium tracking-wide text-muted-foreground/70">
                              {actor.scopeLabel}
                            </div>
                          )}
                        <ActorRailButton
                          actor={actor}
                          parentLabel={controller?.label ?? "Company Controller"}
                          selected={selectedId === actor.id}
                          onSelect={onSelect}
                        />
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </section>
          );
        })}
      </div>
    </div>
  );
}

function ActorRailButton({
  actor,
  parentLabel,
  selected,
  onSelect,
}: {
  actor: StageActor;
  parentLabel?: string;
  selected: boolean;
  onSelect: (id: string) => void;
}) {
  return (
    <button
      type="button"
      aria-label={`${actor.label}${actor.scopeLabel ? ` · ${actor.scopeLabel}` : ""} · ${parentLabel ? "子 Agent" : "调度根节点"} · ${actor.stateLabel}`}
      aria-pressed={selected}
      className={cn(
        "flex w-full items-start gap-2 rounded-md border px-2.5 py-2 text-left transition-colors",
        selected ? "border-sky-400/45 bg-sky-400/10" : "border-transparent hover:bg-accent/30"
      )}
      onClick={() => onSelect(actor.id)}
    >
      <span className={cn("mt-1.5 h-2 w-2 shrink-0 rounded-full", STATE_DOT[actor.state])} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[11px] font-medium text-foreground">
          {actor.label}
        </span>
        <span className="mt-0.5 block truncate text-[10px] text-muted-foreground">
          <span>{parentLabel ? `由 ${parentLabel} 调用` : "调度根节点"} · </span>
          <span>{actor.stateLabel}</span>
        </span>
        {actor.worker &&
          (actor.worker.generation > 1 ||
            actor.worker.attemptEpoch > 1 ||
            actor.worker.recoveryState === "manual_required") && (
            <span className="mt-0.5 block truncate text-[10px] text-muted-foreground/70">
              generation {actor.worker.generation} · attempt {actor.worker.attemptEpoch}
              {actor.worker.recoveryState === "manual_required" ? " · operator recovery" : ""}
            </span>
          )}
      </span>
    </button>
  );
}

function displayTime(value: string | number | undefined): string {
  if (value === undefined) return "";
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function toolSummary(tool: SubAgentToolCall): string {
  if (tool.streamingOutput?.trim()) return tool.streamingOutput.trim().slice(0, 600);
  if (tool.result !== undefined) return safeStringify(tool.result, 600);
  return safeStringify(tool.args, 600);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function dispatchResult(tool: SubAgentToolCall): Record<string, unknown> | null {
  if (!isRecord(tool.result)) return null;
  if (Array.isArray(tool.result.requests)) return tool.result;
  if (typeof tool.result.response !== "string") return tool.result;
  try {
    const parsed = JSON.parse(tool.result.response) as unknown;
    return isRecord(parsed) ? parsed : tool.result;
  } catch {
    return tool.result;
  }
}

function delegatedActorsForTool(
  tool: SubAgentToolCall,
  parentActor: StageActor,
  actors: readonly StageActor[]
): StageActor[] {
  const exactActivities = actors.filter(
    (candidate) =>
      candidate.activity?.parentRequestId === tool.id ||
      candidate.activity?.parentRequestId.startsWith(`${tool.id}::worker:`)
  );
  if (exactActivities.length > 0) return exactActivities;
  if (tool.name !== "stage_team_dispatch_workers" || !parentActor.worker) return [];

  const result = dispatchResult(tool);
  const resultRequests = Array.isArray(result?.requests) ? result.requests.filter(isRecord) : [];
  const workItemIds = new Set(
    resultRequests.flatMap((request) => {
      const value = request.created_work_item_id ?? request.accepted_work_item_id;
      return typeof value === "string" && value.trim() ? [value.trim()] : [];
    })
  );
  const workers = Array.isArray(tool.args.workers) ? tool.args.workers.filter(isRecord) : [];
  const dedupeKeys = new Set(
    workers.flatMap((worker) => {
      const value = worker.dedupe_key;
      return typeof value === "string" && value.trim() ? [value.trim()] : [];
    })
  );
  for (const request of parentActor.unit.plan?.requests ?? []) {
    if (request.parentWorkerRunId !== parentActor.worker.workerRunId) continue;
    if (
      dedupeKeys.size > 0 &&
      (typeof request.dedupeKey !== "string" || !dedupeKeys.has(request.dedupeKey))
    ) {
      continue;
    }
    if (request.acceptedWorkItemId) workItemIds.add(request.acceptedWorkItemId);
  }
  return actors.filter(
    (candidate) =>
      candidate.unit.stageRunUnitId === parentActor.unit.stageRunUnitId &&
      Boolean(candidate.item && workItemIds.has(candidate.item.workItemId))
  );
}

function SubAgentDispatchEntry({
  tool,
  parentActor,
  actors,
  onSelectActor,
}: {
  tool: SubAgentToolCall;
  parentActor: StageActor;
  actors: readonly StageActor[];
  onSelectActor: (actorId: string) => void;
}) {
  const delegatedActors = delegatedActorsForTool(tool, parentActor, actors);
  const requestedWorkers = Array.isArray(tool.args.workers)
    ? tool.args.workers.filter(isRecord)
    : [];
  const live = delegatedActors.filter((actor) => actor.state === "running").length;
  const requestedCount = Math.max(delegatedActors.length, requestedWorkers.length, 1);
  return (
    <article className="mx-4 my-2 overflow-hidden rounded-md border border-violet-400/25 border-l-2 border-l-violet-300/70 bg-violet-400/[0.045]">
      <header className="flex items-center gap-2 border-b border-violet-400/15 px-3 py-2">
        <Bot className="h-3.5 w-3.5 text-violet-200" aria-hidden="true" />
        <span className="text-xs font-medium text-violet-100">SubAgent 调用</span>
        <code className="truncate font-mono text-[10px] text-muted-foreground">{tool.name}</code>
        <span className="ml-auto text-[10px] text-muted-foreground">
          {requestedCount} 个派发{live > 0 ? ` · ${live} 运行中` : ""}
        </span>
      </header>
      <div className="space-y-1.5 p-2">
        {delegatedActors.length > 0 ? (
          delegatedActors.map((delegated) => (
            <button
              key={delegated.id}
              type="button"
              aria-label={`切换到子 Agent ${delegated.label}`}
              className="flex w-full items-start gap-2 rounded border border-border/35 bg-background/45 px-2.5 py-2 text-left hover:border-violet-300/35 hover:bg-violet-300/[0.05]"
              onClick={() => onSelectActor(delegated.id)}
            >
              <span
                className={cn("mt-1.5 h-2 w-2 shrink-0 rounded-full", STATE_DOT[delegated.state])}
              />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[11px] font-medium text-foreground/90">
                  {delegated.label}
                </span>
                <span className="mt-0.5 block truncate text-[10px] text-muted-foreground">
                  {delegated.activity?.task || delegated.scopeLabel || delegated.item?.stableKey}
                </span>
              </span>
              <span className="shrink-0 text-[10px] text-muted-foreground">
                {delegated.stateLabel}
              </span>
            </button>
          ))
        ) : requestedWorkers.length > 0 ? (
          requestedWorkers.map((worker, index) => (
            <div
              key={`${tool.id}:requested:${index}`}
              className="rounded border border-border/30 bg-background/35 px-2.5 py-2 text-[10px] text-muted-foreground"
            >
              <span className="font-medium text-foreground/80">
                {typeof worker.role === "string"
                  ? humanizeToken(worker.role)
                  : `Agent ${index + 1}`}
              </span>
              {typeof worker.objective === "string" && <span> · {worker.objective}</span>}
              <span> · 等待 Agent 运行身份</span>
            </div>
          ))
        ) : (
          <div className="px-1 py-1 text-[10px] text-muted-foreground">
            派发请求已经记录；尚未收到可绑定的子 Agent 运行事件。
          </div>
        )}
      </div>
    </article>
  );
}

function ActivityEntry({
  entry,
  activity,
  actor,
  actors,
  currentPlanToolId,
  onSelectActor,
}: {
  entry: SubAgentEntry;
  activity: ActiveSubAgent;
  actor: StageActor;
  actors: readonly StageActor[];
  currentPlanToolId: string | null;
  onSelectActor: (actorId: string) => void;
}) {
  if (entry.kind === "tool_call") {
    const tool = activity.toolCalls.find((candidate) => candidate.id === entry.toolCallId);
    if (!tool) return null;
    if (tool.name === "update_plan") {
      if (tool.id !== currentPlanToolId) return null;
      return (
        <AgentPlanCard
          tool={tool}
          parentStagePassed={Boolean(
            actor.unit.status === "passed" && actor.unit.gate.finalHandoffId
          )}
        />
      );
    }
    if (tool.name.startsWith("sub_agent_") || tool.name === "stage_team_dispatch_workers") {
      return (
        <SubAgentDispatchEntry
          tool={tool}
          parentActor={actor}
          actors={actors}
          onSelectActor={onSelectActor}
        />
      );
    }
    const blocked = tool.status === "error" || tool.status === "interrupted";
    return (
      <article className="grid grid-cols-[1.75rem_minmax(0,1fr)] gap-2.5 px-4 py-3">
        <div className="grid h-7 w-7 place-items-center rounded-md bg-sky-400/10 text-sky-200">
          <Wrench className="h-3.5 w-3.5" aria-hidden="true" />
        </div>
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <span className="font-medium text-foreground">{actor.label}</span>
            <span className="text-muted-foreground">{displayTime(tool.startedAt)}</span>
          </div>
          <div className="mt-2 overflow-hidden rounded-md border border-border/60 bg-background/70">
            <div className="flex items-center gap-2 border-b border-border/50 px-2.5 py-1.5 text-xs">
              <span className="font-medium text-foreground/85">{tool.name}</span>
              <span
                className={cn(
                  "ml-auto rounded px-1.5 py-0.5 text-[10px]",
                  blocked
                    ? "bg-rose-400/10 text-rose-200"
                    : tool.status === "running" || tool.status === "backgrounded"
                      ? "bg-sky-400/10 text-sky-200"
                      : "bg-emerald-400/10 text-emerald-200"
                )}
              >
                {tool.status}
              </span>
            </div>
            <pre className="whitespace-pre-wrap break-words px-2.5 py-2 font-mono text-[11px] leading-relaxed text-muted-foreground">
              {toolSummary(tool)}
            </pre>
          </div>
        </div>
      </article>
    );
  }
  if (!entry.text?.trim()) return null;
  return (
    <AgentTranscriptMessage
      kind={entry.kind}
      actorLabel={actor.label}
      text={entry.text}
      timeLabel={displayTime(entry.startedAt)}
      startedAt={entry.startedAt}
      endedAt={entry.endedAt}
      thinkingActive={isLiveAgentThinkingEntry(activity, entry)}
    />
  );
}

function ConversationPanel({
  actor,
  actors,
  onSelectActor,
}: {
  actor: StageActor;
  actors: readonly StageActor[];
  onSelectActor: (actorId: string) => void;
}) {
  const surfaceRef = useRef<HTMLElement>(null);
  const [taskExpanded, setTaskExpanded] = useState(false);
  const activity = actor.activity;
  const entries: SubAgentEntry[] =
    activity?.entries && activity.entries.length > 0
      ? activity.entries
      : (activity?.toolCalls ?? []).map((tool) => ({ kind: "tool_call", toolCallId: tool.id }));
  const {
    viewportRef: conversationRef,
    contentRef: conversationContentRef,
    onViewportScroll,
    onViewportWheel,
    syncViewportPosition,
  } = useTranscriptAutoScroll();
  const parentStagePassed = Boolean(
    actor.unit.status === "passed" && actor.unit.gate.finalHandoffId
  );
  const currentPlanToolId =
    resolveLatestVisibleAgentPlanRequest(activity?.toolCalls ?? [], {
      entries,
      parentStageStopped: !parentStagePassed && actor.state === "blocked",
    })?.id ?? null;
  const visibleEntries = boundedTranscriptEntries(entries, currentPlanToolId);
  const conversationStateLabel = actor.item
    ? actor.stateLabel
    : actor.state === "blocked"
      ? "需要重新运行"
      : "Controller 准备中";
  const task =
    activity?.task || `${actor.item?.role ?? "Controller"} · ${actor.item?.kind ?? "plan pending"}`;

  useEffect(() => {
    const surface = surfaceRef.current;
    const viewport = conversationRef.current;
    if (!surface || !viewport) return;
    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY === 0 || viewport.scrollHeight <= viewport.clientHeight) return;
      const maximum = viewport.scrollHeight - viewport.clientHeight;
      const next = Math.max(0, Math.min(maximum, viewport.scrollTop + event.deltaY));
      if (next === viewport.scrollTop) return;
      viewport.scrollTop = next;
      syncViewportPosition(viewport);
      event.preventDefault();
      event.stopPropagation();
    };
    surface.addEventListener("wheel", handleWheel, { passive: false });
    return () => surface.removeEventListener("wheel", handleWheel);
  }, [syncViewportPosition]);

  return (
    <section
      ref={surfaceRef}
      data-testid="stage-agent-conversation-surface"
      className="flex h-full min-h-0 flex-col overflow-hidden border-b border-border/50"
    >
      <header className="flex min-h-14 shrink-0 flex-wrap items-center gap-2 border-b border-border/50 bg-card/80 px-4 py-2">
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-foreground">{actor.label}</div>
          <div className="mt-0.5 truncate text-xs text-muted-foreground">
            {actor.unit.organizationName} · {actor.item?.stableKey ?? "awaiting plan"}
          </div>
        </div>
        <span
          className={cn(
            "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs",
            STATE_BADGE[actor.state]
          )}
        >
          {actor.state === "running" ? (
            <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
          ) : actor.state === "completed" ? (
            <CheckCircle2 className="h-3 w-3" aria-hidden="true" />
          ) : actor.state === "blocked" ? (
            <AlertTriangle className="h-3 w-3" aria-hidden="true" />
          ) : (
            <Clock3 className="h-3 w-3" aria-hidden="true" />
          )}
          {conversationStateLabel}
        </span>
      </header>
      <div className="shrink-0 border-b border-border/40 bg-background/35 text-[11px] text-muted-foreground">
        <button
          type="button"
          aria-expanded={taskExpanded}
          data-testid="stage-agent-task-toggle"
          className="flex w-full items-center gap-1.5 px-4 py-2 text-left hover:bg-accent/20"
          onClick={() => setTaskExpanded((current) => !current)}
        >
          {taskExpanded ? (
            <ChevronDown className="h-3 w-3 shrink-0" aria-hidden="true" />
          ) : (
            <ChevronRight className="h-3 w-3 shrink-0" aria-hidden="true" />
          )}
          <span className="shrink-0 font-medium text-foreground/80">任务</span>
          <span className="truncate text-muted-foreground/65">{task}</span>
        </button>
        {taskExpanded && (
          <p
            data-testid="stage-agent-task-detail"
            className="max-h-24 overflow-y-auto border-t border-border/30 px-4 py-2 whitespace-pre-wrap text-foreground/75"
          >
            {task}
          </p>
        )}
      </div>
      <div
        ref={conversationRef}
        data-testid="stage-agent-conversation"
        role="log"
        aria-label={`${actor.label} 对话`}
        className="stage-detail-scroll min-h-0 flex-1 overflow-y-auto overscroll-contain bg-card/35"
        onScroll={onViewportScroll}
        onWheel={onViewportWheel}
      >
        <div
          ref={conversationContentRef}
          data-testid="stage-agent-conversation-content"
          className="min-h-full"
        >
          {visibleEntries.omitted > 0 && (
            <div
              data-testid="stage-transcript-omission-notice"
              className="mx-4 my-2 rounded border border-border/40 bg-background/55 px-3 py-2 text-center text-[11px] text-muted-foreground"
            >
              已隐藏较早的 {visibleEntries.omitted} 条运行记录；完整历史保留在 transcript 和 run.log
              中。
            </div>
          )}
          {activity ? (
            visibleEntries.entries.length > 0 ? (
              visibleEntries.entries.map((entry, index) => (
                <ActivityEntry
                  key={`${entry.kind}:${entry.toolCallId ?? index}`}
                  entry={entry}
                  activity={activity}
                  actor={actor}
                  actors={actors}
                  currentPlanToolId={currentPlanToolId}
                  onSelectActor={onSelectActor}
                />
              ))
            ) : (
              <div className="grid h-full min-h-32 place-items-center px-4 text-center text-sm text-muted-foreground">
                Agent transcript 已建立，但当前 attempt 尚无可见消息或工具事件。
              </div>
            )
          ) : (
            <div className="grid h-full min-h-32 place-items-center px-6 text-center">
              <div>
                <Circle className="mx-auto h-4 w-4 text-muted-foreground" />
                <p className="mt-2 text-sm text-muted-foreground">
                  Durable worker state is available, but this session has no visible Agent
                  transcript.
                </p>
                <p className="mt-1 text-[11px] text-muted-foreground/70">
                  不会根据 Worker 状态伪造对话；历史 transcript 需要独立的持久化读取入口。
                </p>
              </div>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

function EvidenceInspector({
  actor,
  coverage,
  supplementary,
  onClose,
}: {
  actor: StageActor;
  coverage: StageAssetCoverageSnapshot | null;
  supplementary?: ReactNode;
  onClose: () => void;
}) {
  const output = actor.item?.output ?? null;
  const gate = actor.unit.gate;
  const unresolved = [
    ...(output?.blockerCodes ?? []),
    ...(actor.item && actor.item.status !== "completed" && !output
      ? ["Worker output 尚未提交"]
      : []),
    ...(gate.finalHandoffId ? [] : ["Stage Gate 尚未完成"]),
  ];
  return (
    <section
      aria-label="证据与阶段记忆"
      data-testid="stage-evidence-inspector"
      className="flex h-full min-h-0 flex-col overflow-hidden bg-background/45"
    >
      <header className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/50 px-4 py-2.5">
        <button
          type="button"
          className="inline-flex items-center gap-1.5 rounded border border-border/50 px-2 py-1 text-[11px] text-muted-foreground hover:text-foreground"
          onClick={onClose}
        >
          <ArrowLeft className="h-3 w-3" aria-hidden="true" />
          返回 Agent 对话
        </button>
        <FileCheck2 className="h-4 w-4 text-emerald-300" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium text-foreground">证据与阶段记忆</div>
          <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
            {actor.item?.outputSchema ?? "No output schema yet"}
          </div>
        </div>
        <span
          className={cn(
            "rounded border px-2 py-0.5 text-[10px]",
            output
              ? "border-emerald-400/25 bg-emerald-400/10 text-emerald-200"
              : "border-border/50 bg-muted/30 text-muted-foreground"
          )}
        >
          {output?.businessDisposition ?? "not submitted"}
        </span>
      </header>
      <div className="stage-detail-scroll min-h-0 flex-1 overflow-y-auto overscroll-contain p-3">
        <div className="grid gap-3 xl:grid-cols-2">
          <div className="min-w-0 space-y-2">
            <dl className="grid gap-2 sm:grid-cols-2">
              <div className="rounded-md border border-border/50 bg-card/60 p-2">
                <dt className="text-[10px] text-muted-foreground">WorkerRun</dt>
                <dd className="mt-1 break-all font-mono text-xs text-foreground/90">
                  {actor.worker?.workerRunId ?? "not created"}
                </dd>
              </div>
              <div className="rounded-md border border-border/50 bg-card/60 p-2">
                <dt className="text-[10px] text-muted-foreground">Gate</dt>
                <dd className="mt-1 text-xs text-foreground/90">
                  {gate.finalHandoffId ? (
                    <>
                      <span>Gate 已通过</span>
                      <span> · {gate.finalHandoffEvidenceCount} 条证据</span>
                    </>
                  ) : gate.status.includes("blocked") ? (
                    "Gate 已阻塞"
                  ) : (
                    "Gate 未完成"
                  )}
                </dd>
              </div>
              <div className="rounded-md border border-border/50 bg-card/60 p-2">
                <dt className="text-[10px] text-muted-foreground">Canonical facts</dt>
                <dd className="mt-1 text-xs text-foreground/90">
                  {output?.canonicalFactRefCount ?? 0}
                </dd>
              </div>
              <div className="rounded-md border border-border/50 bg-card/60 p-2">
                <dt className="text-[10px] text-muted-foreground">Checked empty</dt>
                <dd className="mt-1 text-xs text-foreground/90">
                  {output?.checkedEmptyCellCount ?? 0}
                </dd>
              </div>
            </dl>
            <div className="rounded-md border border-border/50 bg-card/60 p-3">
              <div className="text-xs font-medium text-foreground">Evidence references</div>
              {output?.evidenceIds.length ? (
                <div className="mt-2 flex flex-wrap gap-1.5">
                  {output.evidenceIds.map((id) => (
                    <span
                      key={id}
                      className="rounded border border-emerald-400/25 bg-emerald-400/10 px-2 py-1 font-mono text-[11px] text-emerald-200"
                    >
                      evidence #{id}
                    </span>
                  ))}
                </div>
              ) : (
                <div className="mt-2 text-[11px] text-muted-foreground">
                  当前 output 没有 evidence reference；这不等于已检查为空。
                </div>
              )}
            </div>
            {coverage && (
              <div className="rounded-md border border-border/50 bg-card/60 p-3">
                <div className="text-xs font-medium text-foreground">
                  目标范围 · {coverage.assets.length}
                </div>
                <div className="mt-2 space-y-1">
                  {coverage.assets.slice(0, 6).map((asset) => (
                    <div key={asset.target_id} className="flex min-w-0 gap-2 text-[11px]">
                      <span className="shrink-0 rounded bg-muted/40 px-1 text-muted-foreground">
                        {asset.target_type}
                      </span>
                      <span className="truncate font-mono text-foreground/80" title={asset.value}>
                        {asset.value}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
          <div className="grid content-start gap-2 sm:grid-cols-2 xl:grid-cols-1">
            <div className="rounded-md border border-emerald-400/20 bg-emerald-400/[0.04] p-3">
              <div className="text-xs font-medium text-emerald-100">已记录</div>
              <div className="mt-2 text-[11px] leading-relaxed text-muted-foreground">
                {output
                  ? `${output.evidenceIds.length} evidence refs · ${output.canonicalFactRefCount} canonical fact refs · ${output.checkedEmptyCellCount} checked-empty cells`
                  : "尚无 DB-authoritative Worker output。"}
              </div>
            </div>
            <div className="rounded-md border border-amber-400/20 bg-amber-400/[0.04] p-3">
              <div className="text-xs font-medium text-amber-100">未解决</div>
              {unresolved.length ? (
                <ul className="mt-2 space-y-1.5 text-[11px] text-muted-foreground">
                  {unresolved.map((item) => (
                    <li key={item} className="flex gap-2">
                      <span className="mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full bg-amber-300" />
                      <span>{item}</span>
                    </li>
                  ))}
                </ul>
              ) : (
                <div className="mt-2 text-[11px] text-emerald-200">当前节点无未解决项。</div>
              )}
            </div>
          </div>
        </div>
        {supplementary !== undefined && supplementary !== null && (
          <div className="mt-3 space-y-2 border-t border-border/40 pt-3">{supplementary}</div>
        )}
      </div>
    </section>
  );
}

export function StageTeamWorkspaceView({
  model,
  children,
  agentActivities = [],
  agentRequestIdsByWorker = {},
  focusedAgentRequestId = null,
  coverageByOrg = {},
}: StageTeamWorkspaceViewProps) {
  const actors = useMemo(
    () => actorsFromModel(model, agentActivities, agentRequestIdsByWorker),
    [agentActivities, agentRequestIdsByWorker, model]
  );
  const preferred =
    actors.find((actor) => actor.requestId === focusedAgentRequestId) ?? defaultActor(actors);
  const [selectedActorId, setSelectedActorId] = useState<string | null>(null);
  const [activeSurface, setActiveSurface] = useState<"conversation" | "evidence">("conversation");
  const appliedFocusRef = useRef<string | null>(null);
  const selectedActor = actors.find((actor) => actor.id === selectedActorId) ?? preferred;

  useEffect(() => {
    if (!focusedAgentRequestId) {
      appliedFocusRef.current = null;
      return;
    }
    if (appliedFocusRef.current === focusedAgentRequestId) return;
    const focusedActor = actors.find((actor) => actor.requestId === focusedAgentRequestId);
    if (!focusedActor) return;
    appliedFocusRef.current = focusedAgentRequestId;
    setSelectedActorId(focusedActor.id);
    setActiveSurface("conversation");
  }, [actors, focusedAgentRequestId]);
  const plans = model.units.flatMap((unit) => (unit.plan ? [unit.plan] : []));
  const required = plans.reduce((sum, plan) => sum + plan.barrier.requiredWorkItems, 0);
  const terminal = plans.reduce((sum, plan) => sum + plan.barrier.terminalRequiredWorkItems, 0);
  const live = plans.reduce((sum, plan) => sum + plan.barrier.liveWorkers, 0);
  const evidenceIds = new Set(
    model.units.flatMap(
      (unit) => unit.plan?.workItems.flatMap((item) => item.output?.evidenceIds ?? []) ?? []
    )
  );
  const passedUnits = model.units.filter(
    (unit) => unit.status === "passed" && unit.gate.finalHandoffId
  ).length;
  const visibleWorkItems = plans.flatMap((plan) =>
    plan.workItems.filter((item) => !item.isAggregator && !isPrimaryLeader(item))
  );
  const collectingItems = visibleWorkItems.filter(
    (item) => item.status === "running" || item.status === "claimed"
  ).length;
  const blockedItems = visibleWorkItems.filter(
    (item) =>
      item.output?.businessDisposition === "blocked" ||
      item.status.includes("blocked") ||
      item.status === "recovery_required" ||
      item.status === "exhausted"
  ).length;
  const completedItems = visibleWorkItems.filter(
    (item) => item.status === "completed" && item.output?.businessDisposition !== "blocked"
  ).length;
  const stageMetricLabel =
    model.stageKind === "vuln_triage"
      ? "漏洞扫描进度"
      : model.stageKind === "application_understanding"
        ? "应用理解进度"
        : "任务执行进度";
  const workCaption =
    model.stageKind === "vuln_triage"
      ? "扫描分片"
      : model.stageKind === "application_understanding"
        ? "分析分片"
        : "采集";
  const visibleWorkItemNoun =
    model.stageKind === "vuln_triage"
      ? "扫描分片"
      : model.stageKind === "application_understanding"
        ? "分析 Agent"
        : "SubAgent";
  const visibleWorkItemLabel = `${visibleWorkItems.length} 个${
    model.stageKind === "vuln_triage" ? visibleWorkItemNoun : ` ${visibleWorkItemNoun}`
  }`;
  const metricSlots = (
    <>
      <MetricCard
        label="Companies"
        value={`${passedUnits}/${model.units.length}`}
        caption={`${model.units.length} company units`}
        progress={model.units.length ? (passedUnits / model.units.length) * 100 : 0}
      />
      <MetricCard
        label={stageMetricLabel}
        value={`${terminal}/${required}`}
        caption={
          <span className="flex flex-wrap gap-x-2">
            <span>{`${workCaption} ${terminal}/${required} 已返回`}</span>
          </span>
        }
        progress={required ? (terminal / required) * 100 : 0}
      />
      <MetricCard
        label="Active"
        value={String(live)}
        caption={
          <span className="flex flex-wrap gap-x-2">
            <span>{visibleWorkItemLabel}</span>
            {collectingItems > 0 && (
              <span>
                {collectingItems} 个{model.stageKind === "vuln_triage" ? "执行中" : "运行中"}
              </span>
            )}
            {completedItems > 0 && <span>{completedItems} 个已完成</span>}
            {blockedItems > 0 && <span>{blockedItems} 个阻塞</span>}
            {collectingItems + completedItems + blockedItems === 0 && (
              <span>{actors.length} visible runtime nodes</span>
            )}
          </span>
        }
        progress={actors.length ? ((actors.length - live) / actors.length) * 100 : 0}
      />
      <MetricCard
        label="Evidence"
        value={String(evidenceIds.size)}
        caption={`${passedUnits}/${model.units.length} Gate passed`}
        progress={model.units.length ? (passedUnits / model.units.length) * 100 : 0}
        active={activeSurface === "evidence"}
        onActivate={() => setActiveSurface("evidence")}
      />
    </>
  );

  if (!selectedActor) {
    return (
      <StageRunDetailShell
        stageKey={model.stageKind}
        operationId={model.operationId}
        statusLabel={stageStatus(model)}
        metricSlots={metricSlots}
      >
        <div className="grid min-h-64 place-items-center p-6 text-sm text-muted-foreground">
          No visible Stage runtime nodes exist for this exact execution.
        </div>
      </StageRunDetailShell>
    );
  }

  return (
    <StageRunDetailShell
      stageKey={model.stageKind}
      operationId={model.operationId}
      statusLabel={stageStatus(model)}
      metricSlots={metricSlots}
      sideRail={
        <ActorRail
          actors={actors}
          selectedId={selectedActor.id}
          onSelect={(actorId) => {
            setSelectedActorId(actorId);
            setActiveSurface("conversation");
          }}
        />
      }
    >
      <div data-testid="stage-team-workspace-layout" className="h-full min-h-0 overflow-hidden">
        {activeSurface === "conversation" ? (
          <ConversationPanel
            key={selectedActor.id}
            actor={selectedActor}
            actors={actors}
            onSelectActor={(actorId) => setSelectedActorId(actorId)}
          />
        ) : (
          <EvidenceInspector
            actor={selectedActor}
            coverage={coverageByOrg[selectedActor.unit.organizationId] ?? null}
            supplementary={children}
            onClose={() => setActiveSurface("conversation")}
          />
        )}
      </div>
    </StageRunDetailShell>
  );
}
