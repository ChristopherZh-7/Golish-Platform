import { useCallback, useMemo, useState } from "react";
import {
  ChevronDown, Crosshair, Globe, Hash,
  Network, Plus, Search, Shield, ShieldOff, Tag, Trash2, Wifi, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getRootDomain } from "@/lib/domain";
import { MiniDropdown } from "@/components/ui/MiniDropdown";
import { TargetDetailView } from "./TargetDetail";
import { type Target, type TargetStatus } from "@/lib/pentest/types";

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

interface TargetListViewProps {
  targets: Target[];
  stats: { total: number; inScope: number; outOfScope: number };
  t: (key: string) => string;
  onAdd: (form: { name: string; value: string; notes: string; tags: string }) => Promise<string | null>;
  onBatchAdd: (input: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onToggleScope: (target: Target) => Promise<void>;
  onUpdateNotes: (id: string, notes: string) => void;
  onClearAll: (confirmMsg: string) => Promise<void>;
  onScan: (target: Target) => void;
}

export function TargetListView({
  targets, stats, t, onAdd, onBatchAdd, onDelete,
  onToggleScope, onUpdateNotes, onClearAll, onScan,
}: TargetListViewProps) {
  const [search, setSearch] = useState("");
  const [scopeFilter, setScopeFilter] = useState<"all" | "in" | "out">("all");
  const [showAdd, setShowAdd] = useState(false);
  const [showBatch, setShowBatch] = useState(false);
  const [batchInput, setBatchInput] = useState("");
  const [addForm, setAddForm] = useState({ name: "", value: "", notes: "", tags: "" });
  const [addError, setAddError] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [expandedParents, setExpandedParents] = useState<Set<string>>(new Set());
  const [expandedDomains, setExpandedDomains] = useState<Set<string>>(new Set());

  const filtered = useMemo(() => {
    let list = targets;
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
  }, [targets, search, scopeFilter]);

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

  const domainGroups = useMemo(() => {
    const map = new Map<string, Target[]>();
    for (const t of rootTargets) {
      const domain = getRootDomain(t.value);
      const arr = map.get(domain) || [];
      arr.push(t);
      map.set(domain, arr);
    }
    return [...map.entries()]
      .sort((a, b) => {
        const aIn = a[1].some((t) => t.scope === "in");
        const bIn = b[1].some((t) => t.scope === "in");
        if (aIn !== bIn) return aIn ? -1 : 1;
        return b[1].length - a[1].length;
      });
  }, [rootTargets]);

  const handleAdd = useCallback(async () => {
    if (!addForm.value.trim()) return;
    setAddError(null);
    const err = await onAdd(addForm);
    if (err) {
      setAddError(err);
    } else {
      setAddForm({ name: "", value: "", notes: "", tags: "" });
      setShowAdd(false);
    }
  }, [addForm, onAdd]);

  const handleBatchAdd = useCallback(async () => {
    if (!batchInput.trim()) return;
    await onBatchAdd(batchInput);
    setBatchInput("");
    setShowBatch(false);
  }, [batchInput, onBatchAdd]);

  return (
    <>
      {/* Filter bar + actions */}
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
          variant="standard"
          buttonClassName="py-1.5 text-xs"
          value={scopeFilter}
          onChange={(v) => setScopeFilter(v as "all" | "in" | "out")}
          options={[
            { value: "all", label: t("targets.all") },
            { value: "in", label: t("targets.inScope") },
            { value: "out", label: t("targets.outOfScope") },
          ]}
        />
        <div className="flex items-center gap-0.5 ml-1">
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
              onClick={() => onClearAll(t("targets.clearConfirm"))}
              title={t("targets.clearAll")}
            >
              <Trash2 className="w-3.5 h-3.5" />
            </button>
          )}
        </div>
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
            {domainGroups.map(([domain, domTargets]) => {
              const isDomainExpanded = expandedDomains.has(domain);
              const inCount = domTargets.filter((t) => t.scope === "in").length;
              const outCount = domTargets.length - inCount;
              const allChildCount = domTargets.reduce((acc, t) => acc + (childrenMap.get(t.id)?.length || 0), 0);
              const isSingleFlat = domTargets.length === 1 && getRootDomain(domTargets[0].value) === domain;
              return (
              <div key={domain}>
                {!isSingleFlat && (
                <button
                  type="button"
                  className="flex items-center gap-2 w-full px-4 py-2 hover:bg-muted/20 transition-colors text-left"
                  onClick={() => setExpandedDomains((prev) => {
                    const next = new Set(prev);
                    if (next.has(domain)) next.delete(domain); else next.add(domain);
                    return next;
                  })}
                >
                  <ChevronDown className={cn("w-3 h-3 text-muted-foreground/50 transition-transform", !isDomainExpanded && "-rotate-90")} />
                  <Globe className="w-3.5 h-3.5 text-blue-400" />
                  <span className="text-xs font-medium text-foreground">{domain}</span>
                  <span className="text-[10px] text-muted-foreground/50 tabular-nums">{domTargets.length}</span>
                  {inCount > 0 && (
                    <span className="text-[9px] px-1.5 py-0.5 rounded bg-green-500/10 text-green-400">{inCount} in</span>
                  )}
                  {outCount > 0 && (
                    <span className="text-[9px] px-1.5 py-0.5 rounded bg-red-500/10 text-red-400">{outCount} out</span>
                  )}
                  {allChildCount > 0 && (
                    <span className="text-[9px] px-1.5 py-0.5 rounded bg-accent/10 text-accent/70">{allChildCount} sub</span>
                  )}
                </button>
                )}

                {(isSingleFlat || isDomainExpanded) && domTargets.map((target) => {
                  const children = childrenMap.get(target.id) || [];
                  const hasChildren = children.length > 0;
                  const isParentExpanded = expandedParents.has(target.id);
                  return (
                  <div key={target.id} className={isSingleFlat ? "" : "border-l-2 border-blue-400/20 ml-4"}>
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
                        onClick={(e) => { e.stopPropagation(); onToggleScope(target); }}
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
                        onClick={(e) => { e.stopPropagation(); onDelete(target.id); }}
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
                      <TargetDetailView target={target} t={t} onUpdateNotes={onUpdateNotes} onScan={onScan} />
                    )}
                  </div>

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
              );
            })}
          </div>
        )}
      </div>
    </>
  );
}
