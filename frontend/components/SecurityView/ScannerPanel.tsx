import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Check, ChevronDown, ChevronRight, Crosshair, Globe, KeyRound,
  List, Loader2, Pause, Play, Plus, Square,
  ShieldAlert, ShieldCheck, ShieldX, Trash2, X, Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import type { ZapAlert } from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";
import { useZapScanQueue } from "./hooks/useZapScanQueue";
import type { ScanEndpoint } from "@/lib/pentest/scan-queue";

interface VaultEntrySafe {
  id: string;
  name: string;
  entry_type: string;
  username: string;
  notes: string;
  project: string;
  tags: string[];
  created_at: string;
}

function PolicyDropdown({ value, onChange, options }: { value: string; onChange: (v: string) => void; options: { value: string; label: string }[] }) {
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
          "bg-[var(--bg-hover)]/20 border-border/20 text-muted-foreground/60",
          "hover:bg-[var(--bg-hover)]/40 hover:border-border/40 hover:text-muted-foreground/80",
          open && "border-accent/30 bg-[var(--bg-hover)]/30",
        )}
        onClick={() => setOpen(!open)}
      >
        <ShieldAlert className="w-3 h-3 flex-shrink-0" />
        <span className="truncate max-w-[120px]">{selected?.label ?? value}</span>
        <ChevronDown className={cn("w-2.5 h-2.5 text-muted-foreground/40 transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute top-full left-0 mt-1 min-w-[180px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1">
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

function CredentialDropdown({ value, onChange, open, onToggle, entries }: {
  value: string;
  onChange: (v: string) => void;
  open: boolean;
  onToggle: () => void;
  entries: VaultEntrySafe[];
}) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);
  const selected = entries.find((e) => e.id === value);
  const filtered = entries.filter((e) => ["password", "token", "cookie", "apiKey"].includes(e.entry_type));

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onToggle();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open, onToggle]);

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        className={cn(
          "flex items-center gap-1.5 px-2 py-1 text-[10px] rounded-md border transition-colors",
          "bg-[var(--bg-hover)]/20 border-border/20 text-muted-foreground/60",
          "hover:bg-[var(--bg-hover)]/40 hover:border-border/40 hover:text-muted-foreground/80",
          open && "border-accent/30 bg-[var(--bg-hover)]/30",
        )}
        onClick={onToggle}
      >
        <KeyRound className="w-3 h-3 flex-shrink-0" />
        <span className="truncate max-w-[140px]">{selected ? selected.name : t("security.noCredential", "No credential (unauthenticated)")}</span>
        <ChevronDown className={cn("w-2.5 h-2.5 text-muted-foreground/40 transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute top-full left-0 mt-1 min-w-[200px] max-h-48 overflow-y-auto rounded-lg border border-border/30 bg-card shadow-lg z-50 py-1">
          <button
            type="button"
            className={cn(
              "w-full text-left px-3 py-1.5 text-[10px] transition-colors hover:bg-[var(--bg-hover)]/60",
              !value && "text-accent bg-accent/5 font-medium",
            )}
            onClick={() => { onChange(""); onToggle(); }}
          >
            {t("security.noCredential", "No credential (unauthenticated)")}
          </button>
          {filtered.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className={cn(
                "w-full text-left px-3 py-1.5 text-[10px] transition-colors hover:bg-[var(--bg-hover)]/60 flex items-center gap-2",
                value === entry.id && "text-accent bg-accent/5 font-medium",
              )}
              onClick={() => { onChange(entry.id); onToggle(); }}
            >
              <KeyRound className="w-2.5 h-2.5 flex-shrink-0" />
              <span className="truncate">{entry.name}</span>
              <span className="text-muted-foreground/40 text-[8px] ml-auto">{entry.entry_type}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function ScannerPanel({ initialUrl, initialBatchUrls, onUrlConsumed }: { initialUrl?: string | null; initialBatchUrls?: string[]; onUrlConsumed?: () => void }) {
  const { t } = useTranslation();
  const projectPath = useStore((s) => s.currentProjectPath);
  const [targetUrl, setTargetUrl] = useState("");
  const [vaultEntries, setVaultEntries] = useState<VaultEntrySafe[]>([]);
  const [selectedCredential, setSelectedCredential] = useState<string>("");
  const [showCredSelector, setShowCredSelector] = useState(false);
  const [scanPolicies, setScanPolicies] = useState<string[]>([]);
  const [selectedPolicy, setSelectedPolicy] = useState<string>("");
  const [scanLogs, setScanLogs] = useState<any[]>([]);
  const [showPlugins, setShowPlugins] = useState(false);
  const [scannerRules, setScannerRules] = useState<{ id: string; name: string; enabled: boolean; quality: string }[]>([]);

  // Load vault entries + scan policies once per project (the scan queue itself is
  // owned by `useZapScanQueue` below).
  useEffect(() => {
    const pp = getProjectPath();
    Promise.all([
      invoke<VaultEntrySafe[]>("vault_list", { projectPath: pp }).catch(() => []),
      invoke<string[]>("zap_list_scan_policies").catch(() => []),
    ]).then(([v, policies]) => {
      setVaultEntries(Array.isArray(v) ? v : []);
      if (Array.isArray(policies)) setScanPolicies(policies);
    });
  }, [projectPath]);

  // Inject vault credentials into ZAP before each scan (best-effort).
  const applyCredential = useCallback(async () => {
    if (!selectedCredential) return;
    try {
      const value = await invoke<string>("vault_get_value", { id: selectedCredential, projectPath: getProjectPath() });
      const entry = vaultEntries.find((e) => e.id === selectedCredential);
      if (entry && value) {
        if (entry.entry_type === "token" || entry.entry_type === "apiKey") {
          await invoke("zap_api_call", {
            component: "replacer", actionType: "action", method: "addRule",
            params: { description: `vault-auth-${entry.name}`, enabled: "true", matchType: "REQ_HEADER", matchRegex: "false", matchString: "Authorization", replacement: `Bearer ${value}` },
          }).catch(() => {});
        } else if (entry.entry_type === "cookie") {
          await invoke("zap_api_call", {
            component: "replacer", actionType: "action", method: "addRule",
            params: { description: `vault-cookie-${entry.name}`, enabled: "true", matchType: "REQ_HEADER", matchRegex: "false", matchString: "Cookie", replacement: value },
          }).catch(() => {});
        }
      }
    } catch { /* continue without credentials */ }
  }, [selectedCredential, vaultEntries]);

  const queue = useZapScanQueue({
    projectPath,
    initialUrl,
    initialBatchUrls,
    onUrlConsumed,
    selectedPolicy,
    beforeScan: applyCredential,
  });
  const {
    endpoints,
    selectedUrl,
    setSelectedUrl,
    selectedEndpoint: sel,
    scanning,
    totalAlerts,
    completedCount,
    scanningCount,
    pausedCount,
    queuedCount,
    addEndpoint,
    removeEndpoint,
    scanSelected: handleScanSelected,
    scanAll: handleScanAll,
    stopAll: handleStopAll,
    pauseAll: handlePauseAll,
    resumeAll: handleResumeAll,
    clearCompleted: handleClearCompleted,
    clearAll: handleClearAll,
  } = queue;

  const handleAddEndpoint = useCallback(() => {
    if (addEndpoint(targetUrl)) {
      setTargetUrl("");
    } else if (targetUrl.trim()) {
      // URL already existed — clear input but keep it focused on the existing entry.
      setTargetUrl("");
    }
  }, [addEndpoint, targetUrl]);

  const handleRemoveEndpoint = useCallback(
    (url: string) => removeEndpoint(url),
    [removeEndpoint]
  );

  useEffect(() => {
    if (!sel || sel.status !== "complete") { setScanLogs([]); return; }
    invoke("passive_scans_by_url", { url: sel.url, limit: 500 })
      .then((data: any) => setScanLogs(Array.isArray(data) ? data : []))
      .catch(() => setScanLogs([]));
  }, [sel?.url, sel?.status]);

  const riskColor = (risk: string) => {
    const c: Record<string, string> = { High: "text-red-400 bg-red-500/10", Medium: "text-orange-400 bg-orange-500/10", Low: "text-yellow-400 bg-yellow-500/10", Informational: "text-blue-400 bg-blue-500/10" };
    return c[risk] || "text-muted-foreground bg-muted/20";
  };

  const riskIcon = (risk: string) => {
    if (risk === "High") return <ShieldX className="w-3 h-3" />;
    if (risk === "Medium") return <ShieldAlert className="w-3 h-3" />;
    return <ShieldCheck className="w-3 h-3" />;
  };

  const statusBadge = (s: ScanEndpoint["status"]) => {
    const map: Record<string, { label: string; cls: string }> = {
      queued: { label: t("security.scanQueued", "Queued"), cls: "text-zinc-400 bg-zinc-500/10" },
      spidering: { label: t("security.spidering", "Crawling"), cls: "text-blue-400 bg-blue-500/10" },
      scanning: { label: t("security.scanning"), cls: "text-orange-400 bg-orange-500/10" },
      paused: { label: "Paused", cls: "text-amber-400 bg-amber-500/10" },
      complete: { label: t("security.scanComplete"), cls: "text-green-400 bg-green-500/10" },
      error: { label: t("common.error"), cls: "text-red-400 bg-red-500/10" },
    };
    const m = map[s] || map.queued;
    return <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full font-medium", m.cls)}>{m.label}</span>;
  };

  return (
    <div className="h-full flex flex-col">
      {/* Add endpoint bar */}
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <Globe className="w-3.5 h-3.5 text-muted-foreground/30 flex-shrink-0" />
        <input
          value={targetUrl}
          onChange={(e) => setTargetUrl(e.target.value)}
          placeholder={t("security.scanTargetPlaceholder")}
          onKeyDown={(e) => e.key === "Enter" && handleAddEndpoint()}
          className="flex-1 h-8 px-3 text-[12px] font-mono bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
        />
        <button type="button" onClick={handleAddEndpoint} disabled={!targetUrl.trim()}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-30">
          <Plus className="w-3 h-3" /> {t("security.addTarget", "Add")}
        </button>
        {queuedCount > 0 && (
          <button type="button" onClick={handleScanAll} disabled={scanning}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-orange-500/10 text-orange-400 hover:bg-orange-500/20 transition-colors disabled:opacity-30">
            {scanning ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
            {t("security.scanAll", "Scan All")} ({queuedCount})
          </button>
        )}
        {scanningCount > 0 && pausedCount === 0 && (
          <button type="button" onClick={handlePauseAll}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-amber-500/10 text-amber-400 hover:bg-amber-500/20 transition-colors">
            <Pause className="w-3 h-3" /> Pause
          </button>
        )}
        {pausedCount > 0 && (
          <button type="button" onClick={handleResumeAll}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-green-500/10 text-green-400 hover:bg-green-500/20 transition-colors">
            <Play className="w-3 h-3" /> Resume
          </button>
        )}
        {scanningCount > 0 && (
          <button type="button" onClick={handleStopAll}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">
            <Square className="w-3 h-3" /> {t("security.stopScan")}
          </button>
        )}
      </div>

      {/* Credential & policy selector */}
      <div className="flex items-center gap-2 px-4 py-1.5 border-b border-border/10 flex-shrink-0">
        <CredentialDropdown
          value={selectedCredential}
          onChange={setSelectedCredential}
          open={showCredSelector}
          onToggle={() => setShowCredSelector(!showCredSelector)}
          entries={vaultEntries}
        />
        <span className="w-px h-3 bg-border/20" />
        <PolicyDropdown
          value={selectedPolicy}
          onChange={setSelectedPolicy}
          options={[
            { value: "", label: t("security.defaultPolicy", "Default Policy (all)") },
            ...scanPolicies.map((p) => ({ value: p, label: p })),
          ]}
        />
        <button
          className={cn("p-1 rounded text-muted-foreground/30 hover:text-muted-foreground/60 transition-colors", showPlugins && "text-accent/60 bg-accent/10")}
          onClick={() => {
            if (!showPlugins && scannerRules.length === 0) {
              invoke<{ id: string; name: string; enabled: boolean; quality: string }[]>("zap_get_scanners")
                .then((r) => setScannerRules(Array.isArray(r) ? r : []))
                .catch(() => {});
            }
            setShowPlugins(!showPlugins);
          }}
          title={t("security.configurePlugins", "Configure scan plugins")}
        >
          <Crosshair className="w-3 h-3" />
        </button>
        <div className="flex-1" />
        <span className="text-[9px] text-muted-foreground/30">
          {endpoints.length} {t("security.endpoints")} · {totalAlerts} {t("security.alertsTotal")}
        </span>
        {completedCount > 0 && (
          <button type="button" onClick={handleClearCompleted}
            className="text-[9px] text-muted-foreground/30 hover:text-foreground transition-colors">
            {t("security.clearCompleted", "Clear Done")}
          </button>
        )}
        {endpoints.length > 0 && (
          <button type="button" onClick={handleClearAll}
            className="flex items-center gap-1 text-[9px] text-muted-foreground/30 hover:text-destructive transition-colors">
            <Trash2 className="w-2.5 h-2.5" /> {t("security.clearAll", "Clear All")}
          </button>
        )}
      </div>
      {showPlugins && (
        <div className="border-b border-border/10 bg-muted/5 max-h-[300px] flex flex-col">
          <div className="flex items-center gap-2 px-4 py-1.5 border-b border-border/5 flex-shrink-0">
            <span className="text-[10px] font-medium text-foreground/50">{t("security.scanPlugins", "Scan Plugins")}</span>
            <span className="text-[9px] text-muted-foreground/30">
              {scannerRules.filter((r) => r.enabled).length}/{scannerRules.length} {t("security.enabled", "enabled")}
            </span>
            <div className="flex-1" />
            <button className="text-[9px] text-accent/50 hover:text-accent/80 transition-colors"
              onClick={() => {
                const allEnabled = scannerRules.every((r) => r.enabled);
                const ids = scannerRules.map((r) => r.id);
                invoke("zap_set_scanners_enabled", { ids, enabled: !allEnabled }).then(() => {
                  setScannerRules((prev) => prev.map((r) => ({ ...r, enabled: !allEnabled })));
                }).catch(() => {});
              }}>
              {scannerRules.every((r) => r.enabled) ? t("security.disableAll", "Disable All") : t("security.enableAll", "Enable All")}
            </button>
          </div>
          <div className="overflow-y-auto flex-1">
            {scannerRules.length === 0 ? (
              <div className="flex items-center justify-center py-6 text-muted-foreground/20 text-[11px]">
                <Loader2 className="w-4 h-4 animate-spin mr-2" /> {t("security.loadingPlugins", "Loading plugins...")}
              </div>
            ) : (
              <div className="divide-y divide-border/5">
                {scannerRules.map((rule) => (
                  <div key={rule.id} className="flex items-center gap-2 px-4 py-1 hover:bg-[var(--bg-hover)]/20 transition-colors">
                    <button
                      className={cn("w-3.5 h-3.5 rounded border flex items-center justify-center transition-colors flex-shrink-0",
                        rule.enabled ? "bg-accent/20 border-accent/40 text-accent" : "border-border/30 text-transparent"
                      )}
                      onClick={() => {
                        invoke("zap_set_scanners_enabled", { ids: [rule.id], enabled: !rule.enabled }).then(() => {
                          setScannerRules((prev) => prev.map((r) => r.id === rule.id ? { ...r, enabled: !r.enabled } : r));
                        }).catch(() => {});
                      }}
                    >
                      {rule.enabled && <Check className="w-2.5 h-2.5" />}
                    </button>
                    <span className="text-[10px] text-foreground/60 flex-1 truncate">{rule.name}</span>
                    <span className="text-[8px] text-muted-foreground/25 font-mono">{rule.id}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}

      {/* Main content: split view */}
      <div className="flex-1 flex overflow-hidden min-h-0">
        {/* Left: endpoint queue */}
        <div className="w-[300px] flex-shrink-0 border-r border-border/10 flex flex-col overflow-hidden">
          <div className="flex-1 overflow-y-auto">
            {endpoints.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
                <Zap className="w-10 h-10" />
                <p className="text-[12px] font-medium">{t("security.noScanTargets", "No scan targets")}</p>
                <p className="text-[10px] text-muted-foreground/15 max-w-[220px] text-center">
                  {t("security.addTargetsHint", "Add URLs above or right-click endpoints in HTTP History / Site Map → Active Scan")}
                </p>
              </div>
            ) : (
              <div className="divide-y divide-border/5">
                {[...endpoints].sort((a, b) => b.addedAt - a.addedAt).map((ep) => {
                  const alertsByRisk = { High: 0, Medium: 0, Low: 0, Info: 0 };
                  for (const a of ep.alerts) {
                    if (a.risk === "High") alertsByRisk.High++;
                    else if (a.risk === "Medium") alertsByRisk.Medium++;
                    else if (a.risk === "Low") alertsByRisk.Low++;
                    else alertsByRisk.Info++;
                  }
                  let host: string; let path: string;
                  try { const u = new URL(ep.url); host = u.host; path = u.pathname; } catch { host = ep.url; path = ""; }

                  return (
                    <div
                      key={ep.url}
                      onClick={() => setSelectedUrl(ep.url)}
                      className={cn("px-3 py-2.5 cursor-pointer transition-colors group", selectedUrl === ep.url ? "bg-accent/8" : "hover:bg-[var(--bg-hover)]/30")}
                    >
                      <div className="flex items-center gap-2 mb-1">
                        {statusBadge(ep.status)}
                        {(ep.status === "scanning" || ep.status === "spidering" || ep.status === "paused") && (
                          <>
                            <span className="text-[9px] text-muted-foreground/40">{ep.progress}%</span>
                            {ep.messageCount != null && ep.messageCount > 0 && (
                              <span className="text-[8px] text-muted-foreground/25">{ep.messageCount} req</span>
                            )}
                          </>
                        )}
                        <div className="flex-1" />
                        <button type="button" onClick={(e) => { e.stopPropagation(); handleRemoveEndpoint(ep.url); }}
                          className="p-0.5 rounded text-muted-foreground/0 group-hover:text-muted-foreground/30 hover:!text-destructive transition-colors">
                          <X className="w-3 h-3" />
                        </button>
                      </div>
                      <div className="text-[11px] font-mono text-foreground/70 truncate">{host}</div>
                      {path && path !== "/" && <div className="text-[10px] font-mono text-muted-foreground/30 truncate">{path}</div>}
                      {(ep.status === "scanning" || ep.status === "spidering" || ep.status === "paused") && (
                        <div className="h-1 rounded-full bg-muted/20 overflow-hidden mt-1.5">
                          <div className={cn("h-full rounded-full transition-all duration-500", ep.status === "paused" ? "bg-amber-500/60" : "bg-accent")} style={{ width: `${ep.progress}%` }} />
                        </div>
                      )}
                      {ep.alerts.length > 0 && (
                        <div className="flex items-center gap-2 mt-1.5">
                          {alertsByRisk.High > 0 && <span className="text-[8px] text-red-400 font-medium">{alertsByRisk.High}H</span>}
                          {alertsByRisk.Medium > 0 && <span className="text-[8px] text-orange-400 font-medium">{alertsByRisk.Medium}M</span>}
                          {alertsByRisk.Low > 0 && <span className="text-[8px] text-yellow-400 font-medium">{alertsByRisk.Low}L</span>}
                          {alertsByRisk.Info > 0 && <span className="text-[8px] text-blue-400 font-medium">{alertsByRisk.Info}I</span>}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>

        {/* Right: selected endpoint detail */}
        <div className="flex-1 overflow-hidden flex flex-col">
          {sel ? (
            <>
              <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
                <Globe className="w-3.5 h-3.5 text-blue-400 flex-shrink-0" />
                <span className="text-[12px] font-mono text-foreground/80 truncate flex-1">{sel.url}</span>
                {statusBadge(sel.status)}
                {sel.status === "queued" && (
                  <button type="button" onClick={handleScanSelected}
                    className="flex items-center gap-1 px-2.5 py-1 rounded-lg text-[10px] font-medium bg-orange-500/10 text-orange-400 hover:bg-orange-500/20 transition-colors">
                    <Zap className="w-3 h-3" /> {t("security.activeScan")}
                  </button>
                )}
              </div>
              {sel.status === "complete" && (sel.alerts.length > 0 || scanLogs.length > 0) ? (
                <ScanDetailTabs alerts={sel.alerts} scanLogs={scanLogs} riskColor={riskColor} riskIcon={riskIcon} />
              ) : sel.alerts.length > 0 ? (
                <div className="flex-1 overflow-y-auto px-4 py-3 space-y-2">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-[11px] font-medium text-foreground/60">{sel.alerts.length} {t("security.alertsTotal")}</span>
                    {(() => {
                      const vulnTypes = new Set(sel.alerts.map((a) => a.name));
                      return <span className="text-[10px] text-muted-foreground/30">{vulnTypes.size} {t("security.uniqueVulnTypes")}</span>;
                    })()}
                  </div>
                  {sel.alerts.map((alert) => (
                    <AlertCard key={`${alert.id}-${alert.url}`} alert={alert} riskColor={riskColor} riskIcon={riskIcon} />
                  ))}
                </div>
              ) : sel.status === "scanning" || sel.status === "spidering" ? (
                <div className="flex-1 flex flex-col items-center justify-center gap-4 text-muted-foreground/30">
                  <Loader2 className="w-8 h-8 animate-spin text-orange-400/40" />
                  <div className="text-center space-y-1.5">
                    <p className="text-[13px] font-medium text-foreground/50">{t("security.scanInProgress")}</p>
                    <div className="flex items-center gap-3 justify-center">
                      <span className="text-[11px] text-orange-400/60 font-mono">{sel.progress}%</span>
                      {sel.messageCount != null && sel.messageCount > 0 && (
                        <span className="text-[11px] text-muted-foreground/40">{sel.messageCount} {t("security.requestsSent", "requests sent")}</span>
                      )}
                      {sel.alerts.length > 0 && (
                        <span className="text-[11px] text-red-400/60">{sel.alerts.length} alerts</span>
                      )}
                    </div>
                  </div>
                </div>
              ) : (
                <div className="flex-1 flex flex-col items-center justify-center gap-3 text-muted-foreground/20">
                  <ShieldCheck className="w-12 h-12" />
                  <p className="text-[13px] font-medium">
                    {sel.status === "complete" ? t("security.scanNoResults") : t("security.scanHint")}
                  </p>
                </div>
              )}
            </>
          ) : (
            <div className="h-full flex flex-col items-center justify-center gap-3 text-muted-foreground/20">
              <ShieldCheck className="w-12 h-12" />
              <p className="text-[13px] font-medium">
                {endpoints.length > 0 ? t("security.selectEndpoint", "Select an endpoint to view results") : t("security.scanHint")}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ScanDetailTabs({ alerts, scanLogs, riskColor, riskIcon }: {
  alerts: ZapAlert[];
  scanLogs: any[];
  riskColor: (r: string) => string;
  riskIcon: (r: string) => React.ReactNode;
}) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<"alerts" | "tests">(alerts.length > 0 ? "alerts" : "tests");
  const vulnCount = scanLogs.filter((l) => l.result === "vulnerable" || l.result === "potential").length;
  const testTypes = useMemo(() => {
    const map = new Map<string, number>();
    for (const l of scanLogs) {
      map.set(l.test_type || "unknown", (map.get(l.test_type || "unknown") || 0) + 1);
    }
    return [...map.entries()].sort((a, b) => b[1] - a[1]);
  }, [scanLogs]);

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="flex items-center gap-1 px-4 py-1.5 border-b border-border/10 flex-shrink-0">
        <button
          className={cn("px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors", tab === "alerts" ? "bg-accent/10 text-accent" : "text-muted-foreground/40 hover:text-muted-foreground/70")}
          onClick={() => setTab("alerts")}
        >
          {t("security.alertsTab", "Alerts")} ({alerts.length})
        </button>
        <button
          className={cn("px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors", tab === "tests" ? "bg-accent/10 text-accent" : "text-muted-foreground/40 hover:text-muted-foreground/70")}
          onClick={() => setTab("tests")}
        >
          {t("security.testsTab", "Test Details")} ({scanLogs.length})
        </button>
        {tab === "tests" && vulnCount > 0 && (
          <span className="text-[9px] text-red-400 font-medium ml-1">{vulnCount} vulnerable</span>
        )}
      </div>
      <div className="flex-1 overflow-y-auto">
        {tab === "alerts" ? (
          alerts.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/20">
              <ShieldCheck className="w-10 h-10" />
              <p className="text-[12px]">{t("security.scanNoResults")}</p>
            </div>
          ) : (
            <div className="px-4 py-3 space-y-2">
              <div className="flex items-center gap-2 mb-2">
                <span className="text-[11px] font-medium text-foreground/60">{alerts.length} {t("security.alertsTotal")}</span>
                {(() => {
                  const vulnTypes = new Set(alerts.map((a) => a.name));
                  return <span className="text-[10px] text-muted-foreground/30">{vulnTypes.size} {t("security.uniqueVulnTypes")}</span>;
                })()}
              </div>
              {alerts.map((alert) => (
                <AlertCard key={`${alert.id}-${alert.url}`} alert={alert} riskColor={riskColor} riskIcon={riskIcon} />
              ))}
            </div>
          )
        ) : (
          scanLogs.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/20">
              <List className="w-10 h-10" />
              <p className="text-[12px]">{t("security.noTestLogs", "No test logs available")}</p>
            </div>
          ) : (
            <div className="px-4 py-3 space-y-1">
              {testTypes.length > 1 && (
                <div className="flex flex-wrap gap-1.5 mb-3">
                  {testTypes.map(([type, count]) => (
                    <span key={type} className="text-[9px] px-1.5 py-0.5 rounded-full bg-muted/20 text-muted-foreground/50">
                      {type} ({count})
                    </span>
                  ))}
                </div>
              )}
              <div className="space-y-0.5">
                {scanLogs.map((log) => (
                  <ScanLogRow key={log.id} log={log} />
                ))}
              </div>
            </div>
          )
        )}
      </div>
    </div>
  );
}

function ScanLogRow({ log }: { log: any }) {
  const [expanded, setExpanded] = useState(false);
  const isVuln = log.result === "vulnerable" || log.result === "potential";
  const detail = typeof log.detail === "string" ? JSON.parse(log.detail || "{}") : (log.detail || {});

  return (
    <div className={cn("rounded-lg border transition-colors", isVuln ? "border-red-500/20 bg-red-500/5" : "border-border/5 bg-transparent hover:bg-[var(--bg-hover)]/20")}>
      <button type="button" onClick={() => setExpanded(!expanded)} className="w-full flex items-center gap-2 px-3 py-1.5 text-left">
        <span className={cn("w-1.5 h-1.5 rounded-full flex-shrink-0", isVuln ? "bg-red-400" : "bg-muted-foreground/15")} />
        <span className="text-[10px] font-medium text-muted-foreground/50 w-[80px] flex-shrink-0 truncate">{log.test_type || "unknown"}</span>
        <span className="text-[10px] font-mono text-foreground/50 flex-1 truncate">{log.parameter || "-"}</span>
        <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", isVuln ? "bg-red-500/10 text-red-400" : "text-muted-foreground/30 bg-muted/10")}>
          {log.result}
        </span>
        {detail.status_code && (
          <span className="text-[9px] text-muted-foreground/30 font-mono">{detail.status_code}</span>
        )}
        {detail.response_time_ms != null && (
          <span className="text-[9px] text-muted-foreground/20 font-mono w-[40px] text-right">{detail.response_time_ms}ms</span>
        )}
        {expanded ? <ChevronDown className="w-2.5 h-2.5 text-muted-foreground/20 flex-shrink-0" /> : <ChevronRight className="w-2.5 h-2.5 text-muted-foreground/20 flex-shrink-0" />}
      </button>
      {expanded && (
        <div className="px-3 pb-2 space-y-1.5 border-t border-border/5">
          {log.payload && (
            <div className="mt-1.5">
              <span className="text-[9px] text-muted-foreground/40 font-medium">Payload</span>
              <pre className="text-[10px] font-mono text-foreground/50 bg-[var(--bg-hover)]/30 p-2 rounded-lg overflow-x-auto mt-0.5 whitespace-pre-wrap break-all">{log.payload}</pre>
            </div>
          )}
          {log.url && (
            <div>
              <span className="text-[9px] text-muted-foreground/40 font-medium">URL</span>
              <p className="text-[10px] font-mono text-foreground/40 truncate">{log.url}</p>
            </div>
          )}
          {log.evidence && (
            <div>
              <span className="text-[9px] text-muted-foreground/40 font-medium">Evidence</span>
              <pre className="text-[10px] font-mono text-foreground/50 bg-[var(--bg-hover)]/30 p-2 rounded-lg overflow-x-auto mt-0.5 whitespace-pre-wrap break-all">{log.evidence}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function AlertCard({
  alert,
  riskColor,
  riskIcon,
}: {
  alert: ZapAlert;
  riskColor: (r: string) => string;
  riskIcon: (r: string) => React.ReactNode;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="rounded-xl border border-border/10 bg-[var(--bg-hover)]/15 overflow-hidden">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-start gap-3 p-3 text-left hover:bg-[var(--bg-hover)]/30 transition-colors"
      >
        <span className={cn("p-1 rounded-md flex-shrink-0 mt-0.5", riskColor(alert.risk))}>
          {riskIcon(alert.risk)}
        </span>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[12px] font-medium text-foreground">{alert.name}</span>
            <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(alert.risk))}>
              {alert.risk}
            </span>
          </div>
          <p className="text-[10px] text-muted-foreground/40 truncate mt-0.5 font-mono">
            {alert.method} {alert.url}
          </p>
        </div>
        {expanded ? <ChevronDown className="w-3 h-3 text-muted-foreground/30 mt-1" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/30 mt-1" />}
      </button>
      {expanded && (
        <div className="px-3 pb-3 space-y-2 text-[11px]">
          {alert.description && (
            <div>
              <span className="text-muted-foreground/40 text-[10px] font-medium">Description</span>
              <p className="text-foreground/70 mt-0.5">{alert.description}</p>
            </div>
          )}
          {alert.solution && (
            <div>
              <span className="text-muted-foreground/40 text-[10px] font-medium">Solution</span>
              <p className="text-foreground/70 mt-0.5">{alert.solution}</p>
            </div>
          )}
          {alert.evidence && (
            <div>
              <span className="text-muted-foreground/40 text-[10px] font-medium">Evidence</span>
              <pre className="text-foreground/50 mt-0.5 font-mono text-[10px] bg-[var(--bg-hover)]/30 p-2 rounded-lg overflow-x-auto">
                {alert.evidence}
              </pre>
            </div>
          )}
          {alert.param && (
            <div className="flex gap-2">
              <span className="text-muted-foreground/40 text-[10px] font-medium">Parameter:</span>
              <code className="text-accent/70 text-[10px]">{alert.param}</code>
            </div>
          )}
          <div className="flex gap-3 text-[9px] text-muted-foreground/30">
            {alert.cweid !== "-1" && <span>CWE-{alert.cweid}</span>}
            {alert.wascid !== "-1" && <span>WASC-{alert.wascid}</span>}
            <span>Plugin: {alert.pluginId}</span>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Repeater Panel ──


