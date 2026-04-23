import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  FileSearch,
  Globe,
  Loader2,
  Network,
  Pencil,
  Search,
  Terminal,
  Wrench,
  XCircle,
} from "lucide-react";
import { memo, useEffect, useMemo, useState } from "react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import type { AiToolExecution } from "@/store";
import { useStore } from "@/store";
import { useTaskPlanState } from "@/store/selectors";

interface InlineTaskPlanProps {
  sessionId: string;
  className?: string;
}

const TOOL_ICONS: Record<string, typeof Terminal> = {
  run_command: Terminal,
  run_pty_cmd: Terminal,
  read_file: FileSearch,
  write_file: Pencil,
  edit_file: Pencil,
  search_files: Search,
  web_search: Globe,
  web_fetch: Globe,
  manage_targets: Network,
};

const TOOL_LABELS: Record<string, string> = {
  run_command: "Shell Command",
  run_pty_cmd: "Shell Command",
  read_file: "Read File",
  write_file: "Write File",
  edit_file: "Edit File",
  search_files: "Search Files",
  web_search: "Web Search",
  web_fetch: "Fetch URL",
  manage_targets: "Manage Targets",
  record_finding: "Record Finding",
  credential_vault: "Credential Vault",
  js_collect: "JS Collect",
  run_pipeline: "Run Pipeline",
  flow_compose: "Flow Compose",
  pentest_run: "Pentest Run",
  pentest_list_tools: "List Tools",
  pentest_read_skill: "Read Skill",
};

function getToolLabel(name: string): string {
  return TOOL_LABELS[name] || name.replace(/_/g, " ");
}

function getToolPrimaryArg(name: string, args: Record<string, unknown>): string | null {
  if ((name === "run_command" || name === "run_pty_cmd") && args.command)
    return String(args.command);
  if (args.path) return String(args.path);
  if (args.file_path) return String(args.file_path);
  if (args.url) return String(args.url);
  if (args.query) return String(args.query);
  if (args.pattern) return String(args.pattern);
  return null;
}

/** Compact inline tool execution item for nesting under plan steps. */
const CompactToolItem = memo(function CompactToolItem({
  exec,
}: {
  exec: AiToolExecution;
}) {
  const Icon = TOOL_ICONS[exec.toolName] || Wrench;
  const label = getToolLabel(exec.toolName);
  const primaryArg = getToolPrimaryArg(exec.toolName, exec.args);
  const isRunning = exec.status === "running";
  const isError = exec.status === "error";
  const isOk = exec.status === "completed";

  return (
    <div className="flex items-center gap-1.5 py-0.5 text-[11px] text-muted-foreground">
      <Icon className="w-3 h-3 flex-shrink-0 opacity-60" />
      <span className="truncate">
        {label}
        {primaryArg && (
          <span className="ml-1 opacity-50 truncate">{primaryArg}</span>
        )}
      </span>
      <span className="ml-auto flex-shrink-0 flex items-center gap-1">
        {isRunning && <Loader2 className="w-3 h-3 animate-spin text-[#7aa2f7]" />}
        {isOk && (
          <>
            <CheckCircle2 className="w-3 h-3 text-green-500" />
            {exec.durationMs != null && (
              <span className="text-[10px] opacity-50">{exec.durationMs}ms</span>
            )}
          </>
        )}
        {isError && <XCircle className="w-3 h-3 text-red-400" />}
      </span>
    </div>
  );
});

/**
 * Inline Task Plan component that displays above the path/git badges in UnifiedInput.
 * Shows a compact progress bar when collapsed, and full step list when expanded.
 * Tool executions are grouped under their parent plan step via planStepIndex.
 * Only renders when a plan exists for the session.
 * Shows a completion badge when all steps are done (auto-hides after 8s).
 */
