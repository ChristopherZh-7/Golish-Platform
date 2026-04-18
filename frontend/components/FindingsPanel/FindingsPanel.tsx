import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  AlertTriangle, Bug, Check, ChevronDown, ChevronRight, Download, ExternalLink,
  Filter, Image, Info, Loader2, Merge, Paperclip, Plus, Search, Shield, ShieldAlert, Trash2, X,
} from "lucide-react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { QuickNotes } from "@/components/QuickNotes/QuickNotes";
import { cn } from "@/lib/utils";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";
import { getProjectPath } from "@/lib/projects";

type Severity = "critical" | "high" | "medium" | "low" | "info";
type FindingStatus = "open" | "confirmed" | "falsePositive" | "resolved";

interface Evidence {
  id: string;
  filename: string;
  mime_type: string;
  caption: string;
  added_at: number;
}

interface Finding {
  id: string;
  title: string;
  severity: Severity;
  cvss?: number;
  url: string;
  target: string;
  targetId?: string;
  description: string;
  steps: string;
  remediation: string;
  tags: string[];
  tool: string;
  template: string;
  references: string[];
  evidence: Evidence[];
  status: FindingStatus;
  created_at: number;
  updated_at: number;
}

interface FindingsStore {
  findings: Finding[];
}

const SEVERITY_CONFIG: Record<Severity, { color: string; bg: string; icon: typeof ShieldAlert; label: string }> = {
  critical: { color: "text-red-400", bg: "bg-red-500/10 border-red-500/20", icon: ShieldAlert, label: "Critical" },
  high:     { color: "text-orange-400", bg: "bg-orange-500/10 border-orange-500/20", icon: AlertTriangle, label: "High" },
  medium:   { color: "text-yellow-400", bg: "bg-yellow-500/10 border-yellow-500/20", icon: Shield, label: "Medium" },
  low:      { color: "text-blue-400", bg: "bg-blue-500/10 border-blue-500/20", icon: Info, label: "Low" },
  info:     { color: "text-slate-400", bg: "bg-slate-500/10 border-slate-500/20", icon: Info, label: "Info" },
};

