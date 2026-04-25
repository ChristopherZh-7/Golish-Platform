import {
  CheckCircle2,
  ChevronDown,
  Clock,
  Loader2,
  Wrench,
  XCircle,
} from "lucide-react";
import { useEffect, useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { getToolColor, getToolLabel, getToolPrimaryArg } from "@/lib/tools";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";

export function parseToolPrimary(name: string, argsStr?: string): string | null {
  if (!argsStr) return null;
  try {
    return getToolPrimaryArg(name, JSON.parse(argsStr));
  } catch { return null; }
}

function ToolCallCard({
  tc,
  onClick,
  isMessageComplete,
  isSelected,
}: {
  tc: { name: string; args?: string; result?: string; success?: boolean };
  onClick: () => void;
  isMessageComplete?: boolean;
  isSelected?: boolean;
}) {
  let label = getToolLabel(tc.name, "short");
  if (tc.name === "run_pipeline" && tc.args) {
    try {
      const parsed = JSON.parse(tc.args);
      if (parsed.action === "list") label = "List Pipelines";
      else if (parsed.action === "run") {
        const name = (parsed.pipeline_id || "pipeline")
          .replace(/_/g, " ")
          .replace(/\b\w/g, (c: string) => c.toUpperCase());
        label = name;
      }
    } catch { /* keep default */ }
  }
  const color = getToolColor(tc.name);
  const isNoResult = tc.success === undefined;
  const isExpired = isNoResult && isMessageComplete;
  const isRunning = isNoResult && !isMessageComplete;
  const isError = tc.success === false;
  const isShell = tc.name === "run_command" || tc.name === "run_pty_cmd";
  const primary = parseToolPrimary(tc.name, tc.args);

  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "w-full rounded-lg border bg-background/50 px-3 py-2 text-left transition-colors cursor-pointer group",
        isSelected && "ring-1 ring-accent/50 border-accent/40 bg-accent/5",
        isExpired
          ? "border-[#565f89]/30 opacity-60"
          : isRunning
            ? "border-l-2 animate-[pulse-border_2s_ease-in-out_infinite]"
            : isError
              ? "border-red-500/30 hover:border-red-500/50"
              : "border-border/30 hover:border-accent/40",
      )}
      style={isRunning ? { borderLeftColor: color } : undefined}
    >
      <div className="flex items-center gap-2">
        <Wrench className="w-3.5 h-3.5 flex-shrink-0" style={{ color: isExpired ? "var(--muted-foreground)" : color }} />
        <span className="text-[11px] font-medium text-foreground/80">{label}</span>
        <div className="ml-auto flex items-center gap-1.5">
          {isExpired ? (
            <Clock className="w-3 h-3 text-[#565f89]" />
          ) : isRunning ? (
            <Loader2 className="w-3 h-3 text-blue-400 animate-spin" />
          ) : isError ? (
            <XCircle className="w-3 h-3 text-red-400" />
          ) : (
            <CheckCircle2 className="w-3 h-3 text-[var(--ansi-green)]" />
          )}
          {isExpired ? (
            <span className="text-[10px] text-[#565f89]">Expired</span>
          ) : (
            <span className="text-[10px] text-muted-foreground/60 group-hover:text-accent/60 transition-colors">
              Details →
            </span>
          )}
        </div>
      </div>
      {primary && (
        <div
          className={cn(
            "mt-1.5 text-[10px] font-mono truncate px-1.5 py-0.5 rounded",
            isShell
              ? "bg-[var(--ansi-black)]/30 text-[var(--ansi-green)]/80"
              : "bg-muted/30 text-muted-foreground/70",
          )}
        >
          {isShell && <span className="text-muted-foreground/60 mr-1">$</span>}
          {primary}
        </div>
      )}
    </button>
  );
}

export function CollapsibleToolCall({
  tc,
  approval,
  onApprove,
  onDeny,
  approvalMode,
  onApprovalModeChange,
}: {
  tc: { name: string; args?: string; result?: string; success?: boolean };
  approval?: { requestId: string } | null;
  onApprove?: (requestId: string) => void;
  onDeny?: (requestId: string) => void;
  approvalMode?: string;
  onApprovalModeChange?: (mode: "ask" | "allowlist" | "run-all") => void;
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
        {tc.success !== undefined && (
          <span className={cn("ml-auto", tc.success ? "text-green-500" : "text-red-500")}>
            {tc.success ? "\u2713" : "\u2717"}
          </span>
        )}
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
              <pre className="text-[11px] text-muted-foreground/80 font-mono whitespace-pre-wrap max-h-[200px] overflow-auto">
                {tc.result.length > 2000 ? `${tc.result.slice(0, 2000)}...` : tc.result}
              </pre>
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
              {approvalMode === "run-all"
                ? "Run Everything"
                : approvalMode === "allowlist"
                  ? "Use Allowlist"
                  : "Ask Every Time"}
              <ChevronDown className="w-2.5 h-2.5" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="start"
            className="bg-card border-[var(--border-medium)] min-w-[160px]"
          >
            {[
              { id: "ask" as const, label: "Ask Every Time" },
              { id: "allowlist" as const, label: "Use Allowlist" },
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
}: {
  toolCalls: Array<{ name: string; args?: string; result?: string; success?: boolean; requestId?: string }>;
  requestIds?: string[];
  isMessageComplete?: boolean;
}) {
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);

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

  const backfillTimeline = (state: ReturnType<typeof useStore.getState>, sessionId: string, calls: typeof toolCalls) => {
    const timeline = state.timelines[sessionId] ?? [];
    const existingIds = new Set(
      timeline
        .filter((b): b is { type: "ai_tool_execution"; data: { requestId: string } } & typeof b =>
          b.type === "ai_tool_execution"
        )
        .map((b) => b.data.requestId)
    );

    for (const tc of calls) {
      if (!tc.requestId || existingIds.has(tc.requestId)) continue;
      if (tc.name.startsWith("sub_agent_")) continue;
      if (tc.name === "run_pipeline") continue;

      let parsedArgs: Record<string, unknown> = {};
      try {
        if (tc.args) parsedArgs = JSON.parse(tc.args);
      } catch { /* keep empty */ }

      state.addToolExecutionBlock(sessionId, {
        requestId: tc.requestId,
        toolName: tc.name,
        args: parsedArgs,
      });

      if (tc.success !== undefined) {
        state.completeToolExecutionBlock(sessionId, tc.requestId, tc.success, tc.result);
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
    backfillTimeline(state, sessionId, toolCalls);
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
          />
        );
      })}
    </div>
  );
}
