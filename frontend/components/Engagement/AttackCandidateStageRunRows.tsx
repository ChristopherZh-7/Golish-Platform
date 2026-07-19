import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  ChevronRight,
  Clock3,
  Loader2,
  OctagonX,
} from "lucide-react";
import { getStageRunAgentLabel } from "@/lib/tools";
import { cn } from "@/lib/utils";
import type { StageRunRow, StageRunStatus } from "./StageRunOrgRows";

interface AttackCandidateStageRunRowsProps {
  rows: StageRunRow[];
  roleLabel?: string;
  onDrillIn?: (agentRequestId: string) => void;
}

const STATUS_VIEW: Record<
  StageRunStatus,
  { label: string; className: string; icon: typeof Loader2 }
> = {
  running: { label: "运行中", className: "text-sky-300", icon: Loader2 },
  queued: { label: "排队中", className: "text-indigo-300", icon: Clock3 },
  pending: { label: "等待中", className: "text-indigo-300", icon: Clock3 },
  passed: { label: "已通过", className: "text-emerald-300", icon: CheckCircle2 },
  blocked: { label: "已阻塞", className: "text-amber-300", icon: AlertTriangle },
  stopped: { label: "已停止", className: "text-rose-300", icon: OctagonX },
};

export function isAttackCandidateStageRunRows(rows: readonly StageRunRow[]): boolean {
  return rows.length > 0 && rows.every((row) => row.stage === "attack_candidate");
}

export function AttackCandidateStageRunRows({
  rows,
  roleLabel,
  onDrillIn,
}: AttackCandidateStageRunRowsProps) {
  if (!isAttackCandidateStageRunRows(rows)) return null;

  const agentLabel = getStageRunAgentLabel(roleLabel) ?? "Attack Analyst Agent";

  return (
    <div className="space-y-2" data-testid="attack-candidate-stage-run">
      {rows.map((row) => {
        const status = STATUS_VIEW[row.status];
        const StatusIcon = status.icon;
        const agentRequestId = row.agentRequestId?.trim() || null;
        const canDrillIn = Boolean(onDrillIn && agentRequestId);
        const content = (
          <>
            <div className="flex min-w-0 items-center gap-2">
              <Bot className="h-4 w-4 shrink-0 text-cyan-300" />
              <span className="truncate text-xs font-medium text-foreground/90">{agentLabel}</span>
              <span className="truncate text-[11px] text-muted-foreground">{row.name}</span>
              <span
                className={cn(
                  "ml-auto inline-flex shrink-0 items-center gap-1 rounded bg-muted/35 px-1.5 py-0.5 text-[10px]",
                  status.className
                )}
              >
                <StatusIcon className={cn("h-3 w-3", row.status === "running" && "animate-spin")} />
                {status.label}
              </span>
              {canDrillIn && <ChevronRight className="h-3.5 w-3.5 shrink-0 text-cyan-300/70" />}
            </div>

            {row.activity && (
              <div
                className={cn(
                  "mt-2 break-words rounded border px-2 py-1.5 text-[11px] leading-relaxed",
                  row.status === "blocked"
                    ? "border-amber-500/20 bg-amber-500/[0.06] text-amber-100/80"
                    : "border-border/20 bg-background/30 text-muted-foreground"
                )}
              >
                {row.activity}
              </div>
            )}

            <div className="mt-1.5 flex items-center gap-2 text-[10px] text-muted-foreground/65">
              {row.evidenceCount > 0 && <span>{row.evidenceCount} 条 evidence</span>}
              {canDrillIn && <span className="ml-auto text-cyan-300/75">查看运行流</span>}
            </div>
          </>
        );

        if (canDrillIn && agentRequestId) {
          return (
            <button
              key={row.id}
              type="button"
              aria-label={`查看 ${row.name} 的 ${agentLabel} 运行流`}
              className="w-full rounded-md border border-cyan-500/25 bg-cyan-500/[0.04] px-3 py-2.5 text-left transition-colors hover:border-cyan-400/45 hover:bg-cyan-500/[0.07]"
              onClick={() => onDrillIn?.(agentRequestId)}
            >
              {content}
            </button>
          );
        }

        return (
          <div
            key={row.id}
            className="rounded-md border border-border/25 bg-muted/[0.08] px-3 py-2.5"
          >
            {content}
          </div>
        );
      })}
    </div>
  );
}
