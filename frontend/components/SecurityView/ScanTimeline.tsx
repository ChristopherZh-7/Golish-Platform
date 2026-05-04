import { ChevronDown, ChevronRight, History, Loader2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { onCustomEvent } from "@/lib/events";
import { getProjectPath } from "@/lib/projects";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";
import { type AuditRow, oplogListByTarget, oplogSearch } from "@/lib/security-analysis";
import { formatDurationShort } from "@/lib/time";
import { cn } from "@/lib/utils";

interface StepSummary {
  tool: string;
  stored: number;
  new?: number;
  parsed: number;
  exit: number | null;
  ms: number;
}

export function ScanTimeline({ targetId, targetValue }: { targetId: string; targetValue: string }) {
  const [logs, setLogs] = useState<AuditRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedRun, setExpandedRun] = useState<number | null>(null);

  const loadLogs = useCallback(async () => {
    try {
      const [byTarget, bySearch] = await Promise.all([
        oplogListByTarget(targetId, 100).catch(() => [] as AuditRow[]),
        oplogSearch(getProjectPath() ?? "", targetValue, 200).catch(() => [] as AuditRow[]),
      ]);
      const seen = new Set<number>();
      const merged: AuditRow[] = [];
      for (const entry of [...byTarget, ...bySearch]) {
        if (entry.action !== "pipeline_executed" || seen.has(entry.id)) continue;
        const detail = entry.detail as Record<string, unknown> | null;
        const detailTarget = detail?.target as string | undefined;
        if (entry.targetId === targetId || detailTarget === targetValue) {
          seen.add(entry.id);
          merged.push(entry);
        }
      }
      merged.sort((a, b) => b.createdAt - a.createdAt);
      setLogs(merged);
    } catch {
      setLogs([]);
    }
    setLoading(false);
  }, [targetId, targetValue]);

  useEffect(() => {
    setLoading(true);
    loadLogs();
  }, [loadLogs]);

  useEffect(() => {
    const unlistenTargets = onCustomEvent("targets-changed", () => loadLogs());
    return () => {
      runTauriUnlistenFromPromise(unlistenTargets);
    };
  }, [loadLogs]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 py-4 text-[10px] text-muted-foreground/30">
        <Loader2 className="w-3 h-3 animate-spin" />
        Loading scan history...
      </div>
    );
  }

  if (logs.length === 0) {
    return (
      <div className="text-center py-6 text-[10px] text-muted-foreground/20">
        No scan history for this target yet
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-border/10 overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10">
        <History className="w-3.5 h-3.5 text-blue-400" />
        <span className="text-[11px] font-semibold text-foreground/80">Scan History</span>
        <span className="text-[10px] text-muted-foreground/30 ml-auto">
          {logs.length} run{logs.length !== 1 ? "s" : ""}
        </span>
      </div>

      <div className="divide-y divide-border/5">
        {logs.map((run, i) => {
          const detail = run.detail ?? {};
          const steps = (detail.steps ?? []) as StepSummary[];
          const totalStored = (detail.total_stored ?? 0) as number;
          const totalNew = (detail.total_new ?? null) as number | null;
          const durationMs = (detail.duration_ms ?? 0) as number;
          const completedSteps = (detail.completed_steps ?? 0) as number;
          const totalSteps = (detail.total_steps ?? 0) as number;
          const status = run.status;
          const isExpanded = expandedRun === run.id;

          return (
            <div key={run.id}>
              <button
                type="button"
                onClick={() => setExpandedRun(isExpanded ? null : run.id)}
                className="flex items-center gap-2.5 w-full px-3 py-2 text-left hover:bg-white/[0.02] transition-colors"
              >
                {/* Timeline dot */}
                <div className="flex flex-col items-center gap-0.5 w-3 flex-shrink-0">
                  <span
                    className={cn(
                      "w-2 h-2 rounded-full flex-shrink-0",
                      status === "completed"
                        ? "bg-emerald-400"
                        : status === "partial"
                          ? "bg-yellow-400"
                          : "bg-red-400"
                    )}
                  />
                  {i < logs.length - 1 && <div className="w-px flex-1 min-h-[8px] bg-border/10" />}
                </div>

                {/* Run info */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-[10px] font-mono text-muted-foreground/40">
                      {new Date(run.createdAt).toLocaleDateString(undefined, {
                        month: "short",
                        day: "numeric",
                      })}{" "}
                      {new Date(run.createdAt).toLocaleTimeString(undefined, {
                        hour: "2-digit",
                        minute: "2-digit",
                      })}
                    </span>
                    <span className="text-[10px] text-foreground/60 font-medium">
                      {run.toolName ?? "Pipeline"}
                    </span>
                    {totalNew !== null ? (
                      totalNew > 0 ? (
                        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-emerald-500/15 text-emerald-300 font-medium">
                          +{totalNew} new
                        </span>
                      ) : totalStored > 0 ? (
                        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-white/[0.05] text-muted-foreground/40 font-medium">
                          no changes
                        </span>
                      ) : null
                    ) : totalStored > 0 ? (
                      <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-300 font-medium">
                        +{totalStored} items
                      </span>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-2 mt-0.5 text-[9px] text-muted-foreground/30">
                    <span>
                      {completedSteps}/{totalSteps} steps
                    </span>
                    {durationMs > 0 && <span>{formatDurationShort(durationMs)}</span>}
                    {steps
                      .filter((s) => s.stored > 0)
                      .map((s) => (
                        <span
                          key={s.tool}
                          className="text-[8px] px-1 py-0.5 rounded bg-white/[0.03]"
                        >
                          {s.tool}: {s.new != null ? s.new : s.stored}
                        </span>
                      ))}
                  </div>
                </div>

                {steps.length > 0 &&
                  (isExpanded ? (
                    <ChevronDown className="w-3 h-3 text-muted-foreground/20 flex-shrink-0" />
                  ) : (
                    <ChevronRight className="w-3 h-3 text-muted-foreground/20 flex-shrink-0" />
                  ))}
              </button>

              {/* Expanded step details */}
              {isExpanded && steps.length > 0 && (
                <div className="border-t border-border/5 px-3 py-2 ml-5 space-y-1">
                  {steps.map((step, si) => (
                    <div key={si} className="flex items-center gap-2 text-[10px]">
                      <span
                        className={cn(
                          "w-1.5 h-1.5 rounded-full flex-shrink-0",
                          step.exit === 0
                            ? "bg-emerald-400/60"
                            : step.exit === null
                              ? "bg-zinc-500/40"
                              : "bg-red-400/60"
                        )}
                      />
                      <span className="font-mono text-foreground/50 w-16 flex-shrink-0">
                        {step.tool}
                      </span>
                      {step.new != null && step.new > 0 ? (
                        <span className="text-emerald-300/60">+{step.new} new</span>
                      ) : step.stored > 0 ? (
                        <span className="text-blue-300/60">+{step.stored} stored</span>
                      ) : null}
                      {step.new != null && step.stored > step.new && step.new > 0 && (
                        <span className="text-muted-foreground/25">
                          ({step.stored - step.new} existing)
                        </span>
                      )}
                      {step.parsed > 0 && step.parsed !== step.stored && (
                        <span className="text-muted-foreground/20">({step.parsed} parsed)</span>
                      )}
                      {step.ms > 0 && (
                        <span className="text-muted-foreground/20 ml-auto">
                          {formatDurationShort(step.ms)}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
