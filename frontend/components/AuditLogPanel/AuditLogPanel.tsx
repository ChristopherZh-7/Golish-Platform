import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  ClipboardList,
  Filter,
  Layers,
  Loader2,
  RefreshCw,
  Search,
  Target,
  Trash2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";
import { oplogList, oplogSearch, type AuditRow } from "@/lib/security-analysis";
import { invoke } from "@tauri-apps/api/core";

const CATEGORY_COLORS: Record<string, string> = {
  targets: "text-blue-400 bg-blue-500/10",
  findings: "text-orange-400 bg-orange-500/10",
  vault: "text-purple-400 bg-purple-500/10",
  topology: "text-cyan-400 bg-cyan-500/10",
  methodology: "text-green-400 bg-green-500/10",
  notes: "text-yellow-400 bg-yellow-500/10",
  pipeline: "text-pink-400 bg-pink-500/10",
  tools: "text-red-400 bg-red-500/10",
  project: "text-indigo-400 bg-indigo-500/10",
  security: "text-emerald-400 bg-emerald-500/10",
  scan: "text-teal-400 bg-teal-500/10",
  recon: "text-sky-400 bg-sky-500/10",
  ai: "text-violet-400 bg-violet-500/10",
};

const STATUS_COLORS: Record<string, string> = {
  completed: "text-green-400",
  running: "text-yellow-400 animate-pulse",
  failed: "text-red-400",
  pending: "text-muted-foreground/40",
};

const CATEGORY_OPTIONS = [
  "all",
  "targets",
  "findings",
  "security",
  "scan",
  "recon",
  "vault",
  "topology",
  "methodology",
  "notes",
  "pipeline",
  "tools",
  "project",
  "ai",
];

type ViewMode = "byTarget";

interface TargetGroup {
  targetId: string | null;
  label: string;
  entries: AuditRow[];
  latestTime: number;
  categories: Set<string>;
}

function extractHost(entry: AuditRow): string | null {
  const d = entry.detail as Record<string, unknown>;
  if (d?.host) return String(d.host);
  if (d?.url) {
    try { return new URL(String(d.url)).host; } catch { /* */ }
  }
  if (entry.action === "target_added" && entry.details) return entry.details;
  return null;
}

function extractTargetLabel(entries: AuditRow[]): string {
  for (const e of entries) {
    const host = extractHost(e);
    if (host) return host;
    const d = e.detail as Record<string, unknown>;
    if (d?.target_value) return String(d.target_value);
    if (d?.target) return String(d.target);
    const onMatch = e.details?.match(/ on ([^:]+):/);
    if (onMatch?.[1]) return onMatch[1];
  }
  return entries[0]?.targetId?.slice(0, 12) ?? "Unknown";
}

