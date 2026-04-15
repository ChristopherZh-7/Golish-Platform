import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { QuickNotes } from "@/components/QuickNotes/QuickNotes";
import {
  Check, ChevronDown, ChevronRight, Crosshair, Database, FileCode2, Globe,
  Hash, Loader2, Map as MapIcon, Network,
  Plus, Search, Server, Shield, ShieldOff, Tag, Trash2, Wifi, X, Zap,
} from "lucide-react";
import { TopologyView } from "@/components/TopologyView/TopologyView";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";
import {
  targetAssetsList,
  apiEndpointsList,
  fingerprintsList,
  jsAnalysisList,
  oplogListByTarget,
  type TargetAsset,
  type ApiEndpoint,
  type Fingerprint,
  type JsAnalysisResult,
  type AuditRow,
} from "@/lib/security-analysis";

type TargetStatus = "new" | "recon" | "recondone" | "scanning" | "tested";

interface PortInfo {
  port: number;
  protocol?: string;
  service?: string;
  state?: string;
}

interface Target {
  id: string;
  name: string;
  type: "domain" | "ip" | "cidr" | "url" | "wildcard";
  value: string;
  tags: string[];
  notes: string;
  scope: "in" | "out";
  status: TargetStatus;
  source: string;
  parent_id: string | null;
  ports: PortInfo[];
  technologies: string[];
  real_ip: string;
  cdn_waf: string;
  http_title: string;
  http_status: number | null;
  webserver: string;
  os_info: string;
  content_type: string;
  created_at: number;
  updated_at: number;
}

interface TargetStore {
  targets: Target[];
}

const TYPE_ICONS: Record<string, React.ReactNode> = {
  domain: <Globe className="w-3.5 h-3.5 text-blue-400" />,
  ip: <Hash className="w-3.5 h-3.5 text-green-400" />,
  cidr: <Network className="w-3.5 h-3.5 text-yellow-400" />,
  url: <Globe className="w-3.5 h-3.5 text-purple-400" />,
  wildcard: <Crosshair className="w-3.5 h-3.5 text-orange-400" />,
};

const TYPE_LABELS: Record<string, string> = {
  domain: "targets.domain",
  ip: "targets.ip",
  cidr: "targets.cidr",
  url: "targets.url",
  wildcard: "targets.wildcard",
};

const STATUS_CONFIG: Record<TargetStatus, { label: string; color: string; bg: string }> = {
  new: { label: "New", color: "text-gray-400", bg: "bg-gray-500/10" },
  recon: { label: "Recon", color: "text-blue-400", bg: "bg-blue-500/10" },
  recondone: { label: "Recon Done", color: "text-cyan-400", bg: "bg-cyan-500/10" },
  scanning: { label: "Scanning", color: "text-yellow-400", bg: "bg-yellow-500/10" },
  tested: { label: "Tested", color: "text-green-400", bg: "bg-green-500/10" },
};

