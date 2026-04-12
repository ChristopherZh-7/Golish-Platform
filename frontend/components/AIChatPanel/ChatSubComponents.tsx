import {
  CheckCircle2,
  ChevronDown,
  GitBranch,
  KeyRound,
  List,
  Loader2,
  MessageSquare,
  ShieldQuestion,
  Wrench,
  XCircle,
  Zap,
} from "lucide-react";
import { useState } from "react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";

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
        <div className="mt-1.5 pl-4.5 text-[12px] text-muted-foreground/40 leading-[1.6] whitespace-pre-wrap border-l-2 border-muted-foreground/10 ml-1.5 pl-3">
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

      {/* Approval mode dropdown - second row */}
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

      {expanded && tc.result && (
        <div className="px-2 pb-2">
          <pre className="text-[11px] text-muted-foreground/80 font-mono whitespace-pre-wrap max-h-[200px] overflow-auto">
            {tc.result.length > 2000 ? `${tc.result.slice(0, 2000)}...` : tc.result}
          </pre>
        </div>
      )}
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
}: {
  tc: { name: string; args?: string; result?: string; success?: boolean };
  onClick: () => void;
}) {
  const label = TOOL_LABEL[tc.name] || tc.name.replace(/_/g, " ");
  const color = TOOL_ICON_COLOR[tc.name] || "var(--ansi-blue)";
  const isRunning = tc.success === undefined;
  const isError = tc.success === false;
  const isShell = tc.name === "run_command" || tc.name === "run_pty_cmd";
  const primary = parseToolPrimary(tc.name, tc.args);

  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "w-full rounded-lg border bg-background/50 px-3 py-2 text-left transition-colors cursor-pointer group",
        isRunning
          ? "border-l-2 animate-[pulse-border_2s_ease-in-out_infinite]"
          : isError
            ? "border-red-500/30 hover:border-red-500/50"
            : "border-border/30 hover:border-accent/40",
      )}
      style={isRunning ? { borderLeftColor: color } : undefined}
    >
      <div className="flex items-center gap-2">
        <Wrench className="w-3.5 h-3.5 flex-shrink-0" style={{ color }} />
        <span className="text-[11px] font-medium text-foreground/80">{label}</span>
        <div className="ml-auto flex items-center gap-1.5">
          {isRunning ? (
            <Loader2 className="w-3 h-3 text-blue-400 animate-spin" />
          ) : isError ? (
            <XCircle className="w-3 h-3 text-red-400" />
          ) : (
            <CheckCircle2 className="w-3 h-3 text-[var(--ansi-green)]" />
          )}
          <span className="text-[10px] text-muted-foreground/40 group-hover:text-accent/60 transition-colors">
            Details →
          </span>
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
          {isShell && <span className="text-muted-foreground/40 mr-1">$</span>}
          {primary}
        </div>
      )}
    </button>
  );
}

export function ToolCallSummary({
  toolCalls,
  requestIds,
}: {
  toolCalls: Array<{ name: string; args?: string; result?: string; success?: boolean }>;
  requestIds?: string[];
}) {
  if (toolCalls.length === 0) return null;

  const handleShowDetail = () => {
    const state = useStore.getState();
    const activeConvId = state.activeConversationId;
    if (!activeConvId) return;
    const termIds = state.conversationTerminals[activeConvId];
    const termId = termIds?.[0];
    if (termId) {
      state.setToolDetailRequestIds(termId, requestIds ?? null);
      state.setDetailViewMode(termId, "tool-detail");
    }
  };

  return (
    <div className="mt-2 space-y-1.5">
      {toolCalls.map((tc, i) => (
        <ToolCallCard
          key={`${tc.name}-${i}`}
          tc={tc}
          onClick={handleShowDetail}
        />
      ))}
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
              <span className="ml-auto text-[10px] text-muted-foreground/40">
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
  steps: Array<{ step: string; status: "pending" | "in_progress" | "completed" }>;
  summary: { total: number; completed: number; in_progress: number; pending: number };
}

export function TaskPlanCard({ plan }: { plan: TaskPlanState }) {
  const progress = plan.summary.total > 0 ? (plan.summary.completed / plan.summary.total) * 100 : 0;
  const isAllDone = plan.summary.total > 0 && plan.summary.completed === plan.summary.total;

  const handleShowDetail = () => {
    const state = useStore.getState();
    const activeConvId = state.activeConversationId;
    if (!activeConvId) return;
    const termIds = state.conversationTerminals[activeConvId];
    const termId = termIds?.[0];
    if (termId) {
      const current = state.sessions[termId]?.detailViewMode;
      state.setDetailViewMode(termId, current === "plan" ? "timeline" : "plan");
    }
  };

  return (
    <button
      type="button"
      onClick={handleShowDetail}
      className="mx-4 my-2 rounded-lg border border-border/30 bg-background/50 p-3 w-[calc(100%-2rem)] text-left hover:border-accent/40 hover:bg-accent/5 transition-colors cursor-pointer group"
    >
      <div className="flex items-center gap-2">
        <List className="w-3.5 h-3.5 text-accent flex-shrink-0" />
        <span className="text-[12px] font-medium text-foreground">
          Task Plan ({plan.summary.completed}/{plan.summary.total})
        </span>
        {isAllDone && (
          <span className="text-[9px] px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 font-medium">
            Done
          </span>
        )}
        <span className="ml-auto text-[10px] text-muted-foreground/50 group-hover:text-accent/70 transition-colors">
          View details →
        </span>
      </div>

      <div className="mt-2 h-1 rounded-full bg-muted/50 overflow-hidden">
        <div
          className={cn(
            "h-full rounded-full transition-all duration-300",
            isAllDone ? "bg-emerald-400" : "bg-accent",
          )}
          style={{ width: `${progress}%` }}
        />
      </div>

      <div className="mt-2 space-y-1">
        {plan.steps.map((step, i) => (
          <div key={`${step.step}-${i}`} className="flex items-center gap-1.5 text-[11px]">
            {step.status === "completed" && (
              <CheckCircle2 className="w-2.5 h-2.5 text-green-500 flex-shrink-0" />
            )}
            {step.status === "in_progress" && (
              <Loader2 className="w-2.5 h-2.5 animate-spin text-accent flex-shrink-0" />
            )}
            {step.status === "pending" && (
              <div className="w-2.5 h-2.5 rounded-full border border-muted-foreground/30 flex-shrink-0" />
            )}
            <span
              className={cn(
                step.status === "completed"
                  ? "text-muted-foreground/60 line-through"
                  : step.status === "in_progress"
                    ? "text-accent"
                    : "text-muted-foreground"
              )}
            >
              {step.step}
            </span>
          </div>
        ))}
      </div>
    </button>
  );
}

/** Agent summary bar showing active/completed sub-agent counts */