const STATUS_LABELS: Record<FindingStatus, string> = {
  open: "Open",
  confirmed: "Confirmed",
  falsePositive: "False Positive",
  resolved: "Resolved",
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
          "flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border transition-colors",
          "bg-[var(--bg-hover)]/30 border-border/30 text-foreground",
          "hover:bg-[var(--bg-hover)]/60 hover:border-border/50",
          open && "border-accent/40 bg-[var(--bg-hover)]/50",
        )}
        onClick={() => setOpen(!open)}
      >
        <span className="truncate max-w-[90px]">{selected?.label ?? value}</span>
        <ChevronDown className={cn("w-3 h-3 text-muted-foreground transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute top-full left-0 mt-1 min-w-[120px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1">
          {options.map((opt) => (
            <button
              key={opt.value}
              type="button"
              className={cn(
                "w-full text-left px-3 py-1.5 text-[10px] transition-colors",
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

export function FindingsPanel() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [findings, setFindings] = useState<Finding[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [severityFilter, setSeverityFilter] = useState<Severity | "all">("all");
  const [statusFilter, setStatusFilter] = useState<FindingStatus | "all">("all");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [addForm, setAddForm] = useState({ title: "", severity: "medium" as Severity, url: "", description: "" });

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const store = await invoke<FindingsStore>("findings_list", { projectPath: getProjectPath() });
      setFindings(store?.findings || []);
    } catch {
      setFindings([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { load(); }, [load, currentProjectPath]);

  const safeFindings = findings ?? [];
  const filtered = useMemo(() => {
    let list = safeFindings;
    if (severityFilter !== "all") list = list.filter((f) => f.severity === severityFilter);
    if (statusFilter !== "all") list = list.filter((f) => f.status === statusFilter);
    if (search.trim()) {
      const q = search.toLowerCase();
      list = list.filter((f) =>
        f.title.toLowerCase().includes(q) ||
        f.url.toLowerCase().includes(q) ||
        f.tool.toLowerCase().includes(q) ||
        f.template.toLowerCase().includes(q)
      );
    }
    return list.sort((a, b) => {
      const order: Severity[] = ["critical", "high", "medium", "low", "info"];
      return order.indexOf(a.severity) - order.indexOf(b.severity);
    });
  }, [safeFindings, severityFilter, statusFilter, search]);
  const stats = useMemo(() => {
    const s = { critical: 0, high: 0, medium: 0, low: 0, info: 0, total: safeFindings.length };
    for (const f of safeFindings) s[f.severity]++;
    return s;
  }, [safeFindings]);

  const handleAdd = useCallback(async () => {
    if (!addForm.title.trim()) return;
    await invoke("findings_add", {
      finding: {
        id: "",
        title: addForm.title,
        severity: addForm.severity,
        url: addForm.url,
        description: addForm.description,
        steps: "",
        remediation: "",
        tags: [],
        tool: "",
        template: "",
        references: [],
        status: "open",
        created_at: 0,
        updated_at: 0,
      },
      projectPath: getProjectPath(),
    });
    setAddForm({ title: "", severity: "medium", url: "", description: "" });
    setShowAdd(false);
    load();
  }, [addForm, load]);

  const handleDelete = useCallback(async (id: string) => {
    await invoke("findings_delete", { id, projectPath: getProjectPath() });
    load();
    invoke("audit_log", { action: "finding_deleted", category: "findings", details: id, entityType: "finding", entityId: id, projectPath: getProjectPath() }).catch(() => {});
  }, [load]);

  const handleStatusChange = useCallback(async (finding: Finding, status: FindingStatus) => {
    await invoke("findings_update", {
      finding: { ...finding, status },
      projectPath: getProjectPath(),
    });
    load();
    invoke("audit_log", { action: "finding_status_changed", category: "findings", details: `${finding.title} → ${status}`, entityType: "finding", entityId: finding.id, projectPath: getProjectPath() }).catch(() => {});
  }, [load]);

  const handleAddEvidence = useCallback(async (findingId: string) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/*,.pdf,.txt,.json,.xml,.html";
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = async () => {
        const base64 = (reader.result as string).split(",")[1];
        await invoke("findings_add_evidence", {
          findingId,
          filename: file.name,
          mimeType: file.type || "application/octet-stream",
          caption: "",
          dataBase64: base64,
          projectPath: getProjectPath(),
        });
        load();
      };
      reader.readAsDataURL(file);
    };
    input.click();
  }, [load]);

  const handleRemoveEvidence = useCallback(async (findingId: string, evidenceId: string) => {
    await invoke("findings_remove_evidence", {
      findingId,
      evidenceId,
      projectPath: getProjectPath(),
    });
    load();
  }, [load]);

  const [evidencePaths, setEvidencePaths] = useState<Record<string, string>>({});

  const loadEvidencePath = useCallback(async (findingId: string, ev: Evidence) => {
    const key = `${findingId}/${ev.id}`;
    if (evidencePaths[key]) return;
    try {
      const path = await invoke<string>("findings_evidence_path", {
        findingId,
        evidenceId: ev.id,
        projectPath: getProjectPath(),
      });
      setEvidencePaths((prev) => ({ ...prev, [key]: convertFileSrc(path) }));
    } catch { /* ignore */ }
  }, [evidencePaths]);

  const exportFindings = useCallback((format: "json" | "csv") => {
    const data = filtered.length > 0 ? filtered : findings;
    let blob: Blob;
    let filename: string;

    if (format === "json") {
      blob = new Blob([JSON.stringify(data, null, 2)], { type: "application/json" });
      filename = "findings.json";
    } else {
      const headers = ["Title", "Severity", "Status", "Tool", "Target", "Description", "Created"];
      const rows = data.map((f) => [
        `"${(f.title || "").replace(/"/g, '""')}"`,
        f.severity,
        f.status,
        f.tool || "",
        f.target || "",
        `"${(f.description || "").replace(/"/g, '""')}"`,
        f.created_at,
      ]);
      blob = new Blob([headers.join(",") + "\n" + rows.map((r) => r.join(",")).join("\n")], { type: "text/csv" });
      filename = "findings.csv";
    }

    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.click();
    URL.revokeObjectURL(url);
  }, [filtered, findings]);

  const [showExportMenu, setShowExportMenu] = useState(false);

  const handleDedup = useCallback(async () => {
    try {
      const removed = await invoke<number>("findings_deduplicate", { projectPath: getProjectPath() });
      if (removed > 0) {
        load();
      }
    } catch { /* ignore */ }
  }, [load]);

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div className="flex-shrink-0 px-4 py-3 border-b border-border/30">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <Bug className="w-4 h-4 text-accent" />
            <h2 className="text-sm font-semibold">{t("activity.findings", "Findings")}</h2>
            <span className="text-[10px] text-muted-foreground/50 bg-muted/20 px-1.5 py-0.5 rounded-full">
              {stats.total}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <button
              onClick={handleDedup}
              disabled={findings.length < 2}
              title="Merge duplicate findings"
              className="flex items-center gap-1 px-2 py-1 text-[10px] rounded-md text-muted-foreground/50 hover:text-foreground hover:bg-muted/20 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
            >
              <Merge className="w-3 h-3" />
              Dedup
            </button>
            <div className="relative">
              <button
                onClick={() => setShowExportMenu(!showExportMenu)}
                disabled={findings.length === 0}
                className="flex items-center gap-1 px-2 py-1 text-[10px] rounded-md text-muted-foreground/50 hover:text-foreground hover:bg-muted/20 transition-colors disabled:opacity-30 disabled:cursor-not-allowed"
              >
                <Download className="w-3 h-3" />
                Export
              </button>
              {showExportMenu && (
                <div className="absolute right-0 top-full mt-1 bg-[#1e1e2e] border border-border/20 rounded-lg shadow-xl z-50 min-w-[100px] overflow-hidden">
                  <button onClick={() => { exportFindings("json"); setShowExportMenu(false); }}
                    className="w-full px-3 py-1.5 text-[10px] text-left text-foreground/80 hover:bg-accent/10 transition-colors">
                    JSON
                  </button>
                  <button onClick={() => { exportFindings("csv"); setShowExportMenu(false); }}
                    className="w-full px-3 py-1.5 text-[10px] text-left text-foreground/80 hover:bg-accent/10 transition-colors">
                    CSV
                  </button>
                </div>
              )}
            </div>
            <button
              onClick={() => setShowAdd(!showAdd)}
              className="flex items-center gap-1 px-2 py-1 text-[10px] rounded-md bg-accent/10 text-accent hover:bg-accent/20 transition-colors"
            >
              <Plus className="w-3 h-3" />
              Add
            </button>
          </div>
        </div>

        {/* Severity stats */}
        <div className="flex items-center gap-1.5 mb-2.5">
          {(["critical", "high", "medium", "low", "info"] as Severity[]).map((sev) => {
            const cfg = SEVERITY_CONFIG[sev];
            return (
              <button
                key={sev}
                onClick={() => setSeverityFilter(severityFilter === sev ? "all" : sev)}
                className={cn(
                  "flex items-center gap-1 px-2 py-0.5 rounded-md text-[9px] font-medium border transition-colors",
                  severityFilter === sev ? cfg.bg : "border-transparent text-muted-foreground/40 hover:text-muted-foreground/60"
                )}
              >
                <span className={cfg.color}>{stats[sev]}</span>
                <span className={severityFilter === sev ? cfg.color : ""}>{cfg.label}</span>
              </button>
            );
          })}
        </div>

        {/* Search & filters */}
        <div className="flex items-center gap-2">
          <div className="relative flex-1">
            <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3 h-3 text-muted-foreground/30" />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search findings..."
              className="w-full pl-7 pr-2 py-1 text-[11px] rounded-md bg-muted/10 border border-border/10 text-foreground placeholder:text-muted-foreground/20 outline-none focus:border-accent/30"
            />
          </div>
          <MiniDropdown
            value={statusFilter}
            onChange={(v) => setStatusFilter(v as FindingStatus | "all")}
            options={[
              { value: "all", label: "All Status" },
              ...Object.entries(STATUS_LABELS).map(([k, v]) => ({ value: k, label: v })),
            ]}
          />
        </div>
      </div>

      {/* Add form */}
      {showAdd && (
        <div className="flex-shrink-0 px-4 py-3 border-b border-border/30 bg-muted/5 space-y-2">
          <input
            value={addForm.title}
            onChange={(e) => setAddForm((p) => ({ ...p, title: e.target.value }))}
            placeholder="Finding title"
            className="w-full px-2 py-1.5 text-[11px] rounded-md bg-transparent border border-border/20 text-foreground placeholder:text-muted-foreground/20 outline-none"
            autoFocus
          />
          <div className="flex gap-2">
            <MiniDropdown
              value={addForm.severity}
              onChange={(v) => setAddForm((p) => ({ ...p, severity: v as Severity }))}
              options={(["critical", "high", "medium", "low", "info"] as Severity[]).map((s) => ({
                value: s, label: SEVERITY_CONFIG[s].label,
              }))}
            />
            <input
              value={addForm.url}
              onChange={(e) => setAddForm((p) => ({ ...p, url: e.target.value }))}
              placeholder="URL (optional)"
              className="flex-1 px-2 py-1 text-[10px] rounded-md bg-transparent border border-border/20 text-foreground placeholder:text-muted-foreground/20 outline-none"
            />
          </div>
          <textarea
            value={addForm.description}
            onChange={(e) => setAddForm((p) => ({ ...p, description: e.target.value }))}
            placeholder="Description (optional)"
            rows={2}
            className="w-full px-2 py-1.5 text-[10px] rounded-md bg-transparent border border-border/20 text-foreground placeholder:text-muted-foreground/20 outline-none resize-none"
          />
          <div className="flex justify-end gap-2">
            <button onClick={() => setShowAdd(false)}
              className="px-2.5 py-1 text-[10px] rounded-md text-muted-foreground/50 hover:text-foreground transition-colors">
              Cancel
            </button>
            <button onClick={handleAdd} disabled={!addForm.title.trim()}
              className={cn("px-2.5 py-1 text-[10px] rounded-md font-medium transition-colors",
                addForm.title.trim() ? "bg-accent text-accent-foreground hover:bg-accent/90" : "bg-muted/30 text-muted-foreground/30 cursor-not-allowed")}>
              Add Finding
            </button>
          </div>
        </div>
      )}

      {/* Findings list */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center h-32">
            <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-32 gap-2 text-muted-foreground/30">
            <Bug className="w-8 h-8" />
            <p className="text-[11px]">{search || severityFilter !== "all" || statusFilter !== "all" ? "No matching findings" : "No findings yet"}</p>
          </div>
        ) : (
          <div className="divide-y divide-border/10">
            {filtered.map((finding) => {
              const cfg = SEVERITY_CONFIG[finding.severity];
              const isExpanded = expanded === finding.id;
              const SevIcon = cfg.icon;
              return (
                <div key={finding.id} className="group">
                  <button
                    onClick={() => setExpanded(isExpanded ? null : finding.id)}
                    className="w-full flex items-center gap-2 px-4 py-2.5 text-left hover:bg-muted/10 transition-colors"
                  >
                    {isExpanded ? <ChevronDown className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/40 flex-shrink-0" />}
                    <span className={cn("flex-shrink-0", cfg.color)}>
                      <SevIcon className="w-3.5 h-3.5" />
                    </span>
                    <span className="flex-1 text-[11px] font-medium truncate">{finding.title}</span>
                    {finding.tool && (
                      <span className="text-[9px] text-muted-foreground/30 bg-muted/15 px-1.5 py-0.5 rounded flex-shrink-0">
                        {finding.tool}
                      </span>
                    )}
                    <span className={cn(
                      "text-[9px] px-1.5 py-0.5 rounded flex-shrink-0",
                      finding.status === "open" ? "text-yellow-400 bg-yellow-500/10" :
                      finding.status === "confirmed" ? "text-red-400 bg-red-500/10" :
                      finding.status === "resolved" ? "text-emerald-400 bg-emerald-500/10" :
                      "text-muted-foreground/40 bg-muted/10"
                    )}>
                      {STATUS_LABELS[finding.status]}
                    </span>
                  </button>
                  {isExpanded && (
                    <div className="px-4 pb-3 pl-10 space-y-2 animate-in fade-in slide-in-from-top-1 duration-200">
                      {finding.url && (
                        <div className="flex items-center gap-1.5">
                          <ExternalLink className="w-3 h-3 text-muted-foreground/30" />
                          <span className="text-[10px] font-mono text-accent/70 truncate">{finding.url}</span>
                        </div>
                      )}
                      {finding.description && (
                        <p className="text-[10px] text-muted-foreground/60 leading-relaxed">{finding.description}</p>
                      )}
                      {finding.template && (
                        <div className="text-[9px] text-muted-foreground/40">
                          Template: <span className="font-mono">{finding.template}</span>
                        </div>
                      )}
                      {finding.references.length > 0 && (
                        <div className="text-[9px] text-muted-foreground/40">
                          Refs: {finding.references.map((r, i) => (
                            <a key={i} href={r} target="_blank" rel="noopener" className="text-accent/50 hover:text-accent mr-1.5">{r}</a>
                          ))}
                        </div>
                      )}
                      {/* Evidence */}
                      <div className="space-y-1.5">
                        <div className="flex items-center gap-1.5">
                          <Paperclip className="w-3 h-3 text-muted-foreground/30" />
                          <span className="text-[9px] text-muted-foreground/40">
                            Evidence ({finding.evidence?.length || 0})
                          </span>
                          <button
                            onClick={() => handleAddEvidence(finding.id)}
                            className="text-[9px] text-accent/50 hover:text-accent transition-colors ml-1"
                          >
                            + Add
                          </button>
                        </div>
                        {finding.evidence?.length > 0 && (
                          <div className="flex gap-2 flex-wrap">
                            {finding.evidence.map((ev) => {
                              const key = `${finding.id}/${ev.id}`;
                              const src = evidencePaths[key];
                              if (!src && ev.mime_type.startsWith("image/")) {
                                loadEvidencePath(finding.id, ev);
                              }
                              return (
                                <div key={ev.id} className="relative group/ev">
                                  {ev.mime_type.startsWith("image/") ? (
                                    src ? (
                                      <img
                                        src={src}
                                        alt={ev.caption || ev.filename}
                                        className="w-20 h-14 object-cover rounded border border-border/20 cursor-pointer hover:border-accent/40 transition-colors"
                                        onClick={() => window.open(src, "_blank")}
                                      />
                                    ) : (
                                      <div className="w-20 h-14 rounded border border-border/20 flex items-center justify-center bg-muted/10">
                                        <Image className="w-4 h-4 text-muted-foreground/20" />
                                      </div>
                                    )
                                  ) : (
                                    <div className="px-2 py-1 rounded border border-border/20 bg-muted/10 text-[9px] text-muted-foreground/50 max-w-[120px] truncate">
                                      {ev.filename}
                                    </div>
                                  )}
                                  <button
                                    onClick={() => handleRemoveEvidence(finding.id, ev.id)}
                                    className="absolute -top-1 -right-1 p-0.5 rounded-full bg-card border border-border/30 text-muted-foreground/30 hover:text-red-400 opacity-0 group-hover/ev:opacity-100 transition-opacity"
                                  >
                                    <X className="w-2.5 h-2.5" />
                                  </button>
                                </div>
                              );
                            })}
                          </div>
                        )}
                      </div>

                      <div className="pt-1 border-t border-border/10">
                        <QuickNotes entityType="finding" entityId={finding.id} compact />
                      </div>

                      <div className="flex items-center gap-2 pt-1">
                        {(["open", "confirmed", "falsePositive", "resolved"] as FindingStatus[]).map((s) => (
                          <button
                            key={s}
                            onClick={() => handleStatusChange(finding, s)}
                            className={cn(
                              "text-[9px] px-2 py-0.5 rounded transition-colors",
                              finding.status === s
                                ? "bg-accent/15 text-accent"
                                : "text-muted-foreground/30 hover:text-muted-foreground/60 hover:bg-muted/10"
                            )}
                          >
                            {s === "resolved" && <Check className="w-2.5 h-2.5 inline mr-0.5" />}
                            {STATUS_LABELS[s]}
                          </button>
                        ))}
                        <div className="flex-1" />
                        <button
                          onClick={() => handleDelete(finding.id)}
                          className="p-1 text-muted-foreground/20 hover:text-red-400 transition-colors opacity-0 group-hover:opacity-100"
                        >
                          <Trash2 className="w-3 h-3" />
                        </button>
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
