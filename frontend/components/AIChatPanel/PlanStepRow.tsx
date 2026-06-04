/**
 * PlanStepRow — shared plan-step row + step status icon.
 *
 * Extracted from `InlinePlanCard` (design 2026-06-04 · roadmap UX overhaul) so the
 * chat-stream single card (`InlinePlanCard`) and the per-stage roadmap cards
 * (`StageRow`) render todo steps with identical visuals. Pure refactor — no
 * behaviour change.
 */
import { CheckCircle2, XCircle } from "lucide-react";
import { cn } from "@/lib/utils";

export function StepIcon({ status, index }: { status: string; index: number }) {
  switch (status) {
    case "completed":
      return <CheckCircle2 className="w-3.5 h-3.5 text-green-500 flex-shrink-0" />;
    case "in_progress":
      return (
        <span className="inline-flex items-center justify-center w-4 h-4 rounded-full bg-accent/90 text-[10px] font-bold text-accent-foreground flex-shrink-0 tabular-nums">
          {index + 1}
        </span>
      );
    case "failed":
    case "cancelled":
      return <XCircle className="w-3.5 h-3.5 text-red-400/70 flex-shrink-0" />;
    default:
      return (
        <span className="inline-flex items-center justify-center w-4 h-4 rounded-full border-[1.5px] border-muted-foreground/25 text-[10px] text-muted-foreground/50 flex-shrink-0 tabular-nums">
          {index + 1}
        </span>
      );
  }
}

/** Compact badge that surfaces the refiner failure category (P0-2). */
function FailureKindBadge({
  kind,
}: {
  kind: "technical" | "environmental" | "conceptual" | "external";
}) {
  const tone: Record<typeof kind, string> = {
    technical: "bg-amber-500/15 text-amber-500/90 border-amber-500/30",
    environmental: "bg-purple-500/15 text-purple-500/90 border-purple-500/30",
    conceptual: "bg-rose-500/15 text-rose-500/90 border-rose-500/30",
    external: "bg-sky-500/15 text-sky-500/90 border-sky-500/30",
  };
  const label: Record<typeof kind, string> = {
    technical: "tech",
    environmental: "env",
    conceptual: "concept",
    external: "external",
  };
  return (
    <span
      className={cn(
        "inline-flex items-center px-1.5 py-px rounded text-[9.5px] font-medium border tabular-nums",
        tone[kind]
      )}
      title={`Failure category: ${kind}`}
    >
      {label[kind]}
    </span>
  );
}

export function StepRow({
  step,
  index,
}: {
  step: {
    step: string;
    status: string;
    failure_kind?: "technical" | "environmental" | "conceptual" | "external" | null;
  };
  index: number;
}) {
  const isCompleted = step.status === "completed";
  const isInProgress = step.status === "in_progress";
  const isFailed = step.status === "failed" || step.status === "cancelled";

  return (
    <div
      className={cn(
        "flex items-center gap-2 py-1 px-2 rounded text-[11.5px]",
        isInProgress && "bg-accent/[0.06]"
      )}
    >
      <StepIcon status={step.status} index={index} />
      <span
        className={cn(
          "flex-1 truncate leading-relaxed",
          isCompleted && "text-muted-foreground/70",
          isInProgress && "font-semibold text-foreground",
          isFailed && "line-through text-red-400/70",
          !isCompleted && !isInProgress && !isFailed && "text-muted-foreground/50"
        )}
      >
        {step.step}
      </span>
      {isFailed && step.failure_kind && <FailureKindBadge kind={step.failure_kind} />}
    </div>
  );
}
