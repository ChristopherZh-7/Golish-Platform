import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { js_beautify as jsBeautify, css_beautify as cssBeautify } from "js-beautify";
import {
  Activity, ArrowDown, ArrowRight, ArrowUp, Check, ChevronDown,
  ChevronRight, ClipboardList, Copy, Crosshair, Database, Download, Eye, FileCode2, Globe, History, KeyRound, List,
  FileSearch, Loader2, Pause, Play, Plus, RefreshCw, Search, Send, Shield, ShieldAlert, ShieldCheck,
  ShieldX, Square, Trash2, TreePine, X, XCircle, Zap,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  zapStart, zapStop, zapStatus, zapDetectPath, zapGetHistory, zapGetHistoryCount,
  zapGetMessage, zapStartScan, zapScanProgress, zapStopScan,
  zapGetAlerts, zapGetAlertCount, zapSendRequest, zapStartSpider,
  zapSpiderProgress, zapStopSpider,
  zapDownloadRootCert, zapInstallRootCert,
  zapGetSiteMapData,
  zapBatchScan,
  zapPauseScan, zapResumeScan,
} from "@/lib/pentest/zap-api";
import type { SiteMapEntry, SiteMapData } from "@/lib/pentest/zap-api";
import type {
  ZapStatusInfo, HttpHistoryEntry, HttpMessageDetail,
  ZapAlert, ScanProgress, ManualRequestResult,
} from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import { getProjectPath } from "@/lib/projects";
import { useStore } from "@/store";
import {
  oplogList,
  oplogListByTarget,
  oplogSearch,
  targetAssetsList,
  apiEndpointsList,
  fingerprintsList,
  jsAnalysisList,
  type AuditRow,
  type TargetAsset,
  type ApiEndpoint,
  type Fingerprint,
  type JsAnalysisResult,
} from "@/lib/security-analysis";

import { lazy, Suspense } from "react";
const VaultSettings = lazy(() =>
  import("@/components/Settings/VaultSettings").then((m) => ({ default: m.VaultSettings }))
);
import { PipelineLauncher } from "@/components/TargetPanel/PipelineLauncher";
import { IntruderPanel } from "@/components/SecurityView/IntruderPanel";
import {
  matchPocsForTarget,
  scanNucleiTargeted,
  type ToolScanResult,
  type PocMatch,
  type NucleiScanOptions,
} from "@/lib/pentest/scan-runner";
export type SecurityTab = "history" | "sitemap" | "scanner" | "repeater" | "intruder" | "passive" | "vault" | "scantools" | "sensitive";

