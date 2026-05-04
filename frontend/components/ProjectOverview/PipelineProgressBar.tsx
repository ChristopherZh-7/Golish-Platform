import { cn } from "@/lib/utils";
import type { PipelineProgress } from "./types";
import { friendly } from "./utils";

/** Compact stacked-bar visualisation of recon-pipeline progress. */
export function PipelineProgressBar({ progress }: { progress: PipelineProgress }) {
  return (
    <div className="flex-shrink-0 px-4 py-2.5 border-b border-border/10 bg-muted/5">
      <div className="flex items-center justify-between mb-2">
        <span className="text-[10px] font-medium text-muted-foreground/50 uppercase tracking-wider">
          Pipeline
        </span>
        <span className="text-[10px] text-muted-foreground/30">
          {progress.status === "running"
            ? `Step ${progress.currentStepIndex + 1} of ${progress.totalSteps}`
            : progress.status === "completed"
              ? `${progress.totalSteps} steps complete`
              : "Failed"}
        </span>
      </div>

      {/* Step bars */}
      <div className="flex items-center gap-1">
        {progress.stepNames.map((name, i) => {
          const isDone = i < progress.completedSteps;
          const isCurrent = i === progress.currentStepIndex && progress.status === "running";

          return (
            <div key={name} className="flex flex-col items-center gap-1 flex-1 min-w-0">
              <div
                className={cn(
                  "w-full h-1.5 rounded-full transition-all duration-500",
                  isDone && "bg-green-400/50",
                  isCurrent && "bg-blue-400/60 animate-pulse",
                  !isDone && !isCurrent && "bg-muted-foreground/10"
                )}
              />
              <span
                className={cn(
                  "text-[9px] font-medium truncate max-w-full",
                  isDone && "text-green-400/60",
                  isCurrent && "text-blue-300",
                  !isDone && !isCurrent && "text-muted-foreground/30"
                )}
              >
                {friendly(name)}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