export const InlineTaskPlan = memo(function InlineTaskPlan({
  sessionId,
  className,
}: InlineTaskPlanProps) {
  const { plan } = useTaskPlanState(sessionId);
  const timeline = useStore((state) => state.timelines[sessionId]);
  const [isExpanded, setIsExpanded] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  const isPlanComplete =
    !!plan && plan.summary.total > 0 && plan.summary.completed === plan.summary.total;

  useEffect(() => {
    if (!isPlanComplete) {
      setDismissed(false);
      return;
    }
    const timer = setTimeout(() => setDismissed(true), 8000);
    return () => clearTimeout(timer);
  }, [isPlanComplete]);

  const toolsByStep = useMemo(() => {
    if (!timeline) return new Map<number, AiToolExecution[]>();
    const map = new Map<number, AiToolExecution[]>();
    for (const block of timeline) {
      if (block.type !== "ai_tool_execution") continue;
      const exec = block.data as AiToolExecution;
      const idx = exec.planStepIndex;
      if (idx == null) continue;
      let list = map.get(idx);
      if (!list) {
        list = [];
        map.set(idx, list);
      }
      list.push(exec);
    }
    return map;
  }, [timeline]);

  if (!plan || plan.steps.length === 0) return null;
  if (isPlanComplete && dismissed) return null;

  const { summary, steps, explanation } = plan;
  const progressPercentage = summary.total > 0 ? (summary.completed / summary.total) * 100 : 0;

  return (
    <Collapsible open={isExpanded} onOpenChange={setIsExpanded}>
      <div className={cn("border-b border-[var(--border-subtle)]", className)}>
        <CollapsibleTrigger className="w-full">
          <div className="flex items-center gap-2 px-4 py-1.5 hover:bg-accent/30 transition-colors">
            {isPlanComplete ? (
              <CheckCircle2 className="w-3.5 h-3.5 text-green-500 flex-shrink-0" />
            ) : isExpanded ? (
              <ChevronDown className="w-3.5 h-3.5 text-[#7aa2f7] flex-shrink-0" />
            ) : (
              <ChevronRight className="w-3.5 h-3.5 text-[#7aa2f7] flex-shrink-0" />
            )}

            <span className="text-xs font-medium text-foreground">Task Plan</span>

            {!isPlanComplete && (
              <div className="flex-1 mx-2 h-1.5 bg-muted/30 rounded-full overflow-hidden max-w-[200px]">
                <div
                  className="h-full bg-[#7aa2f7] transition-all duration-300 ease-out"
                  style={{ width: `${progressPercentage}%` }}
                />
              </div>
            )}

            <span className="text-xs text-muted-foreground">
              {summary.completed}/{summary.total} steps
            </span>
            {isPlanComplete ? (
              <span className="text-[9px] px-1.5 py-0.5 rounded bg-emerald-500/15 text-emerald-400 font-medium">
                Done
              </span>
            ) : (
              <span className="text-xs font-medium text-[#7aa2f7]">
                ({Math.round(progressPercentage)}%)
              </span>
            )}
          </div>
        </CollapsibleTrigger>

        <CollapsibleContent>
          <div className="px-4 pb-2 space-y-0.5">
            {explanation && (
              <p className="text-xs text-muted-foreground italic border-l-2 border-l-muted pl-2 py-1 ml-5 mb-1">
                {explanation}
              </p>
            )}

            <div className="ml-5">
              {steps.map((step, index) => {
                const isCompleted = step.status === "completed";
                const isInProgress = step.status === "in_progress";
                const isPending = step.status === "pending";
                const isCancelled = step.status === "cancelled" || step.status === "failed";
                const stepTools = toolsByStep.get(index) ?? [];
                const hasTools = stepTools.length > 0;
                const showTools = (isCompleted || isInProgress) && hasTools;

                return (
                  <StepRow
                    key={`${index}-${step.step}`}
                    label={step.step}
                    isCompleted={isCompleted}
                    isInProgress={isInProgress}
                    isPending={isPending}
                    isCancelled={isCancelled}
                    tools={showTools ? stepTools : []}
                    defaultOpen={isInProgress}
                  />
                );
              })}
            </div>
          </div>
        </CollapsibleContent>
      </div>
    </Collapsible>
  );
});

/** A single plan step row with collapsible tool list. */
const StepRow = memo(function StepRow({
  label,
  isCompleted,
  isInProgress,
  isPending,
  isCancelled,
  tools,
  defaultOpen,
}: {
  label: string;
  isCompleted: boolean;
  isInProgress: boolean;
  isPending: boolean;
  isCancelled: boolean;
  tools: AiToolExecution[];
  defaultOpen: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);

  useEffect(() => {
    if (isInProgress) setOpen(true);
  }, [isInProgress]);

  const hasTools = tools.length > 0;

  const stepContent = (
    <div
      className={cn(
        "flex items-start gap-2 px-2 py-1 rounded text-xs transition-colors",
        isInProgress && "bg-accent/30",
        isCompleted && "opacity-70",
        isCancelled && "opacity-40",
        hasTools && "cursor-pointer hover:bg-accent/20"
      )}
      onClick={hasTools ? () => setOpen((prev) => !prev) : undefined}
    >
      {isCompleted && (
        <CheckCircle2 className="w-3.5 h-3.5 text-green-500 flex-shrink-0 mt-0.5" />
      )}
      {isInProgress && (
        <Loader2 className="w-3.5 h-3.5 text-[#7aa2f7] animate-spin flex-shrink-0 mt-0.5" />
      )}
      {isCancelled && (
        <XCircle className="w-3.5 h-3.5 text-red-400/70 flex-shrink-0 mt-0.5" />
      )}
      {isPending && (
        <Circle className="w-3.5 h-3.5 text-muted-foreground flex-shrink-0 mt-0.5" />
      )}

      <span
        className={cn(
          "flex-1 leading-relaxed",
          isCompleted && "line-through text-muted-foreground",
          isInProgress && "font-medium text-foreground",
          isCancelled && "line-through text-red-400/60",
          isPending && "text-muted-foreground"
        )}
      >
        {label}
      </span>

      {hasTools && (
        <span className="flex-shrink-0 text-[10px] text-muted-foreground/50">
          {tools.length} {open ? "▾" : "▸"}
        </span>
      )}
    </div>
  );

  if (!hasTools) return stepContent;

  return (
    <div>
      {stepContent}
      {open && (
        <div className="ml-7 pl-2 border-l border-[var(--border-subtle)] mb-1">
          {tools.map((exec) => (
            <CompactToolItem key={exec.requestId} exec={exec} />
          ))}
        </div>
      )}
    </div>
  );
});
