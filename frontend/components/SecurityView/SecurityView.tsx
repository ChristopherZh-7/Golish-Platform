import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity, AlertTriangle, ArrowRight, Check, ChevronDown, ChevronRight,
  Clock, Download, Globe, History, KeyRound, Loader2, Pause, Play, RefreshCw, Search,
  Send, Shield, ShieldAlert, ShieldCheck, ShieldX, Square, X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import {
  zapStart, zapStop, zapStatus, zapDetectPath, zapGetHistory, zapGetHistoryCount,
  zapGetMessage, zapStartScan, zapScanProgress, zapStopScan,
  zapGetAlerts, zapGetAlertCount, zapSendRequest, zapStartSpider,
  zapSpiderProgress, zapStopSpider,
} from "@/lib/pentest/zap-api";
import type {
  ZapStatusInfo, HttpHistoryEntry, HttpMessageDetail,
  ZapAlert, ScanProgress, ManualRequestResult,
} from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";

import { lazy, Suspense } from "react";
const VaultSettings = lazy(() =>
  import("@/components/Settings/VaultSettings").then((m) => ({ default: m.VaultSettings }))
);

type SecurityTab = "history" | "scanner" | "repeater" | "alerts" | "vault";

export function SecurityView() {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [activeTab, setActiveTab] = useState<SecurityTab>("history");
  const [zapState, setZapState] = useState<ZapStatusInfo>({
    status: "stopped",
    port: 8090,
  });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [zapInstalled, setZapInstalled] = useState<boolean | null>(null);
  const [checkingInstall, setCheckingInstall] = useState(true);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [status, path] = await Promise.all([
          zapStatus().catch(() => ({ status: "stopped", port: 8090 }) as ZapStatusInfo),
          zapDetectPath().catch(() => null),
        ]);
        if (cancelled) return;
        setZapState(status);
        setZapInstalled(status.status === "running" || path !== null);
      } catch {
        if (!cancelled) setZapInstalled(false);
      } finally {
        if (!cancelled) setCheckingInstall(false);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const handleStart = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await zapStart();
      setZapState(result);
    } catch (e) {
      setError(String(e));
      setZapState((s) => ({ ...s, status: "error", error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, []);

  const handleStop = useCallback(async () => {
    setLoading(true);
    try {
      await zapStop();
      setZapState({ status: "stopped", port: zapState.port });
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [zapState.port]);

  const isRunning = zapState.status === "running";

  const tabs: { id: SecurityTab; label: string; icon: React.ElementType }[] = [
    { id: "history", label: t("security.history"), icon: History },
    { id: "scanner", label: t("security.scanner"), icon: ShieldAlert },
    { id: "repeater", label: t("security.repeater"), icon: Send },
    { id: "alerts", label: t("security.alerts"), icon: AlertTriangle },
    { id: "vault", label: t("vault.title", "Credential Vault"), icon: KeyRound },
  ];

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-border/15 flex-shrink-0">
        <div className="flex items-center gap-3">
          <Shield className="w-4 h-4 text-accent" />
          <h1 className="text-[14px] font-semibold text-foreground">
            {t("security.title")}
          </h1>
          <StatusBadge status={zapState} />
        </div>

        <div className="flex items-center gap-2">
          {error && (
            <span className="text-[10px] text-destructive/70 max-w-[200px] truncate">
              {error}
            </span>
          )}
          {isRunning ? (
            <button
              type="button"
              onClick={handleStop}
              disabled={loading}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors disabled:opacity-50"
            >
              {loading ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <Square className="w-3 h-3" />
              )}
              {t("security.stopZap")}
            </button>
          ) : (
            <button
              type="button"
              onClick={handleStart}
              disabled={loading}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-50"
            >
              {loading ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : (
                <Play className="w-3 h-3" />
              )}
              {t("security.startZap")}
            </button>
          )}
        </div>
      </div>

      {/* Sub-tabs */}
      <div className="flex items-center gap-1 px-4 py-2 border-b border-border/10 flex-shrink-0">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            type="button"
            onClick={() => setActiveTab(tab.id)}
            className={cn(
              "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[11px] transition-colors",
              activeTab === tab.id
                ? "bg-accent/15 text-accent font-medium"
                : "text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)]"
            )}
          >
            <tab.icon className="w-3 h-3" />
            {tab.label}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-hidden">
        {activeTab === "vault" ? (
          <Suspense fallback={<div className="h-full flex items-center justify-center"><Loader2 className="w-5 h-5 animate-spin text-muted-foreground/20" /></div>}>
            <VaultSettings />
          </Suspense>
        ) : checkingInstall ? (
          <div className="h-full flex items-center justify-center">
            <Loader2 className="w-6 h-6 animate-spin text-muted-foreground/20" />
          </div>
        ) : zapInstalled === false ? (
          <ZapNotInstalled onRetry={() => {
            setCheckingInstall(true);
            zapDetectPath().then((p) => {
              setZapInstalled(p !== null);
              setCheckingInstall(false);
            }).catch(() => { setZapInstalled(false); setCheckingInstall(false); });
          }} />
        ) : !isRunning ? (
          <ZapNotRunning onStart={handleStart} loading={loading} error={error} />
        ) : activeTab === "history" ? (
          <HttpHistoryPanel />
        ) : activeTab === "scanner" ? (
          <ScannerPanel />
        ) : activeTab === "repeater" ? (
          <RepeaterPanel />
        ) : (
          <AlertsPanel />
        )}
      </div>
    </div>
  );
}

// ── Status Badge ──

function StatusBadge({ status }: { status: ZapStatusInfo }) {
  const colors: Record<string, string> = {
    running: "bg-green-500/15 text-green-400",
    starting: "bg-yellow-500/15 text-yellow-400",
    stopped: "bg-zinc-500/15 text-zinc-400",
    error: "bg-red-500/15 text-red-400",
  };

  return (
    <span
      className={cn(
        "text-[9px] px-2 py-0.5 rounded-full font-medium flex items-center gap-1",
        colors[status.status] || colors.stopped
      )}
    >
      {status.status === "running" && (
        <span className="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse" />
      )}
      {status.status}
      {status.version && ` v${status.version}`}
      {status.status === "running" && ` :${status.port}`}
    </span>
  );
}

// ── ZAP Not Installed ──

function ZapNotInstalled({ onRetry }: { onRetry: () => void }) {
  const { t } = useTranslation();
  const [installing, setInstalling] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);

  const handleBrewInstall = useCallback(async () => {
    setInstalling(true);
    setInstallError(null);
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const { getSettings } = await import("@/lib/settings");
      const settings = await getSettings().catch(() => null);
      const proxyUrl = settings?.network?.proxy_url || null;
      await invoke("pentest_install_runtime", { runtimeType: "brew-cask:zap", proxyUrl });
      onRetry();
    } catch (e) {
      setInstallError(String(e));
    } finally {
      setInstalling(false);
    }
  }, [onRetry]);

  return (
    <div className="h-full flex flex-col items-center justify-center gap-5 text-muted-foreground/30">
      <ShieldX className="w-16 h-16" />
      <div className="text-center">
        <p className="text-[14px] font-medium text-foreground/60">{t("security.zapNotInstalled")}</p>
        <p className="text-[12px] text-muted-foreground/30 max-w-md mt-1.5">
          {t("security.zapNotInstalledHint")}
        </p>
      </div>

      {installError && (
        <p className="text-[11px] text-destructive/70 max-w-sm text-center">{installError}</p>
      )}

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={handleBrewInstall}
          disabled={installing}
          className="flex items-center gap-2 px-4 py-2 rounded-lg text-[12px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-50"
        >
          {installing ? (
            <Loader2 className="w-4 h-4 animate-spin" />
          ) : (
            <Download className="w-4 h-4" />
          )}
          {t("security.installViaBrew")}
        </button>
        <button
          type="button"
          onClick={onRetry}
          className="flex items-center gap-2 px-4 py-2 rounded-lg text-[12px] font-medium text-muted-foreground/50 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          {t("security.recheckInstall")}
        </button>
      </div>

      <div className="max-w-md mt-2 text-center">
        <p className="text-[11px] text-muted-foreground/20">
          {t("security.manualInstallHint")}
        </p>
        <code className="text-[11px] text-muted-foreground/40 bg-muted/20 px-2 py-0.5 rounded mt-1 inline-block font-mono">
          brew install --cask zap
        </code>
      </div>
    </div>
  );
}

