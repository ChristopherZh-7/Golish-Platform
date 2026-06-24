import {
  Bot,
  CheckCircle2,
  ChevronDown,
  Clock,
  Loader2,
  ShieldCheck,
  Wrench,
  XCircle,
  Zap,
} from "lucide-react";
import { useEffect, useState } from "react";
import { AnchorChip } from "@/components/ui/AnchorChip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  getToolColor,
  getToolLabel,
  getToolPrimaryArg,
  toolResultIndicatesFailure,
} from "@/lib/tools";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";

type ApprovalMode = "ask" | "run-all";

function roleAgentLabel(roleLabel?: string) {
  const label = roleLabel?.trim();
  if (!label) return null;
  return /agent$/i.test(label) ? label : `${label} Agent`;
}

/**
 * Compact tool-approval mode switch rendered inline on a tool-call card / row.
 *
 * Shared by the per-tool pending-approval card ([`CollapsibleToolCall`]) and the
 * always-visible timeline card ([`ToolCallCard`]) so the "Run Everything / Ask
 * Every Time" control is reachable from *every* tool call, not just the toolbar
 * or a pending approval. `stopPropagation` keeps clicks from also toggling the
 * card's detail pane.
 */
function ApprovalModeInlineDropdown({
  approvalMode,
  onApprovalModeChange,
}: {
  approvalMode?: string;
  onApprovalModeChange: (mode: ApprovalMode) => void;
}) {
  const isRunAll = approvalMode === "run-all";
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Tool approval mode"
          onClick={(e) => e.stopPropagation()}
          className={cn(
            "flex items-center gap-1 text-[10px] transition-colors",
            isRunAll
              ? "text-[var(--ansi-yellow)]/80 hover:text-[var(--ansi-yellow)]"
              : "text-muted-foreground/50 hover:text-muted-foreground"
          )}
        >
          {isRunAll ? <Zap className="w-2.5 h-2.5" /> : <ShieldCheck className="w-2.5 h-2.5" />}
          {isRunAll ? "Run Everything" : "Ask Every Time"}
          <ChevronDown className="w-2.5 h-2.5" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="bg-card border-[var(--border-medium)] min-w-[160px]"
      >
        {[
          { id: "ask" as const, label: "Ask Every Time" },
          { id: "run-all" as const, label: "Run Everything" },
        ].map((opt) => (
          <DropdownMenuItem
            key={opt.id}
            onClick={(e) => {
              e.stopPropagation();
              onApprovalModeChange(opt.id);
            }}
            className={cn(
              "text-xs cursor-pointer",
              approvalMode === opt.id && "bg-accent/10 text-accent"
            )}
          >
            {opt.label}
            {approvalMode === opt.id && <span className="ml-auto text-accent">✓</span>}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function parseToolPrimary(name: string, argsStr?: string): string | null {
  if (!argsStr) return null;
  try {
    return getToolPrimaryArg(name, JSON.parse(argsStr));
  } catch {
    return null;
  }
}

/**
 * Some tools complete as a CALL (success=true) but carry the real outcome in a
 * `status` field — e.g. `submit_stage_deliverable` returns
 * `accepted` / `rejected` / `needs_fix`. Treat a known non-accepted status as a
 * failure so the card shows ❌ instead of a misleading ✅.
 */
export function toolResultIsFailure(result?: string): boolean {
  return toolResultIndicatesFailure(result) || toolResultStatus(result) === "killed";
}

export function toolResultStatus(result?: string): string | null {
  if (!result) return null;
  try {
    const parsed = JSON.parse(result);
    const status =
      parsed && typeof parsed === "object" ? (parsed as { status?: unknown }).status : null;
    return typeof status === "string" ? status : null;
  } catch {
    return null;
  }
}

export function toolResultIsBackgrounded(result?: string): boolean {
  return toolResultStatus(result) === "backgrounded";
}

function ToolCallCard({
  tc,
  onClick,
  isMessageComplete,
  isSelected,
  sessionId,
  requestId,
  approvalMode,
  onApprovalModeChange,
}: {
  tc: { name: string; args?: string; result?: string; success?: boolean };
  onClick: () => void;
  isMessageComplete?: boolean;
  isSelected?: boolean;
  sessionId?: string | null;
  requestId?: string | null;
  approvalMode?: string;
  onApprovalModeChange?: (mode: ApprovalMode) => void;
}) {
  const label = getToolLabel(tc.name, "short");
  const color = getToolColor(tc.name);
  const isNoResult = tc.success === undefined;
  const isExpired = isNoResult && isMessageComplete;
  const isBackgrounded = toolResultIsBackgrounded(tc.result);
  const isRunning = isNoResult && !isMessageComplete;
  const isError = tc.success === false || toolResultIsFailure(tc.result);
  const isShell = tc.name === "run_command" || tc.name === "run_pty_cmd";
  const primary = parseToolPrimary(tc.name, tc.args);

  // For a `stage_run` tool card, surface its live worker fan-out inline so the
  // user sees that the tool is orchestrating specialist agents, not doing a
  // silent background batch. Matched to THIS tool row by requestId (the same tie
  // the detail pane uses).
  const stageRunSummary = useStore((s) => {
    if (tc.name !== "stage_run" || !sessionId) return null;
    const session = s.sessions[sessionId];
    const sr = requestId
      ? (session?.stageRuns?.[requestId] ?? session?.stageRun)
      : session?.stageRun;
    if (!sr || sr.summary.total === 0) return null;
    if (sr.requestId && requestId && sr.requestId !== requestId) return null;
    return sr.summary;
  });
  const stageRunRoleLabel = useStore((s) => {
    if (tc.name !== "stage_run" || !sessionId) return null;
    const session = s.sessions[sessionId];
    const sr = requestId
      ? (session?.stageRuns?.[requestId] ?? session?.stageRun)
      : session?.stageRun;
    if (!sr || sr.summary.total === 0) return null;
    if (sr.requestId && requestId && sr.requestId !== requestId) return null;
    return sr.roleLabel;
  });
  const stageRunWorkerLabel = roleAgentLabel(stageRunRoleLabel ?? undefined);
  const workerCount = stageRunSummary?.total ?? 0;
  const workerText = `${workerCount} ${workerCount === 1 ? "worker" : "workers"}`;
  const stoppedWorkerCount = isExpired
    ? (stageRunSummary?.active ?? 0) + (stageRunSummary?.queued ?? 0)
    : 0;
  const displayActiveWorkers = isExpired ? 0 : (stageRunSummary?.active ?? 0);
  const displayQueuedWorkers = isExpired ? 0 : (stageRunSummary?.queued ?? 0);

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick();
        }
      }}
      className={cn(
        "w-full rounded-lg border bg-background/50 px-3 py-2 text-left transition-colors cursor-pointer group",
        isSelected && "ring-1 ring-accent/50 border-accent/40 bg-accent/5",
        isExpired
          ? "border-[#565f89]/30 opacity-60"
          : isRunning || isBackgrounded
            ? "border-l-2 animate-[pulse-border_2s_ease-in-out_infinite]"
            : isError
              ? "border-red-500/30 hover:border-red-500/50"
              : "border-border/30 hover:border-accent/40"
      )}
      style={
        isRunning || isBackgrounded
          ? { borderLeftColor: isBackgrounded ? "var(--ansi-yellow)" : color }
          : undefined
      }
    >
      <div className="flex items-center gap-2">
        <Wrench
          className="w-3.5 h-3.5 flex-shrink-0"
          style={{ color: isExpired ? "var(--muted-foreground)" : color }}
        />
        <span className="text-[11px] font-medium text-foreground/80">{label}</span>
        <AnchorChip sessionId={sessionId} requestId={requestId} />
        <div className="ml-auto flex items-center gap-1.5">
          {isExpired ? (
            <Clock className="w-3 h-3 text-[#565f89]" />
          ) : isBackgrounded ? (
            <Loader2 className="w-3 h-3 text-[var(--ansi-yellow)] animate-spin" />
          ) : isRunning ? (
            <Loader2 className="w-3 h-3 text-blue-400 animate-spin" />
          ) : isError ? (
            <XCircle className="w-3 h-3 text-red-400" />
          ) : (
            <CheckCircle2 className="w-3 h-3 text-[var(--ansi-green)]" />
          )}
          {isExpired ? (
            <span className="text-[10px] text-[#565f89]">Expired</span>
          ) : isBackgrounded ? (
            <span className="text-[10px] text-[var(--ansi-yellow)]/80">Background →</span>
          ) : (
            <span className="text-[10px] text-muted-foreground/60 group-hover:text-accent/60 transition-colors">
              Details →
            </span>
          )}
        </div>
      </div>
      {stageRunSummary && (
        <div className="mt-1.5 space-y-1">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px]">
            <span className="inline-flex items-center gap-1 font-medium text-cyan-300">
              <Bot className="h-2.5 w-2.5" />
              {stageRunWorkerLabel ? `${stageRunWorkerLabel} · ${workerText}` : workerText}
            </span>
            <span className="text-foreground/70">
              {stageRunSummary.covered}/{stageRunSummary.total} passed
            </span>
            {displayActiveWorkers > 0 && (
              <span className="text-sky-400">{displayActiveWorkers} 进行</span>
            )}
            {displayQueuedWorkers > 0 && (
              <span className="text-indigo-400">{displayQueuedWorkers} 排队</span>
            )}
            {stoppedWorkerCount > 0 && (
              <span className="text-yellow-400">{stoppedWorkerCount} stopped</span>
            )}
            {stageRunSummary.blocked > 0 && (
              <span className="text-amber-400">{stageRunSummary.blocked} 阻塞</span>
            )}
          </div>
          <div className="h-1 w-full overflow-hidden rounded-full bg-muted/40">
            <div
              className="h-full rounded-full bg-[var(--success)]/70 transition-all"
              style={{
                width: `${Math.round((stageRunSummary.covered / stageRunSummary.total) * 100)}%`,
              }}
            />
          </div>
        </div>
      )}
      {primary && (
        <div
          className={cn(
            "mt-1.5 text-[10px] font-mono truncate px-1.5 py-0.5 rounded",
            isShell
              ? "bg-[var(--ansi-black)]/30 text-[var(--ansi-green)]/80"
              : "bg-muted/30 text-muted-foreground/70"
          )}
        >
          {isShell && <span className="text-muted-foreground/60 mr-1">$</span>}
          {primary}
        </div>
      )}
      {onApprovalModeChange && (
        <div className="mt-1.5 flex justify-end">
          <ApprovalModeInlineDropdown
            approvalMode={approvalMode}
            onApprovalModeChange={onApprovalModeChange}
          />
        </div>
      )}
    </div>
  );
}

