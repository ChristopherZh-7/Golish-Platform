import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ClipboardList, RefreshCw, Trash2, Filter, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";

interface AuditEntry {
  timestamp: number;
  action: string;
  category: string;
  details: string;
  entity_type?: string;
  entity_id?: string;
}

const CATEGORY_COLORS: Record<string, string> = {
  targets: "text-blue-400",
  findings: "text-orange-400",
  vault: "text-purple-400",
  topology: "text-cyan-400",
  methodology: "text-green-400",
  notes: "text-yellow-400",
  pipeline: "text-pink-400",
  tools: "text-red-400",
  project: "text-indigo-400",
};

const CATEGORY_OPTIONS = [
  "all", "targets", "findings", "vault", "topology", "methodology",
  "notes", "pipeline", "tools", "project",
];

export function AuditLogPanel() {
  const [entries, setEntries] = useState<AuditEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [filterCategory, setFilterCategory] = useState("all");
  const [showFilter, setShowFilter] = useState(false);
  const currentProjectPath = useStore((s) => s.currentProjectPath);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const list = await invoke<AuditEntry[]>("audit_list", {
        limit: 500,
        projectPath: getProjectPath(),
      });
      setEntries(Array.isArray(list) ? list : []);
    } catch {
      setEntries([]);
    }
    setLoading(false);
  }, []);

  useEffect(() => { load(); }, [load, currentProjectPath]);

  const handleClear = useCallback(async () => {
    try {
      await invoke("audit_clear", { projectPath: getProjectPath() });
      setEntries([]);
    } catch { /* ignore */ }
  }, []);

  const safeEntries = entries ?? [];
  const filtered = filterCategory === "all" ? safeEntries : safeEntries.filter((e) => e.category === filterCategory);

  const formatTime = (ts: number) => {
    const d = new Date(ts * 1000);
    return d.toLocaleString(undefined, {
      month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit",
    });
  };

  return (
    <div className="h-full flex flex-col bg-background/95">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/20">
        <ClipboardList className="w-3.5 h-3.5 text-accent/70" />
        <span className="text-[11px] font-medium flex-1">Audit Log</span>
        <span className="text-[9px] text-muted-foreground/40">{filtered.length} entries</span>
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
            <div className="absolute right-0 top-full mt-1 z-50 w-32 rounded-lg border border-border/30 bg-[#1e1e2e] shadow-xl py-1">
              {CATEGORY_OPTIONS.map((c) => (
                <button
                  key={c}
                  onClick={() => { setFilterCategory(c); setShowFilter(false); }}
                  className={cn(
                    "block w-full text-left px-3 py-1 text-[10px] transition-colors",
                    filterCategory === c
                      ? "text-accent bg-accent/10"
                      : "text-foreground/70 hover:bg-muted/10"
                  )}
                >
                  {c === "all" ? "All Categories" : c.charAt(0).toUpperCase() + c.slice(1)}
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

      <div className="flex-1 overflow-y-auto px-3 py-2 space-y-0.5">
        {filtered.length === 0 ? (
          <div className="text-center text-[11px] text-muted-foreground/30 py-12">
            No audit entries
          </div>
        ) : (
          filtered.map((entry, i) => (
            <div
              key={`${entry.timestamp}-${i}`}
              className="flex items-start gap-2 py-1.5 px-2 rounded hover:bg-muted/5 transition-colors group"
            >
              <span className="text-[9px] text-muted-foreground/30 whitespace-nowrap pt-0.5 w-28 flex-shrink-0">
                {formatTime(entry.timestamp)}
              </span>
              <span className={cn(
                "text-[9px] font-medium px-1.5 py-0.5 rounded bg-muted/5 flex-shrink-0",
                CATEGORY_COLORS[entry.category] || "text-muted-foreground/50"
              )}>
                {entry.category}
              </span>
              <div className="flex-1 min-w-0">
                <span className="text-[10px] text-foreground/70">{entry.action}</span>
                {entry.details && (
                  <span className="text-[9px] text-muted-foreground/30 ml-1.5">{entry.details}</span>
                )}
              </div>
              {entry.entity_type && (
                <span className="text-[8px] text-muted-foreground/20 flex-shrink-0">
                  {entry.entity_type}:{entry.entity_id?.slice(0, 8)}
                </span>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