// ── ZAP Not Running ──

function ZapNotRunning({ onStart, loading, error }: { onStart: () => void; loading: boolean; error: string | null }) {
  const { t } = useTranslation();
  return (
    <div className="h-full flex flex-col items-center justify-center gap-4 text-muted-foreground/30">
      <Shield className="w-16 h-16" />
      <p className="text-[14px] font-medium">{t("security.zapNotRunning")}</p>
      <p className="text-[12px] text-muted-foreground/20 max-w-sm text-center">
        {t("security.zapNotRunningHint")}
      </p>
      {error && (
        <p className="text-[11px] text-destructive/70 max-w-sm text-center">{error}</p>
      )}
      <button
        type="button"
        onClick={onStart}
        disabled={loading}
        className="flex items-center gap-2 px-4 py-2 rounded-lg text-[12px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-50"
      >
        {loading ? (
          <Loader2 className="w-4 h-4 animate-spin" />
        ) : (
          <Play className="w-4 h-4" />
        )}
        {t("security.startZap")}
      </button>
    </div>
  );
}

// ── HTTP History Panel ──

function HttpHistoryPanel() {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<HttpHistoryEntry[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<HttpMessageDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const loadHistory = useCallback(async () => {
    try {
      const [items, count] = await Promise.all([
        zapGetHistory(0, 200),
        zapGetHistoryCount(),
      ]);
      setEntries(items);
      setTotalCount(count);
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    loadHistory();
    intervalRef.current = setInterval(loadHistory, 3000);
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [loadHistory]);

  const handleSelect = useCallback(async (id: number) => {
    setSelectedId(id);
    setLoading(true);
    try {
      const msg = await zapGetMessage(id);
      setDetail(msg);
    } catch {
      setDetail(null);
    } finally {
      setLoading(false);
    }
  }, []);

  const filtered = useMemo(() => {
    if (!search.trim()) return entries;
    const q = search.toLowerCase();
    return entries.filter(
      (e) =>
        e.url.toLowerCase().includes(q) ||
        e.host.toLowerCase().includes(q) ||
        e.method.toLowerCase().includes(q)
    );
  }, [entries, search]);

  const methodColor = (m: string) => {
    const c: Record<string, string> = {
      GET: "text-green-400",
      POST: "text-blue-400",
      PUT: "text-yellow-400",
      DELETE: "text-red-400",
      PATCH: "text-purple-400",
      OPTIONS: "text-zinc-400",
    };
    return c[m] || "text-muted-foreground";
  };

  const statusColor = (code: number) => {
    if (code >= 200 && code < 300) return "text-green-400";
    if (code >= 300 && code < 400) return "text-blue-400";
    if (code >= 400 && code < 500) return "text-yellow-400";
    if (code >= 500) return "text-red-400";
    return "text-muted-foreground";
  };

  return (
    <div className="h-full flex flex-col">
      {/* Search bar */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
        <div className="relative flex-1 max-w-sm">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("security.filterHistory")}
            className="w-full h-7 pl-8 pr-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
          />
        </div>
        <span className="text-[10px] text-muted-foreground/30">
          {filtered.length} / {totalCount} {t("security.requests")}
        </span>
        <button
          type="button"
          onClick={loadHistory}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <RefreshCw className="w-3 h-3" />
        </button>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* Request list */}
        <div className="w-full flex-1 overflow-y-auto">
          <table className="w-full text-[11px]">
            <thead className="sticky top-0 bg-card z-10">
              <tr className="text-muted-foreground/40 text-left">
                <th className="px-3 py-1.5 font-medium w-[50px]">#</th>
                <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.method")}</th>
                <th className="px-3 py-1.5 font-medium">{t("security.url")}</th>
                <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.status")}</th>
                <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.size")}</th>
                <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.time")}</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((entry) => (
                <tr
                  key={entry.id}
                  onClick={() => handleSelect(entry.id)}
                  className={cn(
                    "cursor-pointer transition-colors border-b border-border/5",
                    selectedId === entry.id
                      ? "bg-accent/10"
                      : "hover:bg-[var(--bg-hover)]/40"
                  )}
                >
                  <td className="px-3 py-1.5 text-muted-foreground/30 font-mono">
                    {entry.id}
                  </td>
                  <td className={cn("px-3 py-1.5 font-mono font-medium", methodColor(entry.method))}>
                    {entry.method}
                  </td>
                  <td className="px-3 py-1.5 text-foreground/80 truncate max-w-[400px] font-mono">
                    {entry.host}
                    <span className="text-muted-foreground/40">{entry.path}</span>
                  </td>
                  <td className={cn("px-3 py-1.5 font-mono", statusColor(entry.status_code))}>
                    {entry.status_code || "-"}
                  </td>
                  <td className="px-3 py-1.5 text-muted-foreground/40 font-mono">
                    {formatSize(entry.content_length)}
                  </td>
                  <td className="px-3 py-1.5 text-muted-foreground/40 font-mono">
                    {entry.time_ms ? `${entry.time_ms}ms` : "-"}
                  </td>
                </tr>
              ))}
              {filtered.length === 0 && (
                <tr>
                  <td colSpan={6} className="px-3 py-8 text-center text-muted-foreground/30">
                    {entries.length === 0
                      ? t("security.noHistory")
                      : t("common.noResults")}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>

        {/* Detail panel */}
        {selectedId !== null && (
          <div className="w-[400px] flex-shrink-0 border-l border-border/10 flex flex-col overflow-hidden">
            <div className="flex items-center justify-between px-3 py-2 border-b border-border/10">
              <span className="text-[11px] font-medium text-muted-foreground/40">
                #{selectedId} {t("security.detail")}
              </span>
              <button
                type="button"
                onClick={() => { setSelectedId(null); setDetail(null); }}
                className="p-1 rounded text-muted-foreground/30 hover:text-foreground transition-colors"
              >
                <X className="w-3 h-3" />
              </button>
            </div>
            {loading ? (
              <div className="flex-1 flex items-center justify-center">
                <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" />
              </div>
            ) : detail ? (
              <div className="flex-1 overflow-y-auto">
                <DetailSection title={t("security.requestHeaders")} content={detail.request_headers} />
                {detail.request_body && (
                  <DetailSection title={t("security.requestBody")} content={detail.request_body} />
                )}
                <DetailSection title={t("security.responseHeaders")} content={detail.response_headers} />
                <DetailSection title={t("security.responseBody")} content={detail.response_body} maxHeight={300} />
              </div>
            ) : (
              <div className="flex-1 flex items-center justify-center text-muted-foreground/20 text-[12px]">
                {t("security.noDetail")}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function DetailSection({ title, content, maxHeight }: { title: string; content: string; maxHeight?: number }) {
  const [expanded, setExpanded] = useState(true);
  return (
    <div className="border-b border-border/5">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-1.5 px-3 py-1.5 text-[10px] font-medium text-muted-foreground/40 hover:text-foreground transition-colors"
      >
        {expanded ? <ChevronDown className="w-2.5 h-2.5" /> : <ChevronRight className="w-2.5 h-2.5" />}
        {title}
      </button>
      {expanded && (
        <pre
          className="px-3 pb-2 text-[10px] font-mono text-foreground/70 whitespace-pre-wrap break-all overflow-y-auto"
          style={{ maxHeight: maxHeight ? `${maxHeight}px` : undefined }}
        >
          {content || "(empty)"}
        </pre>
      )}
    </div>
  );
}

// ── Scanner Panel ──

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

function ScannerPanel() {
  const { t } = useTranslation();
  const [targetUrl, setTargetUrl] = useState("");
  const [scanId, setScanId] = useState<string | null>(null);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [alerts, setAlerts] = useState<ZapAlert[]>([]);
  const [scanning, setScanning] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [vaultEntries, setVaultEntries] = useState<VaultEntrySafe[]>([]);
  const [selectedCredential, setSelectedCredential] = useState<string>("");
  const [showCredSelector, setShowCredSelector] = useState(false);

  useEffect(() => {
    invoke<VaultEntrySafe[]>("vault_list", { projectPath: getProjectPath() })
      .then(setVaultEntries)
      .catch(() => {});
  }, [currentProjectPath]);

  const handleStartScan = useCallback(async () => {
    if (!targetUrl.trim()) return;
    setScanning(true);
    setAlerts([]);
    try {
      // If a credential is selected, resolve it and set as scan header
      if (selectedCredential) {
        try {
          const value = await invoke<string>("vault_get_value", { id: selectedCredential, projectPath: getProjectPath() });
          const entry = vaultEntries.find((e) => e.id === selectedCredential);
          if (entry && value) {
            if (entry.entry_type === "token" || entry.entry_type === "apiKey") {
              await invoke("zap_api_call", {
                component: "replacer",
                actionType: "action",
                method: "addRule",
                params: {
                  description: `vault-auth-${entry.name}`,
                  enabled: "true",
                  matchType: "REQ_HEADER",
                  matchRegex: "false",
                  matchString: "Authorization",
                  replacement: `Bearer ${value}`,
                },
              }).catch(() => {});
            } else if (entry.entry_type === "cookie") {
              await invoke("zap_api_call", {
                component: "replacer",
                actionType: "action",
                method: "addRule",
                params: {
                  description: `vault-cookie-${entry.name}`,
                  enabled: "true",
                  matchType: "REQ_HEADER",
                  matchRegex: "false",
                  matchString: "Cookie",
                  replacement: value,
                },
              }).catch(() => {});
            } else if (entry.entry_type === "password" && entry.username) {
              await invoke("zap_api_call", {
                component: "forcedUser",
                actionType: "action",
                method: "setForcedUserModeEnabled",
                params: { boolean: "true" },
              }).catch(() => {});
            }
          }
        } catch {
          // If vault resolution fails, continue without credentials
        }
      }
      // Start spider first, then active scan
      await zapStartSpider(targetUrl);
      const id = await zapStartScan(targetUrl);
      setScanId(id);
    } catch (e) {
      setScanning(false);
      return;
    }
  }, [targetUrl, selectedCredential, vaultEntries]);

  useEffect(() => {
    if (!scanId || !scanning) return;
    const poll = async () => {
      try {
        const prog = await zapScanProgress(scanId);
        setProgress(prog);
        const foundAlerts = await zapGetAlerts(targetUrl, 0, 200);
        setAlerts(foundAlerts);
        if (prog.progress >= 100) {
          setScanning(false);
          if (intervalRef.current) clearInterval(intervalRef.current);
        }
      } catch { /* ignore */ }
    };
    poll();
    intervalRef.current = setInterval(poll, 2000);
    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [scanId, scanning, targetUrl]);

  const handleStop = useCallback(async () => {
    if (scanId) {
      await zapStopScan(scanId).catch(() => {});
      setScanning(false);
    }
  }, [scanId]);

  const riskColor = (risk: string) => {
    const c: Record<string, string> = {
      High: "text-red-400 bg-red-500/10",
      Medium: "text-orange-400 bg-orange-500/10",
      Low: "text-yellow-400 bg-yellow-500/10",
      Informational: "text-blue-400 bg-blue-500/10",
    };
    return c[risk] || "text-muted-foreground bg-muted/20";
  };

  const riskIcon = (risk: string) => {
    if (risk === "High") return <ShieldX className="w-3 h-3" />;
    if (risk === "Medium") return <ShieldAlert className="w-3 h-3" />;
    return <ShieldCheck className="w-3 h-3" />;
  };

  return (
    <div className="h-full flex flex-col">
      {/* Target input */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-border/10 flex-shrink-0">
        <Globe className="w-3.5 h-3.5 text-muted-foreground/30 flex-shrink-0" />
        <input
          value={targetUrl}
          onChange={(e) => setTargetUrl(e.target.value)}
          placeholder={t("security.scanTargetPlaceholder")}
          onKeyDown={(e) => e.key === "Enter" && !scanning && handleStartScan()}
          className="flex-1 h-8 px-3 text-[12px] font-mono bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
        />
        {scanning ? (
          <button
            type="button"
            onClick={handleStop}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors"
          >
            <Square className="w-3 h-3" />
            {t("security.stopScan")}
          </button>
        ) : (
          <button
            type="button"
            onClick={handleStartScan}
            disabled={!targetUrl.trim()}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-30"
          >
            <Play className="w-3 h-3" />
            {t("security.startScan")}
          </button>
        )}
      </div>

      {/* Credential selector */}
      <div className="flex items-center gap-2 px-4 py-1.5 border-b border-border/10 flex-shrink-0">
        <KeyRound className="w-3 h-3 text-muted-foreground/30 flex-shrink-0" />
        <button
          className="flex items-center gap-1.5 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/80 transition-colors"
          onClick={() => setShowCredSelector(!showCredSelector)}
        >
          {selectedCredential ? (
            <span className="text-accent/70 font-medium">
              {vaultEntries.find((e) => e.id === selectedCredential)?.name || "Credential"}
            </span>
          ) : (
            <span>{t("security.noCredential", "No credential (unauthenticated)")}</span>
          )}
          <ChevronDown className="w-3 h-3" />
        </button>
        {selectedCredential && (
          <button
            className="text-[10px] text-muted-foreground/40 hover:text-muted-foreground/70"
            onClick={() => setSelectedCredential("")}
          >
            ×
          </button>
        )}
      </div>
      {showCredSelector && (
        <div className="px-4 py-2 border-b border-border/10 space-y-1 max-h-32 overflow-y-auto bg-muted/5">
          <div
            className={cn(
              "px-2 py-1 rounded text-[10px] cursor-pointer hover:bg-muted/30 transition-colors",
              !selectedCredential && "bg-accent/10 text-accent"
            )}
            onClick={() => { setSelectedCredential(""); setShowCredSelector(false); }}
          >
            {t("security.noCredential", "No credential (unauthenticated)")}
          </div>
          {vaultEntries.filter((e) => ["password", "token", "cookie", "apiKey"].includes(e.entry_type)).map((entry) => (
            <div
              key={entry.id}
              className={cn(
                "px-2 py-1 rounded text-[10px] cursor-pointer hover:bg-muted/30 transition-colors flex items-center gap-2",
                selectedCredential === entry.id && "bg-accent/10 text-accent"
              )}
              onClick={() => { setSelectedCredential(entry.id); setShowCredSelector(false); }}
            >
              <KeyRound className="w-2.5 h-2.5 flex-shrink-0" />
              <span className="truncate">{entry.name}</span>
              <span className="text-muted-foreground/40 text-[9px]">{entry.entry_type}</span>
              {entry.username && (
                <span className="text-muted-foreground/40 text-[9px]">{entry.username}</span>
              )}
            </div>
          ))}
          {vaultEntries.filter((e) => ["password", "token", "cookie", "apiKey"].includes(e.entry_type)).length === 0 && (
            <div className="text-[10px] text-muted-foreground/30 py-1 text-center">
              {t("security.noVaultEntries", "No credentials in vault")}
            </div>
          )}
        </div>
      )}

      {/* Progress */}
      {(scanning || progress) && (
        <div className="px-4 py-2 border-b border-border/10 flex-shrink-0">
          <div className="flex items-center justify-between text-[10px] text-muted-foreground/50 mb-1">
            <span>
              {scanning ? t("security.scanning") : t("security.scanComplete")}
            </span>
            <span>{progress?.progress ?? 0}%</span>
          </div>
          <div className="h-1.5 rounded-full bg-muted/20 overflow-hidden">
            <div
              className="h-full rounded-full bg-accent transition-all duration-500"
              style={{ width: `${progress?.progress ?? 0}%` }}
            />
          </div>
        </div>
      )}

      {/* Alerts list */}
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {alerts.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <ShieldCheck className="w-12 h-12" />
            <p className="text-[13px] font-medium">
              {scanning ? t("security.scanInProgress") : t("security.noAlerts")}
            </p>
            <p className="text-[11px] text-muted-foreground/15 max-w-xs text-center">
              {t("security.scanHint")}
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            {alerts.map((alert) => (
              <AlertCard key={`${alert.id}-${alert.url}`} alert={alert} riskColor={riskColor} riskIcon={riskIcon} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function AlertCard({
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

function RepeaterPanel() {
  const { t } = useTranslation();
  const [request, setRequest] = useState(
    "GET / HTTP/1.1\nHost: example.com\nUser-Agent: golish-platform/1.0\n\n"
  );
  const [response, setResponse] = useState<ManualRequestResult | null>(null);
  const [sending, setSending] = useState(false);

  const handleSend = useCallback(async () => {
    setSending(true);
    try {
      const result = await zapSendRequest(request);
      setResponse(result);
    } catch {
      setResponse(null);
    } finally {
      setSending(false);
    }
  }, [request]);

  return (
    <div className="h-full flex">
      {/* Request editor */}
      <div className="flex-1 flex flex-col border-r border-border/10">
        <div className="flex items-center justify-between px-3 py-2 border-b border-border/10">
          <span className="text-[11px] font-medium text-muted-foreground/40">
            {t("security.request")}
          </span>
          <button
            type="button"
            onClick={handleSend}
            disabled={sending || !request.trim()}
            className="flex items-center gap-1.5 px-3 py-1 rounded-lg text-[11px] font-medium bg-accent/10 text-accent hover:bg-accent/20 transition-colors disabled:opacity-30"
          >
            {sending ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : (
              <Send className="w-3 h-3" />
            )}
            {t("security.send")}
          </button>
        </div>
        <textarea
          value={request}
          onChange={(e) => setRequest(e.target.value)}
          spellCheck={false}
          className="flex-1 w-full px-4 py-3 text-[11px] font-mono leading-[1.6] bg-transparent text-foreground outline-none resize-none"
          style={{ tabSize: 2 }}
        />
      </div>

      {/* Response display */}
      <div className="flex-1 flex flex-col">
        <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10">
          <span className="text-[11px] font-medium text-muted-foreground/40">
            {t("security.response")}
          </span>
          {response && (
            <span
              className={cn(
                "text-[10px] px-1.5 py-0.5 rounded-full font-mono",
                response.status_code >= 200 && response.status_code < 300
                  ? "bg-green-500/10 text-green-400"
                  : response.status_code >= 400
                    ? "bg-red-500/10 text-red-400"
                    : "bg-yellow-500/10 text-yellow-400"
              )}
            >
              {response.status_code}
            </span>
          )}
        </div>
        {response ? (
          <div className="flex-1 overflow-y-auto">
            <DetailSection title={t("security.responseHeaders")} content={response.response_header} />
            <pre className="px-4 py-3 text-[11px] font-mono leading-[1.6] text-foreground/70 whitespace-pre-wrap break-all">
              {response.response_body || "(empty body)"}
            </pre>
          </div>
        ) : (
          <div className="flex-1 flex items-center justify-center text-muted-foreground/20">
            <div className="flex flex-col items-center gap-2">
              <ArrowRight className="w-8 h-8" />
              <p className="text-[12px]">{t("security.sendToSee")}</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Alerts Panel ──

function AlertsPanel() {
  const { t } = useTranslation();
  const [alerts, setAlerts] = useState<ZapAlert[]>([]);
  const [loading, setLoading] = useState(true);
  const [count, setCount] = useState(0);

  useEffect(() => {
    async function load() {
      setLoading(true);
      try {
        const [items, total] = await Promise.all([
          zapGetAlerts(undefined, 0, 500),
          zapGetAlertCount(),
        ]);
        setAlerts(items);
        setCount(total);
      } catch { /* ignore */ }
      setLoading(false);
    }
    load();
  }, []);

  const riskColor = (risk: string) => {
    const c: Record<string, string> = {
      High: "text-red-400 bg-red-500/10",
      Medium: "text-orange-400 bg-orange-500/10",
      Low: "text-yellow-400 bg-yellow-500/10",
      Informational: "text-blue-400 bg-blue-500/10",
    };
    return c[risk] || "text-muted-foreground bg-muted/20";
  };

  const riskIcon = (risk: string) => {
    if (risk === "High") return <ShieldX className="w-3 h-3" />;
    if (risk === "Medium") return <ShieldAlert className="w-3 h-3" />;
    return <ShieldCheck className="w-3 h-3" />;
  };

  const grouped = useMemo(() => {
    const groups: Record<string, ZapAlert[]> = {};
    for (const a of alerts) {
      (groups[a.risk] ??= []).push(a);
    }
    return groups;
  }, [alerts]);

  const riskOrder = ["High", "Medium", "Low", "Informational"];

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-4 py-2 border-b border-border/10 flex-shrink-0">
        <span className="text-[11px] text-muted-foreground/40">
          {count} {t("security.alertsTotal")}
        </span>
        <div className="flex items-center gap-2">
          {riskOrder.map((risk) => {
            const c = grouped[risk]?.length || 0;
            return c > 0 ? (
              <span key={risk} className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(risk))}>
                {risk}: {c}
              </span>
            ) : null;
          })}
        </div>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {alerts.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <ShieldCheck className="w-12 h-12" />
            <p className="text-[13px] font-medium">{t("security.noAlerts")}</p>
          </div>
        ) : (
          <div className="space-y-2">
            {riskOrder.map((risk) =>
              (grouped[risk] || []).map((alert) => (
                <AlertCard
                  key={`${alert.id}-${alert.url}`}
                  alert={alert}
                  riskColor={riskColor}
                  riskIcon={riskIcon}
                />
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}

// ── Helpers ──

function formatSize(bytes: number): string {
  if (bytes === 0) return "-";
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}K`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}M`;
}