function MiniDropdown({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        className={cn(
          "flex items-center gap-1.5 px-2 py-1.5 text-xs rounded-md border transition-colors",
          "bg-[var(--bg-hover)]/30 border-border/30 text-foreground",
          "hover:bg-[var(--bg-hover)]/60 hover:border-border/50",
          open && "border-accent/40 bg-[var(--bg-hover)]/50",
        )}
        onClick={() => setOpen(!open)}
      >
        <span className="truncate max-w-[80px]">{selected?.label ?? value}</span>
        <ChevronDown className={cn("w-3 h-3 text-muted-foreground transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute top-full left-0 mt-1 min-w-[120px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1">
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={cn(
                "w-full text-left px-3 py-1.5 text-xs transition-colors",
                "hover:bg-[var(--bg-hover)]/60",
                opt.value === value && "text-accent bg-accent/5 font-medium",
              )}
              onClick={() => { onChange(opt.value); setOpen(false); }}
            >
              {opt.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

import { lazy, Suspense } from "react";
const SecurityViewLazy = lazy(() =>
  import("@/components/SecurityView/SecurityView").then((m) => ({ default: m.SecurityView }))
);
import { ScanPanel } from "@/components/ScanPanel/ScanPanel";

type TargetTab = "targets" | "topology" | "security";

export function TargetPanel() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [activeTab, setActiveTab] = useState<TargetTab>("targets");
  const [store, setStore] = useState<TargetStore>({ targets: [] });
  const [search, setSearch] = useState("");
  const [scopeFilter, setScopeFilter] = useState<"all" | "in" | "out">("all");
  const [showAdd, setShowAdd] = useState(false);
  const [showBatch, setShowBatch] = useState(false);
  const [batchInput, setBatchInput] = useState("");
  const [addForm, setAddForm] = useState({ name: "", value: "", notes: "", tags: "" });
  const [editingId, setEditingId] = useState<string | null>(null);

  const workspaceReady = useStore((s) => s.workspaceDataReady);

  const loadTargets = useCallback(async () => {
    try {
      const data = await invoke<TargetStore>("target_list", { projectPath: getProjectPath() });
      setStore(data && data.targets ? data : { targets: [] });
    } catch (e) {
      console.error("Failed to load targets:", e);
      setTimeout(() => {
        invoke<TargetStore>("target_list", { projectPath: getProjectPath() })
          .then((data) => setStore(data && data.targets ? data : { targets: [] }))
          .catch(() => {});
      }, 3000);
    }
  }, []);

  useEffect(() => {
    if (workspaceReady) loadTargets();
  }, [loadTargets, currentProjectPath, workspaceReady]);

  useEffect(() => {
    const REFRESH_TOOLS = new Set(["manage_targets", "record_finding", "run_pipeline"]);
    const unlistenAi = listen<{ type: string; tool_name?: string }>("ai-event", (event) => {
      if (event.payload.type === "tool_result" && event.payload.tool_name && REFRESH_TOOLS.has(event.payload.tool_name)) {
        loadTargets();
      }
    });
    const unlistenPipeline = listen<{ status: string }>("pipeline-event", (event) => {
      if (event.payload.status === "completed" || event.payload.status === "error") {
        loadTargets();
      }
    });
    const unlistenDb = listen("db-ready", () => {
      loadTargets();
    });
    return () => {
      unlistenAi.then((fn) => fn());
      unlistenPipeline.then((fn) => fn());
      unlistenDb.then((fn) => fn());
    };
  }, [loadTargets]);

  const [addError, setAddError] = useState<string | null>(null);

  const handleAdd = useCallback(async () => {
    if (!addForm.value.trim()) return;
    setAddError(null);
    try {
      await invoke("target_add", {
        name: addForm.name,
        value: addForm.value.trim(),
        notes: addForm.notes,
        tags: addForm.tags ? addForm.tags.split(",").map((s) => s.trim()).filter(Boolean) : [],
        projectPath: getProjectPath(),
      });
      setAddForm({ name: "", value: "", notes: "", tags: "" });
      setShowAdd(false);
      loadTargets();
      emit("targets-changed").catch(() => {});
      invoke("audit_log", { action: "target_added", category: "targets", details: addForm.value.trim(), projectPath: getProjectPath() }).catch(() => {});
    } catch (e) {
      const msg = String(e);
      if (msg.includes("duplicate") || msg.includes("unique") || msg.includes("already exists")) {
        setAddError("Target already exists");
      } else {
        setAddError(msg.slice(0, 100));
      }
      console.error("Failed to add target:", e);
    }
  }, [addForm, loadTargets]);

  const handleBatchAdd = useCallback(async () => {
    if (!batchInput.trim()) return;
    try {
      const added = await invoke<Target[]>("target_batch_add", {
        values: batchInput,
        projectPath: getProjectPath(),
      });
      setBatchInput("");
      setShowBatch(false);
      loadTargets();
      if (added.length > 0) {
        console.info(`Imported ${added.length} targets`);
      }
    } catch (e) {
      console.error("Failed to batch add:", e);
    }
  }, [batchInput, loadTargets]);

  const handleDelete = useCallback(async (id: string) => {
    try {
      await invoke("target_delete", { id, projectPath: getProjectPath() });
      loadTargets();
      invoke("audit_log", { action: "target_deleted", category: "targets", details: id, entityType: "target", entityId: id, projectPath: getProjectPath() }).catch(() => {});
    } catch (e) {
      console.error("Failed to delete target:", e);
    }
  }, [loadTargets]);

  const handleToggleScope = useCallback(async (target: Target) => {
    try {
      await invoke("target_update", {
        id: target.id,
        scope: target.scope === "in" ? "out" : "in",
        projectPath: getProjectPath(),
      });
      loadTargets();
    } catch (e) {
      console.error("Failed to update scope:", e);
    }
  }, [loadTargets]);

  const handleUpdateNotes = useCallback(async (id: string, notes: string) => {
    try {
      await invoke("target_update", { id, notes, projectPath: getProjectPath() });
      loadTargets();
    } catch (e) {
      console.error("Failed to update notes:", e);
    }
  }, [loadTargets]);

  const handleClearAll = useCallback(async () => {
    if (!confirm(t("targets.clearConfirm"))) return;
    try {
      await invoke("target_clear_all", { projectPath: getProjectPath() });
      loadTargets();
    } catch (e) {
      console.error("Failed to clear:", e);
    }
  }, [t, loadTargets]);

  const safeTargets = store?.targets ?? [];

  const filtered = useMemo(() => {
    let list = safeTargets;
    if (search) {
      const q = search.toLowerCase();
      list = list.filter((t) =>
        t.name.toLowerCase().includes(q) ||
        t.value.toLowerCase().includes(q) ||
        t.tags.some((tag) => tag.toLowerCase().includes(q))
      );
    }
    if (scopeFilter !== "all") {
      list = list.filter((t) => t.scope === scopeFilter);
    }
    return list;
  }, [safeTargets, search, scopeFilter]);

  const childrenMap = useMemo(() => {
    const map = new Map<string, Target[]>();
    for (const t of filtered) {
      if (t.parent_id) {
        const arr = map.get(t.parent_id) || [];
        arr.push(t);
        map.set(t.parent_id, arr);
      }
    }
    return map;
  }, [filtered]);

  const rootTargets = useMemo(() =>
    filtered.filter((t) => !t.parent_id),
  [filtered]);

  const [expandedParents, setExpandedParents] = useState<Set<string>>(new Set());

  const stats = useMemo(() => ({
    total: safeTargets.length,
    inScope: safeTargets.filter((t) => t.scope === "in").length,
    outOfScope: safeTargets.filter((t) => t.scope === "out").length,
  }), [safeTargets]);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2.5 border-b border-border/50">
        <div className="flex items-center gap-1">
          <button
            type="button"
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md transition-colors",
              activeTab === "targets"
                ? "bg-accent/15 text-accent font-medium"
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40",
            )}
            onClick={() => setActiveTab("targets")}
          >
            <Crosshair className="w-3.5 h-3.5" />
            {t("targets.title")}
            <span className="text-[10px] text-muted-foreground/60 tabular-nums">{stats.total}</span>
          </button>
          <button
            type="button"
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md transition-colors",
              activeTab === "topology"
                ? "bg-accent/15 text-accent font-medium"
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40",
            )}
            onClick={() => setActiveTab("topology")}
          >
            <MapIcon className="w-3.5 h-3.5" />
            {t("activity.topology")}
          </button>
          <button
            type="button"
            className={cn(
              "flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md transition-colors",
              activeTab === "security"
                ? "bg-accent/15 text-accent font-medium"
                : "text-muted-foreground hover:text-foreground hover:bg-muted/40",
            )}
            onClick={() => setActiveTab("security")}
          >
            <Shield className="w-3.5 h-3.5" />
            {t("security.title", "Security")}
          </button>
        </div>
        {activeTab === "targets" && (
        <div className="flex items-center gap-1">
          <button
            className="p-1.5 rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
            onClick={() => { setShowBatch(true); setShowAdd(false); }}
            title={t("targets.batchAdd")}
          >
            <Hash className="w-3.5 h-3.5" />
          </button>
          <button
            className="p-1.5 rounded hover:bg-muted/50 text-muted-foreground hover:text-foreground transition-colors"
            onClick={() => { setShowAdd(true); setShowBatch(false); }}
            title={t("targets.addTarget")}
          >
            <Plus className="w-3.5 h-3.5" />
          </button>
          {stats.total > 0 && (
            <button
              className="p-1.5 rounded hover:bg-red-500/20 text-muted-foreground hover:text-red-400 transition-colors"
              onClick={handleClearAll}
              title={t("targets.clearAll")}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          )}
        </div>
        )}
      </div>

      {activeTab === "topology" ? (
        <div className="flex-1 min-h-0"><TopologyView /></div>
      ) : activeTab === "security" ? (
        <div className="flex-1 min-h-0 overflow-hidden">
          <Suspense fallback={<div className="h-full flex items-center justify-center"><Loader2 className="w-5 h-5 animate-spin text-muted-foreground/20" /></div>}>
            <SecurityViewLazy />
          </Suspense>
        </div>
      ) : (
      <>
      {/* Filter bar */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/30">
        <div className="flex-1 relative">
          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
          <input
            className="w-full pl-7 pr-2 py-1.5 text-xs bg-background border border-border/50 rounded focus:border-accent outline-none"
            placeholder={t("common.search")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <MiniDropdown
          value={scopeFilter}
          onChange={(v) => setScopeFilter(v as "all" | "in" | "out")}
          options={[
            { value: "all", label: t("targets.all") },
            { value: "in", label: t("targets.inScope") },
            { value: "out", label: t("targets.outOfScope") },
          ]}
        />
      </div>

      {/* Add single target form */}
      {showAdd && (
        <div className="px-4 py-3 border-b border-border/30 bg-muted/10 space-y-2">
          <div className="flex items-center gap-2">
            <input
              className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
              placeholder={`${t("targets.value")} *  (e.g. example.com, 192.168.1.0/24, https://...)`}
              value={addForm.value}
              onChange={(e) => setAddForm((f) => ({ ...f, value: e.target.value }))}
              onKeyDown={(e) => { if (e.key === "Enter") handleAdd(); if (e.key === "Escape") setShowAdd(false); }}
              autoFocus
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
              placeholder={`${t("targets.name")} (${t("common.default")}: ${t("targets.value")})`}
              value={addForm.name}
              onChange={(e) => setAddForm((f) => ({ ...f, name: e.target.value }))}
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
              placeholder={`${t("targets.tags")} (comma separated)`}
              value={addForm.tags}
              onChange={(e) => setAddForm((f) => ({ ...f, tags: e.target.value }))}
            />
            <input
              className="flex-1 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent"
              placeholder={t("targets.notes")}
              value={addForm.notes}
              onChange={(e) => setAddForm((f) => ({ ...f, notes: e.target.value }))}
            />
          </div>
          {addError && (
            <p className="text-[11px] text-red-400">{addError}</p>
          )}
          <div className="flex justify-end gap-2">
            <button
              className="px-3 py-1 text-xs rounded bg-muted/50 hover:bg-muted text-foreground"
              onClick={() => { setShowAdd(false); setAddError(null); }}
            >{t("common.cancel")}</button>
            <button
              className="px-3 py-1 text-xs rounded bg-accent text-accent-foreground hover:bg-accent/90"
              onClick={handleAdd}
            >{t("targets.addTarget")}</button>
          </div>
        </div>
      )}

      {/* Batch import */}
      {showBatch && (
        <div className="px-4 py-3 border-b border-border/30 bg-muted/10 space-y-2">
          <textarea
            className="w-full h-32 text-xs bg-background border border-border/50 rounded px-2 py-1.5 outline-none focus:border-accent resize-none font-mono"
            placeholder={t("targets.batchPlaceholder")}
            value={batchInput}
            onChange={(e) => setBatchInput(e.target.value)}
            autoFocus
          />
          <div className="flex justify-end gap-2">
            <button
              className="px-3 py-1 text-xs rounded bg-muted/50 hover:bg-muted text-foreground"
              onClick={() => setShowBatch(false)}
            >{t("common.cancel")}</button>
            <button
              className="px-3 py-1 text-xs rounded bg-accent text-accent-foreground hover:bg-accent/90"
              onClick={handleBatchAdd}
            >{t("targets.batchAdd")}</button>
          </div>
        </div>
      )}

      {/* Stats bar */}
      {stats.total > 0 && (
        <div className="flex items-center gap-3 px-4 py-1.5 border-b border-border/20 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1">
            <Shield className="w-3 h-3 text-green-400" />
            {stats.inScope} {t("targets.inScope")}
          </span>
          <span className="flex items-center gap-1">
            <ShieldOff className="w-3 h-3 text-red-400" />
            {stats.outOfScope} {t("targets.outOfScope")}
          </span>
        </div>
      )}

      {/* Target list */}
      <div className="flex-1 overflow-y-auto">
        {filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground">
            <Crosshair className="w-8 h-8 mb-2 opacity-30" />
            <p className="text-xs">{t("targets.noTargets")}</p>
          </div>
        ) : (
          <div className="divide-y divide-border/20">
            {rootTargets.map((target) => {
              const children = childrenMap.get(target.id) || [];
              const hasChildren = children.length > 0;
              const isParentExpanded = expandedParents.has(target.id);
              return (
              <div key={target.id}>
              <div
                className={cn(
                  "px-4 py-2.5 hover:bg-muted/30 transition-colors group cursor-pointer",
                  target.scope === "out" && "opacity-50",
                  editingId === target.id && "bg-muted/20",
                )}
                onClick={() => setEditingId(editingId === target.id ? null : target.id)}
              >
                <div className="flex items-center gap-2">
                  {TYPE_ICONS[target.type] || <Globe className="w-3.5 h-3.5" />}

                  <button
                    className={cn(
                      "p-0.5 rounded transition-colors",
                      target.scope === "in"
                        ? "text-green-400 hover:text-green-300"
                        : "text-red-400 hover:text-red-300",
                    )}
                    onClick={(e) => { e.stopPropagation(); handleToggleScope(target); }}
                    title={target.scope === "in" ? t("targets.inScope") : t("targets.outOfScope")}
                  >
                    {target.scope === "in" ? <Shield className="w-3 h-3" /> : <ShieldOff className="w-3 h-3" />}
                  </button>

                  <span className="text-xs font-mono text-foreground flex-1 truncate">{target.value}</span>

                  {target.status && target.status !== "new" && (() => {
                    const cfg = STATUS_CONFIG[target.status] || STATUS_CONFIG.new;
                    return (
                      <span className={cn("text-[10px] px-1.5 py-0.5 rounded font-medium", cfg.color, cfg.bg)}>
                        {cfg.label}
                      </span>
                    );
                  })()}

                  {target.ports && target.ports.length > 0 && (
                    <span className="flex items-center gap-0.5 text-[10px] text-emerald-400/80" title={`${target.ports.length} open port(s)`}>
                      <Wifi className="w-2.5 h-2.5" />
                      {target.ports.length}
                    </span>
                  )}

                  <span className="text-[10px] text-muted-foreground px-1.5 py-0.5 rounded bg-muted/30">
                    {t(TYPE_LABELS[target.type] || target.type)}
                  </span>

                  <button
                    className="p-1 rounded opacity-0 group-hover:opacity-100 hover:bg-red-500/20 text-muted-foreground hover:text-red-400 transition-all"
                    onClick={(e) => { e.stopPropagation(); handleDelete(target.id); }}
                  >
                    <X className="w-3 h-3" />
                  </button>
                </div>

                {target.tags.length > 0 && (
                  <div className="flex items-center gap-1 mt-1 ml-5">
                    <Tag className="w-2.5 h-2.5 text-muted-foreground" />
                    {target.tags.map((tag) => (
                      <span key={tag} className="text-[10px] px-1.5 py-0.5 rounded bg-muted/40 text-muted-foreground">
                        {tag}
                      </span>
                    ))}
                  </div>
                )}

                {editingId === target.id && (
                  <TargetDetailView target={target} t={t} onUpdateNotes={handleUpdateNotes} />
                )}
              </div>

              {/* Children toggle + count */}
              {hasChildren && (
                <button
                  type="button"
                  className="flex items-center gap-1 px-4 py-1 text-[11px] text-muted-foreground hover:text-foreground transition-colors w-full"
                  onClick={(e) => {
                    e.stopPropagation();
                    setExpandedParents((prev) => {
                      const next = new Set(prev);
                      if (next.has(target.id)) next.delete(target.id); else next.add(target.id);
                      return next;
                    });
                  }}
                >
                  <ChevronDown className={cn("w-3 h-3 transition-transform", !isParentExpanded && "-rotate-90")} />
                  <Network className="w-3 h-3 text-accent/60" />
                  <span>{children.length} subdomain{children.length > 1 ? "s" : ""}</span>
                </button>
              )}

              {/* Child targets */}
              {hasChildren && isParentExpanded && (
                <div className="border-l-2 border-accent/20 ml-6">
                  {children.map((child) => (
                    <div
                      key={child.id}
                      className={cn(
                        "pl-3 pr-4 py-1.5 hover:bg-muted/20 transition-colors cursor-pointer",
                        child.scope === "out" && "opacity-50",
                        editingId === child.id && "bg-muted/15",
                      )}
                      onClick={() => setEditingId(editingId === child.id ? null : child.id)}
                    >
                      <div className="flex items-center gap-1.5">
                        {TYPE_ICONS[child.type] || <Globe className="w-3 h-3" />}
                        <span className="text-[11px] font-mono text-foreground/80 flex-1 truncate">{child.value}</span>
                        {child.http_status != null && (
                          <span className={cn("text-[10px] font-mono", child.http_status < 400 ? "text-green-400/70" : "text-red-400/70")}>{child.http_status}</span>
                        )}
                        {child.ports && child.ports.length > 0 && (
                          <span className="flex items-center gap-0.5 text-[10px] text-emerald-400/60">
                            <Wifi className="w-2.5 h-2.5" />{child.ports.length}
                          </span>
                        )}
                      </div>
                      {editingId === child.id && (
                        <div className="mt-1 ml-4 space-y-1 text-[10px]">
                          {child.http_title && <div className="text-muted-foreground"><span className="text-blue-400">Title:</span> {child.http_title}</div>}
                          {child.technologies && child.technologies.length > 0 && (
                            <div className="flex flex-wrap gap-0.5">
                              {child.technologies.map((tech) => (
                                <span key={tech} className="px-1 py-0.5 rounded bg-purple-500/10 text-purple-400/80">{tech}</span>
                              ))}
                            </div>
                          )}
                          {child.real_ip && <div className="text-muted-foreground font-mono"><span className="text-emerald-400">IP:</span> {child.real_ip}</div>}
                          {child.webserver && <div className="text-muted-foreground"><span className="text-orange-400">Server:</span> {child.webserver}</div>}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              )}
              </div>
              );
            })}
          </div>
        )}
      </div>
      </>
      )}
    </div>
  );
}

// ── Target Detail View (hierarchical: info → ports → security data) ──

function TargetDetailView({
  target,
  t,
  onUpdateNotes,
}: {
  target: Target;
  t: (key: string) => string;
  onUpdateNotes: (id: string, notes: string) => void;
}) {
  const [secData, setSecData] = useState<{
    assets: TargetAsset[];
    endpoints: ApiEndpoint[];
    fingerprints: Fingerprint[];
    jsResults: JsAnalysisResult[];
    logs: AuditRow[];
  }>({ assets: [], endpoints: [], fingerprints: [], jsResults: [], logs: [] });
  const [secLoading, setSecLoading] = useState(true);
  const [expandedPorts, setExpandedPorts] = useState<Set<number>>(new Set());
  const [showLogs, setShowLogs] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setSecLoading(true);
      try {
        const [a, e, f, j, l] = await Promise.all([
          targetAssetsList(target.id).catch(() => []),
          apiEndpointsList(target.id).catch(() => []),
          fingerprintsList(target.id).catch(() => []),
          jsAnalysisList(target.id).catch(() => []),
          oplogListByTarget(target.id, 50).catch(() => []),
        ]);
        if (!cancelled) {
          setSecData({
            assets: Array.isArray(a) ? a : [],
            endpoints: Array.isArray(e) ? e : [],
            fingerprints: Array.isArray(f) ? f : [],
            jsResults: Array.isArray(j) ? j : [],
            logs: Array.isArray(l) ? l : [],
          });
        }
      } catch { /* ignore */ }
      if (!cancelled) setSecLoading(false);
    })();
    return () => { cancelled = true; };
  }, [target.id]);

  const togglePort = (port: number) => {
    setExpandedPorts((prev) => {
      const next = new Set(prev);
      if (next.has(port)) next.delete(port); else next.add(port);
      return next;
    });
  };

  const extractPort = (url: string): number | null => {
    try {
      const u = new URL(url);
      if (u.port) return Number.parseInt(u.port);
      return u.protocol === "https:" ? 443 : 80;
    } catch {
      return null;
    }
  };

  const endpointsByPort = useMemo(() => {
    const map = new Map<number, ApiEndpoint[]>();
    for (const ep of secData.endpoints) {
      const port = extractPort(ep.url);
      if (port != null) {
        const arr = map.get(port) || [];
        arr.push(ep);
        map.set(port, arr);
      }
    }
    return map;
  }, [secData.endpoints]);

  const jsByPort = useMemo(() => {
    const map = new Map<number, JsAnalysisResult[]>();
    for (const js of secData.jsResults) {
      const port = extractPort(js.url);
      if (port != null) {
        const arr = map.get(port) || [];
        arr.push(js);
        map.set(port, arr);
      }
    }
    return map;
  }, [secData.jsResults]);

  const allPorts = useMemo(() => {
    const portSet = new Set<number>();
    for (const p of target.ports || []) portSet.add(p.port);
    for (const k of endpointsByPort.keys()) portSet.add(k);
    for (const k of jsByPort.keys()) portSet.add(k);
    return [...portSet].sort((a, b) => a - b);
  }, [target.ports, endpointsByPort, jsByPort]);

  const getPortInfo = (port: number): PortInfo | undefined =>
    target.ports?.find((p) => p.port === port);

  const methodCol: Record<string, string> = {
    GET: "text-green-400", POST: "text-blue-400", PUT: "text-yellow-400",
    DELETE: "text-red-400", PATCH: "text-purple-400",
  };
  const riskCol: Record<string, string> = {
    critical: "bg-red-500/10 text-red-400", high: "bg-orange-500/10 text-orange-400",
    medium: "bg-yellow-500/10 text-yellow-400", low: "bg-blue-500/10 text-blue-400",
  };

  return (
    <div className="mt-2 ml-5 space-y-2" onClick={(e) => e.stopPropagation()}>
      {/* ── Basic Info ── */}
      {target.name !== target.value && (
        <div className="text-[11px] text-muted-foreground">
          <span className="font-medium">{t("targets.name")}:</span> {target.name}
        </div>
      )}
      {target.source && target.source !== "manual" && (
        <div className="text-[11px] text-muted-foreground">
          <span className="font-medium">Source:</span> {target.source}
        </div>
      )}

      {/* ── Recon Info ── */}
      {(target.http_title || target.http_status || target.real_ip || target.webserver || target.cdn_waf || target.os_info || target.content_type) && (
        <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[11px]">
          {target.http_title && (
            <div className="col-span-2 text-muted-foreground"><span className="font-medium text-blue-400">Title:</span> {target.http_title}</div>
          )}
          {target.http_status != null && (
            <div className="text-muted-foreground"><span className="font-medium text-cyan-400">Status:</span> <span className={cn("font-mono", target.http_status < 400 ? "text-green-400" : "text-red-400")}>{target.http_status}</span></div>
          )}
          {target.real_ip && (
            <div className="text-muted-foreground"><span className="font-medium text-emerald-400">IP:</span> <span className="font-mono">{target.real_ip}</span></div>
          )}
          {target.webserver && (
            <div className="text-muted-foreground"><span className="font-medium text-orange-400">Server:</span> {target.webserver}</div>
          )}
          {target.cdn_waf && (
            <div className="text-muted-foreground"><span className="font-medium text-yellow-400">CDN/WAF:</span> {target.cdn_waf}</div>
          )}
          {target.os_info && (
            <div className="text-muted-foreground"><span className="font-medium text-pink-400">OS:</span> {target.os_info}</div>
          )}
          {target.content_type && (
            <div className="text-muted-foreground"><span className="font-medium text-violet-400">Content:</span> {target.content_type}</div>
          )}
        </div>
      )}

      {/* ── Technologies (target-level fingerprints) ── */}
      {target.technologies && target.technologies.length > 0 && (
        <div className="flex flex-wrap items-center gap-1">
          <Zap className="w-3 h-3 text-purple-400 shrink-0" />
          {target.technologies.map((tech) => (
            <span key={tech} className="text-[10px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400">{tech}</span>
          ))}
        </div>
      )}

      {/* ── Fingerprints from DB ── */}
      {secData.fingerprints.length > 0 && (
        <div className="space-y-0.5">
          <div className="flex items-center gap-1 text-[11px] text-muted-foreground font-medium">
            <Shield className="w-3 h-3 text-purple-400" />
            Fingerprints ({secData.fingerprints.length})
          </div>
          <div className="pl-4 space-y-0.5">
            {secData.fingerprints.map((fp) => (
              <div key={fp.id} className="flex items-center gap-2 text-[10px]">
                <span className="px-1 py-0.5 rounded bg-purple-500/10 text-purple-400 text-[9px]">{fp.category}</span>
                <span className="text-foreground/70">{fp.name}</span>
                {fp.version && <span className="font-mono text-muted-foreground/40">{fp.version}</span>}
                <div className="flex items-center gap-0.5 ml-auto">
                  <div className="w-6 h-1 rounded-full bg-muted/20 overflow-hidden">
                    <div className={cn("h-full rounded-full", fp.confidence >= 80 ? "bg-green-500" : fp.confidence >= 50 ? "bg-yellow-500" : "bg-red-500")} style={{ width: `${Math.min(100, fp.confidence)}%` }} />
                  </div>
                  <span className="text-[8px] text-muted-foreground/30">{fp.confidence}%</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* ── Scan Workflow Panel ── */}
      {(target.type === "url" || target.type === "domain") && (
        <ScanPanel targetId={target.id} targetUrl={target.value} />
      )}

      {/* ── Ports (expandable, hierarchical) ── */}
      {allPorts.length > 0 && (
        <div className="space-y-0.5">
          <div className="flex items-center gap-1 text-[11px] text-muted-foreground font-medium">
            <Server className="w-3 h-3" />
            Ports ({allPorts.length})
            {secLoading && <Loader2 className="w-3 h-3 animate-spin text-muted-foreground/20 ml-1" />}
          </div>
          <div className="pl-2 space-y-0.5">
            {allPorts.map((port) => {
              const info = getPortInfo(port);
              const portEndpoints = endpointsByPort.get(port) || [];
              const portJs = jsByPort.get(port) || [];
              const isExpanded = expandedPorts.has(port);
              const hasData = portEndpoints.length > 0 || portJs.length > 0;

              return (
                <div key={port} className="rounded-lg border border-border/10 overflow-hidden">
                  <button
                    type="button"
                    onClick={() => togglePort(port)}
                    className="flex items-center gap-2 w-full px-2 py-1.5 text-left hover:bg-muted/10 transition-colors"
                  >
                    {hasData ? (
                      isExpanded ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/30" />
                    ) : (
                      <span className="w-2.5" />
                    )}
                    <span className="text-[11px] font-mono text-emerald-400 font-medium">{port}</span>
                    {info?.protocol && <span className="text-[9px] text-muted-foreground/40">/{info.protocol}</span>}
                    {info?.service && <span className="text-[10px] text-foreground/60">{info.service}</span>}
                    {info?.state && info.state !== "open" && (
                      <span className="text-[9px] text-muted-foreground/30">[{info.state}]</span>
                    )}
                    {portEndpoints.length > 0 && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 ml-auto">
                        {portEndpoints.length} endpoints
                      </span>
                    )}
                    {portJs.length > 0 && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded bg-yellow-500/10 text-yellow-400">
                        {portJs.length} JS
                      </span>
                    )}
                  </button>

                  {isExpanded && hasData && (
                    <div className="border-t border-border/5 px-2 py-1 space-y-1.5 bg-[var(--bg-hover)]/10">
                      {/* Endpoints under this port */}
                      {portEndpoints.length > 0 && (
                        <div>
                          <div className="text-[9px] text-muted-foreground/40 font-medium mb-0.5">Endpoints</div>
                          {portEndpoints.map((ep) => (
                            <div key={ep.id} className="flex items-center gap-2 py-0.5 text-[10px]">
                              <span className={cn("font-mono font-medium w-10 text-right flex-shrink-0", methodCol[ep.method] ?? "text-muted-foreground")}>{ep.method}</span>
                              <span className="font-mono text-foreground/60 flex-1 truncate">{ep.path}</span>
                              {ep.authType && <span className="text-muted-foreground/30 text-[9px]">{ep.authType}</span>}
                              <span className={cn("text-[8px] px-1 py-0.5 rounded", riskCol[ep.riskLevel] ?? "bg-zinc-500/10 text-zinc-400")}>{ep.riskLevel}</span>
                              {ep.tested ? <Check className="w-2.5 h-2.5 text-green-400" /> : <span className="text-muted-foreground/15 text-[9px]">—</span>}
                            </div>
                          ))}
                        </div>
                      )}

                      {/* JS files under this port */}
                      {portJs.length > 0 && (
                        <div>
                          <div className="text-[9px] text-muted-foreground/40 font-medium mb-0.5">JS Files</div>
                          {portJs.map((js) => {
                            const secrets = Array.isArray(js.secretsFound) ? js.secretsFound.length : 0;
                            return (
                              <div key={js.id} className="flex items-center gap-2 py-0.5 text-[10px]">
                                <FileCode2 className="w-3 h-3 text-yellow-400 flex-shrink-0" />
                                <span className="font-mono text-foreground/60 flex-1 truncate">{js.filename || js.url}</span>
                                {secrets > 0 && <span className="text-[8px] px-1 py-0.5 rounded bg-red-500/10 text-red-400">{secrets} secrets</span>}
                                {js.sourceMaps && <span className="text-[8px] px-1 py-0.5 rounded bg-yellow-500/10 text-yellow-400">srcmaps</span>}
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* ── Operation Logs (collapsible) ── */}
      {secData.logs.length > 0 && (
        <div className="rounded-lg border border-border/10 overflow-hidden">
          <button
            type="button"
            onClick={() => setShowLogs(!showLogs)}
            className="flex items-center gap-2 w-full px-2 py-1.5 text-left hover:bg-muted/10 transition-colors"
          >
            {showLogs ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/30" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/30" />}
            <Database className="w-3 h-3 text-accent/50" />
            <span className="text-[11px] text-muted-foreground font-medium">Operation Logs ({secData.logs.length})</span>
          </button>
          {showLogs && (
            <div className="border-t border-border/5 max-h-[200px] overflow-y-auto">
              <SecLogs data={secData.logs} />
            </div>
          )}
        </div>
      )}

      {/* ── Notes ── */}
      <textarea
        className="w-full text-[11px] bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent resize-none"
        placeholder={t("targets.notes")}
        rows={2}
        defaultValue={target.notes}
        onBlur={(e) => {
          if (e.target.value !== target.notes) {
            onUpdateNotes(target.id, e.target.value);
          }
        }}
      />
      <QuickNotes entityType="target" entityId={target.id} compact />
    </div>
  );
}

// ── Shared sub-components ──

function SecLogs({ data }: { data: AuditRow[] }) {
  if (data.length === 0) return <EmptySecData label="No operation logs for this target" />;
  const statusDot: Record<string, string> = {
    completed: "bg-green-500", running: "bg-yellow-500", failed: "bg-red-500", pending: "bg-zinc-500",
  };
  return (
    <div className="divide-y divide-border/5">
      {data.map((e) => (
        <div key={e.id} className="flex items-center gap-2 px-3 py-1.5 text-[10px]">
          <span className={cn("w-1.5 h-1.5 rounded-full flex-shrink-0", statusDot[e.status] ?? "bg-zinc-500")} />
          <span className="text-muted-foreground/30 font-mono w-24 flex-shrink-0 text-[9px]">
            {new Date(e.createdAt).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
          </span>
          <span className="text-foreground/70 flex-1 truncate">{e.action}</span>
          {e.toolName && <span className="font-mono text-accent/50 bg-accent/5 px-1 py-0.5 rounded text-[9px]">{e.toolName}</span>}
        </div>
      ))}
    </div>
  );
}

function EmptySecData({ label }: { label: string }) {
  return (
    <div className="text-center text-[10px] text-muted-foreground/20 py-6">
      {label}
    </div>
  );
}