export function AuditLogPanel() {
  const [entries, setEntries] = useState<AuditRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [filterCategory, setFilterCategory] = useState("all");
  const [showFilter, setShowFilter] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [showSearch, setShowSearch] = useState(false);
  const [expandedIds, setExpandedIds] = useState<Set<number>>(new Set());
  const [viewMode] = useState<ViewMode>("byTarget");
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string> | "all">("all");
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const pollRef = useRef<ReturnType<typeof setInterval>>();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const pp = getProjectPath() ?? "";
      const list = searchQuery.trim()
        ? await oplogSearch(pp, searchQuery.trim(), 500)
        : await oplogList(pp, 500);
      setEntries(Array.isArray(list) ? list : []);
    } catch {
      setEntries([]);
    }
    setLoading(false);
  }, [searchQuery]);

  useEffect(() => {
    load();
  }, [load, currentProjectPath]);

  // Auto-poll every 10 seconds for new entries
  useEffect(() => {
    pollRef.current = setInterval(() => {
      const pp = getProjectPath() ?? "";
      const fn = searchQuery.trim()
        ? oplogSearch(pp, searchQuery.trim(), 500)
        : oplogList(pp, 500);
      fn.then((list) => {
        if (Array.isArray(list)) setEntries(list);
      }).catch(() => {});
    }, 10_000);
    return () => clearInterval(pollRef.current);
  }, [searchQuery]);

  const handleClear = useCallback(async () => {
    try {
      await invoke("audit_clear", { projectPath: getProjectPath() });
      setEntries([]);
    } catch {
      /* ignore */
    }
  }, []);

  const safeEntries = entries ?? [];
  const filtered = useMemo(() => {
    if (filterCategory === "all") return safeEntries;
    return safeEntries.filter((e) => e.category === filterCategory);
  }, [safeEntries, filterCategory]);

  const targetGroups = useMemo((): TargetGroup[] => {
    const map = new Map<string, AuditRow[]>();
    for (const entry of filtered) {
      // Group by targetId first, then by host from detail, fallback to system
      let key = entry.targetId;
      if (!key) {
        const host = extractHost(entry);
        key = host ? `host:${host}` : null;
      }
      const groupKey = key ?? "__system__";
      const arr = map.get(groupKey);
      if (arr) arr.push(entry);
      else map.set(groupKey, [entry]);
    }
    const groups: TargetGroup[] = [];
    for (const [key, groupEntries] of map) {
      const isSystem = key === "__system__";
      const isHostGroup = key.startsWith("host:");
      groups.push({
        targetId: isSystem || isHostGroup ? null : key,
        label: isSystem
          ? "System / General"
          : isHostGroup
            ? key.slice(5)
            : extractTargetLabel(groupEntries),
        entries: groupEntries,
        latestTime: Math.max(...groupEntries.map((e) => e.createdAt)),
        categories: new Set(groupEntries.map((e) => e.category)),
      });
    }
    groups.sort((a, b) => b.latestTime - a.latestTime);
    return groups;
  }, [filtered]);

  const isGroupCollapsed = (id: string) =>
    collapsedGroups === "all" || collapsedGroups.has(id);

  const toggleGroup = (id: string) => {
    setCollapsedGroups((prev) => {
      if (prev === "all") {
        const allKeys = new Set(targetGroups.map((g) => g.targetId ?? "__system__"));
        allKeys.delete(id);
        return allKeys;
      }
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const formatTime = (ts: number) => {
    const d = new Date(ts);
    return d.toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  };

  const formatTimeShort = (ts: number) => {
    const d = new Date(ts);
    return d.toLocaleString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    });
  };

  const hasDetail = (detail: Record<string, unknown>) =>
    detail && Object.keys(detail).length > 0;

  const toggleExpanded = useCallback((id: number) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const renderEntry = (entry: AuditRow, compact = false) => {
    const isExpanded = expandedIds.has(entry.id);
    const showExpandable = hasDetail(entry.detail);

    return (
      <div key={entry.id}>
        <div
          className={cn(
            "flex items-start gap-2 py-1.5 px-2 rounded transition-colors group",
            showExpandable
              ? "cursor-pointer hover:bg-muted/10"
              : "hover:bg-muted/5"
          )}
          onClick={
            showExpandable
              ? () => toggleExpanded(entry.id)
              : undefined
          }
        >
          {/* Expand indicator */}
          <span className="w-3 flex-shrink-0 pt-0.5">
            {showExpandable &&
              (isExpanded ? (
                <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30" />
              ) : (
                <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/20" />
              ))}
          </span>

          {/* Timestamp */}
          <span className="text-[9px] text-muted-foreground/30 whitespace-nowrap pt-0.5 w-28 flex-shrink-0">
            {compact ? formatTimeShort(entry.createdAt) : formatTime(entry.createdAt)}
          </span>

          {/* Status dot */}
          <span
            className={cn(
              "text-[8px] pt-1 flex-shrink-0",
              STATUS_COLORS[entry.status] ?? "text-muted-foreground/40"
            )}
          >
            ●
          </span>

          {/* Category */}
          <span
            className={cn(
              "text-[9px] font-medium px-1.5 py-0.5 rounded flex-shrink-0",
              CATEGORY_COLORS[entry.category] ??
                "text-muted-foreground/50 bg-muted/5"
            )}
          >
            {entry.category}
          </span>

          {/* Action + details */}
          <div className="flex-1 min-w-0">
            <span className="text-[10px] text-foreground/70">
              {entry.action}
            </span>
            {entry.details && (
              <span className="text-[9px] text-muted-foreground/30 ml-1.5 truncate">
                {entry.details}
              </span>
            )}
          </div>

          {/* Tool name badge */}
          {entry.toolName && (
            <span className="text-[8px] text-accent/50 bg-accent/5 px-1.5 py-0.5 rounded flex-shrink-0 font-mono">
              {entry.toolName}
            </span>
          )}

          {/* Entity ref */}
          {entry.entityType && (
            <span className="text-[8px] text-muted-foreground/20 flex-shrink-0">
              {entry.entityType}:{entry.entityId?.slice(0, 8)}
            </span>
          )}
        </div>

        {/* Expanded detail panel */}
        {isExpanded && showExpandable && (
          <div className="ml-8 mb-2 px-3 py-2 rounded-lg bg-[var(--bg-hover)]/20 border border-border/10">
            {entry.sessionId && (
              <div className="text-[9px] text-muted-foreground/30 mb-1">
                <span className="text-muted-foreground/50">Session:</span>{" "}
                {entry.sessionId}
              </div>
            )}
            {entry.targetId && (
              <div className="text-[9px] text-muted-foreground/30 mb-1">
                <span className="text-muted-foreground/50">Target:</span>{" "}
                <span className="font-mono">
                  {entry.targetId.slice(0, 12)}...
                </span>
              </div>
            )}
            <pre className="text-[9px] text-foreground/50 font-mono whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
              {JSON.stringify(entry.detail, null, 2)}
            </pre>
          </div>
        )}
      </div>
    );
  };

  const renderTargetView = () => {
    if (targetGroups.length === 0) {
      return (
        <div className="text-center text-[11px] text-muted-foreground/30 py-12">
          No operation log entries
        </div>
      );
    }

    return targetGroups.map((group) => {
      const groupKey = group.targetId ?? "__system__";
      const isCollapsed = isGroupCollapsed(groupKey);
      const categoryBadges = Array.from(group.categories).slice(0, 4);

      return (
        <div key={groupKey} className="mb-2">
          {/* Group header */}
          <div
            className="flex items-center gap-2 px-2 py-2 rounded-lg cursor-pointer hover:bg-muted/10 transition-colors sticky top-0 bg-background/95 backdrop-blur-sm z-10 border-b border-border/10"
            onClick={() => toggleGroup(groupKey)}
          >
            {isCollapsed ? (
              <ChevronRight className="w-3 h-3 text-muted-foreground/40" />
            ) : (
              <ChevronDown className="w-3 h-3 text-muted-foreground/40" />
            )}

            {group.targetId ? (
              <Target className="w-3.5 h-3.5 text-blue-400/70" />
            ) : (
              <Layers className="w-3.5 h-3.5 text-muted-foreground/40" />
            )}

            <span className="text-[11px] font-medium text-foreground/80 flex-1">
              {group.label}
            </span>

            {/* Category badges */}
            <div className="flex gap-1">
              {categoryBadges.map((cat) => (
                <span
                  key={cat}
                  className={cn(
                    "text-[8px] px-1 py-0.5 rounded",
                    CATEGORY_COLORS[cat] ?? "text-muted-foreground/50 bg-muted/5"
                  )}
                >
                  {cat}
                </span>
              ))}
            </div>

            <span className="text-[9px] text-muted-foreground/30">
              {group.entries.length}
            </span>
          </div>

          {/* Group entries */}
          {!isCollapsed && (
            <div className="ml-3 border-l border-border/10 pl-2 mt-1 space-y-0.5">
              {group.entries.map((entry) => renderEntry(entry, true))}
            </div>
          )}
        </div>
      );
    });
  };

  return (
    <div className="h-full flex flex-col bg-background/95">
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/20">
        <ClipboardList className="w-3.5 h-3.5 text-accent/70" />
        <span className="text-[11px] font-medium flex-1">Operation Log</span>
        <span className="text-[9px] text-muted-foreground/40">
          {filtered.length} entries
        </span>

        <button
          onClick={() => setShowSearch(!showSearch)}
          className={cn(
            "p-1 rounded transition-colors",
            showSearch
              ? "text-accent bg-accent/10"
              : "text-muted-foreground/30 hover:text-foreground"
          )}
        >
          <Search className="w-3 h-3" />
        </button>

        <div className="relative">
          <button
            onClick={() => setShowFilter(!showFilter)}
            className={cn(
              "p-1 rounded transition-colors",
              filterCategory !== "all"
                ? "text-accent bg-accent/10"
                : "text-muted-foreground/30 hover:text-foreground"
            )}
          >
            <Filter className="w-3 h-3" />
          </button>
          {showFilter && (
            <div className="absolute right-0 top-full mt-1 z-50 w-36 rounded-lg border border-border/30 bg-[#1e1e2e] shadow-xl py-1">
              {CATEGORY_OPTIONS.map((c) => (
                <button
                  key={c}
                  onClick={() => {
                    setFilterCategory(c);
                    setShowFilter(false);
                  }}
                  className={cn(
                    "block w-full text-left px-3 py-1 text-[10px] transition-colors",
                    filterCategory === c
                      ? "text-accent bg-accent/10"
                      : "text-foreground/70 hover:bg-muted/10"
                  )}
                >
                  {c === "all"
                    ? "All Categories"
                    : c.charAt(0).toUpperCase() + c.slice(1)}
                </button>
              ))}
            </div>
          )}
        </div>

        <button
          onClick={load}
          className="p-1 text-muted-foreground/30 hover:text-foreground transition-colors"
        >
          <RefreshCw className={cn("w-3 h-3", loading && "animate-spin")} />
        </button>
        <button
          onClick={handleClear}
          className="p-1 text-muted-foreground/30 hover:text-red-400 transition-colors"
        >
          <Trash2 className="w-3 h-3" />
        </button>
      </div>

      {/* Search bar */}
      {showSearch && (
        <div className="px-3 py-1.5 border-b border-border/10">
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && load()}
            placeholder="Search logs..."
            className="w-full bg-transparent text-[10px] text-foreground outline-none placeholder:text-muted-foreground/20"
            autoFocus
          />
        </div>
      )}

      {/* Entries */}
      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-0.5">
        {loading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" />
          </div>
        ) : (
          renderTargetView()
        )}
      </div>
    </div>
  );
}
