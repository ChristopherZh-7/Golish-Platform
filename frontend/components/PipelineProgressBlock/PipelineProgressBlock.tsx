import { memo, useMemo, useState } from "react";
import {
  CheckCircle2,
  Circle,
  Loader2,
  AlertTriangle,
  SkipForward,
  Play,
  Target,
  Clock,
  ChevronRight,
  ChevronDown,
} from "lucide-react";
import { stripAllAnsi } from "@/lib/ansi";
import { cn } from "@/lib/utils";
import type { PipelineExecution, PipelineStepExecution, PipelineStepStatus } from "@/store";

interface PipelineProgressBlockProps {
  execution: PipelineExecution;
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
  }
}

function formatDuration(ms: number): string {
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
  const canExpand = hasOutput && (step.status === "success" || step.status === "failed");
  const cleanOutput = useMemo(
    () => (step.output ? stripAllAnsi(step.output) : ""),
    [step.output],
  );

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
          "text-[11px] font-medium flex-shrink-0",
          step.status === "pending" && "text-muted-foreground/50",
          step.status === "running" && "text-blue-300",
          step.status === "success" && "text-foreground/70",
          step.status === "failed" && "text-red-300",
          step.status === "skipped" && "text-muted-foreground/40",
        )}>
          {step.name}
        </span>
        <span className="text-[10px] text-muted-foreground/40 font-mono truncate flex-1">
          {step.command}
        </span>
        {step.durationMs != null && (
          <span className="text-[9px] text-muted-foreground/40 tabular-nums flex-shrink-0">
            {formatDuration(step.durationMs)}
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

      {isExpanded && hasOutput && (
        <div className="mx-3 mb-2 rounded border border-border/10 bg-background/50 overflow-hidden">
          <pre className="max-h-[200px] overflow-auto p-2 text-[11px] font-mono text-foreground/70 whitespace-pre-wrap break-words leading-relaxed">
            {cleanOutput}
          </pre>
        </div>
      )}
    </div>
  );
}

export const PipelineProgressBlock = memo(function PipelineProgressBlock({
  execution,
}: PipelineProgressBlockProps) {
  const [expandedSteps, setExpandedSteps] = useState<Set<string>>(new Set());

  const completedSteps = execution.steps.filter(
    (s) => s.status === "success" || s.status === "failed" || s.status === "skipped"
  ).length;
  const totalSteps = execution.steps.length;
  const progressPercent = totalSteps > 0 ? (completedSteps / totalSteps) * 100 : 0;
  const isRunning = execution.status === "running";
  const isCompleted = execution.status === "completed";
  const isFailed = execution.status === "failed";

  const toggleStep = (stepId: string) => {
    setExpandedSteps((prev) => {
      const next = new Set(prev);
      if (next.has(stepId)) next.delete(stepId);
      else next.add(stepId);
      return next;
    });
  };

  return (
    <div className="rounded-lg border border-border/30 bg-card/50 overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 bg-muted/30">
        <div className={cn(
          "w-5 h-5 rounded flex items-center justify-center",
          isRunning && "bg-blue-500/20",
          isCompleted && "bg-emerald-500/20",
          isFailed && "bg-red-500/20",
          !isRunning && !isCompleted && !isFailed && "bg-muted-foreground/10",
        )}>
          {isRunning ? (
            <Play className="w-3 h-3 text-blue-400" />
          ) : isCompleted ? (
            <CheckCircle2 className="w-3 h-3 text-emerald-400" />
          ) : isFailed ? (
            <AlertTriangle className="w-3 h-3 text-red-400" />
          ) : (
            <Clock className="w-3 h-3 text-muted-foreground" />
          )}
        </div>
        <span className="text-[11px] font-medium text-foreground/80">
          {execution.pipelineName}
        </span>
        <div className="flex items-center gap-1 ml-auto">
          <Target className="w-3 h-3 text-muted-foreground/60" />
          <span className="text-[10px] text-muted-foreground font-mono">{execution.target}</span>
        </div>
      </div>

      {/* Progress bar */}
      <div className="h-[2px] bg-muted/30">
        <div
          className={cn(
            "h-full transition-all duration-500",
            isCompleted && "bg-emerald-400",
            isFailed && "bg-red-400",
            isRunning && "bg-blue-400",
            !isRunning && !isCompleted && !isFailed && "bg-muted-foreground/30",
          )}
          style={{ width: `${progressPercent}%` }}
        />
      </div>

      {/* Steps */}
      <div className="divide-y divide-border/10">
        {execution.steps.map((step) => (
          <StepRow
            key={step.stepId}
            step={step}
            isExpanded={expandedSteps.has(step.stepId)}
            onToggle={() => toggleStep(step.stepId)}
          />
        ))}
      </div>

      {/* Footer */}
      <div className="flex items-center gap-2 px-3 py-1.5 bg-muted/20 text-[10px] text-muted-foreground/50">
        <span>{completedSteps}/{totalSteps} steps</span>
        {execution.finishedAt && execution.startedAt && (
          <span className="ml-auto">
            {formatDuration(new Date(execution.finishedAt).getTime() - new Date(execution.startedAt).getTime())}
          </span>
        )}
      </div>
    </div>
  );
});
