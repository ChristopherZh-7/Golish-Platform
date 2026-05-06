import {
  Activity,
  Bug,
  Cable,
  Crosshair,
  Eye,
  FileText,
  Globe,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { type TimelineEntry, targetTimeline } from "@/lib/api/security-analysis";
import { getProjectPath } from "@/lib/projects";
import { securityApi } from "@/lib/security";
import { cn } from "@/lib/utils";

type SourceFilter =
  | "all"
  | "audit_log"
  | "target_assets"
  | "api_endpoints"
  | "passive_scan_logs"
  | "findings";

interface TargetOption {
  id: string;
  value: string;
  type: string;
}

interface SourceMeta {
  label: string;
  icon: typeof Activity;
  color: string;
  bg: string;
}

const SOURCE_META: Record<Exclude<SourceFilter, "all">, SourceMeta> = {
  audit_log: { label: "Oplog", icon: FileText, color: "text-blue-400", bg: "bg-blue-500/10" },
  target_assets: {
    label: "Asset",
    icon: Globe,
    color: "text-emerald-400",
    bg: "bg-emerald-500/10",
  },
  api_endpoints: {
    label: "Endpoint",
    icon: Cable,
    color: "text-purple-400",
    bg: "bg-purple-500/10",
  },
  passive_scan_logs: { label: "Scan", icon: Eye, color: "text-orange-400", bg: "bg-orange-500/10" },
  findings: { label: "Finding", icon: Bug, color: "text-red-400", bg: "bg-red-500/10" },
};

const SOURCE_FILTERS: { id: SourceFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "audit_log", label: "Oplog" },
  { id: "target_assets", label: "Assets" },
  { id: "api_endpoints", label: "Endpoints" },
  { id: "passive_scan_logs", label: "Scans" },
  { id: "findings", label: "Findings" },
];

