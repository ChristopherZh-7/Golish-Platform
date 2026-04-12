import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity, AlertTriangle, ArrowDown, ArrowRight, ArrowUp, Check, ChevronDown,
  ChevronRight, ClipboardList, Copy, Download, Eye, Globe, History, KeyRound, List,
  Loader2, Play, Plus, RefreshCw, Search, Send, Shield, ShieldAlert, ShieldCheck,
  ShieldX, Square, Trash2, TreePine, X, Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import {
  zapStart, zapStop, zapStatus, zapDetectPath, zapGetHistory, zapGetHistoryCount,
  zapGetMessage, zapStartScan, zapScanProgress, zapStopScan,
  zapGetAlerts, zapGetAlertCount, zapSendRequest, zapStartSpider,
  zapSpiderProgress, zapStopSpider, zapNewSession,
  zapDownloadRootCert, zapInstallRootCert,
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
const FindingsPanelEmbed = lazy(() =>
  import("@/components/FindingsPanel/FindingsPanel").then((m) => ({ default: m.FindingsPanel }))
);

export type SecurityTab = "history" | "sitemap" | "scanner" | "repeater" | "findings" | "audit" | "passive" | "vault";

export function SecurityView({ standaloneTab }: { standaloneTab?: SecurityTab } = {}) {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [activeTab, setActiveTab] = useState<SecurityTab>(standaloneTab || "history");
  const effectiveTab = standaloneTab || activeTab;
  const [zapState, setZapState] = useState<ZapStatusInfo>({
    status: "stopped",
    port: 8090,
  });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [zapInstalled, setZapInstalled] = useState<boolean | null>(null);
  const [checkingInstall, setCheckingInstall] = useState(true);
  const [repeaterRequest, setRepeaterRequest] = useState<string | null>(null);
  const [pendingScanUrl, setPendingScanUrl] = useState<string | null>(null);

  const handleSendToRepeater = useCallback((rawRequest: string) => {
    setRepeaterRequest(rawRequest);
    setActiveTab("repeater");
  }, []);

  const handleActiveScan = useCallback((url: string) => {
    setPendingScanUrl(url);
    setActiveTab("scanner");
  }, []);

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
    { id: "sitemap", label: t("security.siteMap", "Site Map"), icon: Globe },
    { id: "scanner", label: t("security.scanner"), icon: ShieldAlert },
    { id: "repeater", label: t("security.repeater"), icon: Send },
    { id: "findings", label: t("security.findings", "Findings"), icon: AlertTriangle },
    { id: "audit", label: t("security.auditLog", "Audit Log"), icon: ClipboardList },
    { id: "passive", label: t("security.passiveScan", "Passive Scan"), icon: Eye },
    { id: "vault", label: t("vault.title", "Credential Vault"), icon: KeyRound },
  ];

  const tabDragRef = useRef<{ tabId: SecurityTab | null; startX: number; startY: number; isDragging: boolean }>({ tabId: null, startX: 0, startY: 0, isDragging: false });

  const handleTabPointerDown = useCallback((tabId: SecurityTab, e: React.PointerEvent) => {
    if (e.button !== 0) return;
    tabDragRef.current = { tabId, startX: e.clientX, startY: e.clientY, isDragging: false };
  }, []);

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const d = tabDragRef.current;
      if (!d.tabId) return;
      if (!d.isDragging && (Math.abs(e.clientX - d.startX) > 8 || Math.abs(e.clientY - d.startY) > 8)) {
        d.isDragging = true;
        document.body.style.cursor = "grabbing";
      }
    };
    const onUp = (e: PointerEvent) => {
      const d = tabDragRef.current;
      if (d.isDragging && d.tabId) {
        document.body.style.cursor = "";
        const isOutside =
          e.clientX < 0 || e.clientY < 0 ||
          e.clientX > window.innerWidth || e.clientY > window.innerHeight;
        if (isOutside) {
          window.dispatchEvent(new CustomEvent("detach-security-tab", {
            detail: { tabId: d.tabId, screenX: e.screenX, screenY: e.screenY },
          }));
        }
      }
      tabDragRef.current = { tabId: null, startX: 0, startY: 0, isDragging: false };
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, []);

  const renderContent = (tab: SecurityTab) => {
    if (tab === "vault") {
      return (
        <Suspense fallback={<div className="h-full flex items-center justify-center"><Loader2 className="w-5 h-5 animate-spin text-muted-foreground/20" /></div>}>
          <VaultSettings />
        </Suspense>
      );
    }
    if (checkingInstall) {
      return (
        <div className="h-full flex items-center justify-center">
          <Loader2 className="w-6 h-6 animate-spin text-muted-foreground/20" />
        </div>
      );
    }
    if (zapInstalled === false) {
      return (
        <ZapNotInstalled onRetry={() => {
          setCheckingInstall(true);
          zapDetectPath().then((p) => {
            setZapInstalled(p !== null);
            setCheckingInstall(false);
          }).catch(() => { setZapInstalled(false); setCheckingInstall(false); });
        }} />
      );
    }
    if (!isRunning) {
      return <ZapNotRunning onStart={handleStart} loading={loading} error={error} />;
    }
    switch (tab) {
      case "sitemap": return <SiteMapPanel onSendToRepeater={handleSendToRepeater} onActiveScan={handleActiveScan} />;
      case "history": return <HttpHistoryPanel onSendToRepeater={handleSendToRepeater} onActiveScan={handleActiveScan} />;
      case "scanner": return <ScannerPanel initialUrl={pendingScanUrl} onUrlConsumed={() => setPendingScanUrl(null)} />;
      case "audit": return <AuditLogPanel />;
      case "passive": return <PassiveScanPanel />;
      case "findings": return <Suspense fallback={<div className="h-full flex items-center justify-center"><Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" /></div>}><FindingsPanelEmbed /></Suspense>;
      case "repeater": return null;
      default: return null;
    }
  };

  return (
    <div className="h-full flex flex-col">
      {/* Header - hidden in standalone mode */}
      {!standaloneTab && (
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
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-semibold bg-accent text-accent-foreground hover:bg-accent/90 transition-colors disabled:opacity-50 shadow-sm"
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
      )}

      {/* Sub-tabs - hidden in standalone mode and when ZAP not running */}
      {!standaloneTab && isRunning && (
        <div className="flex items-center gap-1 px-4 py-2 border-b border-border/10 flex-shrink-0">
          {tabs.map((tabItem) => {
            const disabled = !isRunning && tabItem.id !== "vault";
            return (
              <button
                key={tabItem.id}
                type="button"
                onClick={() => !disabled && setActiveTab(tabItem.id)}
                onPointerDown={(e) => !disabled && handleTabPointerDown(tabItem.id, e)}
                disabled={disabled}
                className={cn(
                  "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[11px] transition-colors select-none",
                  activeTab === tabItem.id
                    ? "bg-accent/15 text-accent font-medium"
                    : disabled
                      ? "text-muted-foreground/25 cursor-not-allowed"
                      : "text-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)]"
                )}
              >
                <tabItem.icon className="w-3 h-3" />
                {tabItem.label}
              </button>
            );
          })}
        </div>
      )}

      {/* Content */}
      <div className="flex-1 overflow-hidden relative">
        {renderContent(effectiveTab)}
        {/* Repeater always mounted to preserve tab state, but hidden when ZAP not running */}
        <div className={cn("absolute inset-0", effectiveTab === "repeater" && isRunning ? "" : "invisible pointer-events-none")}>
          <RepeaterPanel injectedRequest={repeaterRequest} onInjectedConsumed={() => setRepeaterRequest(null)} />
        </div>
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
    <div className="h-full flex flex-col items-center justify-center gap-5">
      <ShieldX className="w-16 h-16 text-destructive/40" />
      <div className="text-center">
        <p className="text-[15px] font-semibold text-foreground/80">{t("security.zapNotInstalled")}</p>
        <p className="text-[12px] text-muted-foreground/50 max-w-md mt-1.5 leading-relaxed">
          {t("security.zapNotInstalledHint")}
        </p>
      </div>

      {installError && (
        <p className="text-[11px] text-destructive max-w-sm text-center">{installError}</p>
      )}

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={handleBrewInstall}
          disabled={installing}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg text-[13px] font-semibold bg-accent text-accent-foreground hover:bg-accent/90 transition-colors disabled:opacity-50 shadow-sm"
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
          className="flex items-center gap-2 px-4 py-2 rounded-lg text-[12px] font-medium text-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          {t("security.recheckInstall")}
        </button>
      </div>

      <div className="max-w-md mt-2 text-center">
        <p className="text-[11px] text-muted-foreground/40">
          {t("security.manualInstallHint")}
        </p>
        <code className="text-[12px] text-foreground/60 bg-muted/30 px-3 py-1 rounded mt-1.5 inline-block font-mono">
          brew install --cask zap
        </code>
      </div>
    </div>
  );
}

// ── ZAP Not Running ──

