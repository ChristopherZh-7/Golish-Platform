/**
 * Engagement overview (Phase C, 设计 2026-06-13-engagement-scoping-fanout §5④).
 *
 * The scoping conversation "upgrades into" this card once the org tree is
 * locked: org forest grouped by family, pool-merged per-org status, active/
 * queued counters, concurrency control and the fan-out start/stop switch.
 * Clicking a row drills into that org's worker conversation.
 *
 * Data sources: DB-truth snapshot (engagement_get_snapshot) as the baseline +
 * the live worker-pool slice overlaid (设计 §10 运行时态 vs DB 真值).
 */

import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Loader2,
  Play,
  RefreshCw,
  Square,
  Target,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { type EngagementSnapshot, getEngagementSnapshot } from "@/lib/api/engagement";
import { isPoolRunning, startEngagementRun, stopEngagementRun } from "@/lib/engagement/runPool";
import { logger } from "@/lib/logger";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { buildOverviewRows, type EffectiveStatus, summarizeRows } from "./engagementOverview.utils";

const STATUS_META: Record<EffectiveStatus, { label: string; className: string }> = {
  passed: { label: "Covered", className: "bg-emerald-500/15 text-emerald-400" },
  skippedAlreadyComplete: { label: "Covered", className: "bg-emerald-500/15 text-emerald-400" },
  running: { label: "Running", className: "bg-sky-500/15 text-sky-400" },
  queued: { label: "Queued", className: "bg-indigo-500/15 text-indigo-400" },
  blocked: { label: "Blocked", className: "bg-amber-500/15 text-amber-400" },
  failed: { label: "Failed", className: "bg-red-500/15 text-red-400" },
  pending: { label: "Pending", className: "bg-slate-500/15 text-slate-400" },
};

function StatusBadge({ status }: { status: EffectiveStatus }) {
  const meta = STATUS_META[status] ?? STATUS_META.pending;
  return (
    <span className={cn("rounded px-2 py-0.5 text-xs font-medium", meta.className)}>
      {status === "running" && <Loader2 className="mr-1 inline h-3 w-3 animate-spin" />}
      {meta.label}
    </span>
  );
}

export interface EngagementOverviewProps {
  /** Model + provider the spawned workers run with (the panel's selection). */
  model: string;
  provider: string;
  /** Harness profile for workers (the engagement's task profile). */
  profileId?: string;
  /** Conversation hosting this overview (the scoping chat). */
  conversationId: string;
}