/**
 * Render a tool result string. When it is a JSON object, render it field-by-field
 * so multi-line string values keep their real newlines instead of showing the
 * literal `\n` / `\t` escape sequences produced by JSON.stringify.
 */
function ToolResultPreview({ result }: { result: string }) {
  const parsed = (() => {
    const trimmed = result.trim();
    if (!trimmed.startsWith("{")) return null;
    try {
      const v = JSON.parse(trimmed);
      return v && typeof v === "object" && !Array.isArray(v)
        ? (v as Record<string, unknown>)
        : null;
    } catch {
      return null;
    }
  })();

  const entries = parsed ? Object.entries(parsed) : null;
  if (entries && entries.length > 0) {
    return (
      <div className="divide-y divide-border/15 max-h-[220px] overflow-auto rounded bg-muted/20">
        {entries.map(([key, val]) => {
          const strValue = typeof val === "string" ? val : JSON.stringify(val, null, 2);
          const isLong = strValue.length > 80 || strValue.includes("\n");
          return (
            <div key={key} className={cn("px-2 py-1", !isLong && "flex items-baseline gap-2")}>
              <span className="text-[10px] font-mono text-[var(--ansi-cyan)]/70 flex-shrink-0">
                {key}
              </span>
              {isLong ? (
                <pre className="mt-0.5 text-[11px] text-muted-foreground/80 font-mono whitespace-pre-wrap break-all max-h-[150px] overflow-auto">
                  {strValue}
                </pre>
              ) : (
                <span
                  className="text-[11px] text-muted-foreground/80 font-mono truncate"
                  title={strValue}
                >
                  {strValue}
                </span>
              )}
            </div>
          );
        })}
      </div>
    );
  }

  return (
    <pre className="text-[11px] text-muted-foreground/80 font-mono whitespace-pre-wrap max-h-[200px] overflow-auto">
      {result.length > 2000 ? `${result.slice(0, 2000)}...` : result}
    </pre>
  );
}

