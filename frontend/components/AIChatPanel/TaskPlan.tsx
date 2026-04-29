import {
  CheckCircle2,
  List,
  Loader2,
  XCircle,
} from "lucide-react";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";

export interface TaskPlanViewModel {
  version: number;
  steps: Array<{ step: string; status: "pending" | "in_progress" | "completed" | "cancelled" | "failed" }>;
  summary: { total: number; completed: number; in_progress: number; pending: number };
  retiredAt?: string;
}

export function PlanStepIcon({ status, size = "sm" }: { status: string; size?: "sm" | "md" }) {
  const s = size === "sm" ? "w-3 h-3" : "w-3.5 h-3.5";
  switch (status) {
    case "completed": return <CheckCircle2 className={cn(s, "text-green-500 flex-shrink-0")} />;
    case "in_progress": return <Loader2 className={cn(s, "text-accent animate-spin flex-shrink-0")} />;
    case "failed":
    case "cancelled": return <XCircle className={cn(s, "text-red-400/70 flex-shrink-0")} />;
    default: return <div className={cn(s, "rounded-full border-[1.5px] border-muted-foreground/25 flex-shrink-0")} />;
  }
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

export function PlanUpdatedNotice({ version }: { version?: number }) {
  const { t } = useTranslation();
  return (
    <div className="mx-4 my-2 flex items-center gap-2 px-2.5 py-1.5 rounded border border-[var(--border-subtle)] bg-background/40 text-[11px] text-muted-foreground">
      <List className="w-3 h-3 text-accent flex-shrink-0" />
      <span>{t("ai.planBar.planUpdated")}</span>
      {version != null && (
        <span className="text-[9.5px] font-mono px-1 py-px rounded bg-[var(--accent-dim)] text-accent/80 tabular-nums">
          v{version}
        </span>
      )}
      <span className="text-[10px] text-muted-foreground/55 ml-auto">{t("ai.planBar.seeAtTop")}</span>
    </div>
  );
}
