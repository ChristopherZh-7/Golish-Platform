import { useCallback, useEffect, useRef, useState } from "react";
import {
  History, Globe, ShieldAlert, Send, Crosshair, Eye, KeyRound,
  FileSearch, Loader2, Play, Shield, Square,
} from "lucide-react";
import { cn } from "@/lib/utils";
import {
  zapStart, zapStop, zapStatus, zapDetectPath,
} from "@/lib/pentest/zap-api";
import type { ZapStatusInfo } from "@/lib/pentest/types";
import { useTranslation } from "react-i18next";
import { useStore } from "@/store";

import { lazy, Suspense } from "react";
const VaultSettings = lazy(() =>
  import("@/components/Settings/VaultSettings").then((m) => ({ default: m.VaultSettings }))
);
import { IntruderPanel } from "@/components/SecurityView/IntruderPanel";

import { StatusBadge, ZapNotInstalled, ZapNotRunning } from "./shared";
import { HttpHistoryPanel } from "./HttpHistoryPanel";
import { ScannerPanel } from "./ScannerPanel";
import { RepeaterPanel } from "./RepeaterPanel";
import { SiteMapPanel } from "./SiteMapPanel";
import { PassiveScanPanel } from "./PassiveScanPanel";
import { ScanToolsPanel } from "./ScanToolsPanel";
import { SensitiveScanPanel } from "./SensitiveScanPanel";

export type SecurityTab = "history" | "sitemap" | "scanner" | "repeater" | "intruder" | "passive" | "vault" | "scantools" | "sensitive";

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
      return null;
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
      case "scanner": return null;
      case "passive": return <PassiveScanPanel />;
      case "repeater": return null;
      case "intruder": return null;
      default: return null;
    }
  };

  return (
    <div className="h-full flex flex-col">
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

      {!standaloneTab && (
        <div className="flex items-center gap-1 px-4 py-2 border-b border-border/10 flex-shrink-0">
          {tabs.map((tabItem) => {
            const zapRequired = !["vault", "scantools", "sensitive"].includes(tabItem.id);
            const disabled = zapRequired && !isRunning;
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

      <div className="flex-1 overflow-hidden relative">
        {renderContent(effectiveTab)}
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