function StyledSelect({ value, onChange, options, className }: {
  value: string; onChange: (v: string) => void;
  options: { value: string; label: string }[];
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const selected = options.find((o) => o.value === value) ?? options[0];

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div ref={ref} className={cn("relative", className)}>
      <button type="button" onClick={() => setOpen(!open)}
        className="flex items-center gap-1 w-full h-full px-2.5 text-[10px] rounded-lg bg-white/[0.03] border border-white/[0.06] hover:border-white/[0.12] text-foreground/70 transition-colors cursor-pointer"
      >
        <span className="flex-1 text-left truncate">{selected?.label}</span>
        <ChevronDown className={cn("w-2.5 h-2.5 text-muted-foreground/30 transition-transform flex-shrink-0", open && "rotate-180")} />
      </button>
      {open && (
        <div className="absolute z-50 mt-0.5 w-full min-w-[100px] rounded-lg border border-white/[0.08] bg-[#1a1a1f] shadow-xl py-0.5 max-h-[200px] overflow-y-auto">
          {options.map((o) => (
            <button key={o.value} type="button" onClick={() => { onChange(o.value); setOpen(false); }}
              className={cn("w-full text-left px-2.5 py-1.5 text-[10px] transition-colors", o.value === value ? "bg-accent/15 text-accent" : "text-foreground/60 hover:bg-white/[0.05] hover:text-foreground")}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function SecurityView({
  standaloneTab,
  initialScanTarget,
}: {
  standaloneTab?: SecurityTab;
  initialScanTarget?: { id: string; value: string };
} = {}) {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const globalZapRunning = useStore((s) => s.zapRunning);
  const setGlobalZapRunning = useStore((s) => s.setZapRunning);
  const [activeTab, setActiveTab] = useState<SecurityTab>(standaloneTab || (initialScanTarget ? "scantools" : "history"));
  const effectiveTab = standaloneTab || activeTab;

  useEffect(() => {
    if (initialScanTarget) setActiveTab("scantools");
  }, [initialScanTarget?.id]);

  const [zapState, setZapState] = useState<ZapStatusInfo>({
    status: globalZapRunning ? "running" : "stopped",
    port: 8090,
  });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [zapInstalled, setZapInstalled] = useState<boolean | null>(globalZapRunning ? true : null);
  const [checkingInstall, setCheckingInstall] = useState(!globalZapRunning);
  const [repeaterRequest, setRepeaterRequest] = useState<string | null>(null);
  const [intruderRequest, setIntruderRequest] = useState<string | null>(null);
  const [pendingScanUrl, setPendingScanUrl] = useState<string | null>(null);

  const handleSendToRepeater = useCallback((rawRequest: string) => {
    setRepeaterRequest(rawRequest);
    setActiveTab("repeater");
  }, []);

  const handleSendToIntruder = useCallback((rawRequest: string) => {
    setIntruderRequest(rawRequest);
    setActiveTab("intruder");
  }, []);

  const [pendingScanUrls, setPendingScanUrls] = useState<string[]>([]);

  const handleActiveScan = useCallback((url: string) => {
    setPendingScanUrl(url);
    setActiveTab("scanner");
  }, []);

  const handleBatchActiveScan = useCallback((urls: string[]) => {
    setPendingScanUrls(urls);
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
        setGlobalZapRunning(status.status === "running");
      } catch {
        if (!cancelled) { setZapInstalled(false); setGlobalZapRunning(false); }
      } finally {
        if (!cancelled) setCheckingInstall(false);
      }
    })();
    return () => { cancelled = true; };
  }, [setGlobalZapRunning]);

  const handleStart = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await zapStart(undefined, undefined, currentProjectPath);
      setZapState(result);
      setGlobalZapRunning(result.status === "running");
    } catch (e) {
      setError(String(e));
      setZapState((s) => ({ ...s, status: "error", error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [currentProjectPath, setGlobalZapRunning]);

  const handleStop = useCallback(async () => {
    setLoading(true);
    try {
      await zapStop();
      setZapState({ status: "stopped", port: zapState.port });
      setGlobalZapRunning(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [zapState.port, setGlobalZapRunning]);

  const isRunning = zapState.status === "running";

  const tabs: { id: SecurityTab; label: string; icon: React.ElementType }[] = [
    { id: "history", label: t("security.history"), icon: History },
    { id: "sitemap", label: t("security.siteMap", "Site Map"), icon: Globe },
    { id: "scanner", label: t("security.scanner"), icon: ShieldAlert },
    { id: "repeater", label: t("security.repeater"), icon: Send },
    { id: "intruder", label: "Intruder", icon: Crosshair },
    { id: "passive", label: t("security.passiveScan", "Passive Scan"), icon: Eye },
    { id: "scantools", label: t("security.scanTools", "Scan Tools"), icon: Crosshair },
    { id: "sensitive", label: "Sensitive Scan", icon: FileSearch },
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
    if (tab === "scantools") {
      return null; // always-mounted below
    }
    if (tab === "sensitive") {
      return <SensitiveScanPanel />;
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
      case "sitemap": return <SiteMapPanel onSendToRepeater={handleSendToRepeater} onSendToIntruder={handleSendToIntruder} onActiveScan={handleActiveScan} onBatchScan={handleBatchActiveScan} />;
      case "history": return <HttpHistoryPanel onSendToRepeater={handleSendToRepeater} onSendToIntruder={handleSendToIntruder} onActiveScan={handleActiveScan} />;
      case "scanner": return null; // always-mounted below
      case "passive": return <PassiveScanPanel />;
      case "repeater": return null;
      case "intruder": return null;
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
        {/* Keep stateful panels mounted so state survives tab switches */}
        <div className={cn("absolute inset-0", effectiveTab === "repeater" && isRunning ? "" : "invisible pointer-events-none")}>
          <RepeaterPanel injectedRequest={repeaterRequest} onInjectedConsumed={() => setRepeaterRequest(null)} />
        </div>
        <div className={cn("absolute inset-0", effectiveTab === "intruder" && isRunning ? "" : "invisible pointer-events-none")}>
          <IntruderPanel injectedRequest={intruderRequest} onInjectedConsumed={() => setIntruderRequest(null)} />
        </div>
        <div className={cn("absolute inset-0", effectiveTab === "scantools" ? "" : "invisible pointer-events-none")}>
          <ScanToolsPanel initialTarget={initialScanTarget} />
        </div>
        <div className={cn("absolute inset-0", effectiveTab === "scanner" && isRunning ? "" : "invisible pointer-events-none")}>
          <ScannerPanel initialUrl={pendingScanUrl} initialBatchUrls={pendingScanUrls} onUrlConsumed={() => { setPendingScanUrl(null); setPendingScanUrls([]); }} />
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

function HttpHistoryPanel({ onSendToRepeater, onSendToIntruder, onActiveScan }: { onSendToRepeater: (raw: string) => void; onSendToIntruder?: (raw: string) => void; onActiveScan?: (url: string) => void }) {
  const { t } = useTranslation();
  const [allEntries, setAllEntries] = useState<HttpHistoryEntry[]>([]);
  const [totalCount, setTotalCount] = useState(0);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<HttpMessageDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [sortOrder, setSortOrder] = useState<"desc" | "asc">("desc");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; entry: HttpHistoryEntry } | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [hideBeforeId, setHideBeforeId] = useState(0);

  const entries = useMemo(() => allEntries.filter((e) => e.id > hideBeforeId), [allEntries, hideBeforeId]);

  const loadHistory = useCallback(async () => {
    try {
      const [items, count] = await Promise.all([
        zapGetHistory(0, 200),
        zapGetHistoryCount(),
      ]);
      setAllEntries(items);
      setTotalCount(count);
    } catch {
      /* ignore */
    }
  }, []);

  useEffect(() => {
    loadHistory();
    let unlisten: (() => void) | null = null;
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("sitemap-updated", () => loadHistory()).then((fn) => { unlisten = fn; });
    });
    intervalRef.current = setInterval(loadHistory, 15000);
    return () => {
      if (unlisten) unlisten();
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

  const handleClearHistory = useCallback(() => {
    if (!confirm(t("security.clearHistoryConfirm"))) return;
    const maxId = allEntries.reduce((m, e) => Math.max(m, e.id), 0);
    setHideBeforeId(maxId);
    setSelectedId(null);
    setDetail(null);
  }, [t, allEntries]);

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

      <HistoryBodyPanel
        filtered={filtered}
        entries={entries}
        selectedId={selectedId}
        detail={detail}
        loading={loading}
        onSelect={handleSelect}
        onSendToRepeater={handleSendToRepeater}
        onSendDetailToRepeater={(d) => onSendToRepeater(buildRawRequest(d))}
        onCtxMenu={(x, y, entry) => setCtxMenu({ x, y, entry })}
        onClose={() => { setSelectedId(null); setDetail(null); }}
      />
      {ctxMenu && createPortal(
        <div
          className="fixed z-[9999] min-w-[160px] rounded-lg border border-border/20 bg-popover shadow-lg py-1 text-[11px]"
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
          {onSendToIntruder && (
            <button
              type="button"
              className="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors"
              onClick={async () => {
                try { const msg = await zapGetMessage(ctxMenu.entry.id); onSendToIntruder(buildRawRequest(msg)); } catch { /* ignore */ }
                setCtxMenu(null);
              }}
            >
              <Crosshair className="w-3 h-3 text-orange-400" />
              Send to Intruder
            </button>
          )}
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
        </div>,
        document.body,
      )}
    </div>
  );
}

function HistoryBodyPanel({
  filtered, entries, selectedId, detail, loading,
  onSelect, onSendToRepeater, onSendDetailToRepeater, onCtxMenu, onClose,
}: {
  filtered: HttpHistoryEntry[]; entries: HttpHistoryEntry[];
  selectedId: number | null; detail: HttpMessageDetail | null; loading: boolean;
  onSelect: (id: number) => void; onSendToRepeater: (id: number) => void;
  onSendDetailToRepeater: (d: HttpMessageDetail) => void;
  onCtxMenu: (x: number, y: number, entry: HttpHistoryEntry) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const detailRef = useRef<HTMLDivElement>(null);
  const [detailWidth, setDetailWidth] = useState<number | null>(null);
  const MIN_DETAIL = 280;

  useEffect(() => {
    if (selectedId !== null && detailWidth === null && containerRef.current) {
      setDetailWidth(Math.floor(containerRef.current.offsetWidth / 2));
    }
  }, [selectedId, detailWidth]);

  const handleResize = useCallback((delta: number) => {
    const maxW = containerRef.current ? containerRef.current.offsetWidth - 280 : 900;
    setDetailWidth((w) => Math.min(maxW, Math.max(MIN_DETAIL, (w ?? 480) - delta)));
  }, []);

  return (
    <div ref={containerRef} className="flex-1 flex min-h-0">
      <div className="flex-1 min-w-0 overflow-y-auto">
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
                onClick={() => onSelect(entry.id)}
                onContextMenu={(e) => { e.preventDefault(); onCtxMenu(e.clientX, e.clientY, entry); }}
                className={cn(
                  "cursor-pointer transition-colors border-b border-border/5 group",
                  selectedId === entry.id ? "bg-accent/10" : "hover:bg-[var(--bg-hover)]/40"
                )}
              >
                <td className="px-3 py-1.5 text-muted-foreground/30 font-mono">{entry.id}</td>
                <td className={cn("px-3 py-1.5 font-mono font-medium", methodColor(entry.method))}>{entry.method}</td>
                <td className="px-3 py-1.5 text-foreground/80 truncate max-w-[400px] font-mono">
                  {entry.host}<span className="text-muted-foreground/40">{entry.path}</span>
                </td>
                <td className={cn("px-3 py-1.5 font-mono", statusColor(entry.status_code))}>{entry.status_code || "-"}</td>
                <td className="px-3 py-1.5 text-muted-foreground/40 font-mono">{formatSize(entry.content_length)}</td>
                <td className="px-3 py-1.5 text-muted-foreground/40 font-mono">{entry.time_ms ? `${entry.time_ms}ms` : "-"}</td>
                <td className="px-1.5 py-1.5">
                  <button type="button" title={t("security.sendToRepeater")}
                    onClick={(e) => { e.stopPropagation(); onSendToRepeater(entry.id); }}
                    className="p-1 rounded text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-accent hover:bg-accent/10 transition-colors">
                    <Send className="w-3 h-3" />
                  </button>
                </td>
              </tr>
            ))}
            {filtered.length === 0 && (
              <tr>
                <td colSpan={7} className="px-3 py-8 text-center text-muted-foreground/30">
                  {entries.length === 0 ? t("security.noHistory") : t("common.noResults")}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {selectedId !== null && (
        <>
          <ResizeHandle onResize={handleResize} />
          <div ref={detailRef} className="flex-shrink-0 flex flex-col overflow-hidden" style={{ width: detailWidth ?? 480 }}>
            <div className="flex items-center justify-between px-3 py-2 border-b border-border/10">
              <span className="text-[11px] font-medium text-muted-foreground/40">
                #{selectedId} {t("security.detail")}
              </span>
              <div className="flex items-center gap-1">
                {detail && (
                  <button type="button" onClick={() => onSendDetailToRepeater(detail)}
                    className="flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium text-muted-foreground/40 hover:text-accent hover:bg-accent/10 transition-colors">
                    <Send className="w-3 h-3" />{t("security.sendToRepeater")}
                  </button>
                )}
                <button type="button" onClick={onClose}
                  className="p-1 rounded text-muted-foreground/30 hover:text-foreground transition-colors">
                  <X className="w-3 h-3" />
                </button>
              </div>
            </div>
            {loading ? (
              <div className="flex-1 flex items-center justify-center">
                <Loader2 className="w-4 h-4 animate-spin text-muted-foreground/30" />
              </div>
            ) : detail ? (
              <DetailTabs detail={detail} />
            ) : (
              <div className="flex-1 flex items-center justify-center text-muted-foreground/20 text-[12px]">
                {t("security.noDetail")}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function prettyPrintBody(content: string, headers?: string): string {
  if (!content || !content.trim()) return content;
  const ct = (headers || "").toLowerCase();
  if (ct.includes("application/json") || ct.includes("+json")) {
    try { return JSON.stringify(JSON.parse(content), null, 2); } catch { /* not valid JSON */ }
  }
  if (/^\s*[{[]/.test(content)) {
    try { return JSON.stringify(JSON.parse(content), null, 2); } catch { /* not valid JSON */ }
  }
  if (ct.includes("text/html") || ct.includes("application/xhtml")) {
    try {
      let depth = 0;
      const lines: string[] = [];
      const raw = content.replace(/>\s*</g, ">\n<");
      for (const line of raw.split("\n")) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        if (/^<\//.test(trimmed)) depth = Math.max(0, depth - 1);
        lines.push("  ".repeat(depth) + trimmed);
        if (/^<[^/!][^>]*[^/]>$/.test(trimmed) && !/<(br|hr|img|input|meta|link|col|area|base|embed|source|track|wbr)[\s/>]/i.test(trimmed)) {
          depth++;
        }
      }
      return lines.join("\n");
    } catch { /* fallback to raw */ }
  }
  if (ct.includes("javascript") || ct.includes("text/css")) {
    try {
      if (content.length > 500_000) return content;
      if (ct.includes("css")) {
        return cssBeautify(content, { indent_size: 2 });
      }
      return jsBeautify(content, { indent_size: 2, space_in_empty_paren: false });
    } catch { /* fallback to raw */ }
  }
  return content;
}

function ResizeHandle({ onResize, direction = "horizontal" }: {
  onResize: (delta: number) => void;
  direction?: "horizontal" | "vertical";
}) {
  const isHorizontal = direction === "horizontal";
  const handleRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = handleRef.current;
    if (!el) return;
    let startPos = 0;

    const onPointerMove = (e: PointerEvent) => {
      const delta = isHorizontal ? e.clientX - startPos : e.clientY - startPos;
      startPos = isHorizontal ? e.clientX : e.clientY;
      onResize(delta);
    };
    const onPointerUp = () => {
      document.removeEventListener("pointermove", onPointerMove);
      document.removeEventListener("pointerup", onPointerUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    const onPointerDown = (e: PointerEvent) => {
      e.preventDefault();
      startPos = isHorizontal ? e.clientX : e.clientY;
      document.body.style.cursor = isHorizontal ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";
      document.addEventListener("pointermove", onPointerMove);
      document.addEventListener("pointerup", onPointerUp);
    };
    el.addEventListener("pointerdown", onPointerDown);
    return () => {
      el.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("pointermove", onPointerMove);
      document.removeEventListener("pointerup", onPointerUp);
    };
  }, [onResize, isHorizontal]);

  return (
    <div
      ref={handleRef}
      className={cn(
        "flex-shrink-0 relative group",
        isHorizontal
          ? "w-[5px] cursor-col-resize hover:bg-accent/20 active:bg-accent/30"
          : "h-[5px] cursor-row-resize hover:bg-accent/20 active:bg-accent/30",
        "transition-colors duration-100"
      )}
    >
      <div className={cn(
        "absolute bg-accent/40 opacity-0 group-hover:opacity-100 group-active:opacity-100 transition-opacity duration-100",
        isHorizontal ? "top-0 bottom-0 left-[2px] w-[1px]" : "left-0 right-0 top-[2px] h-[1px]"
      )} />
    </div>
  );
}

function parseHeaders(raw: string): { firstLine: string; headers: [string, string][] } {
  const lines = raw.split("\n").map((l) => l.replace(/\r$/, ""));
  const firstLine = lines[0] || "";
  const headers: [string, string][] = [];
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) continue;
    const idx = line.indexOf(":");
    if (idx > 0) {
      headers.push([line.substring(0, idx).trim(), line.substring(idx + 1).trim()]);
    }
  }
  return { firstLine, headers };
}

function syntaxHighlightJson(json: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  const re = /("(?:[^"\\]|\\.)*")\s*:|("(?:[^"\\]|\\.)*")|(-?\d+\.?\d*(?:[eE][+-]?\d+)?)|(\btrue\b|\bfalse\b)|(\bnull\b)/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  let key = 0;
  while ((match = re.exec(json)) !== null) {
    if (match.index > lastIndex) {
      nodes.push(json.substring(lastIndex, match.index));
    }
    if (match[1]) {
      nodes.push(<span key={key++} className="text-purple-400">{match[1]}</span>);
      nodes.push(<span key={key++} className="text-foreground/40">:</span>);
    } else if (match[2]) {
      nodes.push(<span key={key++} className="text-green-400">{match[2]}</span>);
    } else if (match[3]) {
      nodes.push(<span key={key++} className="text-blue-400">{match[3]}</span>);
    } else if (match[4]) {
      nodes.push(<span key={key++} className="text-orange-400">{match[4]}</span>);
    } else if (match[5]) {
      nodes.push(<span key={key++} className="text-red-400/70">{match[5]}</span>);
    }
    lastIndex = match.index + match[0].length;
  }
  if (lastIndex < json.length) {
    nodes.push(json.substring(lastIndex));
  }
  return nodes;
}

function HeaderTable({ raw }: { raw: string }) {
  const { firstLine, headers } = useMemo(() => parseHeaders(raw), [raw]);
  if (!raw.trim()) return <div className="px-3 py-2 text-[10px] text-muted-foreground/30">(empty)</div>;

  return (
    <div className="text-[10px]">
      <div className="px-3 py-1.5 font-mono text-foreground/80 bg-[var(--bg-hover)]/20 border-b border-border/5">
        {firstLine}
      </div>
      <table className="w-full">
        <tbody>
          {headers.map(([k, v], i) => (
            <tr key={i} className="border-b border-border/[0.03] hover:bg-[var(--bg-hover)]/20 transition-colors">
              <td className="px-3 py-1 font-mono font-medium text-accent/70 whitespace-nowrap align-top w-[1%]">{k}</td>
              <td className="px-2 py-1 font-mono text-foreground/60 break-all">{v}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function BodyView({ content, headers }: { content: string; headers?: string }) {
  const formatted = useMemo(() => prettyPrintBody(content, headers), [content, headers]);
  const isJson = useMemo(() => {
    try { JSON.parse(content); return true; } catch { return false; }
  }, [content]);

  if (!content || !content.trim()) return <div className="px-3 py-4 text-[10px] text-muted-foreground/30 text-center">(empty)</div>;

  return (
    <div className="relative group">
      <button
        type="button"
        onClick={() => navigator.clipboard.writeText(formatted)}
        className="absolute top-2 right-2 p-1 rounded text-muted-foreground/0 group-hover:text-muted-foreground/40 hover:!text-accent hover:!bg-accent/10 transition-all z-10"
        title="Copy"
      >
        <Copy className="w-3 h-3" />
      </button>
      <pre className="px-3 py-2 text-[10px] font-mono text-foreground/70 whitespace-pre-wrap break-all overflow-y-auto">
        {isJson ? syntaxHighlightJson(formatted) : formatted}
      </pre>
    </div>
  );
}

function DetailTabs({ detail }: { detail: HttpMessageDetail }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<"request" | "response">("response");

  return (
    <div className="flex-1 flex flex-col min-h-0">
      <div className="flex items-center gap-0 border-b border-border/10 px-2 flex-shrink-0">
        {(["request", "response"] as const).map((id) => (
          <button
            key={id}
            type="button"
            onClick={() => setTab(id)}
            className={cn(
              "px-3 py-1.5 text-[10px] font-medium transition-colors relative",
              tab === id
                ? "text-accent"
                : "text-muted-foreground/40 hover:text-foreground"
            )}
          >
            {id === "request" ? t("security.request", "Request") : t("security.response", "Response")}
            {tab === id && <div className="absolute bottom-0 left-1 right-1 h-[1.5px] bg-accent rounded-full" />}
          </button>
        ))}
        <div className="flex-1" />
        <span className="text-[9px] text-muted-foreground/25 font-mono mr-1">
          {tab === "response" && detail.response_body ? `${(detail.response_body.length / 1024).toFixed(1)} KB` : ""}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto">
        {tab === "request" ? (
          <>
            <HeaderTable raw={detail.request_headers} />
            {detail.request_body && (
              <>
                <div className="px-3 py-1 text-[9px] font-medium text-muted-foreground/30 bg-[var(--bg-hover)]/10 border-y border-border/5">
                  {t("security.requestBody", "Request Body")}
                </div>
                <BodyView content={detail.request_body} headers={detail.request_headers} />
              </>
            )}
          </>
        ) : (
          <>
            <HeaderTable raw={detail.response_headers} />
            <div className="px-3 py-1 text-[9px] font-medium text-muted-foreground/30 bg-[var(--bg-hover)]/10 border-y border-border/5">
              {t("security.responseBody", "Response Body")}
            </div>
            <BodyView content={detail.response_body} headers={detail.response_headers} />
          </>
        )}
      </div>
    </div>
  );
}

function DetailSection({ title, content }: { title: string; content: string }) {
  const [expanded, setExpanded] = useState(true);
  return (
    <div className="border-b border-border/5">
      <button type="button" onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-1.5 px-3 py-1.5 text-[10px] font-medium text-muted-foreground/40 hover:text-foreground transition-colors">
        {expanded ? <ChevronDown className="w-2.5 h-2.5" /> : <ChevronRight className="w-2.5 h-2.5" />}
        {title}
      </button>
      {expanded && <HeaderTable raw={content} />}
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
  status: "queued" | "spidering" | "scanning" | "paused" | "complete" | "error";
  alerts: ZapAlert[];
  addedAt: number;
  messageCount?: number;
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

function saveScanQueueToDb(q: ScanEndpoint[], pp: string | null) {
  const seen = new Set<string>();
  const deduped = q.filter((e) => {
    if (seen.has(e.url)) return false;
    seen.add(e.url);
    return true;
  });
  invoke("scan_queue_save_all", {
    endpoints: deduped.map((e) => ({
      url: e.url, status: e.status, alerts: e.alerts,
      addedAt: e.addedAt, scanId: e.scanId, progress: e.progress,
    })),
    projectPath: pp,
  }).catch(() => {});
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

function ScannerPanel({ initialUrl, initialBatchUrls, onUrlConsumed }: { initialUrl?: string | null; initialBatchUrls?: string[]; onUrlConsumed?: () => void }) {
  const { t } = useTranslation();
  const projectPath = useStore((s) => s.currentProjectPath);
  const [targetUrl, setTargetUrl] = useState("");
  const [endpoints, setEndpoints] = useState<ScanEndpoint[]>([]);
  const [selectedUrl, setSelectedUrl] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [vaultEntries, setVaultEntries] = useState<VaultEntrySafe[]>([]);
  const [selectedCredential, setSelectedCredential] = useState<string>("");
  const [showCredSelector, setShowCredSelector] = useState(false);
  const [scanPolicies, setScanPolicies] = useState<string[]>([]);
  const [selectedPolicy, setSelectedPolicy] = useState<string>("");
  const [scanLogs, setScanLogs] = useState<any[]>([]);
  const [showPlugins, setShowPlugins] = useState(false);
  const [scannerRules, setScannerRules] = useState<{ id: string; name: string; enabled: boolean; quality: string }[]>([]);

  useEffect(() => {
    const pp = getProjectPath();
    Promise.all([
      invoke<VaultEntrySafe[]>("vault_list", { projectPath: pp }).catch(() => []),
      invoke<ScanEndpoint[]>("scan_queue_list", { projectPath: pp }).catch(() => []),
      invoke<string[]>("zap_list_scan_policies").catch(() => []),
    ]).then(([v, q, policies]) => {
      setVaultEntries(Array.isArray(v) ? v : []);
      const raw = (Array.isArray(q) ? q : []).map((e) => ({
        ...e,
        status: (e.status === "spidering" && !e.scanId ? "queued" : e.status) as ScanEndpoint["status"],
      } as ScanEndpoint));
      const seen = new Set<string>();
      const restored = raw.filter((e) => {
        if (seen.has(e.url)) return false;
        seen.add(e.url);
        return true;
      });
      if (restored.length > 0) {
        setEndpoints(restored);
        if (restored.length < raw.length) saveScanQueueToDb(restored, pp);
      }
      if (Array.isArray(policies)) setScanPolicies(policies);
    });
  }, [projectPath]);

  useEffect(() => {
    if (initialUrl) {
      setEndpoints((prev) => {
        if (prev.some((e) => e.url === initialUrl)) return prev;
        const ep: ScanEndpoint = { url: initialUrl, scanId: null, progress: 0, status: "queued", alerts: [], addedAt: Date.now() };
        const next = [...prev, ep];
        saveScanQueueToDb(next, projectPath);
        return next;
      });
      setSelectedUrl(initialUrl);
      onUrlConsumed?.();
    }
  }, [initialUrl, onUrlConsumed]);

  useEffect(() => {
    if (!initialBatchUrls?.length) return;
    const now = Date.now();
    setEndpoints((prev) => {
      const existing = new Set(prev.map((e) => e.url));
      const newEps = initialBatchUrls
        .filter((url) => !existing.has(url))
        .map((url, i) => ({
          url,
          scanId: null,
          progress: 0,
          status: "queued" as const,
          alerts: [] as ZapAlert[],
          addedAt: now + i,
        }));
      if (newEps.length === 0) return prev;
      const next = [...prev, ...newEps];
      saveScanQueueToDb(next, projectPath);
      return next;
    });
    onUrlConsumed?.();
  }, [initialBatchUrls]);

  const handleAddEndpoint = useCallback(() => {
    const url = targetUrl.trim();
    if (!url) return;
    if (endpoints.some((e) => e.url === url)) { setSelectedUrl(url); setTargetUrl(""); return; }
    const ep: ScanEndpoint = { url, scanId: null, progress: 0, status: "queued", alerts: [], addedAt: Date.now() };
    setEndpoints((prev) => { const next = [...prev, ep]; saveScanQueueToDb(next, projectPath); return next; });
    setSelectedUrl(url);
    setTargetUrl("");
  }, [targetUrl, endpoints]);

  const handleRemoveEndpoint = useCallback((url: string) => {
    setEndpoints((prev) => { const next = prev.filter((e) => e.url !== url); saveScanQueueToDb(next, projectPath); return next; });
    invoke("scan_queue_remove", { url, projectPath }).catch(() => {});
    if (selectedUrl === url) setSelectedUrl(null);
  }, [selectedUrl, projectPath]);

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
      const id = await zapStartScan(url, undefined, undefined, null, selectedPolicy || null);
      setEndpoints((prev) => prev.map((e) => e.url === url ? { ...e, scanId: id } : e));
    } catch {
      setEndpoints((prev) => prev.map((e) => e.url === url ? { ...e, status: "error" } : e));
    }
  }, [applyCredential, selectedPolicy]);

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
    const active = endpoints.filter((e) => (e.status === "scanning" || e.status === "paused") && e.scanId);
    if (active.length === 0) return;
    const poll = async () => {
      for (const ep of active) {
        try {
          const [prog, alerts, msgCount] = await Promise.all([
            zapScanProgress(ep.scanId!),
            zapGetAlerts(ep.url, 0, 200),
            invoke<number>("zap_scan_message_count", { scanId: ep.scanId! }).catch(() => 0),
          ]);
          const isComplete = prog.progress >= 100;
          if (isComplete && alerts.length > 0) {
            importZapAlerts(alerts, "ZAP Active Scan", projectPath);
          }
          setEndpoints((prev) => {
            const next = prev.map((e) => {
              if (e.url !== ep.url) return e;
              const newStatus = isComplete ? "complete" as const
                : e.status === "paused" ? "paused" as const
                : "scanning" as const;
              return { ...e, progress: prog.progress, alerts, messageCount: msgCount, status: newStatus };
            });
            saveScanQueueToDb(next, projectPath);
            return next;
          });
        } catch { /* ignore */ }
      }
    };
    poll();
    intervalRef.current = setInterval(poll, 2000);
    return () => { if (intervalRef.current) clearInterval(intervalRef.current); };
  }, [endpoints.filter((e) => e.status === "scanning" || e.status === "paused").map((e) => `${e.url}:${e.scanId}`).join(",")]);

  const handleStopAll = useCallback(async () => {
    for (const ep of endpoints.filter((e) => (e.status === "scanning" || e.status === "paused") && e.scanId)) {
      await zapStopScan(ep.scanId!).catch(() => {});
    }
    setEndpoints((prev) => prev.map((e) => e.status === "scanning" || e.status === "spidering" || e.status === "paused" ? { ...e, status: "queued" } : e));
    setScanning(false);
  }, [endpoints]);

  const handlePauseAll = useCallback(async () => {
    for (const ep of endpoints.filter((e) => e.status === "scanning" && e.scanId)) {
      await zapPauseScan(ep.scanId!).catch(() => {});
    }
    setEndpoints((prev) => {
      const next = prev.map((e) => e.status === "scanning" ? { ...e, status: "paused" as const } : e);
      saveScanQueueToDb(next, projectPath);
      return next;
    });
  }, [endpoints, projectPath]);

  const handleResumeAll = useCallback(async () => {
    for (const ep of endpoints.filter((e) => e.status === "paused" && e.scanId)) {
      await zapResumeScan(ep.scanId!).catch(() => {});
    }
    setEndpoints((prev) => {
      const next = prev.map((e) => e.status === "paused" ? { ...e, status: "scanning" as const } : e);
      saveScanQueueToDb(next, projectPath);
      return next;
    });
  }, [endpoints, projectPath]);

  const handleClearCompleted = useCallback(() => {
    setEndpoints((prev) => { const next = prev.filter((e) => e.status !== "complete"); saveScanQueueToDb(next, projectPath); return next; });
    invoke("scan_queue_clear_completed", { projectPath }).catch(() => {});
  }, [projectPath]);

  const handleClearAll = useCallback(async () => {
    for (const ep of endpoints.filter((e) => (e.status === "scanning" || e.status === "paused") && e.scanId)) {
      await zapStopScan(ep.scanId!).catch(() => {});
    }
    setEndpoints([]);
    setSelectedUrl(null);
    setScanning(false);
    saveScanQueueToDb([], projectPath);
  }, [endpoints, projectPath]);

  const sel = endpoints.find((e) => e.url === selectedUrl);

  useEffect(() => {
    if (!sel || sel.status !== "complete") { setScanLogs([]); return; }
    invoke("passive_scans_by_url", { url: sel.url, limit: 500 })
      .then((data: any) => setScanLogs(Array.isArray(data) ? data : []))
      .catch(() => setScanLogs([]));
  }, [sel?.url, sel?.status]);

  const totalAlerts = endpoints.reduce((acc, e) => acc + e.alerts.length, 0);
  const completedCount = endpoints.filter((e) => e.status === "complete").length;
  const scanningCount = endpoints.filter((e) => e.status === "scanning" || e.status === "spidering" || e.status === "paused").length;
  const pausedCount = endpoints.filter((e) => e.status === "paused").length;
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

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
}

function normalizeHost(raw: string): string {
  let h = raw.toLowerCase().trim();
  h = h.replace(/^https?:\/\//, "");
  h = h.replace(/\/.*$/, "");
  return h;
}

function getRootDomain(host: string): string {
  const bare = host.replace(/:\d+$/, "");
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(bare)) return host;
  if (bare.startsWith("[") || bare.includes(":")) return host;
  const parts = bare.split(".");
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

function SiteMapPanel({ onSendToRepeater, onSendToIntruder, onActiveScan, onBatchScan }: { onSendToRepeater: (raw: string) => void; onSendToIntruder?: (raw: string) => void; onActiveScan?: (url: string) => void; onBatchScan?: (urls: string[]) => void }) {
  const { t } = useTranslation();
  const currentProjectPath = useStore((s) => s.currentProjectPath);
  const [siteMapData, setSiteMapData] = useState<SiteMapData | null>(null);
  const [allEntries, setAllEntries] = useState<HttpHistoryEntry[]>([]);
  const [selectedEntry, setSelectedEntry] = useState<HttpHistoryEntry | null>(null);
  const [detail, setDetail] = useState<HttpMessageDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [search, setSearch] = useState("");
  const [filterMode, setFilterMode] = useState<"all" | "api" | "js" | "captured">("all");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; entry: HttpHistoryEntry } | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [hideBeforeId, setHideBeforeId] = useState(0);
  const [scanSelection, setScanSelection] = useState<Set<string>>(new Set());

  const entryKey = (e: HttpHistoryEntry) => `${e.method}:${e.url}`;

  const captureMap = useMemo(() => {
    if (!siteMapData?.entries) return new Map<string, SiteMapEntry>();
    const map = new Map<string, SiteMapEntry>();
    for (const entry of Object.values(siteMapData.entries)) {
      const key = `${entry.method}:${(entry.url || "").split("?")[0].split("#")[0]}`;
      map.set(key, entry);
    }
    return map;
  }, [siteMapData]);

  const entries = useMemo(() => allEntries.filter((e) => e.id > hideBeforeId), [allEntries, hideBeforeId]);

  const loadEntries = useCallback(async () => {
    try {
      const dbData = await zapGetSiteMapData(currentProjectPath);
      setSiteMapData(dbData);

      let items: HttpHistoryEntry[] = [];
      try {
        items = await zapGetHistory(0, 2000);
      } catch { /* ZAP not running */ }

      if (items.length === 0 && dbData?.entries) {
        // Fallback: reconstruct entries from persisted DB data
        let idx = 1;
        items = Object.values(dbData.entries).map((e) => ({
          id: idx++,
          method: e.method || "GET",
          url: e.url,
          status_code: e.status_code,
          content_length: e.content_length || 0,
          time_ms: 0,
          timestamp: e.last_seen || e.first_seen || "",
          host: e.host,
          path: e.path || new URL(e.url).pathname,
        }));
      }
      setAllEntries(items);
    } catch { /* ignore */ }
  }, [currentProjectPath]);

  useEffect(() => {
    loadEntries();
    // Listen for backend event when new sitemap data arrives
    let unlisten: (() => void) | null = null;
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("sitemap-updated", () => loadEntries()).then((fn) => { unlisten = fn; });
    });
    // Fallback: poll every 15s in case events are missed
    intervalRef.current = setInterval(loadEntries, 15000);
    return () => {
      if (unlisten) unlisten();
      if (intervalRef.current) clearInterval(intervalRef.current);
    };
  }, [loadEntries]);

  const isCaptured = useCallback((entry: HttpHistoryEntry) => {
    const pathNoQuery = (entry.url || "").split("?")[0].split("#")[0];
    const key = `${entry.method}:${pathNoQuery}`;
    return captureMap.get(key)?.captured ?? false;
  }, [captureMap]);

  const getCaptureInfo = useCallback((entry: HttpHistoryEntry) => {
    const pathNoQuery = (entry.url || "").split("?")[0].split("#")[0];
    const key = `${entry.method}:${pathNoQuery}`;
    return captureMap.get(key);
  }, [captureMap]);

  const filtered = useMemo(() => {
    let items = entries;
    if (filterMode === "js") {
      items = items.filter((e) => /\.(js|jsx|ts|tsx|mjs|cjs|css|woff2?|svg|png|jpg|gif|ico)(\?|$)/i.test(e.path));
    } else if (filterMode === "api") {
      items = items.filter((e) => !(/\.(js|jsx|ts|tsx|mjs|cjs|css|woff2?|svg|png|jpg|gif|ico|html?)(\?|$)/i.test(e.path)));
    } else if (filterMode === "captured") {
      items = items.filter((e) => isCaptured(e));
    }
    if (search.trim()) {
      const q = search.toLowerCase();
      items = items.filter((e) =>
        e.url.toLowerCase().includes(q) || e.host.toLowerCase().includes(q)
      );
    }
    return items;
  }, [entries, search, filterMode, isCaptured]);

  const deduped = useMemo(() => {
    const seen = new Map<string, HttpHistoryEntry>();
    for (const e of filtered) {
      const pathNoQuery = (e.path || "/").split("?")[0].split("#")[0];
      const host = normalizeHost(e.host || e.url);
      const key = `${e.method}:${host}${pathNoQuery}`;
      const existing = seen.get(key);
      if (!existing) {
        seen.set(key, e);
        continue;
      }
      const newIsOk = e.status_code >= 200 && e.status_code < 300;
      const oldIsOk = existing.status_code >= 200 && existing.status_code < 300;
      // Prefer 2xx over non-2xx; among same quality, prefer the most recent
      if ((newIsOk && !oldIsOk) || (newIsOk === oldIsOk && e.id > existing.id)) {
        seen.set(key, e);
      }
    }
    return [...seen.values()];
  }, [filtered]);

  const handleBatchScan = useCallback(() => {
    if (scanSelection.size === 0) return;
    const urls = deduped
      .filter((e) => scanSelection.has(entryKey(e)))
      .map((e) => e.url);
    if (onBatchScan) {
      onBatchScan(urls);
    } else if (onActiveScan) {
      urls.forEach((url) => onActiveScan(url));
    }
    setScanSelection(new Set());
  }, [scanSelection, deduped, onBatchScan, onActiveScan]);

  const tree = useMemo(() => buildSiteTree(deduped), [deduped]);

  const handleSelectEntry = useCallback(async (entry: HttpHistoryEntry) => {
    setSelectedEntry(entry);
    setDetailLoading(true);
    try {
      const msg = await zapGetMessage(entry.id);
      if (msg && (msg.request_headers || msg.response_headers)) {
        setDetail(msg);
      } else {
        throw new Error("empty");
      }
    } catch {
      setDetail({
        id: entry.id,
        method: entry.method,
        url: entry.url,
        status_code: entry.status_code,
        request_headers: `${entry.method} ${new URL(entry.url).pathname} HTTP/1.1\r\nHost: ${entry.host}\r\n`,
        request_body: "",
        response_headers: entry.status_code ? `HTTP/1.1 ${entry.status_code}\r\nContent-Length: ${entry.content_length}\r\n` : "",
        response_body: `(Response body not available — ZAP session expired.\nURL: ${entry.url}\nStatus: ${entry.status_code}\nSize: ${entry.content_length} bytes)`,
      });
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
  const capturedCount = useMemo(() => deduped.filter((e) => isCaptured(e)).length, [deduped, isCaptured]);
  const siteContainerRef = useRef<HTMLDivElement>(null);
  const [siteDetailWidth, setSiteDetailWidth] = useState<number | null>(null);

  useEffect(() => {
    if (selectedEntry && siteDetailWidth === null && siteContainerRef.current) {
      setSiteDetailWidth(Math.floor(siteContainerRef.current.offsetWidth / 2));
    }
  }, [selectedEntry, siteDetailWidth]);

  const handleSiteResize = useCallback((delta: number) => {
    const maxW = siteContainerRef.current ? siteContainerRef.current.offsetWidth - 280 : 900;
    setSiteDetailWidth((w) => Math.min(maxW, Math.max(280, (w ?? 520) - delta)));
  }, []);

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
        {(["all", "api", "js", "captured"] as const).map((mode) => (
          <button
            key={mode}
            type="button"
            onClick={() => setFilterMode(mode)}
            className={cn(
              "px-2.5 py-1 text-[10px] rounded-md font-medium transition-colors",
              filterMode === mode
                ? mode === "api" ? "bg-green-500/15 text-green-400"
                  : mode === "js" ? "bg-yellow-500/15 text-yellow-400"
                  : mode === "captured" ? "bg-blue-500/15 text-blue-400"
                  : "bg-accent/15 text-accent"
                : "text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)]"
            )}
          >
            {mode === "all" ? "All" : mode === "api" ? "API" : mode === "js" ? "Static" : "Saved"}
          </button>
        ))}
        <div className="flex items-center gap-1 px-1.5 py-0.5 rounded-md bg-accent/8 text-accent text-[10px]">
          <TreePine className="w-3 h-3" />
        </div>
        {scanSelection.size > 0 && (
          <button
            type="button"
            onClick={handleBatchScan}
            className="flex items-center gap-1 px-2.5 py-1 text-[10px] font-medium rounded-md bg-orange-500/15 text-orange-400 hover:bg-orange-500/25 transition-colors"
          >
            <Zap className="w-3 h-3" />
            Scan {scanSelection.size} selected
          </button>
        )}
        <span className="text-[10px] text-muted-foreground/30">
          {hostCount} {t("security.hosts", "hosts")} · {endpointCount} {t("security.endpoints", "endpoints")}{capturedCount > 0 && <> · <span className="text-blue-400">{capturedCount} saved</span></>}
        </span>
        <button
          type="button"
          onClick={() => {
            if (!confirm(t("security.clearSiteMapConfirm", "Clear all site map data?"))) return;
            const maxId = allEntries.reduce((m, e) => Math.max(m, e.id), 0);
            setHideBeforeId(maxId);
            setSelectedEntry(null);
            setDetail(null);
          }}
          title={t("security.clearSiteMap", "Clear site map")}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-destructive hover:bg-destructive/10 transition-colors"
        >
          <Trash2 className="w-3 h-3" />
        </button>
        <button
          type="button"
          onClick={loadEntries}
          className="p-1.5 rounded-md text-muted-foreground/40 hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
        >
          <RefreshCw className="w-3 h-3" />
        </button>
      </div>

      <div ref={siteContainerRef} className="flex-1 flex min-h-0">
        <div className="flex-1 overflow-y-auto min-w-0">
          {deduped.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-muted-foreground/20">
              <Globe className="w-8 h-8" />
              <p className="text-[11px]">{t("security.noSiteData", "No site data yet")}</p>
            </div>
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

        {(selectedEntry || detailLoading) && <ResizeHandle onResize={handleSiteResize} />}

        <div className="flex flex-col overflow-hidden" style={{ width: (selectedEntry || detailLoading) ? (siteDetailWidth ?? 0) : 0, flexShrink: 0 }}>
          {selectedEntry && detail ? (
            <div className="flex-1 flex flex-col min-h-0">
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
                {isCaptured(selectedEntry) && (
                  <span className="flex items-center gap-1 px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 text-[9px] font-medium">
                    <Download className="w-2.5 h-2.5" />
                    Saved
                    {getCaptureInfo(selectedEntry)?.capture?.file_size != null && (
                      <span className="text-blue-400/60">
                        ({formatBytes(getCaptureInfo(selectedEntry)!.capture!.file_size)})
                      </span>
                    )}
                  </span>
                )}
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
              <DetailTabs detail={detail} />
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
      {ctxMenu && createPortal(
        <div
          className="fixed z-[9999] min-w-[160px] rounded-lg border border-border/20 bg-popover shadow-lg py-1 text-[11px]"
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
          {onSendToIntruder && (
            <button
              type="button"
              className="w-full flex items-center gap-2 px-3 py-1.5 text-left hover:bg-accent/10 transition-colors"
              onClick={async () => {
                try { const msg = await zapGetMessage(ctxMenu.entry.id); onSendToIntruder(buildRawRequest(msg)); } catch { /* ignore */ }
                setCtxMenu(null);
              }}
            >
              <Crosshair className="w-3 h-3 text-orange-400" />
              Send to Intruder
            </button>
          )}
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
        </div>,
        document.body,
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
  const [open, setOpen] = useState(false);
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

function saveCustomRulesToDb(rules: CustomPassiveRule[], pp: string | null) {
  invoke("custom_rules_save_all", { rules, projectPath: pp }).catch(() => {});
}

function PassiveScanPanel() {
  const { t } = useTranslation();
  const [enabled, setEnabled] = useState(true);
  const [records, setRecords] = useState(0);
  const [rules, setRules] = useState<PassiveRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const [tab, setTab] = useState<"zap" | "custom">("zap");
  const projectPath = useStore((s) => s.currentProjectPath);
  const [customRules, setCustomRules] = useState<CustomPassiveRule[]>([]);
  const [editing, setEditing] = useState<CustomPassiveRule | null>(null);
  const [matches, setMatches] = useState<CustomRuleMatch[]>([]);
  const [scanning, setScanning] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLoading(true);
      try {
        type ZapJson = Record<string, unknown>;
        const [recordsResult, scannersResult, dbRules] = await Promise.all([
          invoke<ZapJson>("zap_api_call", { component: "pscan", actionType: "view", method: "recordsToScan", params: {} }).catch(() => ({})),
          invoke<ZapJson>("zap_api_call", { component: "pscan", actionType: "view", method: "scanners", params: {} }).catch(() => ({})),
          invoke<CustomPassiveRule[]>("custom_rules_list", { projectPath: getProjectPath() }).catch(() => []),
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
        if (Array.isArray(dbRules) && dbRules.length > 0) setCustomRules(dbRules);
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
      saveCustomRulesToDb(next, projectPath);
      return next;
    });
    invoke("custom_rules_upsert", { rule, projectPath }).catch(() => {});
    setEditing(null);
  }, [projectPath]);

  const handleDeleteCustomRule = useCallback((id: string) => {
    setCustomRules((prev) => {
      const next = prev.filter((r) => r.id !== id);
      saveCustomRulesToDb(next, projectPath);
      return next;
    });
    invoke("custom_rules_delete", { id }).catch(() => {});
    setMatches((prev) => prev.filter((m) => m.ruleId !== id));
  }, [projectPath]);

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
            <StyledSelect
              value={formSeverity}
              onChange={(v) => setFormSeverity(v as "low" | "medium" | "high")}
              options={[{ value: "low", label: "Low" }, { value: "medium", label: "Medium" }, { value: "high", label: "High" }]}
              className="h-7"
            />
            <StyledSelect
              value={formScope}
              onChange={(v) => setFormScope(v as "body" | "headers" | "all")}
              options={[{ value: "all", label: "Body + Headers" }, { value: "body", label: "Body Only" }, { value: "headers", label: "Headers Only" }]}
              className="h-7"
            />
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

// (Op Logs, Recon Data, JS Analysis panels removed — now live in TargetPanel)

function _dead_OpLogPanel() {
  const [entries, setEntries] = useState<AuditRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const currentProjectPath = useStore((s) => s.currentProjectPath);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const pp = getProjectPath() ?? "";
      const data = query.trim()
        ? await oplogSearch(pp, query.trim(), 500)
        : await oplogList(pp, 500);
      setEntries(Array.isArray(data) ? data : []);
    } catch {
      setEntries([]);
    }
    setLoading(false);
  }, [query]);

  useEffect(() => { load(); }, [load, currentProjectPath]);

  const fmt = (ts: number) => new Date(ts).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", second: "2-digit",
  });

  const statusDot: Record<string, string> = {
    completed: "bg-green-500",
    running: "bg-yellow-500 animate-pulse",
    failed: "bg-red-500",
    pending: "bg-zinc-500",
  };

  const catColor: Record<string, string> = {
    security: "text-emerald-400 bg-emerald-500/10",
    scan: "text-teal-400 bg-teal-500/10",
    recon: "text-sky-400 bg-sky-500/10",
    targets: "text-blue-400 bg-blue-500/10",
    findings: "text-orange-400 bg-orange-500/10",
  };

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-3 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <Database className="w-3.5 h-3.5 text-accent" />
        <span className="text-[12px] font-medium text-foreground/80">Operation Logs</span>
        <span className="text-[10px] text-muted-foreground/30">{entries.length} entries</span>
        <div className="flex-1" />
        <div className="flex items-center gap-1.5 bg-muted/10 rounded-lg px-2 py-1">
          <Search className="w-3 h-3 text-muted-foreground/30" />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && load()}
            placeholder="Search..."
            className="bg-transparent text-[10px] text-foreground outline-none w-32 placeholder:text-muted-foreground/20"
          />
        </div>
        <button onClick={load} className="p-1 text-muted-foreground/30 hover:text-foreground transition-colors">
          <RefreshCw className={cn("w-3 h-3", loading && "animate-spin")} />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {entries.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <Database className="w-10 h-10" />
            <p className="text-[12px]">No operation logs yet</p>
            <p className="text-[10px] text-muted-foreground/15">AI agent operations will be logged here automatically</p>
          </div>
        ) : (
          <table className="w-full text-[10px]">
            <thead className="sticky top-0 bg-background/95 z-10">
              <tr className="text-muted-foreground/30 text-left border-b border-border/10">
                <th className="px-4 py-2 font-medium w-[140px]">Time</th>
                <th className="px-2 py-2 font-medium w-[24px]" />
                <th className="px-2 py-2 font-medium w-[80px]">Category</th>
                <th className="px-2 py-2 font-medium">Action</th>
                <th className="px-2 py-2 font-medium w-[100px]">Tool</th>
                <th className="px-2 py-2 font-medium w-[40px]" />
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => {
                const isExpanded = expandedId === e.id;
                const hasJsonDetail = e.detail && Object.keys(e.detail).length > 0;
                return (
                  <tr key={e.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/20 group">
                    <td className="px-4 py-1.5 text-muted-foreground/30 font-mono whitespace-nowrap">{fmt(e.createdAt)}</td>
                    <td className="px-2 py-1.5">
                      <span className={cn("inline-block w-1.5 h-1.5 rounded-full", statusDot[e.status] ?? "bg-zinc-500")} />
                    </td>
                    <td className="px-2 py-1.5">
                      <span className={cn("text-[9px] font-medium px-1.5 py-0.5 rounded", catColor[e.category] ?? "text-muted-foreground/50 bg-muted/5")}>{e.category}</span>
                    </td>
                    <td className="px-2 py-1.5">
                      <span className="text-foreground/70">{e.action}</span>
                      {e.details && <span className="text-muted-foreground/30 ml-1.5">{e.details.length > 80 ? `${e.details.slice(0, 80)}…` : e.details}</span>}
                    </td>
                    <td className="px-2 py-1.5">
                      {e.toolName && <span className="font-mono text-accent/50 bg-accent/5 px-1.5 py-0.5 rounded">{e.toolName}</span>}
                    </td>
                    <td className="px-2 py-1.5">
                      {hasJsonDetail && (
                        <button
                          onClick={() => setExpandedId(isExpanded ? null : e.id)}
                          className="text-muted-foreground/20 hover:text-foreground transition-colors"
                        >
                          {isExpanded ? <ChevronDown className="w-3 h-3" /> : <ChevronRight className="w-3 h-3" />}
                        </button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

// ── Recon Data Panel (Assets, Endpoints, Fingerprints) ──

function ReconDataPanel() {
  const [subTab, setSubTab] = useState<"assets" | "endpoints" | "fingerprints">("assets");
  const [targetId, setTargetId] = useState("");
  const [assets, setAssets] = useState<TargetAsset[]>([]);
  const [endpoints, setEndpoints] = useState<ApiEndpoint[]>([]);
  const [fingerprints, setFingerprints] = useState<Fingerprint[]>([]);
  const [loading, setLoading] = useState(false);

  const loadData = useCallback(async () => {
    if (!targetId.trim()) return;
    setLoading(true);
    try {
      const [a, e, f] = await Promise.all([
        targetAssetsList(targetId).catch(() => []),
        apiEndpointsList(targetId).catch(() => []),
        fingerprintsList(targetId).catch(() => []),
      ]);
      setAssets(Array.isArray(a) ? a : []);
      setEndpoints(Array.isArray(e) ? e : []);
      setFingerprints(Array.isArray(f) ? f : []);
    } catch {
      /* ignore */
    }
    setLoading(false);
  }, [targetId]);

  const subTabs: { id: "assets" | "endpoints" | "fingerprints"; label: string; count: number }[] = [
    { id: "assets", label: "Assets", count: assets.length },
    { id: "endpoints", label: "Endpoints", count: endpoints.length },
    { id: "fingerprints", label: "Fingerprints", count: fingerprints.length },
  ];

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-3 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <Globe className="w-3.5 h-3.5 text-accent" />
        <span className="text-[12px] font-medium text-foreground/80">Recon Data</span>
        <div className="flex-1" />
        <div className="flex items-center gap-1.5 bg-muted/10 rounded-lg px-2 py-1">
          <Search className="w-3 h-3 text-muted-foreground/30" />
          <input
            type="text"
            value={targetId}
            onChange={(e) => setTargetId(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && loadData()}
            placeholder="Target UUID..."
            className="bg-transparent text-[10px] text-foreground outline-none w-48 placeholder:text-muted-foreground/20 font-mono"
          />
        </div>
        <button onClick={loadData} className="px-2.5 py-1 rounded-md text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors">
          Load
        </button>
      </div>

      {/* Sub-tabs */}
      <div className="flex items-center gap-1 px-4 py-1.5 border-b border-border/5 flex-shrink-0">
        {subTabs.map((st) => (
          <button
            key={st.id}
            onClick={() => setSubTab(st.id)}
            className={cn(
              "px-2.5 py-1 rounded-md text-[10px] font-medium transition-colors",
              subTab === st.id ? "bg-accent/15 text-accent" : "text-muted-foreground/40 hover:text-foreground"
            )}
          >
            {st.label}
            {st.count > 0 && <span className="ml-1 text-[8px] text-muted-foreground/30">({st.count})</span>}
          </button>
        ))}
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="h-full flex items-center justify-center">
            <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
          </div>
        ) : !targetId.trim() ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <Globe className="w-10 h-10" />
            <p className="text-[12px]">Enter a target UUID to view recon data</p>
            <p className="text-[10px] text-muted-foreground/15">Data is populated by the AI agent during security analysis</p>
          </div>
        ) : subTab === "assets" ? (
          <ReconAssetsTable data={assets} />
        ) : subTab === "endpoints" ? (
          <ReconEndpointsTable data={endpoints} />
        ) : (
          <ReconFingerprintsTable data={fingerprints} />
        )}
      </div>
    </div>
  );
}

function ReconAssetsTable({ data }: { data: TargetAsset[] }) {
  if (data.length === 0) {
    return <div className="text-center text-[11px] text-muted-foreground/20 py-12">No assets discovered</div>;
  }
  return (
    <table className="w-full text-[10px]">
      <thead className="sticky top-0 bg-background/95 z-10">
        <tr className="text-muted-foreground/30 text-left border-b border-border/10">
          <th className="px-4 py-2 font-medium">Type</th>
          <th className="px-2 py-2 font-medium">Value</th>
          <th className="px-2 py-2 font-medium w-[60px]">Port</th>
          <th className="px-2 py-2 font-medium w-[80px]">Service</th>
          <th className="px-2 py-2 font-medium w-[80px]">Version</th>
          <th className="px-2 py-2 font-medium w-[70px]">Status</th>
        </tr>
      </thead>
      <tbody>
        {data.map((a) => (
          <tr key={a.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/20">
            <td className="px-4 py-1.5">
              <span className="text-[9px] font-medium px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400">{a.assetType}</span>
            </td>
            <td className="px-2 py-1.5 font-mono text-foreground/70">{a.value}</td>
            <td className="px-2 py-1.5 text-muted-foreground/40 font-mono">{a.port ?? "-"}</td>
            <td className="px-2 py-1.5 text-muted-foreground/40">{a.service ?? "-"}</td>
            <td className="px-2 py-1.5 text-muted-foreground/40">{a.version ?? "-"}</td>
            <td className="px-2 py-1.5">
              <span className={cn(
                "text-[9px] px-1.5 py-0.5 rounded",
                a.status === "active" ? "bg-green-500/10 text-green-400" : "bg-zinc-500/10 text-zinc-400"
              )}>{a.status}</span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ReconEndpointsTable({ data }: { data: ApiEndpoint[] }) {
  if (data.length === 0) {
    return <div className="text-center text-[11px] text-muted-foreground/20 py-12">No endpoints discovered</div>;
  }
  const riskBadge: Record<string, string> = {
    critical: "bg-red-500/10 text-red-400",
    high: "bg-orange-500/10 text-orange-400",
    medium: "bg-yellow-500/10 text-yellow-400",
    low: "bg-blue-500/10 text-blue-400",
    info: "bg-zinc-500/10 text-zinc-400",
  };
  return (
    <table className="w-full text-[10px]">
      <thead className="sticky top-0 bg-background/95 z-10">
        <tr className="text-muted-foreground/30 text-left border-b border-border/10">
          <th className="px-4 py-2 font-medium w-[60px]">Method</th>
          <th className="px-2 py-2 font-medium">Path</th>
          <th className="px-2 py-2 font-medium w-[60px]">Auth</th>
          <th className="px-2 py-2 font-medium w-[60px]">Risk</th>
          <th className="px-2 py-2 font-medium w-[60px]">Tested</th>
          <th className="px-2 py-2 font-medium w-[70px]">Source</th>
        </tr>
      </thead>
      <tbody>
        {data.map((ep) => (
          <tr key={ep.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/20">
            <td className={cn("px-4 py-1.5 font-mono font-medium", methodColor(ep.method))}>{ep.method}</td>
            <td className="px-2 py-1.5 font-mono text-foreground/60">{ep.path}</td>
            <td className="px-2 py-1.5 text-muted-foreground/40">{ep.authType ?? "-"}</td>
            <td className="px-2 py-1.5">
              <span className={cn("text-[9px] px-1.5 py-0.5 rounded", riskBadge[ep.riskLevel] ?? "bg-zinc-500/10 text-zinc-400")}>{ep.riskLevel}</span>
            </td>
            <td className="px-2 py-1.5">
              {ep.tested
                ? <Check className="w-3 h-3 text-green-400" />
                : <span className="text-muted-foreground/20">—</span>}
            </td>
            <td className="px-2 py-1.5 text-muted-foreground/30">{ep.source}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function ReconFingerprintsTable({ data }: { data: Fingerprint[] }) {
  if (data.length === 0) {
    return <div className="text-center text-[11px] text-muted-foreground/20 py-12">No fingerprints detected</div>;
  }
  return (
    <table className="w-full text-[10px]">
      <thead className="sticky top-0 bg-background/95 z-10">
        <tr className="text-muted-foreground/30 text-left border-b border-border/10">
          <th className="px-4 py-2 font-medium">Category</th>
          <th className="px-2 py-2 font-medium">Name</th>
          <th className="px-2 py-2 font-medium w-[80px]">Version</th>
          <th className="px-2 py-2 font-medium w-[80px]">Confidence</th>
          <th className="px-2 py-2 font-medium w-[120px]">CPE</th>
          <th className="px-2 py-2 font-medium w-[70px]">Source</th>
        </tr>
      </thead>
      <tbody>
        {data.map((fp) => (
          <tr key={fp.id} className="border-b border-border/5 hover:bg-[var(--bg-hover)]/20">
            <td className="px-4 py-1.5">
              <span className="text-[9px] font-medium px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400">{fp.category}</span>
            </td>
            <td className="px-2 py-1.5 text-foreground/70 font-medium">{fp.name}</td>
            <td className="px-2 py-1.5 font-mono text-muted-foreground/40">{fp.version ?? "-"}</td>
            <td className="px-2 py-1.5">
              <div className="flex items-center gap-1.5">
                <div className="w-12 h-1 rounded-full bg-muted/20 overflow-hidden">
                  <div
                    className={cn("h-full rounded-full", fp.confidence >= 80 ? "bg-green-500" : fp.confidence >= 50 ? "bg-yellow-500" : "bg-red-500")}
                    style={{ width: `${Math.min(100, fp.confidence)}%` }}
                  />
                </div>
                <span className="text-muted-foreground/30">{fp.confidence}%</span>
              </div>
            </td>
            <td className="px-2 py-1.5 font-mono text-[9px] text-muted-foreground/30 truncate max-w-[120px]">{fp.cpe ?? "-"}</td>
            <td className="px-2 py-1.5 text-muted-foreground/30">{fp.source}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ── JS Analysis Panel ──

function JsAnalysisPanel() {
  const [targetId, setTargetId] = useState("");
  const [results, setResults] = useState<JsAnalysisResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const loadData = useCallback(async () => {
    if (!targetId.trim()) return;
    setLoading(true);
    try {
      const data = await jsAnalysisList(targetId);
      setResults(Array.isArray(data) ? data : []);
    } catch {
      setResults([]);
    }
    setLoading(false);
  }, [targetId]);

  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-3 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <FileCode2 className="w-3.5 h-3.5 text-accent" />
        <span className="text-[12px] font-medium text-foreground/80">JS Analysis Results</span>
        <span className="text-[10px] text-muted-foreground/30">{results.length} files</span>
        <div className="flex-1" />
        <div className="flex items-center gap-1.5 bg-muted/10 rounded-lg px-2 py-1">
          <Search className="w-3 h-3 text-muted-foreground/30" />
          <input
            type="text"
            value={targetId}
            onChange={(e) => setTargetId(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && loadData()}
            placeholder="Target UUID..."
            className="bg-transparent text-[10px] text-foreground outline-none w-48 placeholder:text-muted-foreground/20 font-mono"
          />
        </div>
        <button onClick={loadData} className="px-2.5 py-1 rounded-md text-[10px] font-medium bg-accent/15 text-accent hover:bg-accent/25 transition-colors">
          Load
        </button>
      </div>

      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="h-full flex items-center justify-center">
            <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/30" />
          </div>
        ) : !targetId.trim() ? (
          <div className="flex flex-col items-center justify-center h-full gap-3 text-muted-foreground/20">
            <FileCode2 className="w-10 h-10" />
            <p className="text-[12px]">Enter a target UUID to view JS analysis</p>
            <p className="text-[10px] text-muted-foreground/15">Results from the js_analyzer sub-agent appear here</p>
          </div>
        ) : results.length === 0 ? (
          <div className="text-center text-[11px] text-muted-foreground/20 py-12">No JS analysis results</div>
        ) : (
          <div className="space-y-2 p-4">
            {results.map((r) => {
              const isExpanded = expandedId === r.id;
              const secretCount = Array.isArray(r.secretsFound) ? r.secretsFound.length : 0;
              const endpointCount = Array.isArray(r.endpointsFound) ? r.endpointsFound.length : 0;
              return (
                <div key={r.id} className="rounded-xl border border-border/10 bg-[var(--bg-hover)]/15 overflow-hidden">
                  <button
                    type="button"
                    onClick={() => setExpandedId(isExpanded ? null : r.id)}
                    className="w-full flex items-center gap-3 p-3 text-left hover:bg-[var(--bg-hover)]/30 transition-colors"
                  >
                    {isExpanded ? <ChevronDown className="w-3 h-3 text-muted-foreground/40" /> : <ChevronRight className="w-3 h-3 text-muted-foreground/40" />}
                    <FileCode2 className="w-3.5 h-3.5 text-yellow-400 flex-shrink-0" />
                    <span className="text-[11px] font-mono text-foreground/80 flex-1 truncate">{r.filename || r.url}</span>
                    {secretCount > 0 && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded bg-red-500/10 text-red-400 flex-shrink-0">
                        {secretCount} secrets
                      </span>
                    )}
                    {endpointCount > 0 && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-400 flex-shrink-0">
                        {endpointCount} endpoints
                      </span>
                    )}
                    {r.sourceMaps && (
                      <span className="text-[9px] px-1.5 py-0.5 rounded bg-yellow-500/10 text-yellow-400 flex-shrink-0">
                        source maps
                      </span>
                    )}
                  </button>
                  {isExpanded && (
                    <div className="border-t border-border/5 p-3 space-y-2">
                      <div className="grid grid-cols-3 gap-3 text-[10px]">
                        <div>
                          <span className="text-muted-foreground/40 block mb-1">URL</span>
                          <span className="font-mono text-foreground/60 text-[9px] break-all">{r.url}</span>
                        </div>
                        <div>
                          <span className="text-muted-foreground/40 block mb-1">Size</span>
                          <span className="text-foreground/60">{r.sizeBytes ? formatSize(r.sizeBytes) : "-"}</span>
                        </div>
                        <div>
                          <span className="text-muted-foreground/40 block mb-1">Risk Summary</span>
                          <span className="text-foreground/60">{r.riskSummary || "-"}</span>
                        </div>
                      </div>
                      {Array.isArray(r.frameworks) && r.frameworks.length > 0 && (
                        <div>
                          <span className="text-[9px] text-muted-foreground/40 block mb-1">Frameworks</span>
                          <div className="flex flex-wrap gap-1">
                            {r.frameworks.map((fw, i) => (
                              <span key={i} className="text-[9px] px-1.5 py-0.5 rounded bg-purple-500/10 text-purple-400">
                                {String(fw)}
                              </span>
                            ))}
                          </div>
                        </div>
                      )}
                      {Array.isArray(r.libraries) && r.libraries.length > 0 && (
                        <div>
                          <span className="text-[9px] text-muted-foreground/40 block mb-1">Libraries</span>
                          <div className="flex flex-wrap gap-1">
                            {r.libraries.map((lib, i) => (
                              <span key={i} className="text-[9px] px-1.5 py-0.5 rounded bg-cyan-500/10 text-cyan-400">
                                {String(lib)}
                              </span>
                            ))}
                          </div>
                        </div>
                      )}
                      {secretCount > 0 && (
                        <div>
                          <span className="text-[9px] text-red-400/70 block mb-1">Secrets Found</span>
                          <div className="space-y-0.5">
                            {r.secretsFound.map((s, i) => (
                              <div key={i} className="text-[9px] font-mono text-red-400/50 bg-red-500/5 px-2 py-1 rounded">{String(s)}</div>
                            ))}
                          </div>
                        </div>
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

// ═══════════════════════════════════════════════════════════════════════
// Nuclei Section
// ═══════════════════════════════════════════════════════════════════════

const SEV_COLORS: Record<string, string> = {
  critical: "bg-red-500/15 text-red-400 border-red-500/20",
  high: "bg-orange-500/15 text-orange-400 border-orange-500/20",
  medium: "bg-yellow-500/15 text-yellow-400 border-yellow-500/20",
  low: "bg-blue-500/15 text-blue-400 border-blue-500/20",
  info: "bg-zinc-500/10 text-zinc-400 border-zinc-500/20",
};

const SEV_DOT: Record<string, string> = {
  critical: "bg-red-500",
  high: "bg-orange-500",
  medium: "bg-yellow-500",
  low: "bg-blue-500",
  info: "bg-zinc-500",
};

function NucleiSection({ targetId, targetUrl }: { targetId: string; targetUrl: string }) {
  const [expanded, setExpanded] = useState(false);
  const [pocMatches, setPocMatches] = useState<PocMatch[]>([]);
  const [matchDone, setMatchDone] = useState(false);
  const [matching, setMatching] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [scanResult, setScanResult] = useState<ToolScanResult | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [showConfig, setShowConfig] = useState(false);
  const [opts, setOpts] = useState<NucleiScanOptions>({});
  const projectPath = getProjectPath();

  useEffect(() => {
    setPocMatches([]);
    setMatchDone(false);
    setScanResult(null);
    setScanError(null);
  }, [targetId]);

  const templateIds = useMemo(
    () => pocMatches.filter((p) => p.template_id).map((p) => p.template_id!),
    [pocMatches],
  );

  const sevGroups = useMemo(() => {
    const groups: Record<string, PocMatch[]> = {};
    for (const p of pocMatches) {
      const sev = (p.severity || "info").toLowerCase();
      (groups[sev] ??= []).push(p);
    }
    return groups;
  }, [pocMatches]);

  const handleMatchPocs = useCallback(async () => {
    setMatching(true);
    setScanError(null);
    try {
      const matches = await matchPocsForTarget(targetId);
      setPocMatches(matches);
      setMatchDone(true);
    } catch (e) {
      setScanError(String(e));
    } finally {
      setMatching(false);
    }
  }, [targetId]);

  const handleRunNuclei = useCallback(async () => {
    if (templateIds.length === 0) return;
    setScanning(true);
    setScanResult(null);
    setScanError(null);
    try {
      const result = await scanNucleiTargeted(
        targetUrl, targetId, templateIds, projectPath, undefined,
        Object.keys(opts).length > 0 ? opts : undefined,
      );
      setScanResult(result);
    } catch (e) {
      setScanError(String(e));
    } finally {
      setScanning(false);
    }
  }, [targetUrl, targetId, templateIds, projectPath, opts]);

  const handleRunAll = useCallback(async () => {
    setMatching(true);
    setScanResult(null);
    setScanError(null);
    try {
      const matches = await matchPocsForTarget(targetId);
      setPocMatches(matches);
      setMatchDone(true);

      const tids = matches.filter((p) => p.template_id).map((p) => p.template_id!);
      if (tids.length === 0) {
        setScanResult({ tool: "nuclei", success: true, items_found: 0, items_stored: 0, errors: [], duration_ms: 0 });
        return;
      }

      setMatching(false);
      setScanning(true);
      const result = await scanNucleiTargeted(
        targetUrl, targetId, tids, projectPath, undefined,
        Object.keys(opts).length > 0 ? opts : undefined,
      );
      setScanResult(result);
    } catch (e) {
      setScanError(String(e));
    } finally {
      setMatching(false);
      setScanning(false);
    }
  }, [targetId, targetUrl, projectPath, opts]);

  const handleCancel = useCallback(async () => {
    try { await invoke("nuclei_cancel"); } catch { /* ignore */ }
    setMatching(false);
    setScanning(false);
  }, []);

  const anyRunning = matching || scanning;

  return (
    <div className="rounded-lg border border-border/10 overflow-hidden">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 w-full px-3 py-2 text-left hover:bg-muted/10 transition-colors"
      >
        {expanded
          ? <ChevronDown className="w-3 h-3 text-muted-foreground/30" />
          : <ChevronRight className="w-3 h-3 text-muted-foreground/30" />}
        <ShieldAlert className="w-3.5 h-3.5 text-red-400/70" />
        <span className="text-[11px] font-medium text-foreground/80">Nuclei Vulnerability Scan</span>
        {anyRunning && <Loader2 className="w-3 h-3 animate-spin text-accent ml-1" />}
        {scanResult && !anyRunning && (
          <span className={cn(
            "ml-auto text-[9px] px-1.5 py-0.5 rounded",
            scanResult.items_found > 0 ? "bg-red-500/10 text-red-400" : "bg-green-500/10 text-green-400",
          )}>
            {scanResult.items_found > 0
              ? `${scanResult.items_found} vuln${scanResult.items_found !== 1 ? "s" : ""}`
              : "Clean"}
          </span>
        )}
        {pocMatches.length > 0 && !scanResult && !anyRunning && (
          <span className="ml-auto text-[9px] text-muted-foreground/40">
            {templateIds.length} templates ready
          </span>
        )}
        {matchDone && pocMatches.length === 0 && !scanResult && !anyRunning && (
          <span className="ml-auto text-[9px] text-muted-foreground/30">No matches</span>
        )}
      </button>

      {expanded && (
        <div className="border-t border-border/5 px-3 py-2.5 space-y-2.5">
          {/* Action buttons */}
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={anyRunning}
              onClick={handleRunAll}
              className={cn(
                "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[10px] font-medium transition-colors",
                anyRunning
                  ? "bg-muted/10 text-muted-foreground cursor-not-allowed"
                  : "bg-accent/10 text-accent hover:bg-accent/20 border border-accent/20",
              )}
            >
              {anyRunning
                ? <><Loader2 className="w-3 h-3 animate-spin" /> {matching ? "Matching PoCs..." : "Scanning..."}</>
                : <><Play className="w-3 h-3" /> Match & Scan</>}
            </button>

            <button
              type="button"
              disabled={anyRunning}
              onClick={handleMatchPocs}
              className="flex items-center gap-1 px-2 py-1.5 rounded-md text-[10px] text-muted-foreground/50 hover:text-foreground hover:bg-muted/10 transition-colors disabled:opacity-30"
            >
              <Shield className="w-3 h-3" /> Match Only
            </button>

            {templateIds.length > 0 && !anyRunning && (
              <button
                type="button"
                onClick={handleRunNuclei}
                className="flex items-center gap-1 px-2 py-1.5 rounded-md text-[10px] text-muted-foreground/50 hover:text-foreground hover:bg-muted/10 transition-colors"
              >
                <Crosshair className="w-3 h-3" /> Scan Only ({templateIds.length})
              </button>
            )}
            {anyRunning && (
              <button
                type="button"
                onClick={handleCancel}
                className="flex items-center gap-1 px-2 py-1.5 rounded-md text-[10px] text-red-400 hover:bg-red-500/10 transition-colors border border-red-500/20"
              >
                <XCircle className="w-3 h-3" /> Cancel
              </button>
            )}

            <button
              type="button"
              onClick={() => setShowConfig(!showConfig)}
              className={cn(
                "ml-auto p-1 rounded transition-colors",
                showConfig ? "text-accent bg-accent/10" : "text-muted-foreground/20 hover:text-muted-foreground/50",
              )}
              title="Configuration"
            >
              <Activity className="w-3.5 h-3.5" />
            </button>
          </div>

          {/* Configuration */}
          {showConfig && (
            <div className="space-y-1.5 p-2 rounded-md bg-background/30 border border-border/10">
              <div className="text-[9px] font-medium text-accent/60">Nuclei Configuration</div>
              <div className="grid grid-cols-3 gap-1.5">
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Rate (req/s)</label>
                  <input
                    type="number"
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="150"
                    value={opts.rate_limit ?? ""}
                    onChange={(e) => setOpts({ ...opts, rate_limit: e.target.value ? Number(e.target.value) : undefined })}
                  />
                </div>
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Bulk</label>
                  <input
                    type="number"
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="25"
                    value={opts.bulk_size ?? ""}
                    onChange={(e) => setOpts({ ...opts, bulk_size: e.target.value ? Number(e.target.value) : undefined })}
                  />
                </div>
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Concurrency</label>
                  <input
                    type="number"
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="25"
                    value={opts.concurrency ?? ""}
                    onChange={(e) => setOpts({ ...opts, concurrency: e.target.value ? Number(e.target.value) : undefined })}
                  />
                </div>
              </div>
              <div className="space-y-0.5">
                <label className="text-[9px] text-muted-foreground/50">Tags (include)</label>
                <input
                  className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                  placeholder="cve,rce,sqli,xss"
                  value={opts.tags?.join(", ") ?? ""}
                  onChange={(e) => {
                    const v = e.target.value.trim();
                    setOpts({ ...opts, tags: v ? v.split(/\s*,\s*/).filter(Boolean) : undefined });
                  }}
                />
              </div>
              <div className="space-y-0.5">
                <label className="text-[9px] text-muted-foreground/50">Exclude Tags</label>
                <input
                  className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                  placeholder="dos,fuzz"
                  value={opts.exclude_tags?.join(", ") ?? ""}
                  onChange={(e) => {
                    const v = e.target.value.trim();
                    setOpts({ ...opts, exclude_tags: v ? v.split(/\s*,\s*/).filter(Boolean) : undefined });
                  }}
                />
              </div>
              <div className="grid grid-cols-2 gap-1.5">
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Proxy</label>
                  <input
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="http://127.0.0.1:8080"
                    value={opts.proxy ?? ""}
                    onChange={(e) => setOpts({ ...opts, proxy: e.target.value || undefined })}
                  />
                </div>
                <div className="space-y-0.5">
                  <label className="text-[9px] text-muted-foreground/50">Timeout (s)</label>
                  <input
                    type="number"
                    className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                    placeholder="10"
                    value={opts.timeout ?? ""}
                    onChange={(e) => setOpts({ ...opts, timeout: e.target.value ? Number(e.target.value) : undefined })}
                  />
                </div>
              </div>
              <div className="space-y-0.5">
                <label className="text-[9px] text-muted-foreground/50">Template Path</label>
                <input
                  className="w-full px-1.5 py-1 text-[10px] bg-background border border-border/30 rounded outline-none focus:border-accent/40"
                  placeholder="/path/to/custom-templates"
                  value={opts.template_path ?? ""}
                  onChange={(e) => setOpts({ ...opts, template_path: e.target.value || undefined })}
                />
              </div>
            </div>
          )}

          {/* Error */}
          {scanError && (
            <div className="flex items-start gap-1.5 text-[10px] text-red-400/70 p-1.5 rounded bg-red-500/5 border border-red-500/10">
              <ShieldX className="w-3 h-3 flex-shrink-0 mt-0.5" />
              <span className="break-all">{scanError}</span>
            </div>
          )}

          {/* No matches state */}
          {matchDone && pocMatches.length === 0 && !anyRunning && !scanError && (
            <div className="flex items-center gap-2 text-[10px] text-muted-foreground/40 p-2 rounded-md bg-muted/5 border border-border/10">
              <Shield className="w-3.5 h-3.5" />
              <div>
                <span className="text-foreground/50">No matching PoC templates found.</span>
                <span className="ml-1">Run the pipeline first to populate fingerprints, then try again.</span>
              </div>
            </div>
          )}

          {/* PoC Matches */}
          {pocMatches.length > 0 && (
            <div className="space-y-1.5">
              <div className="text-[10px] font-medium text-foreground/60 flex items-center gap-1.5">
                <Shield className="w-3 h-3 text-accent/50" />
                {pocMatches.length} PoCs matched
                {templateIds.length > 0 && (
                  <span className="text-muted-foreground/30">({templateIds.length} Nuclei templates)</span>
                )}
              </div>
              <div className="space-y-1 max-h-[200px] overflow-y-auto">
                {(["critical", "high", "medium", "low", "info"] as const).map((sev) => {
                  const group = sevGroups[sev];
                  if (!group?.length) return null;
                  return (
                    <div key={sev}>
                      <div className="flex items-center gap-1.5 mb-0.5">
                        <span className={cn("w-1.5 h-1.5 rounded-full", SEV_DOT[sev])} />
                        <span className="text-[9px] font-medium text-muted-foreground/50 uppercase">{sev} ({group.length})</span>
                      </div>
                      {group.map((p) => (
                        <div
                          key={p.poc_id}
                          className={cn("flex items-center gap-2 text-[10px] px-1.5 py-0.5 rounded border ml-3 mb-0.5", SEV_COLORS[sev] ?? SEV_COLORS.info)}
                        >
                          <span className="font-mono text-[9px] truncate flex-1">{p.cve_id || p.poc_name}</span>
                          <span className="text-muted-foreground/30 text-[8px]">{p.source}</span>
                          {p.template_id && <Crosshair className="w-2.5 h-2.5 text-accent/50 flex-shrink-0" title={`Template: ${p.template_id}`} />}
                        </div>
                      ))}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* Scan Results */}
          {scanResult && (
            <div className={cn(
              "p-2 rounded-md border",
              scanResult.items_found > 0 ? "bg-red-500/5 border-red-500/15" : "bg-green-500/5 border-green-500/15",
            )}>
              <div className="flex items-center gap-2 text-[10px]">
                {scanResult.items_found > 0 ? (
                  <>
                    <ShieldAlert className="w-3.5 h-3.5 text-red-400" />
                    <span className="text-red-400 font-medium">{scanResult.items_found} vulnerabilities confirmed</span>
                  </>
                ) : (
                  <>
                    <ShieldCheck className="w-3.5 h-3.5 text-green-400" />
                    <span className="text-green-400 font-medium">No vulnerabilities found</span>
                  </>
                )}
                {scanResult.duration_ms > 0 && (
                  <span className="text-muted-foreground/30 text-[9px] ml-auto">
                    {scanResult.duration_ms > 1000 ? `${(scanResult.duration_ms / 1000).toFixed(1)}s` : `${scanResult.duration_ms}ms`}
                  </span>
                )}
              </div>
              {scanResult.items_stored > 0 && scanResult.items_stored !== scanResult.items_found && (
                <div className="text-[9px] text-muted-foreground/40 mt-0.5">{scanResult.items_stored} new (stored)</div>
              )}
              {scanResult.errors.length > 0 && (
                <div className="text-[9px] text-red-400/50 mt-1">{scanResult.errors.length} error(s)</div>
              )}
            </div>
          )}

          {pocMatches.length === 0 && !scanResult && !anyRunning && !scanError && (
            <div className="text-[10px] text-muted-foreground/25 text-center py-2">
              Click "Match & Scan" to find and test vulnerabilities for this target
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ═══════════════════════════════════════════════════════════════════════
// Scan Tools Panel
// ═══════════════════════════════════════════════════════════════════════

interface TargetOption {
  id: string;
  value: string;
  type: string;
}

function ScanToolsPanel({ initialTarget }: { initialTarget?: { id: string; value: string } }) {
  const [targets, setTargets] = useState<TargetOption[]>([]);
  const [selectedTarget, setSelectedTarget] = useState<TargetOption | null>(null);
  const [loading, setLoading] = useState(true);
  const projectPath = useStore((s) => s.currentProjectPath);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setSelectedTarget(null);
    (async () => {
      try {
        const data = await invoke<{ targets: TargetOption[] }>("target_list", {
          projectPath: getProjectPath(),
        });
        if (cancelled) return;
        const scannable = (data?.targets ?? []).filter(
          (t) => t.type === "url" || t.type === "domain" || t.type === "ip",
        );
        setTargets(scannable);
        const initial = initialTarget
          ? scannable.find((t) => t.id === initialTarget.id)
          : null;
        if (initial) {
          setSelectedTarget(initial);
        } else if (scannable.length > 0) {
          setSelectedTarget(scannable[0]);
        }
      } catch {
        if (!cancelled) setTargets([]);
      }
      if (!cancelled) setLoading(false);
    })();
    return () => { cancelled = true; };
  }, [initialTarget?.id, projectPath]);

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center">
        <Loader2 className="w-6 h-6 animate-spin text-muted-foreground/20" />
      </div>
    );
  }

  if (targets.length === 0) {
    return (
      <div className="h-full flex flex-col items-center justify-center gap-3 text-muted-foreground/20">
        <Crosshair className="w-12 h-12" />
        <p className="text-[12px] font-medium">No scannable targets</p>
        <p className="text-[10px] text-muted-foreground/15 max-w-[280px] text-center">
          Add URL or domain targets in the Targets panel first.
          Scan Tools supports WhatWeb fingerprinting, targeted Nuclei scanning, and feroxbuster directory brute-forcing.
        </p>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      {/* Target selector */}
      <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border/10 flex-shrink-0">
        <Crosshair className="w-3.5 h-3.5 text-accent flex-shrink-0" />
        <span className="text-[10px] text-muted-foreground/50 flex-shrink-0">Target:</span>
        <StyledSelect
          value={selectedTarget?.id ?? ""}
          onChange={(v) => { const t = targets.find((t) => t.id === v); if (t) setSelectedTarget(t); }}
          options={targets.map((t) => ({ value: t.id, label: `[${t.type}] ${t.value}` }))}
          className="flex-1"
        />
        <span className="text-[9px] text-muted-foreground/30">
          {targets.length} target{targets.length !== 1 ? "s" : ""}
        </span>
      </div>

      {/* Pipeline launcher + Nuclei + scan history */}
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
        {selectedTarget && (
          <>
            <PipelineLauncher
              key={selectedTarget.id}
              targetId={selectedTarget.id}
              targetValue={selectedTarget.value}
            />
            <NucleiSection
              targetId={selectedTarget.id}
              targetUrl={selectedTarget.value}
            />
            <ScanTimeline targetId={selectedTarget.id} targetValue={selectedTarget.value} />
          </>
        )}
      </div>
    </div>
  );
}

interface StepSummary {
  tool: string;
  stored: number;
  new?: number;
  parsed: number;
  exit: number | null;
  ms: number;
}

function ScanTimeline({ targetId, targetValue }: { targetId: string; targetValue: string }) {
  const [logs, setLogs] = useState<AuditRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedRun, setExpandedRun] = useState<number | null>(null);

  const loadLogs = useCallback(async () => {
    try {
      const [byTarget, bySearch] = await Promise.all([
        oplogListByTarget(targetId, 100).catch(() => [] as AuditRow[]),
        oplogSearch(getProjectPath() ?? "", targetValue, 200).catch(() => [] as AuditRow[]),
      ]);
      const seen = new Set<number>();
      const merged: AuditRow[] = [];
      for (const entry of [...byTarget, ...bySearch]) {
        if (entry.action !== "pipeline_executed" || seen.has(entry.id)) continue;
        const detail = entry.detail as Record<string, unknown> | null;
        const detailTarget = detail?.target as string | undefined;
        if (entry.targetId === targetId || detailTarget === targetValue) {
          seen.add(entry.id);
          merged.push(entry);
        }
      }
      merged.sort((a, b) => b.createdAt - a.createdAt);
      setLogs(merged);
    } catch {
      setLogs([]);
    }
    setLoading(false);
  }, [targetId, targetValue]);

  useEffect(() => {
    setLoading(true);
    loadLogs();
  }, [loadLogs]);

  useEffect(() => {
    const unlistenTargets = listen("targets-changed", () => loadLogs());
    return () => { unlistenTargets.then((fn) => fn()); };
  }, [loadLogs]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 py-4 text-[10px] text-muted-foreground/30">
        <Loader2 className="w-3 h-3 animate-spin" />
        Loading scan history...
      </div>
    );
  }

  if (logs.length === 0) {
    return (
      <div className="text-center py-6 text-[10px] text-muted-foreground/20">
        No scan history for this target yet
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-border/10 overflow-hidden">
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/10">
        <History className="w-3.5 h-3.5 text-blue-400" />
        <span className="text-[11px] font-semibold text-foreground/80">Scan History</span>
        <span className="text-[10px] text-muted-foreground/30 ml-auto">{logs.length} run{logs.length !== 1 ? "s" : ""}</span>
      </div>

      <div className="divide-y divide-border/5">
        {logs.map((run, i) => {
          const detail = run.detail ?? {};
          const steps = (detail.steps ?? []) as StepSummary[];
          const totalStored = (detail.total_stored ?? 0) as number;
          const totalNew = (detail.total_new ?? null) as number | null;
          const durationMs = (detail.duration_ms ?? 0) as number;
          const completedSteps = (detail.completed_steps ?? 0) as number;
          const totalSteps = (detail.total_steps ?? 0) as number;
          const status = run.status;
          const isExpanded = expandedRun === run.id;

          return (
            <div key={run.id}>
              <button
                type="button"
                onClick={() => setExpandedRun(isExpanded ? null : run.id)}
                className="flex items-center gap-2.5 w-full px-3 py-2 text-left hover:bg-white/[0.02] transition-colors"
              >
                {/* Timeline dot */}
                <div className="flex flex-col items-center gap-0.5 w-3 flex-shrink-0">
                  <span className={cn(
                    "w-2 h-2 rounded-full flex-shrink-0",
                    status === "completed" ? "bg-emerald-400" : status === "partial" ? "bg-yellow-400" : "bg-red-400",
                  )} />
                  {i < logs.length - 1 && <div className="w-px flex-1 min-h-[8px] bg-border/10" />}
                </div>

                {/* Run info */}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-[10px] font-mono text-muted-foreground/40">
                      {new Date(run.createdAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                      {" "}
                      {new Date(run.createdAt).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
                    </span>
                    <span className="text-[10px] text-foreground/60 font-medium">
                      {run.toolName ?? "Pipeline"}
                    </span>
                    {totalNew !== null ? (
                      totalNew > 0 ? (
                        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-emerald-500/15 text-emerald-300 font-medium">
                          +{totalNew} new
                        </span>
                      ) : totalStored > 0 ? (
                        <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-white/[0.05] text-muted-foreground/40 font-medium">
                          no changes
                        </span>
                      ) : null
                    ) : totalStored > 0 ? (
                      <span className="text-[9px] px-1.5 py-0.5 rounded-full bg-blue-500/15 text-blue-300 font-medium">
                        +{totalStored} items
                      </span>
                    ) : null}
                  </div>
                  <div className="flex items-center gap-2 mt-0.5 text-[9px] text-muted-foreground/30">
                    <span>{completedSteps}/{totalSteps} steps</span>
                    {durationMs > 0 && <span>{durationMs < 1000 ? `${durationMs}ms` : `${(durationMs / 1000).toFixed(1)}s`}</span>}
                    {steps.filter((s) => s.stored > 0).map((s) => (
                      <span key={s.tool} className="text-[8px] px-1 py-0.5 rounded bg-white/[0.03]">
                        {s.tool}: {s.new != null ? s.new : s.stored}
                      </span>
                    ))}
                  </div>
                </div>

                {steps.length > 0 && (
                  isExpanded
                    ? <ChevronDown className="w-3 h-3 text-muted-foreground/20 flex-shrink-0" />
                    : <ChevronRight className="w-3 h-3 text-muted-foreground/20 flex-shrink-0" />
                )}
              </button>

              {/* Expanded step details */}
              {isExpanded && steps.length > 0 && (
                <div className="border-t border-border/5 px-3 py-2 ml-5 space-y-1">
                  {steps.map((step, si) => (
                    <div key={si} className="flex items-center gap-2 text-[10px]">
                      <span className={cn(
                        "w-1.5 h-1.5 rounded-full flex-shrink-0",
                        step.exit === 0 ? "bg-emerald-400/60" :
                        step.exit === null ? "bg-zinc-500/40" : "bg-red-400/60",
                      )} />
                      <span className="font-mono text-foreground/50 w-16 flex-shrink-0">{step.tool}</span>
                      {step.new != null && step.new > 0 ? (
                        <span className="text-emerald-300/60">+{step.new} new</span>
                      ) : step.stored > 0 ? (
                        <span className="text-blue-300/60">+{step.stored} stored</span>
                      ) : null}
                      {step.new != null && step.stored > step.new && step.new > 0 && (
                        <span className="text-muted-foreground/25">({step.stored - step.new} existing)</span>
                      )}
                      {step.parsed > 0 && step.parsed !== step.stored && (
                        <span className="text-muted-foreground/20">({step.parsed} parsed)</span>
                      )}
                      {step.ms > 0 && (
                        <span className="text-muted-foreground/20 ml-auto">
                          {step.ms < 1000 ? `${step.ms}ms` : `${(step.ms / 1000).toFixed(1)}s`}
                        </span>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Sensitive File Scanner Panel ──

interface WordlistOption { id: string; name: string; category: string; line_count: number }
interface SensitiveResult {
  id: string; baseUrl: string; probePath: string; fullUrl: string;
  statusCode: number; contentLength: number; contentType: string;
  isConfirmed: boolean; aiVerdict: string | null; createdAt: number;
}
interface SensitiveProgress {
  scanId: string; total: number; completed: number; hits: number;
  currentUrl: string; running: boolean; dirsFound: number;
}

function SensitiveScanPanel() {
  const { t } = useTranslation();
  const projectPath = useStore((s) => s.currentProjectPath);
  const [wordlists, setWordlists] = useState<WordlistOption[]>([]);
  const [selectedWordlist, setSelectedWordlist] = useState<string>("");
  const [ratePerSecond, setRatePerSecond] = useState(20);
  const [useSitemapDirs, setUseSitemapDirs] = useState(true);
  const [targetUrl, setTargetUrl] = useState("");
  const [results, setResults] = useState<SensitiveResult[]>([]);
  const [progress, setProgress] = useState<SensitiveProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  useEffect(() => {
    invoke<WordlistOption[]>("wordlist_list").then((wl) => {
      setWordlists(Array.isArray(wl) ? wl : []);
    }).catch(() => {});
    invoke<SensitiveResult[]>("sensitive_scan_results", { projectPath, confirmedOnly: false }).then((r) => {
      setResults(Array.isArray(r) ? r : []);
    }).catch(() => {});
    invoke<boolean>("sensitive_scan_status").then((r) => setRunning(r)).catch(() => {});
  }, [projectPath]);

  useEffect(() => {
    const unlisten = listen<SensitiveProgress>("sensitive-scan-progress", (event) => {
      setProgress(event.payload);
      setRunning(event.payload.running);
      if (!event.payload.running) {
        invoke<SensitiveResult[]>("sensitive_scan_results", { projectPath, confirmedOnly: false }).then((r) => {
          setResults(Array.isArray(r) ? r : []);
        }).catch(() => {});
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [projectPath]);

  const handleStart = useCallback(async () => {
    try {
      await invoke("sensitive_scan_start", {
        config: {
          targetUrl: targetUrl || "http://localhost",
          wordlistId: selectedWordlist || null,
          ratePerSecond,
          useSitemapDirs,
        },
        projectPath,
      });
      setRunning(true);
    } catch (e) {
      console.error("Sensitive scan start failed:", e);
    }
  }, [targetUrl, selectedWordlist, ratePerSecond, useSitemapDirs, projectPath]);

  const handleStop = useCallback(async () => {
    await invoke("sensitive_scan_stop").catch(() => {});
  }, []);

  const handleClear = useCallback(async () => {
    await invoke("sensitive_scan_clear", { projectPath }).catch(() => {});
    setResults([]);
    setSelectedIds(new Set());
  }, [projectPath]);

  const toggleSelect = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  }, []);

  const selectAll = useCallback(() => {
    if (selectedIds.size === results.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(results.map((r) => r.id)));
    }
  }, [results, selectedIds.size]);

  const handleConfirm = useCallback(async () => {
    const ids = Array.from(selectedIds);
    if (ids.length === 0) return;
    await invoke("sensitive_scan_confirm", { ids, confirmed: true }).catch(() => {});
    setResults((prev) => prev.map((r) => selectedIds.has(r.id) ? { ...r, isConfirmed: true } : r));
    setSelectedIds(new Set());
  }, [selectedIds]);

  const [analyzing, setAnalyzing] = useState(false);
  const handleAiAnalyze = useCallback(async (onlyNew: boolean) => {
    const toAnalyze = onlyNew ? results.filter((r) => !r.aiVerdict) : results;
    if (toAnalyze.length === 0) return;
    setAnalyzing(true);
    try {
      const { initAiSession, sendPromptSession, shutdownAiSession } = await import("@/lib/ai");
      const { getSettings } = await import("@/lib/settings");
      const settings = await getSettings();
      const tmpId = `sensitive-ai-${Date.now()}`;

      const apiKey = settings.ai?.anthropic?.api_key || settings.ai?.openai?.api_key || settings.ai?.openrouter?.api_key || "";
      const provider = settings.ai?.anthropic?.api_key ? "anthropic"
        : settings.ai?.openai?.api_key ? "openai"
        : settings.ai?.openrouter?.api_key ? "openrouter" : "";
      const model = provider === "anthropic" ? "claude-sonnet-4-20250514"
        : provider === "openai" ? "gpt-4o"
        : provider === "openrouter" ? "anthropic/claude-sonnet-4-20250514" : "";

      if (!provider || !apiKey) {
        console.warn("[SensitiveScan] No AI provider configured");
        setAnalyzing(false);
        return;
      }

      await initAiSession(tmpId, { provider, workspace: ".", model, api_key: apiKey } as any);

      const items = toAnalyze.map((r) =>
        `- ${r.probePath} [${r.fullUrl}] status=${r.statusCode} size=${r.contentLength} type=${r.contentType}`
      ).join("\n");

      const prompt = `You are a security analyst. Classify each sensitive file scan result as true_positive, false_positive, or needs_review.
Respond ONLY with a raw JSON array: [{"path": "...", "verdict": "...", "reason": "..."}]
No markdown, no explanation outside the JSON.

Results:
${items}`;

      const response = await sendPromptSession(tmpId, prompt);
      shutdownAiSession(tmpId).catch(() => {});

      const jsonStart = response.indexOf("[");
      const jsonEnd = response.lastIndexOf("]");
      if (jsonStart >= 0 && jsonEnd > jsonStart) {
        const verdicts = JSON.parse(response.substring(jsonStart, jsonEnd + 1));
        if (Array.isArray(verdicts)) {
          const applied = await invoke<{ analyzed: number; true_positives: number }>(
            "sensitive_scan_apply_verdicts", { verdicts, projectPath }
          );
          const freshResults = await invoke<SensitiveResult[]>("sensitive_scan_results", { projectPath, confirmedOnly: false });
          setResults(Array.isArray(freshResults) ? freshResults : []);
          console.info("[SensitiveScan] AI analyzed:", applied);
        }
      }
    } catch (e) {
      console.error("[SensitiveScan] AI analysis failed:", e);
    }
    setAnalyzing(false);
  }, [results, projectPath]);

  const pct = progress && progress.total > 0 ? Math.round((progress.completed / progress.total) * 100) : 0;
  const dirWordlists = wordlists.filter((w) => w.category === "directories" || w.category === "fuzz" || w.category === "custom");

  return (
    <div className="h-full flex flex-col">
      <div className="px-4 py-3 border-b border-border/10 space-y-2 flex-shrink-0">
        <div className="flex items-center gap-2">
          <FileSearch className="w-3.5 h-3.5 text-amber-400" />
          <span className="text-[11px] font-semibold text-foreground/80">Sensitive File Scanner</span>
          {results.length > 0 && (
            <span className="text-[9px] text-muted-foreground/30 ml-auto">{results.length} hit{results.length !== 1 ? "s" : ""}</span>
          )}
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          <label className="flex items-center gap-1.5 text-[10px] text-muted-foreground/50">
            <input
              type="checkbox"
              checked={useSitemapDirs}
              onChange={(e) => setUseSitemapDirs(e.target.checked)}
              className="w-3 h-3 rounded accent-accent"
            />
            Sitemap dirs
          </label>

          {!useSitemapDirs && (
            <input
              value={targetUrl}
              onChange={(e) => setTargetUrl(e.target.value)}
              placeholder="https://target.com"
              className="flex-1 min-w-[140px] px-2 py-1 rounded-md text-[10px] bg-[var(--bg-input)] border border-border/10 text-foreground/80 placeholder:text-muted-foreground/20"
            />
          )}

          <StyledSelect
            value={selectedWordlist}
            onChange={setSelectedWordlist}
            options={[
              { value: "", label: `Built-in (${DEFAULT_SENSITIVE_PATH_COUNT} paths)` },
              ...dirWordlists.map((w) => ({ value: w.id, label: `${w.name} (${w.line_count})` })),
            ]}
            className="min-w-[140px]"
          />

          <div className="flex items-center gap-1 text-[10px] text-muted-foreground/40">
            <span>Rate:</span>
            <input
              type="number" min={1} max={200} value={ratePerSecond}
              onChange={(e) => setRatePerSecond(Math.max(1, Math.min(200, Number(e.target.value))))}
              className="w-12 px-1 py-0.5 rounded text-[10px] bg-[var(--bg-input)] border border-border/10 text-foreground/70 text-center"
            />
            <span>/s</span>
          </div>

          {!running ? (
            <button type="button" onClick={handleStart}
              className="flex items-center gap-1 px-2.5 py-1 rounded-md text-[10px] font-medium bg-amber-500/15 text-amber-400 hover:bg-amber-500/25 transition-colors">
              <Play className="w-3 h-3" /> Scan
            </button>
          ) : (
            <button type="button" onClick={handleStop}
              className="flex items-center gap-1 px-2.5 py-1 rounded-md text-[10px] font-medium bg-destructive/10 text-destructive hover:bg-destructive/20 transition-colors">
              <Square className="w-3 h-3" /> Stop
            </button>
          )}

          {results.length > 0 && !running && (
            <>
              {analyzing ? (
                <span className="flex items-center gap-1 text-[9px] text-blue-400">
                  <Loader2 className="w-3 h-3 animate-spin" /> Analyzing...
                </span>
              ) : (
                <>
                  <button type="button" onClick={() => handleAiAnalyze(false)}
                    className="flex items-center gap-1 px-2 py-1 rounded-md text-[9px] font-medium bg-blue-500/10 text-blue-400 hover:bg-blue-500/20 transition-colors">
                    <Zap className="w-2.5 h-2.5" /> AI Analyze All
                  </button>
                  {results.some((r) => !r.aiVerdict) && (
                    <button type="button" onClick={() => handleAiAnalyze(true)}
                      className="flex items-center gap-1 px-2 py-1 rounded-md text-[9px] font-medium bg-blue-500/10 text-blue-300 hover:bg-blue-500/20 transition-colors">
                      <Zap className="w-2.5 h-2.5" /> Analyze New
                    </button>
                  )}
                </>
              )}
              <button type="button" onClick={handleClear}
                className="flex items-center gap-1 px-2 py-1 rounded-md text-[9px] text-muted-foreground/30 hover:text-destructive transition-colors">
                <Trash2 className="w-2.5 h-2.5" /> Clear
              </button>
            </>
          )}
        </div>

        {running && progress && (
          <div className="space-y-1">
            <div className="flex items-center gap-2 text-[9px] text-muted-foreground/40">
              <Loader2 className="w-3 h-3 animate-spin text-amber-400" />
              <span>{progress.completed}/{progress.total}</span>
              <span>{pct}%</span>
              <span className="text-amber-300">{progress.hits} hits</span>
              <span className="text-muted-foreground/20">{progress.dirsFound} dirs</span>
            </div>
            <div className="h-1 rounded-full bg-muted/20 overflow-hidden">
              <div className="h-full rounded-full bg-amber-500 transition-all duration-300" style={{ width: `${pct}%` }} />
            </div>
            {progress.currentUrl && (
              <div className="text-[8px] text-muted-foreground/20 font-mono truncate">{progress.currentUrl}</div>
            )}
          </div>
        )}
      </div>

      {/* Results */}
      <div className="flex-1 overflow-y-auto">
        {results.length === 0 && !running ? (
          <div className="h-full flex flex-col items-center justify-center gap-2 text-muted-foreground/20">
            <FileSearch className="w-10 h-10" />
            <p className="text-[11px]">No results yet</p>
            <p className="text-[9px] text-muted-foreground/15 max-w-[260px] text-center">
              Probes each directory level from sitemap for sensitive files (.env, .git, backups, configs, etc.)
            </p>
          </div>
        ) : (
          <div>
            {results.length > 0 && (
              <div className="flex items-center gap-2 px-4 py-1.5 border-b border-border/5 text-[9px] text-muted-foreground/30">
                <button type="button" onClick={selectAll} className="hover:text-foreground/60 transition-colors">
                  {selectedIds.size === results.length ? "Deselect all" : "Select all"}
                </button>
                {selectedIds.size > 0 && (
                  <>
                    <span>{selectedIds.size} selected</span>
                    <button type="button" onClick={handleConfirm}
                      className="flex items-center gap-1 px-2 py-0.5 rounded bg-emerald-500/10 text-emerald-400 hover:bg-emerald-500/20 transition-colors">
                      <Check className="w-2.5 h-2.5" /> Confirm
                    </button>
                  </>
                )}
              </div>
            )}

            <div className="divide-y divide-border/5">
              {results.map((r) => {
                const statusColor = r.statusCode >= 200 && r.statusCode < 300 ? "text-emerald-400"
                  : r.statusCode >= 300 && r.statusCode < 400 ? "text-yellow-400"
                  : "text-red-400";
                return (
                  <div
                    key={r.id}
                    className={cn("flex items-center gap-2 px-4 py-2 hover:bg-white/[0.02] transition-colors cursor-pointer",
                      selectedIds.has(r.id) && "bg-accent/5")}
                    onClick={() => toggleSelect(r.id)}
                  >
                    <input type="checkbox" checked={selectedIds.has(r.id)} readOnly
                      className="w-3 h-3 rounded accent-accent flex-shrink-0" />
                    <span className={cn("text-[10px] font-mono w-8 text-center", statusColor)}>{r.statusCode}</span>
                    <div className="flex-1 min-w-0">
                      <div className="text-[10px] font-mono text-foreground/70 truncate">{r.probePath}</div>
                      <div className="text-[8px] font-mono text-muted-foreground/25 truncate">{r.baseUrl}</div>
                    </div>
                    <span className="text-[8px] text-muted-foreground/20">{r.contentLength > 0 ? `${r.contentLength}b` : ""}</span>
                    {r.isConfirmed && (
                      <span className="text-[8px] px-1.5 py-0.5 rounded-full bg-emerald-500/15 text-emerald-400">confirmed</span>
                    )}
                    {r.aiVerdict && (
                      <span className={cn("text-[8px] px-1.5 py-0.5 rounded-full",
                        r.aiVerdict === "true_positive" ? "bg-red-500/15 text-red-400"
                        : r.aiVerdict === "false_positive" ? "bg-zinc-500/10 text-zinc-400"
                        : "bg-yellow-500/10 text-yellow-400"
                      )}>{r.aiVerdict.replace("_", " ")}</span>
                    )}
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

const DEFAULT_SENSITIVE_PATH_COUNT = 60;
