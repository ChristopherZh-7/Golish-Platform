import { CheckCircle2, ChevronDown, GitBranch, Loader2, XCircle } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/utils";

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
