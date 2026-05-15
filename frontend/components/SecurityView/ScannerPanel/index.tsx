import {
  Check,
  Crosshair,
  Globe,
  Loader2,
  Pause,
  Play,
  Plus,
  ShieldCheck,
  Square,
  Trash2,
  X,
  Zap,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { logAudit } from "@/lib/audit";
import type { ScanEndpoint } from "@/lib/pentest/scan-queue";
import { getProjectPath } from "@/lib/projects";
import type { VaultEntrySafe } from "@/lib/security";
import { securityApi } from "@/lib/security";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";
import { useZapScanQueue } from "../hooks/useZapScanQueue";
import { CredentialDropdown, PolicyDropdown } from "./ScanConfig";
import { AlertCard, ScanDetailTabs } from "./ScanResults";

export { AlertCard } from "./ScanResults";

export function ScannerPanel({
  initialUrl,
  initialBatchUrls,
  onUrlConsumed,
}: {
  initialUrl?: string | null;
  initialBatchUrls?: string[];
  onUrlConsumed?: () => void;
}) {
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
  const [scannerRules, setScannerRules] = useState<
    { id: string; name: string; enabled: boolean; quality: string }[]
  >([]);

  useEffect(() => {
    const pp = getProjectPath();
    Promise.all([
      securityApi.vaultList(pp).catch(() => []),
      securityApi.zapListScanPolicies().catch(() => []),
    ]).then(([v, policies]) => {
      setVaultEntries(Array.isArray(v) ? v : []);
      if (Array.isArray(policies)) setScanPolicies(policies);
    });
  }, []);

  const applyCredential = useCallback(async () => {
    if (!selectedCredential) return;
    try {
      const value = await securityApi.vaultGetValue(selectedCredential, getProjectPath());
      const entry = vaultEntries.find((e) => e.id === selectedCredential);
      if (entry && value) {
        if (entry.type === "token" || entry.type === "api_key") {
          await securityApi
            .zapApiCall("replacer", "action", "addRule", {
              description: `vault-auth-${entry.name}`,
              enabled: "true",
              matchType: "REQ_HEADER",
              matchRegex: "false",
              matchString: "Authorization",
              replacement: `Bearer ${value}`,
            })
            .catch(() => {});
        } else if (entry.type === "cookie") {
          await securityApi
            .zapApiCall("replacer", "action", "addRule", {
              description: `vault-cookie-${entry.name}`,
              enabled: "true",
              matchType: "REQ_HEADER",
              matchRegex: "false",
              matchString: "Cookie",
              replacement: value,
            })
            .catch(() => {});
        }
      }
    } catch {
      /* continue without credentials */
    }
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
    scanSelected,
    scanAll,
    stopAll: handleStopAll,
    pauseAll: handlePauseAll,
    resumeAll: handleResumeAll,
    clearCompleted: handleClearCompleted,
    clearAll: handleClearAll,
  } = queue;

  const handleScanSelected = useCallback(() => {
    if (sel) {
      logAudit({
        action: "tool_executed",
        category: "scan",
        details: `zap_active_scan on ${sel.url}`,
      });
    }
    scanSelected();
  }, [scanSelected, sel]);

  const handleScanAll = useCallback(() => {
    logAudit({
      action: "tool_executed",
      category: "scan",
      details: `zap_scan_all on ${queuedCount} endpoints`,
    });
    scanAll();
  }, [scanAll, queuedCount]);

  const handleAddEndpoint = useCallback(() => {
    if (addEndpoint(targetUrl)) {
      setTargetUrl("");
    } else if (targetUrl.trim()) {
      setTargetUrl("");
    }
  }, [addEndpoint, targetUrl]);

  const handleRemoveEndpoint = useCallback((url: string) => removeEndpoint(url), [removeEndpoint]);

  useEffect(() => {
    if (!sel || sel.status !== "complete") {
      setScanLogs([]);
      return;
    }
    securityApi
      .passiveScansByUrl(sel.url, 500)
      .then((data: any) => setScanLogs(Array.isArray(data) ? data : []))
      .catch(() => setScanLogs([]));
  }, [sel?.url, sel?.status, sel]);

  const statusBadge = (s: ScanEndpoint["status"]) => {
    const map: Record<string, { label: string; cls: string }> = {
      queued: { label: t("security.scanQueued"), cls: "text-zinc-400 bg-zinc-500/10" },
      spidering: {
        label: t("security.spidering"),
        cls: "text-blue-400 bg-blue-500/10",
      },
      scanning: { label: t("security.scanning"), cls: "text-orange-400 bg-orange-500/10" },
      paused: { label: "Paused", cls: "text-amber-400 bg-amber-500/10" },
      complete: { label: t("security.scanComplete"), cls: "text-green-400 bg-green-500/10" },
      error: { label: t("common.error"), cls: "text-red-400 bg-red-500/10" },
    };
    const m = map[s] || map.queued;
    return (
      <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full font-medium", m.cls)}>
        {m.label}
      </span>
    );
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
        <button
          type="button"
          onClick={handleAddEndpoint}
          disabled={!targetUrl.trim()}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-30"
        >
          <Plus className="w-3 h-3" /> {t("security.addTarget")}
        </button>
        {queuedCount > 0 && (
          <button
            type="button"
            onClick={handleScanAll}
            disabled={scanning}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-orange-500/10 text-orange-400 hover:bg-orange-500/20 transition-colors disabled:opacity-30"
          >
            {scanning ? <Loader2 className="w-3 h-3 animate-spin" /> : <Zap className="w-3 h-3" />}
            {t("security.scanAll")} ({queuedCount})
          </button>
        )}
        {scanningCount > 0 && pausedCount === 0 && (
          <button
            type="button"
            onClick={handlePauseAll}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-amber-500/10 text-amber-400 hover:bg-amber-500/20 transition-colors"
          >
            <Pause className="w-3 h-3" /> Pause
          </button>
        )}
        {pausedCount > 0 && (
          <button
            type="button"
            onClick={handleResumeAll}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-green-500/10 text-green-400 hover:bg-green-500/20 transition-colors"
          >
            <Play className="w-3 h-3" /> Resume
          </button>
        )}
        {scanningCount > 0 && (
          <button
            type="button"
            onClick={handleStopAll}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors"
          >
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
            { value: "", label: t("security.defaultPolicy") },
            ...scanPolicies.map((p) => ({ value: p, label: p })),
          ]}
        />
        <button
          type="button"
          className={cn(
            "p-1 rounded text-muted-foreground/30 hover:text-muted-foreground/60 transition-colors",
            showPlugins && "text-accent/60 bg-accent/10"
          )}
          onClick={() => {
            if (!showPlugins && scannerRules.length === 0) {
              securityApi
                .zapGetScanners()
                .then((r) => setScannerRules(Array.isArray(r) ? r : []))
                .catch(() => {});
            }
            setShowPlugins(!showPlugins);
          }}
          title={t("security.configurePlugins")}
        >
          <Crosshair className="w-3 h-3" />
        </button>
        <div className="flex-1" />
        <span className="text-[9px] text-muted-foreground/30">
          {endpoints.length} {t("security.endpoints")} · {totalAlerts} {t("security.alertsTotal")}
        </span>
        {completedCount > 0 && (
          <button
            type="button"
            onClick={handleClearCompleted}
            className="text-[9px] text-muted-foreground/30 hover:text-foreground transition-colors"
          >
            {t("security.clearCompleted")}
          </button>
        )}
        {endpoints.length > 0 && (
          <button
            type="button"
            onClick={handleClearAll}
            className="flex items-center gap-1 text-[9px] text-muted-foreground/30 hover:text-destructive transition-colors"
          >
            <Trash2 className="w-2.5 h-2.5" /> {t("security.clearAll")}
          </button>
        )}
      </div>

      {showPlugins && (
        <div className="border-b border-border/10 bg-muted/5 max-h-[300px] flex flex-col">
          <div className="flex items-center gap-2 px-4 py-1.5 border-b border-border/5 flex-shrink-0">
            <span className="text-[10px] font-medium text-foreground/50">
              {t("security.scanPlugins")}
            </span>
            <span className="text-[9px] text-muted-foreground/30">
              {scannerRules.filter((r) => r.enabled).length}/{scannerRules.length}{" "}
              {t("security.enabled")}
            </span>
            <div className="flex-1" />
            <button
              type="button"
              className="text-[9px] text-accent/50 hover:text-accent/80 transition-colors"
              onClick={() => {
                const allEnabled = scannerRules.every((r) => r.enabled);
                const ids = scannerRules.map((r) => r.id);
                securityApi
                  .zapSetScannersEnabled(ids, !allEnabled)
                  .then(() => {
                    setScannerRules((prev) => prev.map((r) => ({ ...r, enabled: !allEnabled })));
                  })
                  .catch(() => {});
              }}
            >
              {scannerRules.every((r) => r.enabled)
                ? t("security.disableAll")
                : t("security.enableAll")}
            </button>
          </div>
          <div className="overflow-y-auto flex-1">
            {scannerRules.length === 0 ? (
              <div className="flex items-center justify-center py-6 text-muted-foreground/20 text-[11px]">
                <Loader2 className="w-4 h-4 animate-spin mr-2" /> {t("security.loadingPlugins")}
              </div>
            ) : (
              <div className="divide-y divide-border/5">
                {scannerRules.map((rule) => (
                  <div
                    key={rule.id}
                    className="flex items-center gap-2 px-4 py-1 hover:bg-[var(--bg-hover)]/20 transition-colors"
                  >
                    <button
                      type="button"
                      className={cn(
                        "w-3.5 h-3.5 rounded border flex items-center justify-center transition-colors flex-shrink-0",
                        rule.enabled
                          ? "bg-accent/20 border-accent/40 text-accent"
                          : "border-border/30 text-transparent"
                      )}
                      onClick={() => {
                        securityApi
                          .zapSetScannersEnabled([rule.id], !rule.enabled)
                          .then(() => {
                            setScannerRules((prev) =>
                              prev.map((r) =>
                                r.id === rule.id ? { ...r, enabled: !r.enabled } : r
                              )
                            );
                          })
                          .catch(() => {});
                      }}
                    >
                      {rule.enabled && <Check className="w-2.5 h-2.5" />}
                    </button>
                    <span className="text-[10px] text-foreground/60 flex-1 truncate">
                      {rule.name}
                    </span>
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
                <p className="text-[12px] font-medium">{t("security.noScanTargets")}</p>
                <p className="text-[10px] text-muted-foreground/15 max-w-[220px] text-center">
                  {t("security.addTargetsHint")}
                </p>
              </div>
            ) : (
              <div className="divide-y divide-border/5">
                {[...endpoints]
                  .sort((a, b) => b.addedAt - a.addedAt)
                  .map((ep) => {
                    const alertsByRisk = { High: 0, Medium: 0, Low: 0, Info: 0 };
                    for (const a of ep.alerts) {
                      if (a.risk === "High") alertsByRisk.High++;
                      else if (a.risk === "Medium") alertsByRisk.Medium++;
                      else if (a.risk === "Low") alertsByRisk.Low++;
                      else alertsByRisk.Info++;
                    }
                    let host: string;
                    let path: string;
                    try {
                      const u = new URL(ep.url);
                      host = u.host;
                      path = u.pathname;
                    } catch {
                      host = ep.url;
                      path = "";
                    }

                    return (
                      <div
                        key={ep.url}
                        onClick={() => setSelectedUrl(ep.url)}
                        className={cn(
                          "px-3 py-2.5 cursor-pointer transition-colors group",
                          selectedUrl === ep.url ? "bg-accent/8" : "hover:bg-[var(--bg-hover)]/30"
                        )}
                      >
                        <div className="flex items-center gap-2 mb-1">
                          {statusBadge(ep.status)}
                          {(ep.status === "scanning" ||
                            ep.status === "spidering" ||
                            ep.status === "paused") && (
                            <>
                              <span className="text-[9px] text-muted-foreground/40">
                                {ep.progress}%
                              </span>
                              {ep.messageCount != null && ep.messageCount > 0 && (
                                <span className="text-[8px] text-muted-foreground/25">
                                  {ep.messageCount} req
                                </span>
                              )}
                            </>
                          )}
                          <div className="flex-1" />
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation();
                              handleRemoveEndpoint(ep.url);
                            }}
                            className="p-0.5 rounded text-muted-foreground/0 group-hover:text-muted-foreground/30 hover:!text-destructive transition-colors"
                          >
                            <X className="w-3 h-3" />
                          </button>
                        </div>
                        <div className="text-[11px] font-mono text-foreground/70 truncate">
                          {host}
                        </div>
                        {path && path !== "/" && (
                          <div className="text-[10px] font-mono text-muted-foreground/30 truncate">
                            {path}
                          </div>
                        )}
                        {(ep.status === "scanning" ||
                          ep.status === "spidering" ||
                          ep.status === "paused") && (
                          <div className="h-1 rounded-full bg-muted/20 overflow-hidden mt-1.5">
                            <div
                              className={cn(
                                "h-full rounded-full transition-all duration-500",
                                ep.status === "paused" ? "bg-amber-500/60" : "bg-accent"
                              )}
                              style={{ width: `${ep.progress}%` }}
                            />
                          </div>
                        )}
                        {ep.alerts.length > 0 && (
                          <div className="flex items-center gap-2 mt-1.5">
                            {alertsByRisk.High > 0 && (
                              <span className="text-[8px] text-red-400 font-medium">
                                {alertsByRisk.High}H
                              </span>
                            )}
                            {alertsByRisk.Medium > 0 && (
                              <span className="text-[8px] text-orange-400 font-medium">
                                {alertsByRisk.Medium}M
                              </span>
                            )}
                            {alertsByRisk.Low > 0 && (
                              <span className="text-[8px] text-yellow-400 font-medium">
                                {alertsByRisk.Low}L
                              </span>
                            )}
                            {alertsByRisk.Info > 0 && (
                              <span className="text-[8px] text-blue-400 font-medium">
                                {alertsByRisk.Info}I
                              </span>
                            )}
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
                <span className="text-[12px] font-mono text-foreground/80 truncate flex-1">
                  {sel.url}
                </span>
                {statusBadge(sel.status)}
                {sel.status === "queued" && (
                  <button
                    type="button"
                    onClick={handleScanSelected}
                    className="flex items-center gap-1 px-2.5 py-1 rounded-lg text-[10px] font-medium bg-orange-500/10 text-orange-400 hover:bg-orange-500/20 transition-colors"
                  >
                    <Zap className="w-3 h-3" /> {t("security.activeScan")}
                  </button>
                )}
              </div>
              {sel.status === "complete" && (sel.alerts.length > 0 || scanLogs.length > 0) ? (
                <ScanDetailTabs alerts={sel.alerts} scanLogs={scanLogs} />
              ) : sel.alerts.length > 0 ? (
                <div className="flex-1 overflow-y-auto px-4 py-3 space-y-2">
                  <div className="flex items-center gap-2 mb-2">
                    <span className="text-[11px] font-medium text-foreground/60">
                      {sel.alerts.length} {t("security.alertsTotal")}
                    </span>
                    {(() => {
                      const vulnTypes = new Set(sel.alerts.map((a) => a.name));
                      return (
                        <span className="text-[10px] text-muted-foreground/30">
                          {vulnTypes.size} {t("security.uniqueVulnTypes")}
                        </span>
                      );
                    })()}
                  </div>
                  {sel.alerts.map((alert) => (
                    <AlertCard key={`${alert.id}-${alert.url}`} alert={alert} />
                  ))}
                </div>
              ) : sel.status === "scanning" || sel.status === "spidering" ? (
                <div className="flex-1 flex flex-col items-center justify-center gap-4 text-muted-foreground/30">
                  <Loader2 className="w-8 h-8 animate-spin text-orange-400/40" />
                  <div className="text-center space-y-1.5">
                    <p className="text-[13px] font-medium text-foreground/50">
                      {t("security.scanInProgress")}
                    </p>
                    <div className="flex items-center gap-3 justify-center">
                      <span className="text-[11px] text-orange-400/60 font-mono">
                        {sel.progress}%
                      </span>
                      {sel.messageCount != null && sel.messageCount > 0 && (
                        <span className="text-[11px] text-muted-foreground/40">
                          {sel.messageCount} {t("security.requestsSent")}
                        </span>
                      )}
                      {sel.alerts.length > 0 && (
                        <span className="text-[11px] text-red-400/60">
                          {sel.alerts.length} alerts
                        </span>
                      )}
                    </div>
                  </div>
                </div>
              ) : (
                <div className="flex-1 flex flex-col items-center justify-center gap-3 text-muted-foreground/20">
                  <ShieldCheck className="w-12 h-12" />
                  <p className="text-[13px] font-medium">
                    {sel.status === "complete"
                      ? t("security.scanNoResults")
                      : t("security.scanHint")}
                  </p>
                </div>
              )}
            </>
          ) : (
            <div className="h-full flex flex-col items-center justify-center gap-3 text-muted-foreground/20">
              <ShieldCheck className="w-12 h-12" />
              <p className="text-[13px] font-medium">
                {endpoints.length > 0 ? t("security.selectEndpoint") : t("security.scanHint")}
              </p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