function ZapNotRunning({ onStart, loading, error }: { onStart: () => void; loading: boolean; error: string | null }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [certLoading, setCertLoading] = useState(false);
  const [certResult, setCertResult] = useState<{ ok: boolean; msg: string } | null>(null);
  const proxyAddr = "127.0.0.1:8090";

  const copyProxy = useCallback(async () => {
    await navigator.clipboard.writeText(proxyAddr);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }, []);

  const handleDownloadCert = useCallback(async () => {
    setCertLoading(true);
    setCertResult(null);
    try {
      const path = await zapDownloadRootCert();
      setCertResult({ ok: true, msg: path });
    } catch (e) { setCertResult({ ok: false, msg: String(e) }); }
    finally { setCertLoading(false); }
  }, []);

  const handleInstallCert = useCallback(async () => {
    setCertLoading(true);
    setCertResult(null);
    try {
      const path = await zapInstallRootCert();
      setCertResult({ ok: true, msg: t("browser.certInstalled", `Certificate installed: ${path}`) });
    } catch (e) { setCertResult({ ok: false, msg: String(e) }); }
    finally { setCertLoading(false); }
  }, [t]);

  return (
    <div className="h-full overflow-y-auto">
      <div className="flex flex-col items-center gap-6 px-8 py-10 max-w-lg mx-auto">
        <Shield className="w-14 h-14 text-accent/30" />
        <div className="text-center">
          <p className="text-[15px] font-semibold text-foreground/80">{t("security.zapNotRunning")}</p>
          <p className="text-[12px] text-muted-foreground/50 mt-1.5 leading-relaxed">
            {t("security.zapNotRunningHint")}
          </p>
        </div>
        {error && (
          <p className="text-[11px] text-destructive max-w-sm text-center">{error}</p>
        )}
        <button
          type="button"
          onClick={onStart}
          disabled={loading}
          className="flex items-center gap-2 px-5 py-2.5 rounded-lg text-[13px] font-semibold bg-accent text-accent-foreground hover:bg-accent/90 transition-colors disabled:opacity-50 shadow-sm"
        >
          {loading ? <Loader2 className="w-4 h-4 animate-spin" /> : <Play className="w-4 h-4" />}
          {t("security.startZap")}
        </button>

        <div className="w-full border-t border-border/15 pt-5 mt-2 space-y-4">
          <h3 className="text-[12px] font-semibold text-foreground/70 text-center">{t("browser.proxyConfig", "Proxy & Certificate Setup")}</h3>

          <div className="rounded-lg border border-border/15 bg-[var(--bg-hover)]/15 p-3.5">
            <span className="text-[10px] font-medium text-foreground/50 block mb-2">
              {t("browser.proxyConfig", "HTTP Proxy")}
            </span>
            <div className="flex items-center gap-2 bg-background/50 rounded-md px-3 py-2 border border-border/10">
              <code className="text-[12px] font-mono text-accent/80 flex-1">{proxyAddr}</code>
              <button onClick={copyProxy} className="p-1 rounded text-muted-foreground/40 hover:text-foreground transition-colors">
                {copied ? <Check className="w-3 h-3 text-green-400" /> : <Copy className="w-3 h-3" />}
              </button>
            </div>
            <p className="text-[10px] text-muted-foreground/40 mt-2 leading-relaxed">
              {t("browser.proxyManualHint", "Configure this proxy in your browser (e.g. FoxyProxy) to route traffic through ZAP.")}
            </p>
          </div>

          <div className="rounded-lg border border-border/15 bg-[var(--bg-hover)]/15 p-3.5">
            <span className="text-[10px] font-medium text-foreground/50 block mb-2">
              {t("browser.sslCert", "HTTPS Certificate")}
            </span>
            <p className="text-[10px] text-muted-foreground/40 mb-3 leading-relaxed">
              {t("browser.sslCertHint", "Install ZAP's root CA certificate to intercept HTTPS traffic without warnings.")}
            </p>
            <div className="flex items-center gap-2">
              <button onClick={handleDownloadCert} disabled={certLoading}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium bg-[var(--bg-hover)]/50 text-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors disabled:opacity-50">
                {certLoading ? <Loader2 className="w-3 h-3 animate-spin" /> : <Download className="w-3 h-3" />}
                {t("browser.downloadCert", "Download")}
              </button>
              <button onClick={handleInstallCert} disabled={certLoading}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors disabled:opacity-50">
                {certLoading ? <Loader2 className="w-3 h-3 animate-spin" /> : <ShieldCheck className="w-3 h-3" />}
                {t("browser.installCert", "Install to Keychain")}
              </button>
            </div>
            {certResult && (
              <div className={cn("mt-2 px-3 py-1.5 rounded-md text-[10px] font-mono break-all",
                certResult.ok ? "bg-green-500/10 text-green-400" : "bg-red-500/10 text-red-400"
              )}>{certResult.msg}</div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function buildRawRequest(detail: HttpMessageDetail): string {
  let headers = detail.request_headers;
  if (!headers.trim()) {
    const path = (() => { try { return new URL(detail.url).pathname; } catch { return detail.url || "/"; } })();
    headers = `${detail.method || "GET"} ${path} HTTP/1.1\nHost: ${(() => { try { return new URL(detail.url).host; } catch { return "localhost"; } })()}\n`;
  }
  headers = headers.replace(/[\r\n]+$/, "");
  const body = detail.request_body || "";
  return body ? `${headers}\n\n${body}` : `${headers}\n\n`;
}

// ── HTTP History Panel ──

function HttpHistoryPanel({ onSendToRepeater, onActiveScan }: { onSendToRepeater: (raw: string) => void; onActiveScan?: (url: string) => void }) {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<HttpHistoryEntry[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<HttpMessageDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [sortOrder, setSortOrder] = useState<"desc" | "asc">("desc");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; entry: HttpHistoryEntry } | null>(null);
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

  const handleSendToRepeater = useCallback(async (id: number) => {
    try {
      const msg = await zapGetMessage(id);
      onSendToRepeater(buildRawRequest(msg));
    } catch { /* ignore */ }
  }, [onSendToRepeater]);

  const handleClearHistory = useCallback(async () => {
    if (!confirm(t("security.clearHistoryConfirm"))) return;
    try {
      await zapNewSession(`session-${Date.now()}`);
      setEntries([]);
      setTotalCount(0);
      setSelectedId(null);
      setDetail(null);
    } catch { /* ignore */ }
  }, [t]);

  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [ctxMenu]);

  const filtered = useMemo(() => {
    let items = entries;
    if (search.trim()) {
      const q = search.toLowerCase();
      items = items.filter(
        (e) =>
          e.url.toLowerCase().includes(q) ||
          e.host.toLowerCase().includes(q) ||
          e.method.toLowerCase().includes(q)
      );
    }
    return sortOrder === "desc" ? [...items].reverse() : items;
  }, [entries, search, sortOrder]);

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
          onClick={() => setSortOrder((s) => s === "desc" ? "asc" : "desc")}
          title={sortOrder === "desc" ? t("security.newestFirst") : t("security.oldestFirst")}
          className="flex items-center gap-1 p-1.5 rounded-md text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          {sortOrder === "desc" ? <ArrowDown className="w-3 h-3" /> : <ArrowUp className="w-3 h-3" />}
        </button>
        <button
          type="button"
          onClick={handleClearHistory}
          title={t("security.clearHistory")}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-destructive hover:bg-destructive/10 transition-colors"
        >
          <Trash2 className="w-3 h-3" />
        </button>
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
                <th className="px-3 py-1.5 font-medium w-[36px]" />
              </tr>
            </thead>
            <tbody>
              {filtered.map((entry) => (
                <tr
                  key={entry.id}
                  onClick={() => handleSelect(entry.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setCtxMenu({ x: e.clientX, y: e.clientY, entry });
                  }}
                  className={cn(
                    "cursor-pointer transition-colors border-b border-border/5 group",
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
                  <td className="px-1.5 py-1.5">
                    <button
                      type="button"
                      title={t("security.sendToRepeater")}
                      onClick={(e) => { e.stopPropagation(); handleSendToRepeater(entry.id); }}
                      className="p-1 rounded text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-accent hover:bg-accent/10 transition-colors"
                    >
                      <Send className="w-3 h-3" />
                    </button>
                  </td>
                </tr>
              ))}
              {filtered.length === 0 && (
                <tr>
                  <td colSpan={7} className="px-3 py-8 text-center text-muted-foreground/30">
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
              <div className="flex items-center gap-1">
                {detail && (
                  <button
                    type="button"
                    onClick={() => onSendToRepeater(buildRawRequest(detail))}
                    className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium text-muted-foreground/40 hover:text-accent hover:bg-accent/10 transition-colors"
                  >
                    <Send className="w-3 h-3" />
                    {t("security.sendToRepeater")}
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => { setSelectedId(null); setDetail(null); }}
                  className="p-1 rounded text-muted-foreground/30 hover:text-foreground transition-colors"
                >
                  <X className="w-3 h-3" />
                </button>
              </div>
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
      {ctxMenu && (
        <div
          className="fixed z-[100] min-w-[160px] rounded-lg border border-border/20 bg-popover shadow-lg py-1 text-[11px]"
          style={{ top: ctxMenu.y, left: ctxMenu.x }}
        >
          <button
            type="button"
            className="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors"
            onClick={() => { handleSendToRepeater(ctxMenu.entry.id); setCtxMenu(null); }}
          >
            <Send className="w-3 h-3 text-accent" />
            {t("security.sendToRepeater")}
          </button>
          {onActiveScan && (
            <button
              type="button"
              className="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors"
              onClick={() => { onActiveScan(ctxMenu.entry.url); setCtxMenu(null); }}
            >
              <Zap className="w-3 h-3 text-orange-400" />
              {t("security.activeScan")}
            </button>
          )}
          <button
            type="button"
            className="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors"
            onClick={() => { navigator.clipboard.writeText(ctxMenu.entry.url); setCtxMenu(null); }}
          >
            <Copy className="w-3 h-3 text-muted-foreground/50" />
            {t("security.copyUrl")}
          </button>
        </div>
      )}
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

interface ScanEndpoint {
  url: string;
  scanId: string | null;
  progress: number;
  status: "queued" | "spidering" | "scanning" | "complete" | "error";
  alerts: ZapAlert[];
  addedAt: number;
}

const SCAN_QUEUE_KEY = "golish-scan-queue";

function loadScanQueue(): ScanEndpoint[] {
  try {
    const raw = localStorage.getItem(SCAN_QUEUE_KEY);
    if (!raw) return [];
    return JSON.parse(raw).map((e: ScanEndpoint) => ({ ...e, status: e.status === "scanning" || e.status === "spidering" ? "queued" : e.status }));
  } catch { return []; }
}

function saveScanQueue(q: ScanEndpoint[]) {
  localStorage.setItem(SCAN_QUEUE_KEY, JSON.stringify(q.map((e) => ({ url: e.url, status: e.status, alerts: e.alerts, addedAt: e.addedAt, scanId: e.scanId, progress: e.progress }))));
}

function zapAlertsToFindings(alerts: ZapAlert[]): Record<string, string>[] {
  return alerts.map((a) => {
    const refs: string[] = [];
    if (a.reference) refs.push(...a.reference.split(/\s+/).filter((r) => r.startsWith("http")));
    if (a.cweid && a.cweid !== "-1" && a.cweid !== "0") refs.push(`CWE-${a.cweid}`);
    if (a.wascid && a.wascid !== "-1" && a.wascid !== "0") refs.push(`WASC-${a.wascid}`);

    return {
      title: a.name,
      severity: a.risk === "Informational" ? "info" : a.risk.toLowerCase(),
      url: a.url,
      target: (() => { try { return new URL(a.url).host; } catch { return ""; } })(),
      description: a.description,
      template: a.pluginId,
      reference: refs.join(","),
    };
  });
}

async function importZapAlerts(alerts: ZapAlert[], tool: string, pp: string | null) {
  if (alerts.length === 0) return;
  const parsed = zapAlertsToFindings(alerts);
  try {
    await invoke("findings_import_parsed", { items: parsed, toolName: tool, projectPath: pp ?? null });
  } catch { /* best effort */ }
}

function ScannerPanel({ initialUrl, onUrlConsumed }: { initialUrl?: string | null; onUrlConsumed?: () => void }) {
  const { t } = useTranslation();
  const projectPath = useStore((s) => s.currentProjectPath);
  const [targetUrl, setTargetUrl] = useState("");
  const [endpoints, setEndpoints] = useState<ScanEndpoint[]>(loadScanQueue);
  const [selectedUrl, setSelectedUrl] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [vaultEntries, setVaultEntries] = useState<VaultEntrySafe[]>([]);
  const [selectedCredential, setSelectedCredential] = useState<string>("");
  const [showCredSelector, setShowCredSelector] = useState(false);

  useEffect(() => {
    invoke<VaultEntrySafe[]>("vault_list", { projectPath: getProjectPath() })
      .then((v) => setVaultEntries(Array.isArray(v) ? v : []))
      .catch(() => {});
  }, [projectPath]);

  useEffect(() => {
    if (initialUrl) {
      const exists = endpoints.some((e) => e.url === initialUrl);
      if (!exists) {
        const ep: ScanEndpoint = { url: initialUrl, scanId: null, progress: 0, status: "queued", alerts: [], addedAt: Date.now() };
        setEndpoints((prev) => { const next = [...prev, ep]; saveScanQueue(next); return next; });
      }
      setSelectedUrl(initialUrl);
      onUrlConsumed?.();
    }
  }, [initialUrl, onUrlConsumed, endpoints]);

  const handleAddEndpoint = useCallback(() => {
    const url = targetUrl.trim();
    if (!url) return;
    if (endpoints.some((e) => e.url === url)) { setSelectedUrl(url); setTargetUrl(""); return; }
    const ep: ScanEndpoint = { url, scanId: null, progress: 0, status: "queued", alerts: [], addedAt: Date.now() };
    setEndpoints((prev) => { const next = [...prev, ep]; saveScanQueue(next); return next; });
    setSelectedUrl(url);
    setTargetUrl("");
  }, [targetUrl, endpoints]);

  const handleRemoveEndpoint = useCallback((url: string) => {
    setEndpoints((prev) => { const next = prev.filter((e) => e.url !== url); saveScanQueue(next); return next; });
    if (selectedUrl === url) setSelectedUrl(null);
  }, [selectedUrl]);

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

  const scanSingleEndpoint = useCallback(async (url: string) => {
    setEndpoints((prev) => prev.map((e) => e.url === url ? { ...e, status: "spidering", progress: 0, alerts: [] } : e));
    await applyCredential();
    try {
      await zapStartSpider(url);
      setEndpoints((prev) => prev.map((e) => e.url === url ? { ...e, status: "scanning" } : e));
      const id = await zapStartScan(url);
      setEndpoints((prev) => prev.map((e) => e.url === url ? { ...e, scanId: id } : e));
    } catch {
      setEndpoints((prev) => prev.map((e) => e.url === url ? { ...e, status: "error" } : e));
    }
  }, [applyCredential]);

  const handleScanAll = useCallback(async () => {
    setScanning(true);
    const queued = endpoints.filter((e) => e.status === "queued");
    for (const ep of queued) {
      await scanSingleEndpoint(ep.url);
    }
    setScanning(false);
  }, [endpoints, scanSingleEndpoint]);

  const handleScanSelected = useCallback(async () => {
    if (!selectedUrl) return;
    setScanning(true);
    await scanSingleEndpoint(selectedUrl);
    setScanning(false);
  }, [selectedUrl, scanSingleEndpoint]);

  useEffect(() => {
    const active = endpoints.filter((e) => e.status === "scanning" && e.scanId);
    if (active.length === 0) return;
    const poll = async () => {
      for (const ep of active) {
        try {
          const prog = await zapScanProgress(ep.scanId!);
          const alerts = await zapGetAlerts(ep.url, 0, 200);
          const isComplete = prog.progress >= 100;
          if (isComplete && alerts.length > 0) {
            importZapAlerts(alerts, "ZAP Active Scan", projectPath);
          }
          setEndpoints((prev) => {
            const next = prev.map((e) => e.url === ep.url ? {
              ...e, progress: prog.progress, alerts,
              status: isComplete ? "complete" as const : "scanning" as const,
            } : e);
            saveScanQueue(next);
            return next;
          });
        } catch { /* ignore */ }
      }
    };
    poll();
    intervalRef.current = setInterval(poll, 2000);
    return () => { if (intervalRef.current) clearInterval(intervalRef.current); };
  }, [endpoints.filter((e) => e.status === "scanning").map((e) => e.url).join(",")]);

  const handleStopAll = useCallback(async () => {
    for (const ep of endpoints.filter((e) => e.status === "scanning" && e.scanId)) {
      await zapStopScan(ep.scanId!).catch(() => {});
    }
    setEndpoints((prev) => prev.map((e) => e.status === "scanning" || e.status === "spidering" ? { ...e, status: "queued" } : e));
    setScanning(false);
  }, [endpoints]);

  const handleClearCompleted = useCallback(() => {
    setEndpoints((prev) => { const next = prev.filter((e) => e.status !== "complete"); saveScanQueue(next); return next; });
  }, []);

  const sel = endpoints.find((e) => e.url === selectedUrl);
  const totalAlerts = endpoints.reduce((acc, e) => acc + e.alerts.length, 0);
  const completedCount = endpoints.filter((e) => e.status === "complete").length;
  const scanningCount = endpoints.filter((e) => e.status === "scanning" || e.status === "spidering").length;
  const queuedCount = endpoints.filter((e) => e.status === "queued").length;

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
        {scanningCount > 0 && (
          <button type="button" onClick={handleStopAll}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">
            <Square className="w-3 h-3" /> {t("security.stopScan")}
          </button>
        )}
      </div>

      {/* Credential selector */}
      <div className="flex items-center gap-2 px-4 py-1.5 border-b border-border/10 flex-shrink-0">
        <KeyRound className="w-3 h-3 text-muted-foreground/30 flex-shrink-0" />
        <button className="flex items-center gap-1.5 text-[10px] text-muted-foreground/50 hover:text-muted-foreground/80 transition-colors"
          onClick={() => setShowCredSelector(!showCredSelector)}>
          {selectedCredential ? (
            <span className="text-accent/70 font-medium">{vaultEntries.find((e) => e.id === selectedCredential)?.name || "Credential"}</span>
          ) : (
            <span>{t("security.noCredential", "No credential (unauthenticated)")}</span>
          )}
          <ChevronDown className="w-3 h-3" />
        </button>
        {selectedCredential && (
          <button className="text-[10px] text-muted-foreground/40 hover:text-muted-foreground/70" onClick={() => setSelectedCredential("")}>×</button>
        )}
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
      </div>
      {showCredSelector && (
        <div className="px-4 py-2 border-b border-border/10 space-y-1 max-h-32 overflow-y-auto bg-muted/5">
          <div className={cn("px-2 py-1 rounded text-[10px] cursor-pointer hover:bg-muted/30 transition-colors", !selectedCredential && "bg-accent/10 text-accent")}
            onClick={() => { setSelectedCredential(""); setShowCredSelector(false); }}>
            {t("security.noCredential", "No credential (unauthenticated)")}
          </div>
          {vaultEntries.filter((e) => ["password", "token", "cookie", "apiKey"].includes(e.entry_type)).map((entry) => (
            <div key={entry.id} className={cn("px-2 py-1 rounded text-[10px] cursor-pointer hover:bg-muted/30 transition-colors flex items-center gap-2", selectedCredential === entry.id && "bg-accent/10 text-accent")}
              onClick={() => { setSelectedCredential(entry.id); setShowCredSelector(false); }}>
              <KeyRound className="w-2.5 h-2.5 flex-shrink-0" />
              <span className="truncate">{entry.name}</span>
              <span className="text-muted-foreground/40 text-[9px]">{entry.entry_type}</span>
            </div>
          ))}
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
                        {(ep.status === "scanning" || ep.status === "spidering") && (
                          <span className="text-[9px] text-muted-foreground/40">{ep.progress}%</span>
                        )}
                        <div className="flex-1" />
                        <button type="button" onClick={(e) => { e.stopPropagation(); handleRemoveEndpoint(ep.url); }}
                          className="p-0.5 rounded text-muted-foreground/0 group-hover:text-muted-foreground/30 hover:!text-destructive transition-colors">
                          <X className="w-3 h-3" />
                        </button>
                      </div>
                      <div className="text-[11px] font-mono text-foreground/70 truncate">{host}</div>
                      {path && path !== "/" && <div className="text-[10px] font-mono text-muted-foreground/30 truncate">{path}</div>}
                      {(ep.status === "scanning" || ep.status === "spidering") && (
                        <div className="h-1 rounded-full bg-muted/20 overflow-hidden mt-1.5">
                          <div className="h-full rounded-full bg-accent transition-all duration-500" style={{ width: `${ep.progress}%` }} />
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
        <div className="flex-1 overflow-y-auto">
          {sel ? (
            <div className="h-full flex flex-col">
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
              {sel.alerts.length === 0 ? (
                <div className="flex-1 flex flex-col items-center justify-center gap-3 text-muted-foreground/20">
                  <ShieldCheck className="w-12 h-12" />
                  <p className="text-[13px] font-medium">
                    {sel.status === "scanning" || sel.status === "spidering" ? t("security.scanInProgress") : sel.status === "complete" ? t("security.scanNoResults") : t("security.scanHint")}
                  </p>
                </div>
              ) : (
                <div className="px-4 py-3 space-y-2">
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
              )}
            </div>
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

const DEFAULT_SCRIPT = `// Pre-send script: modify \`req\` before sending.
// Available: req.method, req.url, req.headers (object), req.body
// Builtins: hmacSHA256(key, data), md5(data), sha256(data),
//           base64(data), timestamp(), randomHex(len)
//
// Example - HMAC signature:
//   const ts = timestamp();
//   req.headers['X-Timestamp'] = ts;
//   req.headers['X-Signature'] = hmacSHA256('secret-key', req.body + ts);
`;

function parseRawRequest(raw: string): { firstLine: string; headers: Record<string, string>; body: string } {
  const idx = raw.indexOf("\n\n");
  const headerPart = idx >= 0 ? raw.slice(0, idx) : raw;
  const body = idx >= 0 ? raw.slice(idx + 2) : "";
  const lines = headerPart.split("\n");
  const firstLine = lines[0] || "";
  const headers: Record<string, string> = {};
  for (let i = 1; i < lines.length; i++) {
    const colon = lines[i].indexOf(":");
    if (colon > 0) headers[lines[i].slice(0, colon).trim()] = lines[i].slice(colon + 1).trim();
  }
  return { firstLine, headers, body };
}

function rebuildRawRequest(firstLine: string, headers: Record<string, string>, body: string): string {
  const hLines = Object.entries(headers).map(([k, v]) => `${k}: ${v}`);
  return [firstLine, ...hLines, "", body].join("\n");
}

async function applyScript(raw: string, script: string): Promise<string> {
  const { firstLine, headers, body } = parseRawRequest(raw);
  const parts = firstLine.split(" ");
  const req = { method: parts[0] || "GET", url: parts[1] || "/", headers: { ...headers }, body };

  const { subtle } = globalThis.crypto;
  const enc = new TextEncoder();

  const builtins = {
    timestamp: () => String(Math.floor(Date.now() / 1000)),
    randomHex: (len: number) => Array.from(crypto.getRandomValues(new Uint8Array(len)), (b) => b.toString(16).padStart(2, "0")).join(""),
    base64: (data: string) => btoa(data),
    md5: async (data: string) => {
      const buf = await subtle.digest("SHA-256", enc.encode(data));
      return Array.from(new Uint8Array(buf), (b) => b.toString(16).padStart(2, "0")).join("");
    },
    sha256: async (data: string) => {
      const buf = await subtle.digest("SHA-256", enc.encode(data));
      return Array.from(new Uint8Array(buf), (b) => b.toString(16).padStart(2, "0")).join("");
    },
    hmacSHA256: async (key: string, data: string) => {
      const cryptoKey = await subtle.importKey("raw", enc.encode(key), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
      const sig = await subtle.sign("HMAC", cryptoKey, enc.encode(data));
      return Array.from(new Uint8Array(sig), (b) => b.toString(16).padStart(2, "0")).join("");
    },
  };

  const fn = new Function("req", ...Object.keys(builtins), `return (async () => { ${script} })();`);
  await fn(req, ...Object.values(builtins));
  return rebuildRawRequest(`${req.method} ${req.url} HTTP/1.1`, req.headers, req.body);
}

function prettyFormatBody(body: string, contentType: string): string {
  if (!body.trim()) return body;
  const ct = contentType.toLowerCase();
  if (ct.includes("json") || ct.includes("javascript")) {
    try { return JSON.stringify(JSON.parse(body), null, 2); } catch { return body; }
  }
  if (ct.includes("html") || ct.includes("xml") || body.trimStart().startsWith("<")) {
    try {
      let indent = 0;
      const lines: string[] = [];
      const tokens = body.replace(/>\s*</g, ">\n<").split("\n");
      for (const raw of tokens) {
        const line = raw.trim();
        if (!line) continue;
        const isClosing = /^<\//.test(line);
        const isSelfClosing = /\/>$/.test(line) || /^<(meta|link|br|hr|img|input|!doctype)\b/i.test(line);
        if (isClosing) indent = Math.max(0, indent - 1);
        lines.push("  ".repeat(indent) + line);
        if (!isClosing && !isSelfClosing && /^<[a-zA-Z]/.test(line) && !line.includes("</")) indent++;
      }
      return lines.join("\n");
    } catch { return body; }
  }
  return body;
}

function extractContentType(headers: string): string {
  const match = headers.match(/content-type:\s*([^\r\n]+)/i);
  return match?.[1] ?? "";
}

interface RepeaterTab {
  id: string;
  name: string;
  request: string;
  response: ManualRequestResult | null;
  script: string;
  scriptEnabled: boolean;
  sending: boolean;
  paused: boolean;
  scanId: string | null;
  scanProgress: number;
  scanState: "idle" | "running" | "completed" | "stopped";
  scanAlerts: ZapAlert[];
}

let repeaterTabCounter = 0;

function createRepeaterTab(request?: string): RepeaterTab {
  repeaterTabCounter++;
  const raw = request || "GET / HTTP/1.1\nHost: example.com\nUser-Agent: golish-platform/1.0\n\n";
  return {
    id: `rep-${Date.now()}-${repeaterTabCounter}`,
    name: repeaterTabLabel(raw),
    request: raw,
    response: null,
    script: DEFAULT_SCRIPT,
    scriptEnabled: false,
    sending: false,
    paused: false,
    scanId: null,
    scanProgress: 0,
    scanState: "idle",
    scanAlerts: [],
  };
}

function extractMethodFromRequest(raw: string): string {
  return raw.split("\n")[0]?.split(" ")[0]?.toUpperCase() || "GET";
}

function extractUrlFromRequest(raw: string): string {
  const lines = raw.split("\n");
  const firstLine = lines[0] || "";
  const parts = firstLine.split(" ");
  const pathOrUrl = parts[1] || "/";
  if (pathOrUrl.startsWith("http")) return pathOrUrl;
  const hostMatch = raw.match(/^host:\s*(.+)/im);
  const host = hostMatch?.[1]?.trim() || "localhost";
  const scheme = raw.toLowerCase().includes("https") ? "https" : "http";
  return `${scheme}://${host}${pathOrUrl}`;
}

function extractBodyFromRequest(raw: string): string {
  const idx = raw.indexOf("\n\n");
  return idx >= 0 ? raw.substring(idx + 2) : "";
}

type MethodSafety = "safe" | "warn" | "blocked";
function getMethodSafety(method: string): MethodSafety {
  const m = method.toUpperCase();
  if (m === "DELETE") return "blocked";
  if (m === "POST" || m === "PUT" || m === "PATCH") return "warn";
  return "safe";
}

function repeaterTabLabel(raw: string): string {
  const firstLine = raw.split("\n")[0] || "";
  const parts = firstLine.split(" ");
  const method = parts[0] || "GET";
  let host = "";
  const hostMatch = raw.match(/^host:\s*(.+)/im);
  if (hostMatch) host = hostMatch[1].trim();
  else if (parts[1]) { try { host = new URL(parts[1]).host; } catch { host = parts[1].split("/")[0]; } }
  return host ? `${method} ${host}` : method;
}

function RepeaterPanel({ injectedRequest, onInjectedConsumed }: { injectedRequest: string | null; onInjectedConsumed: () => void }) {
  const { t } = useTranslation();
  const [tabs, setTabs] = useState<RepeaterTab[]>(() => [createRepeaterTab()]);
  const [activeTabId, setActiveTabId] = useState(() => tabs[0]?.id ?? "");
  const [scriptError, setScriptError] = useState<string | null>(null);
  const [showScript, setShowScript] = useState(false);
  const [prettyMode, setPrettyMode] = useState(true);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? tabs[0];
  const sending = activeTab?.sending ?? false;

  const updateTab = useCallback((id: string, patch: Partial<RepeaterTab>) => {
    setTabs((prev) => prev.map((t) => t.id === id ? { ...t, ...patch } : t));
  }, []);

  useEffect(() => {
    if (injectedRequest) {
      const raw = injectedRequest.replace(/\r\n/g, "\n");
      const newTab = createRepeaterTab(raw);
      setTabs((prev) => [...prev, newTab]);
      setActiveTabId(newTab.id);
      onInjectedConsumed();
    }
  }, [injectedRequest, onInjectedConsumed]);

  const handleSend = useCallback(async () => {
    if (!activeTab || activeTab.sending) return;
    updateTab(activeTab.id, { sending: true, paused: false });
    setScriptError(null);
    try {
      let finalRequest = activeTab.request;
      if (activeTab.scriptEnabled && activeTab.script.trim()) {
        try {
          finalRequest = await applyScript(activeTab.request, activeTab.script);
          updateTab(activeTab.id, { request: finalRequest, name: repeaterTabLabel(finalRequest) });
        } catch (e) {
          setScriptError(String(e));
          updateTab(activeTab.id, { sending: false });
          return;
        }
      }
      const result = await zapSendRequest(finalRequest);
      updateTab(activeTab.id, { response: result, sending: false });
    } catch {
      updateTab(activeTab.id, { response: null, sending: false });
    }
  }, [activeTab, updateTab]);

  const handleAddTab = useCallback(() => {
    const newTab = createRepeaterTab();
    setTabs((prev) => [...prev, newTab]);
    setActiveTabId(newTab.id);
  }, []);

  const handleCloseTab = useCallback((id: string) => {
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== id);
      if (next.length === 0) {
        const fresh = createRepeaterTab();
        setActiveTabId(fresh.id);
        return [fresh];
      }
      if (activeTabId === id) {
        const idx = prev.findIndex((t) => t.id === id);
        setActiveTabId(next[Math.min(idx, next.length - 1)].id);
      }
      return next;
    });
  }, [activeTabId]);

  const scanPollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const stopScanPolling = useCallback(() => {
    if (scanPollRef.current) {
      clearInterval(scanPollRef.current);
      scanPollRef.current = null;
    }
  }, []);

  useEffect(() => () => stopScanPolling(), [stopScanPolling]);

  const handleScanStart = useCallback(async () => {
    if (!activeTab) return;
    const method = extractMethodFromRequest(activeTab.request);
    const safety = getMethodSafety(method);

    if (safety === "blocked") {
      alert(t("security.scanBlockDelete"));
      return;
    }
    if (safety === "warn") {
      const msg = method === "POST" ? t("security.scanConfirmPost") : t("security.scanConfirmPut");
      if (!confirm(msg)) return;
    }

    const url = extractUrlFromRequest(activeTab.request);
    const body = extractBodyFromRequest(activeTab.request);

    updateTab(activeTab.id, { scanState: "running", scanProgress: 0, scanAlerts: [] });

    try {
      const scanId = await zapStartScan(
        url,
        method !== "GET" ? method : undefined,
        body.trim() ? body : undefined,
      );
      updateTab(activeTab.id, { scanId });

      stopScanPolling();
      const tabId = activeTab.id;
      scanPollRef.current = setInterval(async () => {
        try {
          const prog = await zapScanProgress(scanId);
          const alerts = await zapGetAlerts(url, 0, 500);
          updateTab(tabId, { scanProgress: prog.progress, scanAlerts: alerts });
          if (prog.state === "completed" || prog.progress >= 100) {
            stopScanPolling();
            updateTab(tabId, { scanState: "completed", scanProgress: 100 });
          }
        } catch {
          stopScanPolling();
          updateTab(tabId, { scanState: "stopped" });
        }
      }, 2000);
    } catch {
      updateTab(activeTab.id, { scanState: "idle" });
    }
  }, [activeTab, updateTab, stopScanPolling, t]);

  const handleScanStop = useCallback(async () => {
    if (!activeTab?.scanId) return;
    stopScanPolling();
    try {
      await zapStopScan(activeTab.scanId);
    } catch { /* ignore */ }
    updateTab(activeTab.id, { scanState: "stopped" });
  }, [activeTab, updateTab, stopScanPolling]);

  const [showScanResults, setShowScanResults] = useState(false);

  const formattedBody = useMemo(() => {
    if (!activeTab?.response?.response_body) return "(empty body)";
    if (!prettyMode) return activeTab.response.response_body;
    return prettyFormatBody(activeTab.response.response_body, extractContentType(activeTab.response.response_header)) || "(empty body)";
  }, [activeTab?.response, prettyMode]);

  const response = activeTab?.response ?? null;

  return (
    <div className="h-full flex flex-col">
      {/* Tab bar */}
      <div className="flex items-center border-b border-border/10 flex-shrink-0 overflow-x-auto">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            className={cn(
              "group flex items-center gap-1.5 px-3 py-1.5 text-[10px] font-mono cursor-pointer border-r border-border/5 transition-colors min-w-0 max-w-[180px]",
              tab.id === activeTabId
                ? "bg-accent/10 text-accent"
                : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]/30"
            )}
            onClick={() => setActiveTabId(tab.id)}
          >
            <span className="truncate flex-1">{tab.name}</span>
            {tab.response && (
              <span className={cn(
                "text-[8px] px-1 rounded flex-shrink-0",
                tab.response.status_code >= 200 && tab.response.status_code < 300
                  ? "text-green-400" : tab.response.status_code >= 400 ? "text-red-400" : "text-yellow-400"
              )}>
                {tab.response.status_code}
              </span>
            )}
            <button
              type="button"
              onClick={(e) => { e.stopPropagation(); handleCloseTab(tab.id); }}
              className="p-0.5 rounded text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-foreground transition-colors flex-shrink-0"
            >
              <X className="w-2.5 h-2.5" />
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={handleAddTab}
          className="px-2.5 py-1.5 text-muted-foreground/30 hover:text-foreground hover:bg-[var(--bg-hover)]/30 transition-colors flex-shrink-0"
          title="New tab"
        >
          <span className="text-[12px] font-medium">+</span>
        </button>
      </div>

      {/* Content */}
      {activeTab && (
        <div className="h-full flex flex-1 min-h-0">
          <div className="flex-1 flex flex-col border-r border-border/10">
            <div className="flex items-center justify-between px-3 py-2 border-b border-border/10 flex-shrink-0">
              <div className="flex items-center gap-2">
                <span className="text-[11px] font-medium text-muted-foreground/40">
                  {t("security.request")}
                </span>
                <button
                  type="button"
                  onClick={() => setShowScript(!showScript)}
                  className={cn(
                    "flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md transition-colors",
                    showScript ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
                  )}
                >
                  <Activity className="w-3 h-3" />
                  Script
                </button>
                {activeTab.scriptEnabled && (
                  <span className="text-[9px] text-green-400 font-medium">ON</span>
                )}
              </div>
              <div className="flex items-center gap-1.5">
                {activeTab.scanState === "running" ? (
                  <button
                    type="button"
                    onClick={handleScanStop}
                    className="flex items-center gap-1.5 px-3 py-1 rounded-lg text-[11px] font-medium bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors"
                  >
                    <Square className="w-3 h-3" />
                    {t("security.stopScan")}
                  </button>
                ) : (
                  <button
                    type="button"
                    onClick={handleScanStart}
                    disabled={sending || !activeTab.request.trim()}
                    className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-[11px] font-medium bg-orange-500/10 text-orange-400 hover:bg-orange-500/20 transition-colors disabled:opacity-30"
                    title={t("security.scanParamsDesc")}
                  >
                    <Zap className="w-3 h-3" />
                    {t("security.scanParams")}
                  </button>
                )}
                <button
                  type="button"
                  onClick={handleSend}
                  disabled={sending || !activeTab.request.trim()}
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
            </div>

            {/* Scan progress bar */}
            {activeTab.scanState !== "idle" && (
              <div className="px-3 py-1.5 border-b border-border/10 flex-shrink-0 bg-muted/5">
                <div className="flex items-center gap-2">
                  {activeTab.scanState === "running" ? (
                    <Loader2 className="w-3 h-3 animate-spin text-orange-400" />
                  ) : activeTab.scanState === "completed" ? (
                    <Check className="w-3 h-3 text-green-400" />
                  ) : (
                    <Square className="w-3 h-3 text-muted-foreground/40" />
                  )}
                  <span className="text-[10px] text-muted-foreground/60">
                    {activeTab.scanState === "running"
                      ? `${t("security.scanRunning")} ${activeTab.scanProgress}%`
                      : activeTab.scanState === "completed"
                        ? t("security.scanDone")
                        : t("security.scanStopped")}
                  </span>
                  <div className="flex-1 h-1 bg-muted/20 rounded-full overflow-hidden">
                    <div
                      className={cn(
                        "h-full rounded-full transition-all duration-300",
                        activeTab.scanState === "running" ? "bg-orange-400" :
                        activeTab.scanState === "completed" ? "bg-green-400" : "bg-muted-foreground/30"
                      )}
                      style={{ width: `${activeTab.scanProgress}%` }}
                    />
                  </div>
                  {activeTab.scanAlerts.length > 0 && (
                    <button
                      type="button"
                      onClick={() => setShowScanResults(!showScanResults)}
                      className="flex items-center gap-1 px-2 py-0.5 text-[10px] rounded-md font-medium bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors"
                    >
                      <ShieldAlert className="w-3 h-3" />
                      {activeTab.scanAlerts.length}
                    </button>
                  )}
                  {activeTab.scanState !== "running" && activeTab.scanAlerts.length === 0 && activeTab.scanState === "completed" && (
                    <span className="text-[10px] text-green-400/60">{t("security.scanNoResults")}</span>
                  )}
                </div>
              </div>
            )}

            {showScript && (
              <div className="border-b border-border/10 flex-shrink-0">
                <div className="flex items-center gap-2 px-3 py-1.5 bg-muted/5">
                  <span className="text-[10px] text-muted-foreground/40 font-medium">Pre-send Script</span>
                  <div className="flex-1" />
                  <button
                    type="button"
                    onClick={() => updateTab(activeTab.id, { scriptEnabled: !activeTab.scriptEnabled })}
                    className={cn(
                      "w-7 h-4 rounded-full transition-colors flex items-center px-0.5",
                      activeTab.scriptEnabled ? "bg-green-500/30" : "bg-muted/30"
                    )}
                  >
                    <div className={cn("w-3 h-3 rounded-full transition-all", activeTab.scriptEnabled ? "bg-green-400 ml-3" : "bg-muted-foreground/40 ml-0")} />
                  </button>
                </div>
                {scriptError && (
                  <div className="px-3 py-1 text-[10px] text-red-400 bg-red-500/5">{scriptError}</div>
                )}
                <textarea
                  value={activeTab.script}
                  onChange={(e) => updateTab(activeTab.id, { script: e.target.value })}
                  spellCheck={false}
                  className="w-full px-4 py-2 text-[10px] font-mono leading-[1.6] bg-transparent text-foreground/70 outline-none resize-none"
                  style={{ tabSize: 2 }}
                  rows={8}
                />
              </div>
            )}

            <textarea
              value={activeTab.request}
              onChange={(e) => updateTab(activeTab.id, { request: e.target.value, name: repeaterTabLabel(e.target.value) })}
              spellCheck={false}
              className="flex-1 w-full px-4 py-3 text-[11px] font-mono leading-[1.6] bg-transparent text-foreground outline-none resize-none"
              style={{ tabSize: 2 }}
            />
          </div>

          <div className="flex-1 flex flex-col">
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10 flex-shrink-0">
              <button
                type="button"
                onClick={() => setShowScanResults(false)}
                className={cn(
                  "text-[11px] font-medium transition-colors",
                  !showScanResults ? "text-accent" : "text-muted-foreground/40 hover:text-foreground"
                )}
              >
                {t("security.response")}
              </button>
              {activeTab.scanAlerts.length > 0 && (
                <button
                  type="button"
                  onClick={() => setShowScanResults(true)}
                  className={cn(
                    "flex items-center gap-1 text-[11px] font-medium transition-colors",
                    showScanResults ? "text-red-400" : "text-muted-foreground/40 hover:text-foreground"
                  )}
                >
                  <ShieldAlert className="w-3 h-3" />
                  {t("security.scanResults")} ({activeTab.scanAlerts.length})
                </button>
              )}
              <div className="flex-1" />
              {!showScanResults && response && (
                <>
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
                  <button
                    type="button"
                    onClick={() => setPrettyMode(!prettyMode)}
                    className={cn(
                      "px-2 py-0.5 text-[10px] rounded-md font-medium transition-colors",
                      prettyMode
                        ? "bg-accent/15 text-accent"
                        : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
                    )}
                  >
                    Pretty
                  </button>
                </>
              )}
            </div>
            {showScanResults ? (
              <ScanResultsView alerts={activeTab.scanAlerts} />
            ) : response ? (
              <div className="flex-1 overflow-y-auto">
                <DetailSection title={t("security.responseHeaders")} content={response.response_header} />
                <pre className="px-4 py-3 text-[11px] font-mono leading-[1.6] text-foreground/70 whitespace-pre-wrap break-all">
                  {formattedBody}
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
      )}
    </div>
  );
}

// ── Scan Results View ──

function ScanResultsView({ alerts }: { alerts: ZapAlert[] }) {
  const { t } = useTranslation();
  const [expandedId, setExpandedId] = useState<string | null>(null);

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

  if (alerts.length === 0) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground/20">
        <div className="flex flex-col items-center gap-2">
          <ShieldCheck className="w-8 h-8" />
          <p className="text-[12px]">{t("security.scanNoResults")}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto">
      {alerts.map((a) => (
        <div key={a.id} className="border-b border-border/5">
          <button
            type="button"
            onClick={() => setExpandedId(expandedId === a.id ? null : a.id)}
            className="w-full flex items-center gap-2 px-3 py-2 text-left hover:bg-[var(--bg-hover)]/20 transition-colors"
          >
            <ChevronRight className={cn("w-3 h-3 transition-transform text-muted-foreground/30", expandedId === a.id && "rotate-90")} />
            <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(a.risk))}>
              {riskIcon(a.risk)}
            </span>
            <span className="text-[11px] font-medium flex-1 truncate">{a.name}</span>
            <span className="text-[9px] text-muted-foreground/30 font-mono truncate max-w-[120px]">
              {a.param && `${t("security.param")}: ${a.param}`}
            </span>
            <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(a.risk))}>
              {a.risk}
            </span>
          </button>
          {expandedId === a.id && (
            <div className="px-8 pb-3 space-y-2">
              <p className="text-[10px] text-foreground/60 leading-relaxed">{a.description}</p>
              {a.param && (
                <div>
                  <span className="text-[9px] text-muted-foreground/40 font-medium">{t("security.param")}:</span>
                  <span className="text-[10px] text-orange-400 ml-1 font-mono">{a.param}</span>
                </div>
              )}
              {a.evidence && (
                <div>
                  <span className="text-[9px] text-muted-foreground/40 font-medium">{t("security.evidence")}:</span>
                  <pre className="text-[10px] text-foreground/50 font-mono mt-0.5 bg-muted/10 rounded p-1.5 overflow-x-auto">{a.evidence}</pre>
                </div>
              )}
              {a.solution && (
                <div>
                  <span className="text-[9px] text-muted-foreground/40 font-medium">{t("security.solution")}:</span>
                  <p className="text-[10px] text-green-400/60 mt-0.5 leading-relaxed">{a.solution}</p>
                </div>
              )}
              <div className="flex items-center gap-3 text-[9px] text-muted-foreground/30">
                <span>CWE-{a.cweid}</span>
                <span>{a.method} {a.url}</span>
              </div>
            </div>
          )}
        </div>
      ))}
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

// ── Site Map Panel ──

interface SiteTreeNode {
  name: string;
  fullPath: string;
  methods: Set<string>;
  entries: HttpHistoryEntry[];
  children: Map<string, SiteTreeNode>;
  isEndpoint: boolean;
  nodeType?: "domain" | "subdomain" | "api" | "static" | "path";
}

function normalizeHost(raw: string): string {
  let h = raw.toLowerCase().trim();
  h = h.replace(/^https?:\/\//, "");
  h = h.replace(/\/.*$/, "");
  return h;
}

function getRootDomain(host: string): string {
  const parts = host.replace(/:\d+$/, "").split(".");
  if (parts.length <= 2) return host;
  return parts.slice(-2).join(".");
}

function buildSiteTree(entries: HttpHistoryEntry[]): Map<string, SiteTreeNode> {
  const domainGroups = new Map<string, Map<string, HttpHistoryEntry[]>>();

  for (const entry of entries) {
    const host = normalizeHost(entry.host || entry.url);
    const root = getRootDomain(host);
    if (!domainGroups.has(root)) domainGroups.set(root, new Map());
    const subMap = domainGroups.get(root)!;
    if (!subMap.has(host)) subMap.set(host, []);
    subMap.get(host)!.push(entry);
  }

  const roots = new Map<string, SiteTreeNode>();

  for (const [rootDomain, subdomainMap] of domainGroups) {
    const rootNode: SiteTreeNode = {
      name: rootDomain, fullPath: rootDomain, methods: new Set(), entries: [],
      children: new Map(), isEndpoint: false, nodeType: "domain",
    };

    for (const [host, hostEntries] of subdomainMap) {
      let hostNode = rootNode;
      if (host !== rootDomain) {
        if (!rootNode.children.has(host)) {
          rootNode.children.set(host, {
            name: host, fullPath: host, methods: new Set(), entries: [],
            children: new Map(), isEndpoint: false, nodeType: "subdomain",
          });
        }
        hostNode = rootNode.children.get(host)!;
      }

      for (const entry of hostEntries) {
        rootNode.entries.push(entry);
        rootNode.methods.add(entry.method);
        if (hostNode !== rootNode) {
          hostNode.entries.push(entry);
          hostNode.methods.add(entry.method);
        }

        const pathStr = (entry.path || "/").split("?")[0].split("#")[0];
        const segments = pathStr.split("/").filter(Boolean);

        let current = hostNode;
        let builtPath = host;
        for (let i = 0; i < segments.length; i++) {
          const seg = segments[i];
          builtPath += `/${seg}`;
          if (!current.children.has(seg)) {
            current.children.set(seg, {
              name: seg, fullPath: builtPath, methods: new Set(), entries: [],
              children: new Map(), isEndpoint: false,
            });
          }
          const child = current.children.get(seg)!;
          child.entries.push(entry);
          child.methods.add(entry.method);
          current = child;
        }
        current.isEndpoint = true;
      }
    }

    roots.set(rootDomain, rootNode);
  }
  return roots;
}

function SiteMapPanel({ onSendToRepeater, onActiveScan }: { onSendToRepeater: (raw: string) => void; onActiveScan?: (url: string) => void }) {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<HttpHistoryEntry[]>([]);
  const [selectedEntry, setSelectedEntry] = useState<HttpHistoryEntry | null>(null);
  const [detail, setDetail] = useState<HttpMessageDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [filterMode, setFilterMode] = useState<"all" | "api" | "js">("all");
  const [viewMode, setViewMode] = useState<"tree" | "flat">("flat");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; entry: HttpHistoryEntry } | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const loadEntries = useCallback(async () => {
    try {
      const items = await zapGetHistory(0, 2000);
      setEntries(items);
    } catch { /* ignore */ }
  }, []);

  useEffect(() => {
    loadEntries();
    intervalRef.current = setInterval(loadEntries, 5000);
    return () => { if (intervalRef.current) clearInterval(intervalRef.current); };
  }, [loadEntries]);

  const filtered = useMemo(() => {
    let items = entries;
    if (filterMode === "js") {
      items = items.filter((e) => /\.(js|jsx|ts|tsx|mjs|cjs|css|woff2?|svg|png|jpg|gif|ico)(\?|$)/i.test(e.path));
    } else if (filterMode === "api") {
      items = items.filter((e) => !(/\.(js|jsx|ts|tsx|mjs|cjs|css|woff2?|svg|png|jpg|gif|ico|html?)(\?|$)/i.test(e.path)));
    }
    if (search.trim()) {
      const q = search.toLowerCase();
      items = items.filter((e) =>
        e.url.toLowerCase().includes(q) || e.host.toLowerCase().includes(q)
      );
    }
    return items;
  }, [entries, search, filterMode]);

  const deduped = useMemo(() => {
    const seen = new Map<string, HttpHistoryEntry>();
    for (const e of filtered) {
      const pathNoQuery = (e.path || "/").split("?")[0].split("#")[0];
      const host = normalizeHost(e.host || e.url);
      const key = `${e.method}:${host}${pathNoQuery}`;
      if (!seen.has(key)) seen.set(key, e);
    }
    return [...seen.values()];
  }, [filtered]);

  const tree = useMemo(() => buildSiteTree(deduped), [deduped]);

  const handleSelectEntry = useCallback(async (entry: HttpHistoryEntry) => {
    setSelectedEntry(entry);
    setDetailLoading(true);
    try {
      const msg = await zapGetMessage(entry.id);
      setDetail(msg);
    } catch {
      setDetail(null);
    } finally {
      setDetailLoading(false);
    }
  }, []);

  const handleCtxSendToRepeater = useCallback(async (entry: HttpHistoryEntry) => {
    try {
      const msg = await zapGetMessage(entry.id);
      onSendToRepeater(buildRawRequest(msg));
    } catch { /* ignore */ }
  }, [onSendToRepeater]);

  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [ctxMenu]);

  const hostCount = tree.size;
  const endpointCount = deduped.length;

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
        <div className="relative flex-1 max-w-xs">
          <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t("security.filterSiteMap", "Filter endpoints...")}
            className="w-full h-7 pl-8 pr-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
          />
        </div>
        {(["all", "api", "js"] as const).map((mode) => (
          <button
            key={mode}
            type="button"
            onClick={() => setFilterMode(mode)}
            className={cn(
              "px-2.5 py-1 text-[10px] rounded-md font-medium transition-colors",
              filterMode === mode
                ? mode === "api" ? "bg-green-500/15 text-green-400"
                  : mode === "js" ? "bg-yellow-500/15 text-yellow-400"
                  : "bg-accent/15 text-accent"
                : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
            )}
          >
            {mode === "all" ? "All" : mode === "api" ? "API" : "Static"}
          </button>
        ))}
        <div className="flex items-center border border-border/15 rounded-md overflow-hidden">
          <button
            type="button"
            onClick={() => setViewMode("flat")}
            className={cn(
              "p-1.5 transition-colors",
              viewMode === "flat" ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
            )}
            title={t("security.flatView")}
          >
            <List className="w-3 h-3" />
          </button>
          <button
            type="button"
            onClick={() => setViewMode("tree")}
            className={cn(
              "p-1.5 transition-colors",
              viewMode === "tree" ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
            )}
            title={t("security.treeView")}
          >
            <TreePine className="w-3 h-3" />
          </button>
        </div>
        <span className="text-[10px] text-muted-foreground/30">
          {hostCount} {t("security.hosts", "hosts")} · {endpointCount} {t("security.endpoints", "endpoints")}
        </span>
        <button
          type="button"
          onClick={loadEntries}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <RefreshCw className="w-3 h-3" />
        </button>
      </div>

      <div className="flex-1 flex min-h-0">
        <div className={cn("flex-shrink-0 border-r border-border/10 overflow-y-auto", viewMode === "flat" ? "flex-1" : "w-[360px]")}>
          {deduped.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/20">
              <Globe className="w-8 h-8" />
              <p className="text-[11px]">{t("security.noSiteData", "No site data yet")}</p>
            </div>
          ) : viewMode === "flat" ? (
            <table className="w-full text-[11px]">
              <thead className="sticky top-0 bg-card z-10">
                <tr className="text-muted-foreground/40 text-left">
                  <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.method")}</th>
                  <th className="px-3 py-1.5 font-medium">Host</th>
                  <th className="px-3 py-1.5 font-medium">Path</th>
                  <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.status")}</th>
                  <th className="px-3 py-1.5 font-medium w-[36px]" />
                </tr>
              </thead>
              <tbody>
                {deduped.map((entry) => (
                  <tr
                    key={`${entry.method}-${entry.id}`}
                    onClick={() => handleSelectEntry(entry)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setCtxMenu({ x: e.clientX, y: e.clientY, entry });
                    }}
                    className={cn(
                      "cursor-pointer transition-colors border-b border-border/5 group",
                      selectedEntry?.id === entry.id ? "bg-accent/10" : "hover:bg-[var(--bg-hover)]/40"
                    )}
                  >
                    <td className={cn("px-3 py-1.5 font-mono font-medium", methodColor(entry.method))}>{entry.method}</td>
                    <td className="px-3 py-1.5 text-foreground/80 font-mono text-[10px]">{normalizeHost(entry.host || entry.url)}</td>
                    <td className="px-3 py-1.5 text-muted-foreground/60 font-mono truncate max-w-[400px]">{(entry.path || "/").split("?")[0]}</td>
                    <td className={cn("px-3 py-1.5 font-mono", statusColor(entry.status_code))}>{entry.status_code || "-"}</td>
                    <td className="px-1.5 py-1.5">
                      <button
                        type="button"
                        title={t("security.sendToRepeater")}
                        onClick={(e) => { e.stopPropagation(); handleCtxSendToRepeater(entry); }}
                        className="p-1 rounded text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-accent hover:bg-accent/10 transition-colors"
                      >
                        <Send className="w-3 h-3" />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <div className="py-1">
              {[...tree.entries()].map(([host, node]) => (
                <SiteTreeNodeView
                  key={host}
                  node={node}
                  depth={0}
                  onSelect={handleSelectEntry}
                  selectedId={selectedEntry?.id ?? null}
                  onContextMenu={(e, entry) => {
                    e.preventDefault();
                    setCtxMenu({ x: e.clientX, y: e.clientY, entry });
                  }}
                />
              ))}
            </div>
          )}
        </div>

        <div className="flex-1 flex flex-col overflow-hidden">
          {selectedEntry && detail ? (
            <div className="flex-1 overflow-y-auto">
              <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10 flex-shrink-0">
                <span className={cn("text-[10px] font-mono font-bold", methodColor(selectedEntry.method))}>
                  {selectedEntry.method}
                </span>
                <span className="text-[11px] font-mono text-foreground/70 truncate flex-1">
                  {selectedEntry.url}
                </span>
                <span className={cn("text-[10px] font-mono", statusColor(selectedEntry.status_code))}>
                  {selectedEntry.status_code}
                </span>
                <button
                  type="button"
                  onClick={() => onSendToRepeater(buildRawRequest(detail))}
                  className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium text-muted-foreground/40 hover:text-accent hover:bg-accent/10 transition-colors"
                >
                  <Send className="w-3 h-3" />
                  {t("security.sendToRepeater")}
                </button>
                <button
                  type="button"
                  onClick={() => { setSelectedEntry(null); setDetail(null); }}
                  className="p-1 rounded text-muted-foreground/30 hover:text-foreground transition-colors"
                >
                  <X className="w-3 h-3" />
                </button>
              </div>
              <DetailSection title={t("security.requestHeaders")} content={detail.request_headers} />
              {detail.request_body && (
                <DetailSection title={t("security.requestBody")} content={detail.request_body} />
              )}
              <DetailSection title={t("security.responseHeaders")} content={detail.response_headers} />
              <DetailSection title={t("security.responseBody")} content={detail.response_body} maxHeight={400} />
            </div>
          ) : detailLoading ? (
            <div className="flex-1 flex items-center justify-center">
              <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" />
            </div>
          ) : (
            <div className="flex-1 flex items-center justify-center text-muted-foreground/20">
              <div className="flex flex-col items-center gap-2">
                <Search className="w-8 h-8" />
                <p className="text-[12px]">{t("security.selectEndpoint", "Select an endpoint to view details")}</p>
              </div>
            </div>
          )}
        </div>
      </div>
      {ctxMenu && (
        <div
          className="fixed z-[100] min-w-[160px] rounded-lg border border-border/20 bg-popover shadow-lg py-1 text-[11px]"
          style={{ top: ctxMenu.y, left: ctxMenu.x }}
        >
          <button
            type="button"
            className="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors"
            onClick={() => { handleCtxSendToRepeater(ctxMenu.entry); setCtxMenu(null); }}
          >
            <Send className="w-3 h-3 text-accent" />
            {t("security.sendToRepeater")}
          </button>
          {onActiveScan && (
            <button
              type="button"
              className="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors"
              onClick={() => { onActiveScan(ctxMenu.entry.url); setCtxMenu(null); }}
            >
              <Zap className="w-3 h-3 text-orange-400" />
              {t("security.activeScan")}
            </button>
          )}
          <button
            type="button"
            className="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors"
            onClick={() => { navigator.clipboard.writeText(ctxMenu.entry.url); setCtxMenu(null); }}
          >
            <Copy className="w-3 h-3 text-muted-foreground/50" />
            {t("security.copyUrl")}
          </button>
        </div>
      )}
    </div>
  );
}

function SiteTreeNodeView({
  node, depth, onSelect, selectedId, onContextMenu,
}: {
  node: SiteTreeNode; depth: number;
  onSelect: (e: HttpHistoryEntry) => void; selectedId: number | null;
  onContextMenu?: (e: React.MouseEvent, entry: HttpHistoryEntry) => void;
}) {
  const [open, setOpen] = useState(depth < 2);
  const hasChildren = node.children.size > 0;
  const isJs = /\.(js|jsx|ts|tsx|mjs|cjs)(\?|$)/i.test(node.name);
  const isApi = /^(api|v\d|graphql|rest)/i.test(node.name);

  const latestEntry = node.entries[node.entries.length - 1];
  const isSelected = latestEntry && selectedId === latestEntry.id;

  return (
    <div>
      <div
        className={cn(
          "flex items-center gap-1 py-0.5 pr-2 cursor-pointer transition-colors text-[11px] hover:bg-[var(--bg-hover)]/40",
          isSelected && "bg-accent/10",
        )}
        style={{ paddingLeft: `${depth * 16 + 8}px` }}
        onClick={() => {
          if (hasChildren) setOpen(!open);
          if (node.isEndpoint && latestEntry) onSelect(latestEntry);
        }}
        onContextMenu={(e) => {
          if (latestEntry && onContextMenu) onContextMenu(e, latestEntry);
        }}
      >
        {hasChildren ? (
          <ChevronRight className={cn("w-3 h-3 text-muted-foreground/40 transition-transform flex-shrink-0", open && "rotate-90")} />
        ) : (
          <span className="w-3 flex-shrink-0" />
        )}
        {node.nodeType === "domain" ? (
          <Globe className="w-3 h-3 text-blue-400 flex-shrink-0" />
        ) : node.nodeType === "subdomain" ? (
          <Globe className="w-3 h-3 text-cyan-400 flex-shrink-0" />
        ) : isJs ? (
          <Activity className="w-3 h-3 text-yellow-400 flex-shrink-0" />
        ) : isApi ? (
          <ArrowRight className="w-3 h-3 text-green-400 flex-shrink-0" />
        ) : (
          <span className="w-3 h-3 flex items-center justify-center text-muted-foreground/40 flex-shrink-0">
            {node.isEndpoint ? "·" : "/"}
          </span>
        )}
        <span className={cn(
          "truncate flex-1",
          node.nodeType === "domain" ? "font-medium text-foreground/80"
            : node.nodeType === "subdomain" ? "font-medium text-foreground/70"
            : "text-foreground/60",
        )}>
          {node.nodeType === "domain" || node.nodeType === "subdomain" ? node.name : `/${node.name}`}
        </span>
        <span className="flex items-center gap-0.5 flex-shrink-0">
          {[...node.methods].map((m) => (
            <span key={m} className={cn("text-[8px] font-mono font-bold px-1 rounded", methodColor(m))}>
              {m}
            </span>
          ))}
        </span>
        <span className="text-[9px] text-muted-foreground/30 ml-1 flex-shrink-0">
          {node.entries.length > 1 ? `${node.entries.length}` : ""}
        </span>
      </div>
      {open && hasChildren && (
        <div>
          {[...node.children.entries()]
            .sort(([a], [b]) => a.localeCompare(b))
            .map(([name, child]) => (
              <SiteTreeNodeView
                key={name}
                node={child}
                depth={depth + 1}
                onSelect={onSelect}
                selectedId={selectedId}
                onContextMenu={onContextMenu}
              />
            ))}
        </div>
      )}
    </div>
  );
}

// ── Audit Log Panel ──

interface AuditTarget {
  host: string;
  endpoints: Map<string, AuditEndpoint>;
}

interface AuditEndpoint {
  method: string;
  path: string;
  vulnTypes: Set<string>;
  tools: Set<string>;
  riskLevels: Set<string>;
  alertCount: number;
}

function AuditLogPanel() {
  const { t } = useTranslation();
  const [alerts, setAlerts] = useState<ZapAlert[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedHost, setExpandedHost] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      setLoading(true);
      try {
        const items = await zapGetAlerts(undefined, 0, 2000);
        setAlerts(items);
      } catch { /* ignore */ }
      setLoading(false);
    })();
  }, []);

  const targets = useMemo(() => {
    const map = new Map<string, AuditTarget>();
    for (const a of alerts) {
      let host: string;
      try { host = new URL(a.url).host; } catch { host = a.url; }

      if (!map.has(host)) map.set(host, { host, endpoints: new Map() });
      const target = map.get(host)!;

      let path: string;
      try { path = new URL(a.url).pathname; } catch { path = a.url; }
      const key = `${a.method}:${path}`;

      if (!target.endpoints.has(key)) {
        target.endpoints.set(key, {
          method: a.method, path,
          vulnTypes: new Set(), tools: new Set(), riskLevels: new Set(), alertCount: 0,
        });
      }
      const ep = target.endpoints.get(key)!;
      ep.vulnTypes.add(a.name);
      ep.tools.add(a.pluginId);
      ep.riskLevels.add(a.risk);
      ep.alertCount++;
    }
    return [...map.values()].sort((a, b) => {
      const aMax = [...a.endpoints.values()].reduce((acc, ep) => acc + ep.alertCount, 0);
      const bMax = [...b.endpoints.values()].reduce((acc, ep) => acc + ep.alertCount, 0);
      return bMax - aMax;
    });
  }, [alerts]);

  const riskColor = (risk: string) => {
    const c: Record<string, string> = {
      High: "text-red-400 bg-red-500/10",
      Medium: "text-orange-400 bg-orange-500/10",
      Low: "text-yellow-400 bg-yellow-500/10",
      Informational: "text-blue-400 bg-blue-500/10",
    };
    return c[risk] || "text-muted-foreground bg-muted/20";
  };

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );
  }

  const totalEndpoints = targets.reduce((acc, t) => acc + t.endpoints.size, 0);
  const allVulnTypes = new Set(targets.flatMap((t) => [...t.endpoints.values()].flatMap((ep) => [...ep.vulnTypes])));
  const allTools = new Set(targets.flatMap((t) => [...t.endpoints.values()].flatMap((ep) => [...ep.tools])));

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-4 py-2 border-b border-border/10 flex-shrink-0">
        <span className="text-[11px] text-muted-foreground/40">
          {targets.length} {t("security.target", "targets")}
        </span>
        <div className="flex items-center gap-3 text-[10px] text-muted-foreground/30">
          <span>{totalEndpoints} {t("security.scannedEndpoints")}</span>
          <span>{allVulnTypes.size} {t("security.uniqueVulnTypes")}</span>
          <span>{allTools.size} {t("security.scanToolsUsed")}</span>
        </div>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-3">
        {targets.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <ClipboardList className="w-12 h-12" />
            <p className="text-[13px] font-medium">{t("security.noAuditData")}</p>
          </div>
        ) : (
          <div className="space-y-3">
            {targets.map((target) => {
              const isExpanded = expandedHost === target.host;
              const endpointList = [...target.endpoints.values()];
              const highestRisk = endpointList.some((ep) => ep.riskLevels.has("High")) ? "High"
                : endpointList.some((ep) => ep.riskLevels.has("Medium")) ? "Medium"
                : endpointList.some((ep) => ep.riskLevels.has("Low")) ? "Low" : "Informational";

              return (
                <div key={target.host} className="rounded-xl border border-border/10 bg-[var(--bg-hover)]/15 overflow-hidden">
                  <button
                    type="button"
                    onClick={() => setExpandedHost(isExpanded ? null : target.host)}
                    className="w-full flex items-center gap-3 p-3 text-left hover:bg-[var(--bg-hover)]/30 transition-colors"
                  >
                    {isExpanded ? <ChevronDown className="w-3 h-3 text-muted-foreground/40" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/40" />}
                    <Globe className="w-3.5 h-3.5 text-blue-400 flex-shrink-0" />
                    <span className="text-[12px] font-medium text-foreground flex-1">{target.host}</span>
                    <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", riskColor(highestRisk))}>
                      {highestRisk}
                    </span>
                    <span className="text-[10px] text-muted-foreground/30">{target.endpoints.size} endpoints</span>
                  </button>
                  {isExpanded && (
                    <div className="border-t border-border/5">
                      <table className="w-full text-[10px]">
                        <thead>
                          <tr className="text-muted-foreground/30 text-left bg-muted/5">
                            <th className="px-3 py-1.5 font-medium w-[60px]">{t("security.method")}</th>
                            <th className="px-3 py-1.5 font-medium">Path</th>
                            <th className="px-3 py-1.5 font-medium">{t("security.vulnTypesTested")}</th>
                            <th className="px-3 py-1.5 font-medium w-[120px]">{t("security.toolsUsed")}</th>
                          </tr>
                        </thead>
                        <tbody>
                          {endpointList.map((ep) => (
                            <tr key={`${ep.method}:${ep.path}`} className="border-t border-border/5 hover:bg-[var(--bg-hover)]/20">
                              <td className={cn("px-3 py-1.5 font-mono font-medium", methodColor(ep.method))}>{ep.method}</td>
                              <td className="px-3 py-1.5 font-mono text-foreground/60">{ep.path}</td>
                              <td className="px-3 py-1.5">
                                <div className="flex flex-wrap gap-1">
                                  {[...ep.vulnTypes].map((vt) => {
                                    const risk = [...ep.riskLevels][0] || "Informational";
                                    return (
                                      <span key={vt} className={cn("text-[8px] px-1.5 py-0.5 rounded-full", riskColor(risk))}>
                                        {vt}
                                      </span>
                                    );
                                  })}
                                </div>
                              </td>
                              <td className="px-3 py-1.5 text-muted-foreground/40 font-mono">
                                {[...ep.tools].join(", ")}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
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

// ── Passive Scan Panel ──

interface PassiveRule {
  id: string;
  name: string;
  enabled: boolean;
  quality: string;
}

interface CustomPassiveRule {
  id: string;
  name: string;
  pattern: string;
  scope: "body" | "headers" | "all";
  severity: "low" | "medium" | "high";
  enabled: boolean;
}

interface CustomRuleMatch {
  ruleId: string;
  ruleName: string;
  severity: string;
  msgId: number;
  url: string;
  matchSnippet: string;
}

const CUSTOM_RULES_KEY = "golish-custom-passive-rules";

function loadCustomRules(): CustomPassiveRule[] {
  try {
    return JSON.parse(localStorage.getItem(CUSTOM_RULES_KEY) || "[]");
  } catch { return []; }
}

function saveCustomRules(rules: CustomPassiveRule[]) {
  localStorage.setItem(CUSTOM_RULES_KEY, JSON.stringify(rules));
}

function PassiveScanPanel() {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState(true);
  const [records, setRecords] = useState(0);
  const [rules, setRules] = useState<PassiveRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [tab, setTab] = useState<"zap" | "custom">("zap");
  const [customRules, setCustomRules] = useState<CustomPassiveRule[]>(loadCustomRules);
  const [editing, setEditing] = useState<CustomPassiveRule | null>(null);
  const [matches, setMatches] = useState<CustomRuleMatch[]>([]);
  const [scanning, setScanning] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLoading(true);
      try {
        type ZapJson = Record<string, unknown>;
        const [recordsResult, scannersResult] = await Promise.all([
          invoke<ZapJson>("zap_api_call", { component: "pscan", actionType: "view", method: "recordsToScan", params: {} }).catch(() => ({})),
          invoke<ZapJson>("zap_api_call", { component: "pscan", actionType: "view", method: "scanners", params: {} }).catch(() => ({})),
        ]);
        if (cancelled) return;
        const recordsVal = recordsResult?.recordsToScan;
        setRecords(typeof recordsVal === "string" ? Number.parseInt(recordsVal) || 0 : 0);
        const scanners = scannersResult?.scanners;
        if (Array.isArray(scanners)) {
          setRules(scanners.map((r: Record<string, string>) => ({
            id: r.id || "",
            name: r.name || "",
            enabled: r.enabled === "true",
            quality: r.quality || "",
          })));
          const anyEnabled = scanners.some((r: Record<string, string>) => r.enabled === "true");
          setEnabled(anyEnabled);
        }
      } catch { /* ignore */ }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, []);

  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const r = await invoke<Record<string, unknown>>("zap_api_call", { component: "pscan", actionType: "view", method: "recordsToScan", params: {} });
        const val = r?.recordsToScan;
        setRecords(typeof val === "string" ? Number.parseInt(val) || 0 : 0);
      } catch { /* ignore */ }
    }, 5000);
    return () => clearInterval(interval);
  }, []);

  const handleToggleAll = useCallback(async (enable: boolean) => {
    try {
      const method = enable ? "enableAllScanners" : "disableAllScanners";
      const result = await invoke<Record<string, unknown>>("zap_api_call", { component: "pscan", actionType: "action", method, params: {} });
      if (result?.Result === "OK") {
        setRules((prev) => prev.map((r) => ({ ...r, enabled: enable })));
        setEnabled(enable);
      }
    } catch (err) {
      console.error("Failed to toggle all passive scanners:", err);
    }
  }, []);

  const handleToggleRule = useCallback(async (ruleId: string, enable: boolean) => {
    try {
      const method = enable ? "enableScanners" : "disableScanners";
      const result = await invoke<Record<string, unknown>>("zap_api_call", { component: "pscan", actionType: "action", method, params: { ids: ruleId } });
      if (result?.Result === "OK") {
        setRules((prev) => prev.map((r) => r.id === ruleId ? { ...r, enabled: enable } : r));
      }
    } catch (err) {
      console.error("Failed to toggle passive scanner:", err);
    }
  }, []);

  const handleSaveCustomRule = useCallback((rule: CustomPassiveRule) => {
    setCustomRules((prev) => {
      const existing = prev.findIndex((r) => r.id === rule.id);
      const next = existing >= 0 ? prev.map((r) => r.id === rule.id ? rule : r) : [...prev, rule];
      saveCustomRules(next);
      return next;
    });
    setEditing(null);
  }, []);

  const handleDeleteCustomRule = useCallback((id: string) => {
    setCustomRules((prev) => {
      const next = prev.filter((r) => r.id !== id);
      saveCustomRules(next);
      return next;
    });
    setMatches((prev) => prev.filter((m) => m.ruleId !== id));
  }, []);

  const handleRunCustomScan = useCallback(async () => {
    const enabledRules = customRules.filter((r) => r.enabled);
    if (enabledRules.length === 0) return;
    setScanning(true);
    setMatches([]);
    const newMatches: CustomRuleMatch[] = [];
    try {
      const count = await zapGetHistoryCount();
      const batchSize = 50;
      for (let start = 0; start < count; start += batchSize) {
        const entries = await zapGetHistory(start, batchSize);
        for (const entry of entries) {
          try {
            const detail = await zapGetMessage(entry.id);
            for (const rule of enabledRules) {
              const re = new RegExp(rule.pattern, "i");
              const targets: string[] = [];
              if (rule.scope === "body" || rule.scope === "all") targets.push(detail.response_body || "");
              if (rule.scope === "headers" || rule.scope === "all") targets.push(detail.response_headers || "");
              for (const text of targets) {
                const match = re.exec(text);
                if (match) {
                  const idx = match.index;
                  const snippet = text.substring(Math.max(0, idx - 30), idx + match[0].length + 30);
                  newMatches.push({
                    ruleId: rule.id, ruleName: rule.name, severity: rule.severity,
                    msgId: entry.id, url: entry.url, matchSnippet: snippet,
                  });
                  break;
                }
              }
            }
          } catch { /* skip individual messages */ }
        }
      }
    } catch { /* ignore */ }
    setMatches(newMatches);
    setScanning(false);
    if (newMatches.length > 0) {
      const items = newMatches.map((m) => ({
        title: m.ruleName,
        severity: m.severity,
        url: m.url,
        target: (() => { try { return new URL(m.url).host; } catch { return ""; } })(),
        description: `Pattern match: ${m.matchSnippet}`,
      }));
      invoke("findings_import_parsed", { items, toolName: "Custom Passive Scan", projectPath: getProjectPath() }).catch(() => {});
    }
  }, [customRules]);

  const filtered = useMemo(() => {
    if (!search.trim()) return rules;
    const q = search.toLowerCase();
    return rules.filter((r) => r.name.toLowerCase().includes(q) || r.id.includes(q));
  }, [rules, search]);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center justify-between px-4 py-3 border-b border-border/10 flex-shrink-0">
        <div className="flex items-center gap-3">
          <Eye className="w-3.5 h-3.5 text-accent" />
          <span className="text-[12px] font-medium text-foreground/80">{t("security.passiveScan")}</span>
          <span className={cn(
            "text-[9px] px-2 py-0.5 rounded-full font-medium",
            enabled ? "bg-green-500/15 text-green-400" : "bg-zinc-500/15 text-zinc-400"
          )}>
            {enabled ? t("security.passiveEnabled") : t("security.passiveDisabled")}
          </span>
          {records > 0 && (
            <span className="text-[10px] text-muted-foreground/40">
              {records} {t("security.passiveRecords")}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setTab("zap")}
            className={cn("px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors", tab === "zap" ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground")}
          >
            {t("security.passiveRules")}
          </button>
          <button
            type="button"
            onClick={() => setTab("custom")}
            className={cn("px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors", tab === "custom" ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground")}
          >
            {t("security.customRules")}
            {customRules.length > 0 && <span className="ml-1 text-[8px] text-muted-foreground/30">({customRules.length})</span>}
          </button>
        </div>
      </div>

      {tab === "zap" ? (
        <>
          <div className="flex items-center gap-2 px-4 py-2 border-b border-border/10 flex-shrink-0">
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => handleToggleAll(true)}
                className="px-2.5 py-1 rounded-md text-[10px] font-medium text-green-400 bg-green-500/10 hover:bg-green-500/20 transition-colors"
              >
                {t("security.enableAllPassive")}
              </button>
              <button
                type="button"
                onClick={() => handleToggleAll(false)}
                className="px-2.5 py-1 rounded-md text-[10px] font-medium text-muted-foreground/40 bg-muted/20 hover:bg-muted/30 transition-colors"
              >
                {t("security.disableAllPassive")}
              </button>
            </div>
            <div className="relative flex-1 max-w-sm">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground/30" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t("security.passiveRules")}
                className="w-full h-7 pl-8 pr-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40 transition-colors"
              />
            </div>
            <span className="text-[10px] text-muted-foreground/30">
              {filtered.length} / {rules.length}
            </span>
          </div>
          <div className="flex-1 overflow-y-auto">
            {filtered.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
                <Eye className="w-12 h-12" />
                <p className="text-[13px] font-medium">{t("security.noPassiveRules")}</p>
              </div>
            ) : (
              <table className="w-full text-[11px]">
                <thead className="sticky top-0 bg-card z-10">
                  <tr className="text-muted-foreground/40 text-left">
                    <th className="px-3 py-1.5 font-medium w-[50px]" />
                    <th className="px-3 py-1.5 font-medium w-[80px]">{t("security.ruleId")}</th>
                    <th className="px-3 py-1.5 font-medium">{t("security.scannerName")}</th>
                    <th className="px-3 py-1.5 font-medium w-[80px]">{t("security.status")}</th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((rule) => (
                    <tr key={rule.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/30 transition-colors">
                      <td className="px-3 py-1.5">
                        <button
                          type="button"
                          onClick={() => handleToggleRule(rule.id, !rule.enabled)}
                          className={cn("w-7 h-4 rounded-full transition-colors flex items-center px-0.5", rule.enabled ? "bg-green-500/30" : "bg-muted/30")}
                        >
                          <div className={cn("w-3 h-3 rounded-full transition-all", rule.enabled ? "bg-green-400 ml-3" : "bg-muted-foreground/40 ml-0")} />
                        </button>
                      </td>
                      <td className="px-3 py-1.5 font-mono text-muted-foreground/40">{rule.id}</td>
                      <td className="px-3 py-1.5 text-foreground/70">{rule.name}</td>
                      <td className="px-3 py-1.5">
                        <span className={cn("text-[9px] px-1.5 py-0.5 rounded-full font-medium", rule.enabled ? "text-green-400 bg-green-500/10" : "text-muted-foreground/40 bg-muted/20")}>
                          {rule.enabled ? "ON" : "OFF"}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </>
      ) : (
        <CustomRulesView
          rules={customRules}
          editing={editing}
          matches={matches}
          scanning={scanning}
          onEdit={setEditing}
          onSave={handleSaveCustomRule}
          onDelete={handleDeleteCustomRule}
          onScan={handleRunCustomScan}
        />
      )}
    </div>
  );
}

function CustomRulesView({
  rules, editing, matches, scanning,
  onEdit, onSave, onDelete, onScan,
}: {
  rules: CustomPassiveRule[];
  editing: CustomPassiveRule | null;
  matches: CustomRuleMatch[];
  scanning: boolean;
  onEdit: (rule: CustomPassiveRule | null) => void;
  onSave: (rule: CustomPassiveRule) => void;
  onDelete: (id: string) => void;
  onScan: () => void;
}) {
  const { t } = useTranslation();
  const [formName, setFormName] = useState("");
  const [formPattern, setFormPattern] = useState("");
  const [formScope, setFormScope] = useState<"body" | "headers" | "all">("all");
  const [formSeverity, setFormSeverity] = useState<"low" | "medium" | "high">("medium");

  useEffect(() => {
    if (editing) {
      setFormName(editing.name);
      setFormPattern(editing.pattern);
      setFormScope(editing.scope);
      setFormSeverity(editing.severity);
    }
  }, [editing]);

  const handleSubmit = () => {
    if (!formName.trim() || !formPattern.trim()) return;
    try { new RegExp(formPattern); } catch { return; }
    onSave({
      id: editing?.id || `custom-${Date.now()}`,
      name: formName.trim(),
      pattern: formPattern.trim(),
      scope: formScope,
      severity: formSeverity,
      enabled: editing?.enabled ?? true,
    });
    setFormName("");
    setFormPattern("");
    setFormScope("all");
    setFormSeverity("medium");
  };

  const handleNewRule = () => {
    setFormName("");
    setFormPattern("");
    setFormScope("all");
    setFormSeverity("medium");
    onEdit({ id: "", name: "", pattern: "", scope: "all", severity: "medium", enabled: true });
  };

  const sevColor = (s: string) => s === "high" ? "text-red-400" : s === "medium" ? "text-yellow-400" : "text-blue-400";

  return (
    <div className="flex-1 flex flex-col overflow-hidden">
      <div className="flex items-center justify-between px-4 py-2 border-b border-border/10 flex-shrink-0">
        <div className="flex items-center gap-2">
          <button type="button" onClick={handleNewRule} className="px-2.5 py-1 rounded-md text-[10px] font-medium text-accent bg-accent/10 hover:bg-accent/20 transition-colors">
            + {t("security.addRule")}
          </button>
          <button
            type="button"
            onClick={onScan}
            disabled={scanning || rules.filter((r) => r.enabled).length === 0}
            className="flex items-center gap-1 px-2.5 py-1 rounded-md text-[10px] font-medium text-green-400 bg-green-500/10 hover:bg-green-500/20 transition-colors disabled:opacity-30"
          >
            {scanning ? <Loader2 className="w-3 h-3 animate-spin" /> : <Play className="w-3 h-3" />}
            {t("security.runScan")}
          </button>
        </div>
        {matches.length > 0 && (
          <span className="text-[10px] text-yellow-400/60">
            {matches.length} {t("security.matchesFound")}
          </span>
        )}
      </div>

      {editing && (
        <div className="px-4 py-3 border-b border-border/10 flex-shrink-0 space-y-2 bg-[var(--bg-hover)]/20">
          <div className="flex items-center gap-2">
            <input
              value={formName}
              onChange={(e) => setFormName(e.target.value)}
              placeholder={t("security.ruleName")}
              className="flex-1 h-7 px-3 text-[11px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
            />
            <select
              value={formSeverity}
              onChange={(e) => setFormSeverity(e.target.value as "low" | "medium" | "high")}
              className="h-7 px-2 text-[10px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground outline-none"
            >
              <option value="low">Low</option>
              <option value="medium">Medium</option>
              <option value="high">High</option>
            </select>
            <select
              value={formScope}
              onChange={(e) => setFormScope(e.target.value as "body" | "headers" | "all")}
              className="h-7 px-2 text-[10px] bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground outline-none"
            >
              <option value="all">Body + Headers</option>
              <option value="body">Body Only</option>
              <option value="headers">Headers Only</option>
            </select>
          </div>
          <div className="flex items-center gap-2">
            <input
              value={formPattern}
              onChange={(e) => setFormPattern(e.target.value)}
              placeholder={t("security.regexPattern")}
              className="flex-1 h-7 px-3 text-[11px] font-mono bg-[var(--bg-hover)]/30 rounded-lg border border-border/15 text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-accent/40"
            />
            <button type="button" onClick={handleSubmit} className="px-3 py-1 rounded-md text-[10px] font-medium text-accent bg-accent/10 hover:bg-accent/20 transition-colors">
              {editing.id ? t("security.updateRule") : t("security.saveRule")}
            </button>
            <button type="button" onClick={() => onEdit(null)} className="px-3 py-1 rounded-md text-[10px] font-medium text-muted-foreground/40 hover:text-foreground transition-colors">
              {t("security.cancel")}
            </button>
          </div>
        </div>
      )}

      <div className="flex-1 overflow-y-auto">
        {rules.length === 0 && matches.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <Eye className="w-12 h-12" />
            <p className="text-[13px] font-medium">{t("security.noCustomRules")}</p>
            <p className="text-[11px] text-muted-foreground/30 max-w-sm text-center">{t("security.customRulesHint")}</p>
          </div>
        ) : (
          <div className="divide-y divide-border/5">
            {rules.map((rule) => (
              <div key={rule.id} className="flex items-center gap-3 px-4 py-2 hover:bg-[var(--bg-hover)]/30 transition-colors group">
                <button
                  type="button"
                  onClick={() => {
                    const updated = { ...rule, enabled: !rule.enabled };
                    onSave(updated);
                  }}
                  className={cn("w-7 h-4 rounded-full transition-colors flex items-center px-0.5 flex-shrink-0", rule.enabled ? "bg-green-500/30" : "bg-muted/30")}
                >
                  <div className={cn("w-3 h-3 rounded-full transition-all", rule.enabled ? "bg-green-400 ml-3" : "bg-muted-foreground/40 ml-0")} />
                </button>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-[11px] text-foreground/70 truncate">{rule.name}</span>
                    <span className={cn("text-[9px] font-medium", sevColor(rule.severity))}>{rule.severity.toUpperCase()}</span>
                  </div>
                  <span className="text-[10px] font-mono text-muted-foreground/30 truncate block">{rule.pattern}</span>
                </div>
                <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0">
                  <button type="button" onClick={() => onEdit(rule)} className="px-1.5 py-0.5 rounded text-[9px] text-muted-foreground/40 hover:text-foreground transition-colors">Edit</button>
                  <button type="button" onClick={() => onDelete(rule.id)} className="px-1.5 py-0.5 rounded text-[9px] text-destructive/50 hover:text-destructive transition-colors">Del</button>
                </div>
              </div>
            ))}
            {matches.length > 0 && (
              <div className="px-4 py-2">
                <h4 className="text-[10px] font-medium text-foreground/60 mb-2">{t("security.matchesFound")} ({matches.length})</h4>
                <div className="space-y-1">
                  {matches.map((m, i) => (
                    <div key={`${m.ruleId}-${m.msgId}-${i}`} className="flex items-start gap-2 text-[10px] py-1">
                      <span className={cn("flex-shrink-0 font-medium", sevColor(m.severity))}>{m.severity.toUpperCase()}</span>
                      <span className="text-foreground/60 truncate flex-1">{m.url}</span>
                      <span className="text-muted-foreground/30 font-mono text-[9px] max-w-[200px] truncate flex-shrink-0">{m.matchSnippet}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

// ── Shared Helpers ──

function methodColor(m: string): string {
  const c: Record<string, string> = {
    GET: "text-green-400", POST: "text-blue-400", PUT: "text-yellow-400",
    DELETE: "text-red-400", PATCH: "text-purple-400", OPTIONS: "text-zinc-400",
    HEAD: "text-cyan-400",
  };
  return c[m] || "text-muted-foreground";
}

function statusColor(code: number): string {
  if (code >= 200 && code < 300) return "text-green-400";
  if (code >= 300 && code < 400) return "text-blue-400";
  if (code >= 400 && code < 500) return "text-yellow-400";
  if (code >= 500) return "text-red-400";
  return "text-muted-foreground";
}

function formatSize(bytes: number): string {
  if (bytes === 0) return "-";
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}K`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}M`;
}