export function CollapsibleToolCall({
  tc,
  approval,
  onApprove,
  onApproveAlways,
  onDeny,
  approvalMode,
  onApprovalModeChange,
}: {
  tc: { name: string; args?: string; result?: string; success?: boolean };
  approval?: { requestId: string } | null;
  onApprove?: (requestId: string) => void;
  onApproveAlways?: (requestId: string) => void;
  onDeny?: (requestId: string) => void;
  approvalMode?: string;
  onApprovalModeChange?: (mode: "ask" | "run-all") => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const isPending = !!approval;

  return (
    <div
      className={cn(
        "rounded-md border bg-background/50",
        isPending ? "border-[#e0af68]/50" : "border-border/30"
      )}
    >
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-1.5 w-full px-2 py-1.5 text-[11px] text-muted-foreground hover:text-muted-foreground/80 transition-colors"
      >
        <ChevronDown className={cn("w-3 h-3 transition-transform", !expanded && "-rotate-90")} />
        <Wrench className="w-3 h-3" />
        <span className="font-mono font-medium">{tc.name}</span>
        {tc.success !== undefined &&
          (() => {
            // success=true but a rejected/needs_fix status body still reads ❌.
            const failed = tc.success === false || toolResultIsFailure(tc.result);
            const backgrounded = toolResultIsBackgrounded(tc.result);
            if (backgrounded) {
              return (
                <span className="ml-auto inline-flex items-center gap-1 text-[var(--ansi-yellow)]">
                  <Loader2 className="h-3 w-3 animate-spin" />
                  Background
                </span>
              );
            }
            return (
              <span className={cn("ml-auto", failed ? "text-red-500" : "text-green-500")}>
                {failed ? "\u2717" : "\u2713"}
              </span>
            );
          })()}
      </button>

      {expanded && (tc.args || tc.result) && (
        <div className="px-2 pb-1.5 space-y-1.5">
          {tc.args && (
            <div>
              <div className="text-[10px] text-muted-foreground/50 mb-0.5">Arguments</div>
              <pre className="text-[11px] text-muted-foreground/70 font-mono whitespace-pre-wrap max-h-[150px] overflow-auto bg-muted/20 rounded px-2 py-1">
                {(() => {
                  try {
                    return JSON.stringify(JSON.parse(tc.args), null, 2);
                  } catch {
                    return tc.args.length > 1500 ? `${tc.args.slice(0, 1500)}...` : tc.args;
                  }
                })()}
              </pre>
            </div>
          )}
          {tc.result && (
            <div>
              <div className="text-[10px] text-muted-foreground/50 mb-0.5">Result</div>
              <ToolResultPreview result={tc.result} />
            </div>
          )}
        </div>
      )}

      {isPending && approval && (
        <div className="px-2 pb-1.5 flex items-center gap-2">
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onApprove?.(approval.requestId);
            }}
            className="px-2.5 py-1 text-[11px] rounded bg-[#7aa2f7] text-[#1a1b26] hover:bg-[#7aa2f7]/80 transition-colors font-medium"
          >
            Run
          </button>
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onDeny?.(approval.requestId);
            }}
            className="px-2.5 py-1 text-[11px] rounded border border-[#3b4261] text-muted-foreground hover:bg-[#3b4261] transition-colors"
          >
            Deny
          </button>
          {onApproveAlways && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onApproveAlways(approval.requestId);
              }}
              title={`Always allow ${tc.name} — auto-run it from now on`}
              className="ml-auto px-2.5 py-1 text-[11px] rounded text-muted-foreground/70 hover:text-foreground hover:bg-[#3b4261]/60 transition-colors"
            >
              ✓ Always allow
            </button>
          )}
        </div>
      )}

      <div className="px-2 pb-1.5">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              onClick={(e) => e.stopPropagation()}
              className="flex items-center gap-1 text-[11px] text-muted-foreground/60 hover:text-muted-foreground transition-colors"
            >
              {approvalMode === "run-all" ? "Run Everything" : "Ask Every Time"}
              <ChevronDown className="w-2.5 h-2.5" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="start"
            className="bg-card border-[var(--border-medium)] min-w-[160px]"
          >
            {[
              { id: "ask" as const, label: "Ask Every Time" },
              { id: "run-all" as const, label: "Run Everything" },
            ].map((opt) => (
              <DropdownMenuItem
                key={opt.id}
                onClick={() => onApprovalModeChange?.(opt.id)}
                className={cn(
                  "text-xs cursor-pointer",
                  approvalMode === opt.id && "bg-accent/10 text-accent"
                )}
              >
                {opt.label}
                {approvalMode === opt.id && <span className="ml-auto text-accent">✓</span>}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}

export function ToolCallSummary({
  toolCalls,
  requestIds,
  isMessageComplete,
  approvalMode,
  onApprovalModeChange,
}: {
  toolCalls: Array<{
    name: string;
    args?: string;
    result?: string;
    success?: boolean;
    requestId?: string;
  }>;
  requestIds?: string[];
  isMessageComplete?: boolean;
  approvalMode?: string;
  onApprovalModeChange?: (mode: ApprovalMode) => void;
}) {
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const sessionId = useStore((s) => s.activeSessionId);

  const activeDetailIds = useStore((s) => {
    const sid = s.activeSessionId;
    if (!sid) return null;
    return s.sessions[sid]?.detailViewMode === "tool-detail"
      ? s.sessions[sid]?.toolDetailRequestIds
      : null;
  });
  useEffect(() => {
    if (selectedIdx == null) return;
    const tc = toolCalls[selectedIdx];
    if (!activeDetailIds) {
      setSelectedIdx(null);
    } else if (tc?.requestId && !activeDetailIds.includes(tc.requestId)) {
      setSelectedIdx(null);
    }
  }, [activeDetailIds, selectedIdx, toolCalls]);

  if (toolCalls.length === 0) return null;

  const backfillTimeline = (
    state: ReturnType<typeof useStore.getState>,
    sessionId: string,
    calls: typeof toolCalls,
    messageComplete?: boolean
  ) => {
    const timeline = state.timelines[sessionId] ?? [];
    const existingIds = new Set(
      timeline
        .filter(
          (b): b is { type: "ai_tool_execution"; data: { requestId: string } } & typeof b =>
            b.type === "ai_tool_execution"
        )
        .map((b) => b.data.requestId)
    );

    for (const tc of calls) {
      if (!tc.requestId) continue;
      if (tc.name.startsWith("sub_agent_")) continue;

      if (!existingIds.has(tc.requestId)) {
        let parsedArgs: Record<string, unknown> = {};
        try {
          if (tc.args) parsedArgs = JSON.parse(tc.args);
        } catch {
          /* keep empty */
        }

        state.addToolExecutionBlock(sessionId, {
          requestId: tc.requestId,
          toolName: tc.name,
          args: parsedArgs,
        });
      }

      if (tc.success !== undefined && toolResultIsBackgrounded(tc.result)) {
        state.backgroundToolExecutionBlock(sessionId, tc.requestId, tc.result);
      } else if (tc.success !== undefined) {
        state.completeToolExecutionBlock(sessionId, tc.requestId, tc.success, tc.result);
      } else if (messageComplete) {
        state.interruptToolExecutionBlock(sessionId, tc.requestId, {
          reason: "Tool call expired before a result was received.",
        });
      }
    }
  };

  const handleCardClick = (idx: number) => {
    const state = useStore.getState();
    const sessionId = state.activeSessionId;
    if (!sessionId) return;

    if (selectedIdx === idx && state.sessions[sessionId]?.detailViewMode === "tool-detail") {
      setSelectedIdx(null);
      state.setDetailViewMode(sessionId, "timeline");
      return;
    }

    setSelectedIdx(idx);

    const tc = toolCalls[idx];
    const ids = tc.requestId ? [tc.requestId] : (requestIds ?? null);
    state.setToolDetailRequestIds(sessionId, ids);
    state.setDetailViewMode(sessionId, "tool-detail");
    backfillTimeline(state, sessionId, toolCalls, isMessageComplete);
  };

  return (
    <div className="mt-2 space-y-1.5">
      {toolCalls.map((tc, i) => {
        if (tc.name.startsWith("sub_agent_")) return null;
        return (
          <ToolCallCard
            key={`${tc.name}-${i}`}
            tc={tc}
            onClick={() => handleCardClick(i)}
            isMessageComplete={isMessageComplete}
            isSelected={selectedIdx === i}
            sessionId={sessionId}
            requestId={tc.requestId ?? requestIds?.[i] ?? null}
            approvalMode={approvalMode}
            onApprovalModeChange={onApprovalModeChange}
          />
        );
      })}
    </div>
  );
}
