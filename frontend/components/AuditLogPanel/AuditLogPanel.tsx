import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  ClipboardList,
  Filter,
  Loader2,
  RefreshCw,
  Search,
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
];

export function AuditLogPanel() {
  const [entries, setEntries] = useState<AuditRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [filterCategory, setFilterCategory] = useState("all");
  const [showFilter, setShowFilter] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [showSearch, setShowSearch] = useState(false);
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const currentProjectPath = useStore((s) => s.currentProjectPath);

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

  const hasDetail = (detail: Record<string, unknown>) =>
    detail && Object.keys(detail).length > 0;

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
        ) : filtered.length === 0 ? (
          <div className="text-center text-[11px] text-muted-foreground/30 py-12">
            No operation log entries
          </div>
        ) : (
          filtered.map((entry) => {
            const isExpanded = expandedId === entry.id;
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
                      ? () => setExpandedId(isExpanded ? null : entry.id)
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
                    {formatTime(entry.createdAt)}
                  </span>

                  {/* Status dot */}
                  <span
                    className={cn(
                      "text-[8px] pt-1 flex-shrink-0",
                      STATUS_COLORS[entry.status] ??
                        "text-muted-foreground/40"
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
                        <span className="text-muted-foreground/50">
                          Session:
                        </span>{" "}
                        {entry.sessionId}
                      </div>
                    )}
                    {entry.targetId && (
                      <div className="text-[9px] text-muted-foreground/30 mb-1">
                        <span className="text-muted-foreground/50">
                          Target:
                        </span>{" "}
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
          })
        )}
      </div>
    </div>
  );
}
