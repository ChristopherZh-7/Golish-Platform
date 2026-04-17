import { memo, useEffect, useMemo, useState } from "react";
import {
  ArrowDownRight,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Circle,
  Loader2,
  SkipForward,
  AlertTriangle,
  Target,
} from "lucide-react";
import { stripAllAnsi } from "@/lib/ansi";
import { cn } from "@/lib/utils";
import type { PipelineExecution, PipelineStepExecution, PipelineStepStatus } from "@/store";
import { SubAgentCard } from "@/components/SubAgentCard";
import { TaskGroupShell } from "@/components/TaskGroupShell";

interface PipelineProgressBlockProps {
  execution: PipelineExecution;
}

const AI_STEP_PREFIXES = ["AI:", "ai:"];

function isAiStep(step: PipelineStepExecution): boolean {
  return AI_STEP_PREFIXES.some((p) => step.command.startsWith(p)) || step.name.includes("(AI)");
}

function StatusIcon({ status }: { status: PipelineStepStatus }) {
  switch (status) {
    case "pending":
      return <Circle className="w-3.5 h-3.5 text-muted-foreground/50" />;
    case "running":
      return <Loader2 className="w-3.5 h-3.5 text-blue-400 animate-spin" />;
    case "success":
      return <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />;
    case "failed":
      return <AlertTriangle className="w-3.5 h-3.5 text-red-400" />;
    case "skipped":
      return <SkipForward className="w-3.5 h-3.5 text-muted-foreground/40" />;
    case "interrupted":
      return <AlertTriangle className="w-3.5 h-3.5 text-amber-400/60" />;
  }
}

function formatStepDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60000)}m ${Math.round((ms % 60000) / 1000)}s`;
}

function StepRow({ step, isExpanded, onToggle }: {
  step: PipelineStepExecution;
  isExpanded: boolean;
  onToggle: () => void;
}) {
  const hasOutput = !!step.output?.trim();
  const hasFanOut = (step.subTargets?.length ?? 0) > 1;
  const hasSubAgents = (step.subAgents?.length ?? 0) > 0;
  const canExpand =
    hasSubAgents ||
    ((hasOutput || hasFanOut) && (step.status === "success" || step.status === "failed"));
  const cleanOutput = useMemo(
    () => (step.output ? stripAllAnsi(step.output) : ""),
    [step.output],
  );
  const ai = isAiStep(step);
  const fanOutCount = step.discoveredTargets?.length ?? 0;

  return (
    <div>
      <button
        type="button"
        className={cn(
          "flex items-center gap-2 px-3 py-1.5 w-full text-left transition-colors",
          step.status === "running" && "bg-blue-500/5",
          step.status === "failed" && "bg-red-500/5",
          canExpand && "hover:bg-muted/20 cursor-pointer",
          !canExpand && "cursor-default",
        )}
        onClick={canExpand ? onToggle : undefined}
      >
        <StatusIcon status={step.status} />
        <span className={cn(
          "text-[11px] font-medium flex-shrink-0 min-w-[80px]",
          step.status === "pending" && "text-muted-foreground/50",
          step.status === "running" && "text-blue-300",
          step.status === "success" && "text-foreground/70",
          step.status === "failed" && "text-red-300",
          step.status === "skipped" && "text-muted-foreground/40",
        )}>
          {step.name}
          {fanOutCount > 1 && (
            <span className="text-[9px] text-muted-foreground/50 ml-1">(x{fanOutCount})</span>
          )}
        </span>
        {ai && (
          <span className="text-[9px] font-mono px-1 rounded bg-amber-500/15 text-amber-400 flex-shrink-0">
            AI
          </span>
        )}
        {step.command && step.command !== step.name && (
          <span className="text-[10px] text-muted-foreground/40 font-mono truncate flex-1">
            {ai ? step.command.replace(/^AI:\s*/, "") : step.command}
          </span>
        )}
        {(!step.command || step.command === step.name) && <span className="flex-1" />}
        {step.durationMs != null && (
          <span className="text-[9px] text-muted-foreground/40 tabular-nums flex-shrink-0">
            {formatStepDuration(step.durationMs)}
          </span>
        )}
        {step.exitCode != null && step.exitCode !== 0 && (
          <span className="text-[9px] text-red-400/70 flex-shrink-0">
            exit {step.exitCode}
          </span>
        )}
        {canExpand && (
          isExpanded
            ? <ChevronDown className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />
            : <ChevronRight className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />
        )}
      </button>

      {isExpanded && hasFanOut && (
        <div className="mx-3 mb-1 space-y-0.5">
          {step.subTargets?.map((sub) => (
            <div key={sub.target} className="flex items-center gap-2 px-2 py-0.5 text-[10px] rounded bg-muted/20">
              <StatusIcon status={sub.status} />
              <span className="font-mono text-muted-foreground/70 truncate">{sub.target}</span>
              {sub.durationMs != null && (
                <span className="ml-auto text-muted-foreground/40 tabular-nums">{formatStepDuration(sub.durationMs)}</span>
              )}
            </div>
          ))}
        </div>
      )}

      {isExpanded && hasOutput && !hasFanOut && (
        <div className="mx-3 mb-2 rounded border border-border/10 bg-background/50 overflow-hidden">
          <pre className="max-h-[200px] overflow-auto p-2 text-[11px] font-mono text-foreground/70 whitespace-pre-wrap break-words leading-relaxed">
            {cleanOutput}
          </pre>
        </div>
      )}

      {/* Nested sub-agents for AI steps (compact inline style) */}
      {isExpanded && hasSubAgents && (
        <div className="mx-3 mb-1.5 space-y-0.5">
          {step.subAgents!.map((agent) => (
            <SubAgentCard
              key={agent.parentRequestId}
              subAgent={agent}
              compact
            />
          ))}
        </div>
      )}
    </div>
  );
}

export const PipelineProgressBlock = memo(function PipelineProgressBlock({
  execution,
}: PipelineProgressBlockProps) {
  const [expandedSteps, setExpandedSteps] = useState<Set<string>>(new Set());

  // Auto-expand AI steps when sub-agents are attached at runtime
  useEffect(() => {
    for (const step of execution.steps) {
      if (step.subAgents && step.subAgents.length > 0) {
        setExpandedSteps((prev) => {
          if (prev.has(step.stepId)) return prev;
          const next = new Set(prev);
          next.add(step.stepId);
          return next;
        });
      }
    }
  }, [execution.steps]);

  const running = execution.steps.filter((s) => s.status === "running").length;
  const completed = execution.steps.filter((s) => s.status === "success").length;
  const failed = execution.steps.filter((s) => s.status === "failed" || s.status === "interrupted").length;
  const skipped = execution.steps.filter((s) => s.status === "skipped").length;

  const totalDurationMs = useMemo(() => {
    if (execution.finishedAt && execution.startedAt) {
      return new Date(execution.finishedAt).getTime() - new Date(execution.startedAt).getTime();
    }
    return execution.steps.reduce((sum, s) => sum + (s.durationMs ?? 0), 0);
  }, [execution.finishedAt, execution.startedAt, execution.steps]);

  const toggleStep = (stepId: string) => {
    setExpandedSteps((prev) => {
      const next = new Set(prev);
      if (next.has(stepId)) next.delete(stepId);
      else next.add(stepId);
      return next;
    });
  };

  return (
    <TaskGroupShell
      title={execution.pipelineName}
      titleExtra={
        <span className="flex items-center gap-1 text-muted-foreground/40">
          <Target className="w-3 h-3" />
          <span className="font-mono text-[10px]">{execution.target}</span>
        </span>
      }
      running={running}
      completed={completed}
      failed={failed}
      total={execution.steps.length}
      totalDurationMs={totalDurationMs}
      hasFailure={failed > 0}
    >
      <div className="divide-y divide-border/10">
        {execution.steps.map((step, idx) => (
          <div key={step.stepId}>
            <StepRow
              step={step}
              isExpanded={expandedSteps.has(step.stepId)}
              onToggle={() => toggleStep(step.stepId)}
            />
            {(step.discoveredTargets?.length ?? 0) > 0 && idx < execution.steps.length - 1 && (
              <div className="flex items-center gap-1.5 px-3 py-0.5 text-[9px] text-cyan-400/60">
                <ArrowDownRight className="w-2.5 h-2.5" />
                <span className="font-mono">
                  {step.discoveredTargets!.length} target{step.discoveredTargets!.length > 1 ? "s" : ""} &rarr;
                </span>
                <span className="truncate text-muted-foreground/40">
                  {step.discoveredTargets!.slice(0, 3).join(", ")}
                  {step.discoveredTargets!.length > 3 && ` +${step.discoveredTargets!.length - 3}`}
                </span>
              </div>
            )}
          </div>
        ))}
      </div>
    </TaskGroupShell>
  );
});
