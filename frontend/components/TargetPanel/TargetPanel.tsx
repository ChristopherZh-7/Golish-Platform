import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { QuickNotes } from "@/components/QuickNotes/QuickNotes";
import {
  ChevronDown, Crosshair, Globe, Hash, Map as MapIcon, Network,
  Plus, Search, Server, Shield, ShieldOff, Tag, Trash2, Wifi, X, Zap,
} from "lucide-react";
import { TopologyView } from "@/components/TopologyView/TopologyView";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";

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

type TargetTab = "targets" | "topology";

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

  const loadTargets = useCallback(async () => {
    try {
      const data = await invoke<TargetStore>("target_list", { projectPath: getProjectPath() });
      setStore(data && data.targets ? data : { targets: [] });
    } catch (e) {
      console.error("Failed to load targets:", e);
    }
  }, []);

  useEffect(() => { loadTargets(); }, [loadTargets, currentProjectPath]);

  useEffect(() => {
    const REFRESH_TOOLS = new Set(["manage_targets", "record_finding"]);
    const unlisten = listen<{ type: string; tool_name?: string }>("ai-event", (event) => {
      if (event.payload.type === "tool_result" && event.payload.tool_name && REFRESH_TOOLS.has(event.payload.tool_name)) {
        loadTargets();
      }
    });
    return () => { unlisten.then((fn) => fn()); };
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
            {filtered.map((target) => (
              <div
                key={target.id}
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
                  <div className="mt-2 ml-5 space-y-1.5" onClick={(e) => e.stopPropagation()}>
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

                    {target.technologies && target.technologies.length > 0 && (
                      <div className="flex flex-wrap items-center gap-1">
                        <Zap className="w-3 h-3 text-purple-400 shrink-0" />
                        {target.technologies.map((tech) => (
                          <span key={tech} className="text-[10px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400">
                            {tech}
                          </span>
                        ))}
                      </div>
                    )}

                    {target.ports && target.ports.length > 0 && (
                      <div className="space-y-0.5">
                        <div className="flex items-center gap-1 text-[11px] text-muted-foreground font-medium">
                          <Server className="w-3 h-3" />
                          Open Ports ({target.ports.length})
                        </div>
                        <div className="grid grid-cols-2 gap-x-3 gap-y-0.5 pl-4">
                          {target.ports.slice(0, 20).map((p: PortInfo) => (
                            <div key={`${p.port}-${p.protocol || ""}`} className="text-[10px] font-mono text-muted-foreground">
                              <span className="text-emerald-400">{p.port}</span>
                              {p.protocol && <span className="text-muted-foreground/50">/{p.protocol}</span>}
                              {p.service && <span className="text-foreground/60 ml-1">{p.service}</span>}
                            </div>
                          ))}
                          {target.ports.length > 20 && (
                            <div className="text-[10px] text-muted-foreground/50">+{target.ports.length - 20} more</div>
                          )}
                        </div>
                      </div>
                    )}

                    <textarea
                      className="w-full text-[11px] bg-background border border-border/50 rounded px-2 py-1 outline-none focus:border-accent resize-none"
                      placeholder={t("targets.notes")}
                      rows={2}
                      defaultValue={target.notes}
                      onBlur={(e) => {
                        if (e.target.value !== target.notes) {
                          handleUpdateNotes(target.id, e.target.value);
                        }
                      }}
                    />
                    <QuickNotes entityType="target" entityId={target.id} compact />
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
      </>
      )}
    </div>
  );
}