export function EngagementOverview({
  model,
  provider,
  profileId = "red_team",
  conversationId,
}: EngagementOverviewProps) {
  const [snapshot, setSnapshot] = useState<EngagementSnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [concurrency, setConcurrency] = useState(3);

  const pool = useStore((s) => s.engagementPool);
  const conversations = useStore((s) => s.conversations);
  const projectPath = useStore((s) => s.currentProjectPath);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const snap = await getEngagementSnapshot({ projectPath: projectPath ?? undefined });
      setSnapshot(snap);
      setExpanded((prev) => {
        const next = new Set(prev);
        for (const root of snap.tree) {
          if (root.children.length > 0) next.add(root.organizationId);
        }
        return next;
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [projectPath]);

  useEffect(() => {
    void load();
  }, [load]);

  // While the pool runs, refresh the DB baseline periodically so freshly
  // landed truth (passed stages) shows up without manual refreshes.
  useEffect(() => {
    if (pool.phase !== "running" && pool.phase !== "stopping") return;
    const interval = setInterval(() => void load(), 10_000);
    return () => clearInterval(interval);
  }, [pool.phase, load]);

  const findConvByUnit = useCallback(
    (unitId: string): string | null => {
      for (const conv of Object.values(conversations)) {
        if (conv.workerMeta?.unitId === unitId) return conv.id;
      }
      return null;
    },
    [conversations]
  );

  const allRows = useMemo(() => {
    if (!snapshot) return [];
    // Summary counts every org (full expansion); display honours `expanded`.
    const everyId = new Set<string>();
    const collect = (nodes: typeof snapshot.tree) => {
      for (const n of nodes) {
        everyId.add(n.organizationId);
        collect(n.children);
      }
    };
    collect(snapshot.tree);
    return buildOverviewRows(snapshot.tree, everyId, pool, findConvByUnit);
  }, [snapshot, pool, findConvByUnit]);

  const rows = useMemo(() => {
    if (!snapshot) return [];
    return buildOverviewRows(snapshot.tree, expanded, pool, findConvByUnit);
  }, [snapshot, expanded, pool, findConvByUnit]);

  const summary = useMemo(() => summarizeRows(allRows), [allRows]);

  const toggle = useCallback((id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const drillIn = useCallback((convId: string | null) => {
    if (convId) useStore.getState().setActiveConversation(convId);
  }, []);

  const handleStart = useCallback(() => {
    if (!projectPath || isPoolRunning()) return;
    startEngagementRun({
      projectPath,
      concurrency,
      overviewConvId: conversationId,
      profileId,
      model,
      provider,
      thresholdPct: 51,
    }).catch((e) => {
      logger.error("[engagement] run failed:", e);
      setError(e instanceof Error ? e.message : String(e));
    });
  }, [projectPath, concurrency, conversationId, profileId, model, provider]);

  const running = pool.phase === "running" || pool.phase === "stopping";
  const canStart = !running && !!projectPath && (snapshot?.rootCount ?? 0) > 0 && !!model;

  return (
    <div className="mx-3 mb-2 rounded-lg border border-border bg-background/60 text-foreground">
      <header className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <Target className="h-4 w-4 shrink-0 text-primary" />
          <h3 className="truncate text-xs font-semibold">Engagement</h3>
          <span className="truncate text-xs text-muted-foreground">
            {summary.covered}/{summary.totalOrgs} covered
            {summary.active > 0 && ` · ${summary.active} active`}
            {summary.queued > 0 && ` · ${summary.queued} queued`}
            {summary.blocked > 0 && ` · ${summary.blocked} blocked`}
            {summary.failed > 0 && ` · ${summary.failed} failed`}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          <label className="flex items-center gap-1 text-xs text-muted-foreground">
            K
            <input
              type="number"
              min={1}
              max={10}
              value={concurrency}
              disabled={running}
              onChange={(e) => {
                const k = Math.max(1, Math.min(10, Number(e.target.value) || 1));
                setConcurrency(k);
                useStore.getState().poolSetConcurrency(k);
              }}
              className="w-12 rounded border border-border bg-transparent px-1 py-0.5 text-xs"
            />
          </label>
          {running ? (
            <button
              type="button"
              onClick={stopEngagementRun}
              className="flex items-center gap-1 rounded border border-border px-2 py-1 text-xs text-amber-400 hover:bg-amber-500/10"
              title="Finish in-flight workers, stop dequeuing"
            >
              <Square className="h-3 w-3" />
              {pool.phase === "stopping" ? "Stopping…" : "Stop"}
            </button>
          ) : (
            <button
              type="button"
              onClick={handleStart}
              disabled={!canStart}
              className={cn(
                "flex items-center gap-1 rounded border px-2 py-1 text-xs",
                canStart
                  ? "border-primary text-primary hover:bg-primary/10"
                  : "cursor-not-allowed border-border text-muted-foreground"
              )}
              title={
                canStart
                  ? "Fan out workers over the locked scope"
                  : "Lock a scope (run scoping) and pick a model first"
              }
            >
              <Play className="h-3 w-3" />
              Fan out
            </button>
          )}
          <button
            type="button"
            onClick={() => void load()}
            className="rounded border border-border p-1 text-muted-foreground hover:text-foreground"
            title="Refresh DB snapshot"
          >
            <RefreshCw className={cn("h-3 w-3", loading && "animate-spin")} />
          </button>
        </div>
      </header>

      {error && (
        <div className="flex items-center gap-2 px-3 py-2 text-xs text-red-400">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{error}</span>
          <button type="button" onClick={() => void load()} className="underline">
            retry
          </button>
        </div>
      )}

      {!error && loading && !snapshot && (
        <div className="flex items-center gap-2 px-3 py-3 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading engagement…
        </div>
      )}

      {!error && snapshot && snapshot.rootCount === 0 && (
        <div className="px-3 py-3 text-xs text-muted-foreground">
          No organizations in scope yet — run scoping in this chat first (paste the company list,
          normalize names, build the org tree), then fan out.
        </div>
      )}

      {!error && snapshot && snapshot.rootCount > 0 && (
        <div className="max-h-72 overflow-y-auto">
          <table className="w-full text-xs">
            <tbody>
              {rows.map(({ node, depth, effective, workerConvId }) => (
                <tr
                  key={node.organizationId}
                  className={cn(
                    "border-t border-border/50",
                    workerConvId && "cursor-pointer hover:bg-muted/40"
                  )}
                  onClick={() => drillIn(workerConvId)}
                >
                  <td className="px-3 py-1.5">
                    <div
                      className="flex min-w-0 items-center gap-1"
                      style={{ paddingLeft: depth * 16 }}
                    >
                      {node.children.length > 0 ? (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            toggle(node.organizationId);
                          }}
                          className="shrink-0 text-muted-foreground hover:text-foreground"
                        >
                          {expanded.has(node.organizationId) ? (
                            <ChevronDown className="h-3 w-3" />
                          ) : (
                            <ChevronRight className="h-3 w-3" />
                          )}
                        </button>
                      ) : (
                        <span className="w-3 shrink-0" />
                      )}
                      <span className="truncate">{node.name}</span>
                      {node.ownershipPercent != null && (
                        <span className="shrink-0 text-muted-foreground">
                          {node.ownershipPercent}%
                        </span>
                      )}
                    </div>
                  </td>
                  <td className="w-24 px-2 py-1.5 text-right">
                    {node.weakness ? (
                      <span className="text-muted-foreground" title="weakness score">
                        w={String(node.weakness.total)}
                      </span>
                    ) : null}
                  </td>
                  <td className="w-24 px-3 py-1.5 text-right">
                    <StatusBadge status={effective} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
