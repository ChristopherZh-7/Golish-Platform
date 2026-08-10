import { CheckCircle2, Circle, Loader2 } from "lucide-react";
import { toolResultIndicatesFailure } from "@/lib/tools";
import { cn } from "@/lib/utils";

export type AgentPlanStepStatus = "pending" | "in_progress" | "completed";

/**
 * Structural tool shape used by stage workspaces. Keeping this independent of
 * the store lets durable transcript readers reuse the plan UI without pulling
 * in an unrelated detail-page runtime.
 */
export interface StageToolRequest {
  readonly id: string;
  readonly name: string;
  readonly args: unknown;
  readonly status: "running" | "backgrounded" | "completed" | "error" | "interrupted";
  readonly result?: unknown;
}

/** Minimal timeline shape needed to resolve the latest visible plan in entry order. */
export interface StageToolEntry {
  readonly kind: string;
  readonly toolCallId?: string;
}

export interface AgentPlanSnapshot {
  explanation: string | null;
  steps: Array<{ step: string; status: AgentPlanStepStatus }>;
  completedCount: number;
  inProgressCount: number;
  totalCount: number;
}

interface ResolveAgentPlanOptions {
  entries?: readonly StageToolEntry[];
  parentStageStopped?: boolean;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseAgentPlanArgs(args: unknown): AgentPlanSnapshot | null {
  if (!isRecord(args) || !Array.isArray(args.plan)) return null;
  if (args.plan.length < 1 || args.plan.length > 12) return null;

  const steps: AgentPlanSnapshot["steps"] = [];
  for (const value of args.plan) {
    if (!isRecord(value)) return null;
    const step = typeof value.step === "string" ? value.step.trim() : "";
    const status = value.status;
    if (!step || (status !== "pending" && status !== "in_progress" && status !== "completed")) {
      return null;
    }
    steps.push({ step, status });
  }

  const inProgressCount = steps.filter((step) => step.status === "in_progress").length;
  if (inProgressCount > 1) return null;

  const explanation =
    typeof args.explanation === "string" && args.explanation.trim()
      ? args.explanation.trim()
      : null;

  return {
    explanation,
    steps,
    completedCount: steps.filter((step) => step.status === "completed").length,
    inProgressCount,
    totalCount: steps.length,
  };
}

export function projectAgentPlanForPassedStage(
  plan: AgentPlanSnapshot,
  parentStagePassed: boolean
): AgentPlanSnapshot {
  if (!parentStagePassed || plan.completedCount === plan.totalCount) return plan;
  return {
    ...plan,
    completedCount: plan.totalCount,
    inProgressCount: 0,
    steps: plan.steps.map((step) => ({ ...step, status: "completed" })),
  };
}

function requestIsVisible(request: StageToolRequest, parentStageStopped: boolean): boolean {
  if (request.name !== "update_plan" || !parseAgentPlanArgs(request.args)) return false;
  if (request.status === "error" || request.status === "interrupted") return false;
  if (parentStageStopped && request.status === "running") return false;
  return request.status !== "completed" || !toolResultIndicatesFailure(request.result);
}

/**
 * Returns only the newest valid live/completed update_plan. When timeline
 * entries are supplied their order wins; requests not yet represented by an
 * entry are then considered in request order so streaming plans stay visible.
 */
export function resolveLatestVisibleAgentPlanRequest(
  requests: readonly StageToolRequest[],
  options: ResolveAgentPlanOptions = {}
): StageToolRequest | null {
  const parentStageStopped = options.parentStageStopped ?? false;
  const requestById = new Map(requests.map((request) => [request.id, request]));
  const ordered: StageToolRequest[] = [];
  const seen = new Set<string>();

  for (const entry of options.entries ?? []) {
    if (entry.kind !== "tool_call" || !entry.toolCallId) continue;
    const request = requestById.get(entry.toolCallId);
    if (!request || seen.has(request.id)) continue;
    seen.add(request.id);
    ordered.push(request);
  }
  for (const request of requests) {
    if (seen.has(request.id)) continue;
    ordered.push(request);
  }

  let latest: StageToolRequest | null = null;
  for (const request of ordered) {
    if (requestIsVisible(request, parentStageStopped)) latest = request;
  }
  return latest;
}

export interface AgentPlanCardProps {
  tool: StageToolRequest;
  parentStagePassed?: boolean;
  parentStageStopped?: boolean;
}

export function AgentPlanCard({
  tool,
  parentStagePassed = false,
  parentStageStopped = false,
}: AgentPlanCardProps) {
  const parsedPlan = parseAgentPlanArgs(tool.args);
  if (!parsedPlan) return null;
  const plan = projectAgentPlanForPassedStage(parsedPlan, parentStagePassed);
  const live =
    !parentStagePassed &&
    !parentStageStopped &&
    (tool.status === "running" || tool.status === "backgrounded");
  const statusLabel = parentStagePassed ? "计划已完成" : live ? "正在规划" : "当前计划";

  return (
    <section
      aria-label="Controller plan"
      className="mx-4 my-1.5 overflow-hidden rounded-md border border-border/20 border-l-2 border-l-[var(--ansi-blue)]/55 bg-background/45"
    >
      <header className="flex min-w-0 items-center gap-2 border-b border-border/10 px-3 py-2">
        {parentStagePassed || tool.status === "completed" ? (
          <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-[var(--ansi-green)]" />
        ) : live ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-[var(--ansi-blue)]" />
        ) : (
          <Circle className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        )}
        <span className="text-[12px] font-semibold text-foreground/85">{statusLabel}</span>
        <span className="min-w-0 flex-1" />
        <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground/75">
          {plan.completedCount}/{plan.totalCount} 已完成
        </span>
      </header>
      <div className="space-y-2 px-3 py-2.5">
        {plan.explanation && (
          <p className="text-[11px] leading-relaxed text-muted-foreground/80">{plan.explanation}</p>
        )}
        <div className="space-y-1.5" role="list">
          {plan.steps.map((planStep, index) => (
            <div
              className="flex min-w-0 items-start gap-2 text-[11px] leading-4"
              key={`${index}-${planStep.step}`}
              role="listitem"
            >
              <span
                aria-label={`步骤状态：${planStep.status}`}
                className="mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center"
                role="img"
              >
                {planStep.status === "completed" ? (
                  <CheckCircle2 className="h-3.5 w-3.5 text-[var(--ansi-green)]/80" />
                ) : planStep.status === "in_progress" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-[var(--ansi-blue)]/85" />
                ) : (
                  <Circle className="h-3 w-3 text-muted-foreground/45" />
                )}
              </span>
              <span
                className={cn(
                  "min-w-0 flex-1 text-foreground/75",
                  planStep.status === "completed" &&
                    "text-muted-foreground/60 line-through decoration-muted-foreground/30",
                  planStep.status === "in_progress" && "font-medium text-foreground/90"
                )}
              >
                {planStep.step}
              </span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