function formatTimestamp(iso: string): string {
  try {
    const d = new Date(iso);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}`;
  } catch {
    return iso;
  }
}

function getSourceMeta(source: string): SourceMeta {
  return (
    SOURCE_META[source as Exclude<SourceFilter, "all">] ?? {
      label: source,
      icon: Activity,
      color: "text-muted-foreground",
      bg: "bg-muted/20",
    }
  );
}

/**
 * Per-target activity timeline. Loads aggregated rows from the
 * `target_timeline` Tauri command (UNION of audit_log / target_assets /
 * api_endpoints / passive_scan_logs / findings) and renders them
 * newest-first with source-icon badges and a source filter chip row.
 *
 * Three explicit states are rendered: loading (spinner), error (banner +
 * retry), empty (icon + hint). Pass `initialTargetId` from the parent so
 * the panel auto-selects when the user opened it from a target context.
 */
export function TargetTimeline({ initialTargetId }: { initialTargetId?: string } = {}) {
  const [targets, setTargets] = useState<TargetOption[]>([]);
  const [selectedId, setSelectedId] = useState<string>(initialTargetId ?? "");
  const [entries, setEntries] = useState<TimelineEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<SourceFilter>("all");

  // Load available targets once.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = (await securityApi.targetList(getProjectPath())) as unknown as {
          targets: TargetOption[];
        };
        if (cancelled) return;
        const list = data?.targets ?? [];
        setTargets(list);
        if (!selectedId && list.length > 0) setSelectedId(list[0].id);
      } catch {
        if (!cancelled) setTargets([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [selectedId]);

  const load = useCallback(async () => {
    if (!selectedId) {
      setEntries([]);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const data = await targetTimeline(selectedId, 200);
      setEntries(Array.isArray(data) ? data : []);
    } catch (e) {
      setError(String(e));
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }, [selectedId]);

  useEffect(() => {
    load();
  }, [load]);

  const filtered = useMemo(() => {
    if (filter === "all") return entries;
    return entries.filter((e) => e.source === filter);
  }, [entries, filter]);

  const sourceCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const e of entries) counts[e.source] = (counts[e.source] ?? 0) + 1;
    return counts;
  }, [entries]);

  const selectedTarget = targets.find((t) => t.id === selectedId);

  return (
    <div className="h-full flex flex-col">
      {/* Header: target selector + reload */}
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <Crosshair className="w-3.5 h-3.5 text-accent flex-shrink-0" />
        <span className="text-[10px] text-muted-foreground/50 flex-shrink-0">Target:</span>
        <select
          value={selectedId}
          onChange={(e) => setSelectedId(e.target.value)}
          className="flex-1 bg-[var(--bg-hover)]/30 border border-border/20 rounded-md px-2 py-1 text-[11px] font-mono text-foreground outline-none focus:border-accent/40"
        >
          <option value="">— select target —</option>
          {targets.map((t) => (
            <option key={t.id} value={t.id}>
              [{t.type}] {t.value}
            </option>
          ))}
        </select>
        <button
          type="button"
          onClick={load}
          disabled={!selectedId || loading}
          title="Reload timeline"
          className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] text-accent bg-accent/10 hover:bg-accent/20 transition-colors disabled:opacity-30"
        >
          {loading ? (
            <Loader2 className="w-3 h-3 animate-spin" />
          ) : (
            <RefreshCw className="w-3 h-3" />
          )}
          Reload
        </button>
        <span className="text-[9px] text-muted-foreground/30">
          {filtered.length}/{entries.length}
        </span>
      </div>

      {/* Source filter chips */}
      <div className="flex items-center gap-1.5 px-4 py-2 border-b border-border/10 flex-shrink-0 overflow-x-auto">
        {SOURCE_FILTERS.map((s) => {
          const count = s.id === "all" ? entries.length : (sourceCounts[s.id] ?? 0);
          const meta = s.id === "all" ? null : SOURCE_META[s.id];
          const Icon = meta?.icon ?? Activity;
          const active = filter === s.id;
          return (
            <button
              key={s.id}
              type="button"
              onClick={() => setFilter(s.id)}
              className={cn(
                "flex items-center gap-1 px-2 py-0.5 rounded-md text-[10px] font-medium border transition-colors",
                active
                  ? meta
                    ? `${meta.bg} ${meta.color} border-border/20`
                    : "bg-accent/15 text-accent border-accent/30"
                  : "border-transparent text-muted-foreground/60 hover:text-foreground hover:bg-muted/10"
              )}
            >
              <Icon className="w-2.5 h-2.5" />
              <span>{s.label}</span>
              <span className="text-[9px] opacity-60 tabular-nums">{count}</span>
            </button>
          );
        })}
      </div>

      {/* Body: 3-state UI (loading / error / empty / list) */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="h-full flex items-center justify-center">
            <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/40" />
          </div>
        ) : error ? (
          <div className="h-full flex flex-col items-center justify-center gap-2 text-destructive/60 px-6 text-center">
            <Bug className="w-8 h-8" />
            <p className="text-[11px] break-all">{error}</p>
            <button
              type="button"
              onClick={load}
              className="text-[10px] text-accent hover:underline mt-1"
            >
              Retry
            </button>
          </div>
        ) : !selectedId ? (
          <div className="h-full flex flex-col items-center justify-center gap-2 text-muted-foreground/30">
            <Crosshair className="w-10 h-10" />
            <p className="text-[12px] font-medium">Select a target above</p>
          </div>
        ) : filtered.length === 0 ? (
          <div className="h-full flex flex-col items-center justify-center gap-2 text-muted-foreground/30">
            <Activity className="w-10 h-10" />
            <p className="text-[12px] font-medium">
              {entries.length === 0 ? "No activity yet" : `No ${filter} entries`}
            </p>
            {selectedTarget && (
              <p className="text-[10px] text-muted-foreground/40 font-mono">
                {selectedTarget.value}
              </p>
            )}
          </div>
        ) : (
          <ul className="divide-y divide-border/5">
            {filtered.map((entry, idx) => {
              const meta = getSourceMeta(entry.source);
              const Icon = meta.icon;
              return (
                <li
                  key={`${entry.source}-${entry.createdAt}-${idx}`}
                  className="flex items-start gap-3 px-4 py-2.5 hover:bg-muted/5 transition-colors"
                >
                  <span
                    className={cn(
                      "flex-shrink-0 w-7 h-7 rounded-md flex items-center justify-center mt-0.5",
                      meta.bg
                    )}
                  >
                    <Icon className={cn("w-3.5 h-3.5", meta.color)} />
                  </span>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-0.5">
                      <span className={cn("text-[10px] font-medium", meta.color)}>
                        {entry.event}
                      </span>
                      {entry.toolName && (
                        <span className="text-[9px] text-muted-foreground/40 bg-muted/15 px-1.5 py-0.5 rounded font-mono">
                          {entry.toolName}
                        </span>
                      )}
                      <span className="text-[9px] text-muted-foreground/30 font-mono ml-auto flex-shrink-0">
                        {formatTimestamp(entry.createdAt)}
                      </span>
                    </div>
                    <div className="text-[11px] text-foreground/70 break-all">{entry.details}</div>
                    <div className="flex items-center gap-2 mt-0.5">
                      <span className="text-[9px] text-muted-foreground/40 uppercase">
                        {meta.label}
                      </span>
                      <span className="text-[9px] text-muted-foreground/30">·</span>
                      <span className="text-[9px] text-muted-foreground/40">{entry.category}</span>
                      <span className="text-[9px] text-muted-foreground/30">·</span>
                      <span
                        className={cn(
                          "text-[9px]",
                          entry.status === "vulnerable" || entry.status === "failed"
                            ? "text-red-400/70"
                            : entry.status === "completed" || entry.status === "tested"
                              ? "text-emerald-400/70"
                              : "text-muted-foreground/40"
                        )}
                      >
                        {entry.status}
                      </span>
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
