import {
  Bot,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Clock,
  Code2,
  Eye,
  FileText,
  GitBranch,
  KeyRound,
  List,
  Loader2,
  MessageSquare,
  Search,
  Shield,
  ShieldQuestion,
  Terminal,
  Wrench,
  XCircle,
  Zap,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import type { ActiveSubAgent } from "@/store/store-types";

export function PlanStepIcon({ status, size = "sm" }: { status: string; size?: "sm" | "md" }) {
  const s = size === "sm" ? "w-2.5 h-2.5" : "w-3.5 h-3.5";
  switch (status) {
    case "completed": return <CheckCircle2 className={cn(s, "text-green-500/70 flex-shrink-0")} />;
    case "in_progress": return <Loader2 className={cn(s, "text-accent animate-spin flex-shrink-0")} />;
    case "failed":
    case "cancelled": return <XCircle className={cn(s, "text-red-400/60 flex-shrink-0")} />;
    default: return <div className={cn(s, "rounded-full border border-muted-foreground/30 flex-shrink-0")} />;
  }
}

export function ThinkingBlock({ content, isActive }: { content: string; isActive: boolean }) {
  const [expanded, setExpanded] = useState(false);
  const preview = content.length > 80 ? content.slice(0, 80) + "..." : content;

  return (
    <div className="mb-2">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-1.5 text-[11px] text-muted-foreground/50 hover:text-muted-foreground/70 transition-colors"
      >
        {isActive ? (
          <Loader2 className="w-3 h-3 animate-spin" />
        ) : (
          <ChevronDown className={cn("w-3 h-3 transition-transform", !expanded && "-rotate-90")} />
        )}
        <span className="italic">{expanded ? "Thinking" : preview}</span>
      </button>
      {expanded && (
        <div className="mt-1.5 pl-4.5 text-[12px] text-muted-foreground/60 leading-[1.6] whitespace-pre-wrap border-l-2 border-muted-foreground/15 ml-1.5 pl-3">
          {content}
        </div>
      )}
    </div>
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

// === Compact tool call summary (always visible) ===

const TOOL_LABEL: Record<string, string> = {
  run_command: "Shell",
  run_pty_cmd: "Shell",
  read_file: "Read",
  write_file: "Write",
  edit_file: "Edit",
  search_files: "Search",
  web_search: "Web",
  web_fetch: "Fetch",
  manage_targets: "Targets",
  record_finding: "Finding",
};

const TOOL_ICON_COLOR: Record<string, string> = {
  run_command: "var(--ansi-green)",
  run_pty_cmd: "var(--ansi-green)",
  read_file: "var(--ansi-cyan)",
  write_file: "var(--ansi-yellow)",
  edit_file: "var(--ansi-yellow)",
  search_files: "var(--ansi-blue)",
  web_search: "var(--ansi-magenta)",
  web_fetch: "var(--ansi-magenta)",
};

function parseToolPrimary(
  name: string,
  argsStr?: string
): string | null {
  if (!argsStr) return null;
  try {
    const args = JSON.parse(argsStr);
    if ((name === "run_command" || name === "run_pty_cmd") && args.command) return args.command;
    if (args.path) return args.path;
    if (args.file_path) return args.file_path;
    if (args.pattern) return args.pattern;
    if (args.query) return args.query;
    if (args.url) return args.url;
  } catch { /* ignore */ }
  return null;
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
  let label = TOOL_LABEL[tc.name] || tc.name.replace(/_/g, " ");
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
  const color = TOOL_ICON_COLOR[tc.name] || "var(--ansi-blue)";
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

const SUB_AGENT_COLORS: Record<string, string> = {
  planner: "var(--ansi-blue)",
  coder: "var(--ansi-green)",
  researcher: "var(--ansi-yellow)",
  reviewer: "var(--ansi-cyan)",
  explorer: "var(--ansi-yellow)",
  analyst: "var(--ansi-cyan)",
  adviser: "var(--ansi-cyan)",
  reporter: "#10b981",
  pentester: "var(--ansi-red)",
  memorist: "var(--ansi-blue)",
  reflector: "var(--ansi-magenta)",
};

const SUB_AGENT_ICONS: Record<string, typeof Bot> = {
  coder: Code2,
  researcher: Search,
  explorer: Search,
  adviser: Shield,
  reporter: FileText,
  pentester: Terminal,
};

function getSubAgentColor(name: string): string {
  const lower = name.toLowerCase();
  for (const [key, color] of Object.entries(SUB_AGENT_COLORS)) {
    if (lower.includes(key)) return color;
  }
  return "var(--ansi-magenta)";
}

function getSubAgentIcon(name: string): typeof Bot {
  const lower = name.toLowerCase();
  for (const [key, icon] of Object.entries(SUB_AGENT_ICONS)) {
    if (lower.includes(key)) return icon;
  }
  return Bot;
}

function SubAgentInlineCard({
  tc,
  agent,
  onClick,
  isMessageComplete,
}: {
  tc: { name: string; args?: string; result?: string; success?: boolean };
  agent?: ActiveSubAgent;
  onClick: () => void;
  isMessageComplete?: boolean;
}) {
  const agentName = agent?.agentName || tc.name.replace(/^sub_agent_/, "").replace(/_/g, " ");
  const color = getSubAgentColor(agentName);
  const AgentIcon = getSubAgentIcon(agentName);
  const isNoResult = tc.success === undefined;
  const isRunning = isNoResult && !isMessageComplete;
  const isError = tc.success === false;
  const task = agent?.task;
  const toolCount = agent?.toolCalls.length ?? 0;
  const durationMs = agent?.durationMs;

  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "w-full rounded-lg border border-border/30 bg-card px-3 py-2 text-left transition-colors cursor-pointer",
        "hover:bg-muted/20",
        isError && "border-red-500/20",
        !isRunning && !isError && tc.success && "opacity-70",
      )}
      style={{ borderLeftWidth: 2, borderLeftColor: isError ? "rgb(239 68 68 / 0.4)" : isRunning ? color : "var(--border)" }}
    >
      <div className="flex items-center gap-2">
        <AgentIcon className="w-3.5 h-3.5 flex-shrink-0 text-muted-foreground/60" />
        <span className="text-xs font-medium text-foreground/80">
          {agentName}
        </span>
        <div className="ml-auto flex items-center gap-1.5">
          {toolCount > 0 && (
            <span className="text-[10px] text-muted-foreground/40">{toolCount} tool{toolCount > 1 ? "s" : ""}</span>
          )}
          {durationMs != null && (
            <span className="text-[10px] text-muted-foreground/30 tabular-nums">
              {durationMs < 1000 ? `${durationMs}ms` : `${(durationMs / 1000).toFixed(1)}s`}
            </span>
          )}
          {isRunning ? (
            <Loader2 className="w-3 h-3 animate-spin text-muted-foreground/40" />
          ) : isError ? (
            <XCircle className="w-3 h-3 text-red-400/60" />
          ) : (
            <CheckCircle2 className="w-3 h-3 text-green-500/60" />
          )}
        </div>
      </div>
      {task && (
        <div className="mt-1 text-[10px] text-muted-foreground/40 truncate pl-5.5">
          {task}
        </div>
      )}
    </button>
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

  // Clear selection ring when another detail view opens or view mode changes away
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

    // Toggle: clicking the same card again closes the detail view
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

// === AskHuman inline card ===

export interface AskHumanState {
  requestId: string;
  sessionId: string;
  question: string;
  inputType: "credentials" | "choice" | "freetext" | "confirmation";
  options: string[];
  context: string;
}

const INPUT_TYPE_ICONS: Record<string, typeof KeyRound> = {
  credentials: KeyRound,
  choice: List,
  freetext: MessageSquare,
  confirmation: ShieldQuestion,
};

export function AskHumanInline({
  request,
  onSubmit,
  onSkip,
}: {
  request: AskHumanState;
  onSubmit: (response: string) => void;
  onSkip: () => void;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [freetext, setFreetext] = useState("");
  const [selectedOptions, setSelectedOptions] = useState<Set<string>>(new Set());

  const Icon = INPUT_TYPE_ICONS[request.inputType] || MessageSquare;

  const handleSubmit = () => {
    let response = "";
    switch (request.inputType) {
      case "credentials":
        response = JSON.stringify({ username, password });
        break;
      case "choice":
        response = Array.from(selectedOptions).join(", ");
        break;
      case "freetext":
        response = freetext;
        break;
      case "confirmation":
        response = "yes";
        break;
    }
    onSubmit(response);
  };

  return (
    <div className="mx-4 my-2 rounded-lg border border-[#e0af68]/30 bg-[#e0af68]/5 p-3">
      <div className="flex items-center gap-2 text-[12px] font-medium text-[#e0af68] mb-2">
        <Icon className="w-3.5 h-3.5" />
        AI Needs Your Input
      </div>
      <p className="text-[13px] text-foreground mb-2 whitespace-pre-wrap">{request.question}</p>
      {request.context && (
        <p className="text-[11px] text-muted-foreground/60 mb-2 italic">{request.context}</p>
      )}

      {request.inputType === "credentials" && (
        <div className="space-y-2 mb-2">
          <input
            type="text"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
            placeholder="Username..."
          />
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent"
            placeholder="Password..."
            onKeyDown={(e) => e.key === "Enter" && handleSubmit()}
          />
        </div>
      )}

      {request.inputType === "choice" && (
        <div className="space-y-1 mb-2">
          {request.options.map((opt) => (
            <button
              key={opt}
              type="button"
              onClick={() =>
                setSelectedOptions((prev) => {
                  const next = new Set(prev);
                  if (next.has(opt)) next.delete(opt);
                  else next.add(opt);
                  return next;
                })
              }
              className={cn(
                "w-full text-left px-2.5 py-1.5 rounded-md border text-[12px] transition-colors",
                selectedOptions.has(opt)
                  ? "bg-accent/10 border-accent/50 text-accent"
                  : "bg-background border-border/50 hover:border-muted-foreground/30"
              )}
            >
              {opt}
            </button>
          ))}
        </div>
      )}

      {request.inputType === "freetext" && (
        <textarea
          value={freetext}
          onChange={(e) => setFreetext(e.target.value)}
          className="w-full px-2.5 py-1.5 rounded-md bg-background border border-border/50 text-[12px] focus:outline-none focus:border-accent min-h-[60px] resize-y mb-2"
          placeholder="Type your response..."
        />
      )}

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={handleSubmit}
          className="px-3 py-1 text-[11px] rounded-md bg-accent text-accent-foreground hover:bg-accent/80 font-medium transition-colors"
        >
          {request.inputType === "confirmation" ? "Confirm" : "Submit"}
        </button>
        <button
          type="button"
          onClick={onSkip}
          className="px-3 py-1 text-[11px] rounded-md border border-border/50 text-muted-foreground hover:bg-muted/50 transition-colors"
        >
          Skip
        </button>
      </div>
    </div>
  );
}

// === Workflow Progress ===

export interface WorkflowState {
  id: string;
  name: string;
  currentStep: string;
  stepIndex: number;
  totalSteps: number;
  completedSteps: Array<{ name: string; output?: string; durationMs: number }>;
  status: "running" | "completed" | "error";
  error?: string;
  totalDurationMs?: number;
}

export function WorkflowProgress({ workflow }: { workflow: WorkflowState }) {
  const [expanded, setExpanded] = useState(false);
  const progress =
    workflow.totalSteps > 0 ? (workflow.completedSteps.length / workflow.totalSteps) * 100 : 0;

  return (
    <div className="mx-4 my-2 rounded-lg border border-border/30 bg-background/50 p-3">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 w-full text-left"
      >
        <GitBranch className="w-3.5 h-3.5 text-accent flex-shrink-0" />
        <span className="text-[12px] font-medium text-foreground flex-1">{workflow.name}</span>
        {workflow.status === "running" && <Loader2 className="w-3 h-3 animate-spin text-accent" />}
        {workflow.status === "completed" && <CheckCircle2 className="w-3 h-3 text-green-500" />}
        {workflow.status === "error" && <XCircle className="w-3 h-3 text-destructive" />}
        <ChevronDown
          className={cn(
            "w-3 h-3 text-muted-foreground transition-transform",
            !expanded && "-rotate-90"
          )}
        />
      </button>

      <div className="mt-2 h-1 rounded-full bg-muted/50 overflow-hidden">
        <div
          className={cn(
            "h-full rounded-full transition-all duration-300",
            workflow.status === "error" ? "bg-destructive" : "bg-accent"
          )}
          style={{ width: `${progress}%` }}
        />
      </div>

      <div className="mt-1 text-[10px] text-muted-foreground/60">
        {workflow.status === "running" &&
          `Step ${workflow.stepIndex + 1}/${workflow.totalSteps}: ${workflow.currentStep}`}
        {workflow.status === "completed" &&
          `Completed in ${(workflow.totalDurationMs ?? 0) / 1000}s`}
        {workflow.status === "error" && `Error: ${workflow.error}`}
      </div>

      {expanded && workflow.completedSteps.length > 0 && (
        <div className="mt-2 space-y-1">
          {workflow.completedSteps.map((step, i) => (
            <div
              key={`${step.name}-${i}`}
              className="flex items-center gap-1.5 text-[11px] text-muted-foreground"
            >
              <CheckCircle2 className="w-2.5 h-2.5 text-green-500 flex-shrink-0" />
              <span>{step.name}</span>
              <span className="ml-auto text-[10px] text-muted-foreground/60">
                {(step.durationMs / 1000).toFixed(1)}s
              </span>
            </div>
          ))}
          {workflow.status === "running" && (
            <div className="flex items-center gap-1.5 text-[11px] text-accent">
              <Loader2 className="w-2.5 h-2.5 animate-spin flex-shrink-0" />
              <span>{workflow.currentStep}</span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// === Compaction Notice ===

export function CompactionNotice({ active, tokensBefore }: { active: boolean; tokensBefore?: number }) {
  return (
    <div className="mx-4 my-2 flex items-center gap-2 rounded-md bg-muted/30 px-3 py-2 text-[11px] text-muted-foreground/70">
      {active ? (
        <>
          <Loader2 className="w-3 h-3 animate-spin text-accent" />
          <span>
            Compacting context{tokensBefore ? ` (${(tokensBefore / 1000).toFixed(0)}K tokens)` : ""}
            ...
          </span>
        </>
      ) : (
        <>
          <Zap className="w-3 h-3 text-accent" />
          <span>
            Context compacted
            {tokensBefore ? ` from ${(tokensBefore / 1000).toFixed(0)}K tokens` : ""}
          </span>
        </>
      )}
    </div>
  );
}

// === Task Plan ===

export interface TaskPlanState {
  version: number;
  steps: Array<{ step: string; status: "pending" | "in_progress" | "completed" | "cancelled" | "failed" }>;
  summary: { total: number; completed: number; in_progress: number; pending: number };
  retiredAt?: string;
}

interface NestedToolInfo {
  requestId: string;
  toolName: string;
  status: "running" | "completed" | "error" | "interrupted";
  durationMs?: number;
  primary?: string | null;
}

function StepToolItem({ tool }: { tool: NestedToolInfo }) {
  const label = TOOL_LABEL[tool.toolName] || tool.toolName.replace(/_/g, " ");
  const color = TOOL_ICON_COLOR[tool.toolName] || "var(--ansi-blue)";
  return (
    <div className="flex items-center gap-1.5 py-0.5 text-[10px] text-muted-foreground/70">
      <Wrench className="w-2.5 h-2.5 flex-shrink-0 opacity-60" style={{ color }} />
      <span className="truncate">
        {label}
        {tool.primary && <span className="ml-1 opacity-50 truncate">{tool.primary}</span>}
      </span>
      <span className="ml-auto flex-shrink-0 flex items-center gap-1">
        {tool.status === "running" && <Loader2 className="w-2.5 h-2.5 animate-spin text-[#7aa2f7]" />}
        {tool.status === "completed" && (
          <>
            <CheckCircle2 className="w-2.5 h-2.5 text-green-500" />
            {tool.durationMs != null && (
              <span className="text-[9px] opacity-50">{tool.durationMs < 1000 ? `${tool.durationMs}ms` : `${(tool.durationMs / 1000).toFixed(1)}s`}</span>
            )}
          </>
        )}
        {(tool.status === "error" || tool.status === "interrupted") && <XCircle className="w-2.5 h-2.5 text-red-400" />}
      </span>
    </div>
  );
}

function useStepTools(terminalId: string | null): Map<number, NestedToolInfo[]> {
  const raw = useStore((s) => {
    if (!terminalId) return "{}";
    const timeline = s.timelines[terminalId];
    if (!timeline) return "{}";
    const obj: Record<number, NestedToolInfo[]> = {};
    for (const block of timeline) {
      if (block.type !== "ai_tool_execution") continue;
      const exec = block.data;
      if (exec.planStepIndex == null) continue;
      if (!obj[exec.planStepIndex]) obj[exec.planStepIndex] = [];
      const primary = parseToolPrimary(exec.toolName, exec.args ? JSON.stringify(exec.args) : undefined);
      obj[exec.planStepIndex].push({
        requestId: exec.requestId,
        toolName: exec.toolName,
        status: exec.status,
        durationMs: exec.durationMs,
        primary,
      });
    }
    return JSON.stringify(obj);
  });
  return useMemo(() => {
    const obj = JSON.parse(raw) as Record<string, NestedToolInfo[]>;
    const map = new Map<number, NestedToolInfo[]>();
    for (const [k, v] of Object.entries(obj)) map.set(Number(k), v);
    return map;
  }, [raw]);
}

/** Set of requestIds that are nested inside the plan card (so they can be hidden from message stream) */
export function usePlanNestedRequestIds(terminalId: string | null): Set<string> {
  const raw = useStore((s) => {
    if (!terminalId) return "";
    const timeline = s.timelines[terminalId];
    if (!timeline) return "";
    const arr: string[] = [];
    for (const block of timeline) {
      if (block.type === "ai_tool_execution" && block.data.planStepIndex != null) {
        arr.push(block.data.requestId);
      }
    }
    return arr.join("\n");
  });
  return useMemo(() => new Set(raw ? raw.split("\n") : []), [raw]);
}

function formatRelativeTime(isoString: string | undefined): string | null {
  if (!isoString) return null;
  const diff = Date.now() - new Date(isoString).getTime();
  if (diff < 0) return null;
  const secs = Math.floor(diff / 1000);
  if (secs < 60) return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

export function TaskPlanCard({ plan, terminalId, retired }: { plan: TaskPlanState; terminalId?: string | null; retired?: boolean }) {
  const [expanded, setExpanded] = useState(!retired);
  const [collapsedSteps, setCollapsedSteps] = useState<Set<number>>(new Set());
  const stepTools = useStepTools(terminalId ?? null);
  const progress = plan.summary.total > 0 ? (plan.summary.completed / plan.summary.total) * 100 : 0;
  const isAllDone = plan.summary.total > 0 && plan.summary.completed === plan.summary.total;
  const relativeTime = retired ? formatRelativeTime(plan.retiredAt) : null;

  const handleShowDetail = () => {
    const state = useStore.getState();
    const sid = state.activeSessionId;
    if (sid) {
      state.setDetailViewMode(sid, "tool-detail");
    }
  };

  const toggleStep = (idx: number) => {
    setCollapsedSteps((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  };

  return (
    <div className={cn(
      "mx-4 my-2 rounded-lg border p-3 w-[calc(100%-2rem)] text-left transition-colors group",
      retired
        ? "border-border/20 bg-muted/5 opacity-80"
        : "border-border/30 bg-background/50 hover:border-accent/40 hover:bg-accent/5"
    )}>
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center gap-2 cursor-pointer"
      >
        <List className={cn("w-3.5 h-3.5 flex-shrink-0", retired ? "text-muted-foreground/60" : "text-accent")} />
        <span className={cn("text-[12px] font-medium", retired ? "text-muted-foreground" : "text-foreground")}>
          Task Plan
        </span>
        <span className={cn(
          "text-[9px] px-1.5 py-0.5 rounded font-medium",
          retired
            ? "bg-muted/30 text-muted-foreground/70"
            : "bg-accent/10 text-accent"
        )}>
          v{plan.version}
        </span>
        {retired && (
          <span className="text-[9px] px-1.5 py-0.5 rounded bg-muted/20 text-muted-foreground/60 font-medium">
            Superseded
          </span>
        )}
        {!retired && isAllDone && (
          <span className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-500/70 font-medium">
            Done
          </span>
        )}
        {relativeTime && (
          <span className="text-[9px] text-muted-foreground/50 ml-auto">
            {relativeTime}
          </span>
        )}
        {!retired && (
          <span
            role="link"
            tabIndex={0}
            className="ml-auto text-[10px] text-muted-foreground/70 hover:text-accent/70 transition-colors cursor-pointer"
            onClick={(e) => { e.stopPropagation(); handleShowDetail(); }}
            onKeyDown={(e) => { if (e.key === "Enter") { e.stopPropagation(); handleShowDetail(); } }}
          >
            View details &rarr;
          </span>
        )}
        {expanded ? (
          <ChevronUp className="w-3 h-3 text-muted-foreground/70 group-hover:text-accent/70 transition-colors flex-shrink-0" />
        ) : (
          <ChevronDown className="w-3 h-3 text-muted-foreground/70 group-hover:text-accent/70 transition-colors flex-shrink-0" />
        )}
      </button>

      <div className="mt-2 h-1 rounded-full bg-muted/50 overflow-hidden">
        <div
          className={cn(
            "h-full rounded-full transition-all duration-300",
            retired
              ? "bg-muted-foreground/30"
              : "bg-accent",
          )}
          style={{ width: `${progress}%` }}
        />
      </div>

      {!expanded && retired && (
        <div className="mt-1.5 text-[10px] text-muted-foreground/50">
          {plan.summary.completed}/{plan.summary.total} steps completed
        </div>
      )}

      {expanded && (
        <div className="mt-2 space-y-0.5">
          {plan.steps.map((step, i) => {
            const tools = stepTools.get(i) ?? [];
            const hasTools = tools.length > 0;
            const isActive = step.status === "in_progress";
            const isDone = step.status === "completed";
            const isStepCollapsed = isDone ? !collapsedSteps.has(i) : collapsedSteps.has(i);

            return (
              <div key={`${step.step}-${i}`}>
                <button
                  type="button"
                  onClick={hasTools ? () => toggleStep(i) : undefined}
                  className={cn(
                    "w-full flex items-center gap-1.5 text-[11px] py-0.5",
                    hasTools && "cursor-pointer hover:text-foreground/90",
                  )}
                >
                  <PlanStepIcon status={step.status} />
                  <span
                    className={cn(
                      "text-left truncate",
                      isDone ? "text-muted-foreground/60"
                        : isActive ? "text-accent"
                        : (step.status === "cancelled" || step.status === "failed") ? "text-red-400/60"
                        : "text-muted-foreground",
                    )}
                  >
                    {step.step}
                  </span>
                  {hasTools && (
                    <span className="ml-auto flex items-center gap-0.5 text-[9px] text-muted-foreground/60">
                      {tools.length}
                      <ChevronDown className={cn("w-2 h-2 transition-transform", isStepCollapsed && "-rotate-90")} />
                    </span>
                  )}
                </button>
                {hasTools && !isStepCollapsed && (
                  <div className="ml-4 pl-2 border-l border-border/20 space-y-0">
                    {tools.map((t) => <StepToolItem key={t.requestId} tool={t} />)}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

export function PlanUpdatedNotice() {
  return (
    <div className="flex items-center gap-1.5 px-3 py-1 text-[10px] text-muted-foreground/70">
      <List className="w-2.5 h-2.5" />
      <span>Plan updated</span>
    </div>
  );
}

export function StickyPlanProgress({ plan }: { plan: TaskPlanState }) {
  const [expanded, setExpanded] = useState(false);
  const current = plan.steps.find((s) => s.status === "in_progress");
  const currentIdx = current ? plan.steps.indexOf(current) : -1;
  const progress = plan.summary.total > 0 ? (plan.summary.completed / plan.summary.total) * 100 : 0;
  const isAllDone = plan.summary.total > 0 && plan.summary.completed === plan.summary.total;

  const handleViewDetail = () => {
    const state = useStore.getState();
    const sid = state.activeSessionId;
    if (sid) {
      state.setDetailViewMode(sid, "tool-detail");
    }
  };

  return (
    <div className="sticky top-0 z-20 mx-0 border-b border-border/30 bg-[var(--background)]/90 backdrop-blur-sm">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-muted/20 transition-colors"
      >
        <List className="w-3 h-3 text-accent flex-shrink-0" />
        <span className="text-[11px] font-medium text-foreground/80 truncate flex-1 text-left">
          {isAllDone
            ? "All steps complete"
            : current
              ? `Step ${currentIdx + 1}/${plan.summary.total}: ${current.step}`
              : `${plan.summary.completed}/${plan.summary.total} steps`}
        </span>
        {plan.version > 1 && (
          <span className="text-[9px] px-1 py-0.5 rounded bg-accent/10 text-accent/60 flex-shrink-0">
            v{plan.version}
          </span>
        )}
        <span className="text-[10px] text-muted-foreground/60 flex-shrink-0">
          {Math.round(progress)}%
        </span>
        <ChevronDown className={cn("w-3 h-3 text-muted-foreground/50 flex-shrink-0 transition-transform", expanded && "rotate-180")} />
      </button>

      {expanded && (
        <div className="px-3 pb-2 space-y-0.5">
          {plan.steps.map((step, idx) => {
            const isActive = step.status === "in_progress";
            const isDone = step.status === "completed";
            const isFailed = step.status === "failed" || step.status === "cancelled";
            return (
              <div key={idx} className={cn(
                "flex items-center gap-2 py-1 px-2 rounded text-[11px]",
                isActive && "bg-accent/8",
              )}>
                <PlanStepIcon status={step.status} />
                <span className={cn(
                  "flex-1 truncate",
                  isDone && "text-muted-foreground/60",
                  isActive && "text-accent font-medium",
                  isFailed && "text-red-400/60",
                  !isDone && !isActive && !isFailed && "text-muted-foreground/70",
                )}>
                  {idx + 1}. {step.step}
                </span>
              </div>
            );
          })}
          
          <button
            type="button"
            onClick={(e) => { e.stopPropagation(); handleViewDetail(); }}
            className="flex items-center gap-1.5 mt-1.5 px-2 py-1 rounded text-[10px] text-accent/70 hover:text-accent hover:bg-accent/10 transition-colors"
          >
            <Eye className="w-3 h-3" />
            <span>View Detail</span>
          </button>
        </div>
      )}

      <div className="h-[2px] bg-muted/30">
        <div
          className="h-full bg-accent transition-all duration-500 ease-out"
          style={{ width: `${progress}%` }}
        />
      </div>
    </div>
  );
}

/** Agent summary bar showing active/completed sub-agent counts */
