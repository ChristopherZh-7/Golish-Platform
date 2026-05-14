import {
  Activity,
  Crosshair,
  Eye,
  FileSearch,
  Globe,
  History,
  KeyRound,
  Loader2,
  Play,
  Send,
  Shield,
  ShieldAlert,
  Square,
} from "lucide-react";
import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { isWindows } from "@/lib/env";
import { ensureJavaInstalled, isJavaMissingError } from "@/lib/pentest/javaInstaller";
import type { ZapStatusInfo } from "@/lib/pentest/types";
import { zapDetectPath, zapStart, zapStatus, zapStop } from "@/lib/pentest/zap-api";
import { cn } from "@/lib/utils";
import { useStore } from "@/store";

const VaultSettings = lazy(() =>
  import("@/components/Settings/VaultSettings").then((m) => ({ default: m.VaultSettings }))
);

// Heavy panels rendered with `invisible-mount` (kept mounted across tab
// switches to preserve form state). Lazy-load + visited-once gating below
// keeps them out of the initial bundle until the user touches the tab.
const IntruderPanel = lazy(() =>
  import("@/components/SecurityView/IntruderPanel").then((m) => ({ default: m.IntruderPanel }))
);
const RepeaterPanel = lazy(() =>
  import("./RepeaterPanel").then((m) => ({ default: m.RepeaterPanel }))
);
const ScannerPanel = lazy(() =>
  import("./ScannerPanel").then((m) => ({ default: m.ScannerPanel }))
);
const ScanToolsPanel = lazy(() =>
  import("./ScanToolsPanel").then((m) => ({ default: m.ScanToolsPanel }))
);

// Conditional-render panels: lazy + Suspense (re-mounted on each tab visit;
// they don't carry form state we need to preserve across tabs).
const HttpHistoryPanel = lazy(() =>
  import("./HttpHistoryPanel").then((m) => ({ default: m.HttpHistoryPanel }))
);
const PassiveScanPanel = lazy(() =>
  import("./PassiveScanPanel").then((m) => ({ default: m.PassiveScanPanel }))
);
const SensitiveScanPanel = lazy(() =>
  import("./SensitiveScanPanel").then((m) => ({ default: m.SensitiveScanPanel }))
);
const SiteMapPanel = lazy(() =>
  import("./SiteMapPanel").then((m) => ({ default: m.SiteMapPanel }))
);
const TargetTimeline = lazy(() =>
  import("@/components/TargetPanel/TargetTimeline").then((m) => ({ default: m.TargetTimeline }))
);

import { SetupPopover } from "./SetupPopover";
import { StatusBadge, ZapNotInstalled, ZapNotRunning } from "./shared";

export type SecurityTab =
  | "history"
  | "sitemap"
  | "scanner"
  | "repeater"
  | "intruder"
  | "passive"
  | "vault"
  | "scantools"
  | "sensitive"
  | "timeline";

