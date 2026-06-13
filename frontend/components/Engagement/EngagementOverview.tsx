/**
 * Engagement overview (Phase C, 设计 2026-06-13-engagement-scoping-fanout §5④).
 *
 * The scoping conversation "upgrades into" this card once the org tree is
 * locked: org forest grouped by family, pool-merged per-org status, active/
 * queued counters, concurrency control and the fan-out start/stop switch.
 * Clicking a row drills into that org's worker conversation.
 *
 * This is the SMART container: it owns the data (DB-truth snapshot via
 * engagement_get_snapshot + the live worker-pool slice overlaid, 设计 §10) and
 * the fan-out lifecycle, and renders the presentational {@link EngagementInlineCard}
 * (the worker-card view that reuses the SubAgentInlineCard visual language).
 */

import { AlertTriangle, Loader2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { type EngagementSnapshot, getEngagementSnapshot } from "@/lib/api/engagement";
import { isPoolRunning, startEngagementRun, stopEngagementRun } from "@/lib/engagement/runPool";
import { logger } from "@/lib/logger";
import { useStore } from "@/store";
import { type EngagementCardRow, EngagementInlineCard } from "./EngagementInlineCard";
import {
  activePhaseFor,
  buildOverviewRows,
  deriveWorkerActivity,
  summarizeRows,
} from "./engagementOverview.utils";

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
  const [collapsed, setCollapsed] = useState(false);

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

  // Map pool-merged rows → presentational worker-card rows. Status/tree/phase/
  // drill-in are live; the per-worker activity line + tool count are deferred
  // (they need the worker session's tool stream — separate plumbing).
  const cardRows: EngagementCardRow[] = useMemo(
    () =>
      rows.map((r) => {
        const workerConv = r.workerConvId ? conversations[r.workerConvId] : undefined;
        const { activity, toolCount } = deriveWorkerActivity(workerConv);
        return {
          id: r.node.organizationId,
          name: r.node.name,
          depth: r.depth,
          ownershipPercent: r.node.ownershipPercent,
          status: r.effective,
          hasChildren: r.node.children.length > 0,
          expanded: expanded.has(r.node.organizationId),
          drillable: r.workerConvId != null,
          phase: activePhaseFor(r.node, pool),
          activity,
          toolCount,
        };
      }),
    [rows, expanded, pool, conversations]
  );

  const convByRowId = useMemo(() => {
    const m: Record<string, string | null> = {};
    for (const r of rows) m[r.node.organizationId] = r.workerConvId;
    return m;
  }, [rows]);

  const onDrillInRow = useCallback(
    (id: string) => drillIn(convByRowId[id] ?? null),
    [convByRowId, drillIn]
  );

  const onConcurrencyChange = useCallback((k: number) => {
    setConcurrency(k);
    useStore.getState().poolSetConcurrency(k);
  }, []);

  if (error) {
    return (
      <div className="mx-3 mb-2 flex items-center gap-2 rounded-lg border border-border bg-background/60 px-3 py-2 text-xs text-red-400">
        <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate">{error}</span>
        <button type="button" onClick={() => void load()} className="underline">
          retry
        </button>
      </div>
    );
  }

  if (loading && !snapshot) {
    return (
      <div className="mx-3 mb-2 flex items-center gap-2 rounded-lg border border-border bg-background/60 px-3 py-3 text-xs text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading engagement…
      </div>
    );
  }

  if (!snapshot || snapshot.rootCount === 0) {
    return (
      <div className="mx-3 mb-2 rounded-lg border border-border bg-background/60 px-3 py-3 text-xs text-muted-foreground">
        No organizations in scope yet — run scoping in this chat first (paste the company list,
        normalize names, build the org tree), then fan out.
      </div>
    );
  }

  return (
    <EngagementInlineCard
      rows={cardRows}
      summary={summary}
      running={running}
      stopping={pool.phase === "stopping"}
      concurrency={concurrency}
      collapsed={collapsed}
      canFanOut={canStart}
      fanOutDisabledReason="Lock a scope (run scoping) and pick a model first"
      onFanOut={handleStart}
      onStop={stopEngagementRun}
      onConcurrencyChange={onConcurrencyChange}
      onToggleRow={toggle}
      onDrillIn={onDrillInRow}
      onToggleCard={() => setCollapsed((c) => !c)}
    />
  );
}