/** Tabs that don't depend on ZAP being running — always visible. */
const ZAP_INDEPENDENT_TABS: SecurityTab[] = ["scantools", "sensitive", "timeline", "vault"];

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
  const [activeTab, setActiveTab] = useState<SecurityTab>(
    standaloneTab || (initialScanTarget ? "scantools" : globalZapRunning ? "history" : "scantools")
  );
  const effectiveTab = standaloneTab || activeTab;

  useEffect(() => {
    if (initialScanTarget) setActiveTab("scantools");
  }, [initialScanTarget?.id, initialScanTarget]);

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

  // Visited-once gating: lazy panels stay un-mounted until the user touches
  // the tab. After the first visit they remain mounted to preserve state.
  const [visitedRepeater, setVisitedRepeater] = useState(false);
  const [visitedIntruder, setVisitedIntruder] = useState(false);
  const [visitedScanTools, setVisitedScanTools] = useState(false);
  const [visitedScanner, setVisitedScanner] = useState(false);

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

  // Mark the panel as visited the first time its tab becomes active so the
  // lazy chunk is fetched on demand instead of as part of the initial bundle.
  useEffect(() => {
    if (effectiveTab === "repeater") setVisitedRepeater(true);
    if (effectiveTab === "intruder") setVisitedIntruder(true);
    if (effectiveTab === "scantools") setVisitedScanTools(true);
    if (effectiveTab === "scanner") setVisitedScanner(true);
  }, [effectiveTab]);

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
        if (!cancelled) {
          setZapInstalled(false);
          setGlobalZapRunning(false);
        }
      } finally {
        if (!cancelled) setCheckingInstall(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [setGlobalZapRunning]);

  const handleStart = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await zapStart(undefined, undefined, currentProjectPath);
      setZapState(result);
      setGlobalZapRunning(result.status === "running");
    } catch (e) {
      const missingMajor = isJavaMissingError(e);
      if (missingMajor) {
        const versionManagerLabel = isWindows() ? "winget" : "SDKMAN";
        try {
          setError(t("security.javaBootstrapping", { ver: missingMajor }));
          await ensureJavaInstalled(missingMajor, {
            onProgress: (stage) => {
              if (stage === "bootstrap-runtime") {
                setError(t("security.javaManagerBootstrap", { manager: versionManagerLabel }));
              } else if (stage === "runtime-bootstrapped") {
                setError(t("security.javaManagerDone", { manager: versionManagerLabel }));
              } else if (stage === "direct-install-microsoft-msi") {
                // Direct-MSI bypass: winget GitHub path unreachable.
                // Show the localized "downloading Microsoft JDK via PowerShell"
                // message that also calls out the upcoming UAC prompt.
                setError(t("install.javaDirectInstall"));
              } else if (stage.startsWith("fallback-after-network-failure-")) {
                const from = stage.replace("fallback-after-network-failure-", "");
                const to = from === "temurin" ? "Microsoft" : "direct-MSI";
                setError(t("install.javaFallbackVendor", { from, to }));
              } else {
                setError(t("security.javaInstalling", { ver: missingMajor, id: stage }));
              }
            },
          });
          setError(t("security.javaInstalledRetrying"));
          const result = await zapStart(undefined, undefined, currentProjectPath);
          setZapState(result);
          setGlobalZapRunning(result.status === "running");
          setError(null);
          return;
        } catch (installErr) {
          const errMsg = installErr instanceof Error ? installErr.message : String(installErr);
          let message: string;
          if (errMsg.startsWith("NO_JAVA_CANDIDATE:")) {
            message = t("security.javaNoCandidate", { ver: missingMajor });
          } else if (errMsg.includes("NETWORK_DOWNLOAD_FAILED:")) {
            // winget + direct-MSI both bombed on transport errors. Use the
            // network-specific hint that explains GitHub blocking + proxy
            // suggestion instead of dumping the raw winget output.
            message = t("install.javaNetworkBlocked", {
              ver: missingMajor,
              error: errMsg,
            });
          } else {
            message = t("security.javaInstallFailed", {
              ver: missingMajor,
              error: errMsg,
            });
          }
          setError(message);
          setZapState((s) => ({ ...s, status: "error", error: message }));
          return;
        }
      }
      setError(String(e));
      setZapState((s) => ({ ...s, status: "error", error: String(e) }));
    } finally {
      setLoading(false);
    }
  }, [currentProjectPath, setGlobalZapRunning, t]);

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

  // Re-run path detection after the user clicks "Recheck" / finishes an
  // in-app install flow. Promoted from an inline closure in `renderContent`
  // so the same handler can be reused by the top-level `ZapNotInstalled`
  // override below (which fires for any non-vault tab when ZAP isn't found).
  const handleRecheckInstall = useCallback(() => {
    setCheckingInstall(true);
    zapDetectPath()
      .then((p) => {
        setZapInstalled(p !== null);
        setCheckingInstall(false);
      })
      .catch(() => {
        setZapInstalled(false);
        setCheckingInstall(false);
      });
  }, []);

  const isRunning = zapState.status === "running";

  // Top-level overlay: when ZAP isn't installed, surface the install prompt
  // on every tab except `vault` (credential vault doesn't depend on ZAP),
  // including the normally ZAP-independent `scantools` / `sensitive` /
  // `timeline` tabs. Without this, a fresh Windows install lands on
  // `scantools` by default and never sees the install guidance because the
  // ZAP-dependent tabs are filtered out of `visibleTabs` when ZAP isn't
  // running. Suppressed during the initial detect to avoid an overlay flash
  // before the path check resolves.
  const showInstallOverlay = !checkingInstall && zapInstalled === false && effectiveTab !== "vault";

  const tabs: { id: SecurityTab; label: string; icon: React.ElementType }[] = [
    { id: "history", label: t("security.history"), icon: History },
    { id: "sitemap", label: t("security.siteMap"), icon: Globe },
    { id: "scanner", label: t("security.scanner"), icon: ShieldAlert },
    { id: "repeater", label: t("security.repeater"), icon: Send },
    { id: "intruder", label: "Intruder", icon: Crosshair },
    { id: "passive", label: t("security.passiveScan"), icon: Eye },
    { id: "scantools", label: t("security.scanTools"), icon: Crosshair },
    { id: "sensitive", label: "Sensitive Scan", icon: FileSearch },
    { id: "timeline", label: t("security.timeline"), icon: Activity },
    { id: "vault", label: t("vault.title"), icon: KeyRound },
  ];

  const visibleTabs = isRunning
    ? tabs
    : tabs.filter((tab) => ZAP_INDEPENDENT_TABS.includes(tab.id));

  // If ZAP transitions stopped while user sits on a ZAP-only tab, drop them
  // to Scan Tools so they don't see a broken page.
  useEffect(() => {
    if (!isRunning && !ZAP_INDEPENDENT_TABS.includes(activeTab)) {
      setActiveTab("scantools");
    }
  }, [isRunning, activeTab]);

  const tabDragRef = useRef<{
    tabId: SecurityTab | null;
    startX: number;
    startY: number;
    isDragging: boolean;
  }>({ tabId: null, startX: 0, startY: 0, isDragging: false });

  const handleTabPointerDown = useCallback((tabId: SecurityTab, e: React.PointerEvent) => {
    if (e.button !== 0) return;
    tabDragRef.current = { tabId, startX: e.clientX, startY: e.clientY, isDragging: false };
  }, []);

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const d = tabDragRef.current;
      if (!d.tabId) return;
      if (
        !d.isDragging &&
        (Math.abs(e.clientX - d.startX) > 8 || Math.abs(e.clientY - d.startY) > 8)
      ) {
        d.isDragging = true;
        document.body.style.cursor = "grabbing";
      }
    };
    const onUp = (e: PointerEvent) => {
      const d = tabDragRef.current;
      if (d.isDragging && d.tabId) {
        document.body.style.cursor = "";
        const isOutside =
          e.clientX < 0 ||
          e.clientY < 0 ||
          e.clientX > window.innerWidth ||
          e.clientY > window.innerHeight;
        if (isOutside) {
          window.dispatchEvent(
            new CustomEvent("detach-security-tab", {
              detail: { tabId: d.tabId, screenX: e.screenX, screenY: e.screenY },
            })
          );
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
        <Suspense
          fallback={
            <div className="h-full flex items-center justify-center">
              <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/40" />
            </div>
          }
        >
          <VaultSettings />
        </Suspense>
      );
    }
    if (tab === "scantools") {
      return null;
    }
    if (tab === "sensitive") {
      return (
        <Suspense fallback={null}>
          <SensitiveScanPanel />
        </Suspense>
      );
    }
    if (tab === "timeline") {
      return (
        <Suspense
          fallback={
            <div className="h-full flex items-center justify-center">
              <Loader2 className="w-5 h-5 animate-spin text-muted-foreground/40" />
            </div>
          }
        >
          <TargetTimeline initialTargetId={initialScanTarget?.id} />
        </Suspense>
      );
    }
    if (checkingInstall) {
      return (
        <div className="h-full flex items-center justify-center">
          <Loader2 className="w-6 h-6 animate-spin text-muted-foreground/40" />
        </div>
      );
    }
    // NOTE: `zapInstalled === false` is also handled at the top-level render
    // (see the `showInstallOverlay` branch below) so that ZAP-independent tabs
    // such as `scantools` / `sensitive` / `timeline` — which short-circuit
    // out of this function above — still surface the install prompt. The
    // branch here is kept as a defensive fallback for any future tab that
    // forgets to opt into the overlay logic.
    if (zapInstalled === false) {
      return <ZapNotInstalled onRetry={handleRecheckInstall} />;
    }
    if (!isRunning) {
      return <ZapNotRunning onStart={handleStart} loading={loading} error={error} />;
    }
    switch (tab) {
      case "sitemap":
        return (
          <Suspense fallback={null}>
            <SiteMapPanel
              onSendToRepeater={handleSendToRepeater}
              onSendToIntruder={handleSendToIntruder}
              onActiveScan={handleActiveScan}
              onBatchScan={handleBatchActiveScan}
            />
          </Suspense>
        );
      case "history":
        return (
          <Suspense fallback={null}>
            <HttpHistoryPanel
              onSendToRepeater={handleSendToRepeater}
              onSendToIntruder={handleSendToIntruder}
              onActiveScan={handleActiveScan}
            />
          </Suspense>
        );
      case "scanner":
        return null;
      case "passive":
        return (
          <Suspense fallback={null}>
            <PassiveScanPanel />
          </Suspense>
        );
      case "repeater":
        return null;
      case "intruder":
        return null;
      default:
        return null;
    }
  };

  return (
    <div className="h-full flex flex-col">
      {!standaloneTab && (
        <div className="flex items-center justify-between px-4 py-3 border-b border-border/15 flex-shrink-0">
          <div className="flex items-center gap-3">
            <Shield className="w-4 h-4 text-accent" />
            <h1 className="text-[14px] font-semibold text-foreground">{t("security.title")}</h1>
            <StatusBadge status={zapState} />
          </div>

          <div className="flex items-center gap-2">
            {error && (
              <span className="text-[10px] text-destructive/70 max-w-[200px] truncate">
                {error}
              </span>
            )}
            <SetupPopover
              isRunning={isRunning}
              onStart={handleStart}
              loading={loading}
              error={error}
            />
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
                disabled={loading || zapInstalled === false}
                title={zapInstalled === false ? t("security.zapNotInstalled") : undefined}
                className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[11px] font-semibold bg-accent text-accent-foreground hover:bg-accent/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed shadow-sm"
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
          {visibleTabs.map((tabItem) => (
            <button
              key={tabItem.id}
              type="button"
              onClick={() => setActiveTab(tabItem.id)}
              onPointerDown={(e) => handleTabPointerDown(tabItem.id, e)}
              className={cn(
                "flex items-center gap-1.5 px-3 py-1.5 rounded-md text-[11px] transition-colors select-none",
                activeTab === tabItem.id
                  ? "bg-accent/15 text-accent font-medium"
                  : "text-foreground/60 hover:text-foreground hover:bg-[var(--bg-hover)]"
              )}
            >
              <tabItem.icon className="w-3 h-3" />
              {tabItem.label}
            </button>
          ))}
        </div>
      )}

      <div className="flex-1 overflow-hidden relative">
        {showInstallOverlay ? (
          <ZapNotInstalled onRetry={handleRecheckInstall} />
        ) : (
          <>
            {renderContent(effectiveTab)}
            {visitedRepeater && (
              <div
                className={cn(
                  "absolute inset-0",
                  effectiveTab === "repeater" && isRunning ? "" : "invisible pointer-events-none"
                )}
              >
                <Suspense fallback={null}>
                  <RepeaterPanel
                    injectedRequest={repeaterRequest}
                    onInjectedConsumed={() => setRepeaterRequest(null)}
                  />
                </Suspense>
              </div>
            )}
            {visitedIntruder && (
              <div
                className={cn(
                  "absolute inset-0",
                  effectiveTab === "intruder" && isRunning ? "" : "invisible pointer-events-none"
                )}
              >
                <Suspense fallback={null}>
                  <IntruderPanel
                    injectedRequest={intruderRequest}
                    onInjectedConsumed={() => setIntruderRequest(null)}
                  />
                </Suspense>
              </div>
            )}
            {visitedScanTools && (
              <div
                className={cn(
                  "absolute inset-0",
                  effectiveTab === "scantools" ? "" : "invisible pointer-events-none"
                )}
              >
                <Suspense fallback={null}>
                  <ScanToolsPanel initialTarget={initialScanTarget} />
                </Suspense>
              </div>
            )}
            {visitedScanner && (
              <div
                className={cn(
                  "absolute inset-0",
                  effectiveTab === "scanner" && isRunning ? "" : "invisible pointer-events-none"
                )}
              >
                <Suspense fallback={null}>
                  <ScannerPanel
                    initialUrl={pendingScanUrl}
                    initialBatchUrls={pendingScanUrls}
                    onUrlConsumed={() => {
                      setPendingScanUrl(null);
                      setPendingScanUrls([]);
                    }}
                  />
                </Suspense>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
