import {
  AlertTriangle,
  Check,
  ChevronRight,
  Download,
  Hammer,
  Loader2,
  Plug,
  RefreshCw,
  ServerCog,
  X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listServers, type McpServerInfo } from "@/lib/api/mcp";
import { logger } from "@/lib/logger";
import { checkEnvSetup, installRuntime, scanTools } from "@/lib/pentest/api";
import type { EnvSetupStatus, ToolConfig } from "@/lib/pentest/types";
import { getSettings } from "@/lib/settings/api";
import { cn } from "@/lib/utils";
import { isMockBrowserMode } from "@/mocks";

/**
 * Custom DOM event dispatched when the user clicks the "go to Tool Manager"
 * action. AppShell listens for this and opens the Tool Manager overlay.
 */
const NAVIGATE_EVENT = "golish:setup-banner-navigate";
type NavigateTarget = "tool-manager" | "settings";

function dispatchNavigate(target: NavigateTarget): void {
  window.dispatchEvent(new CustomEvent<NavigateTarget>(NAVIGATE_EVENT, { detail: target }));
}

export type SetupBannerNavigateEvent = CustomEvent<NavigateTarget>;
export const SETUP_BANNER_NAVIGATE_EVENT = NAVIGATE_EVENT;

interface RuntimeStatus {
  key: string;
  label: string;
  installed: boolean;
}

interface HealthSnapshot {
  missingEssentialTools: number;
  runtimes: RuntimeStatus[];
  failingMcpServers: number;
  totalMcpServers: number;
}

const HEALTH_REFRESH_MS = 30_000;

function buildRuntimeStatuses(env: EnvSetupStatus | null): RuntimeStatus[] {
  if (!env) return [];
  return [
    { key: "homebrew", label: "Homebrew", installed: env.homebrew_installed },
    { key: "conda", label: "Conda (Python)", installed: env.conda_installed },
    { key: "nvm", label: "NVM (Node.js)", installed: env.nvm_installed },
  ];
}

export function SetupHealthBanner() {
  const [snapshot, setSnapshot] = useState<HealthSnapshot | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [installingRuntime, setInstallingRuntime] = useState<string | null>(null);
  const [runtimeDone, setRuntimeDone] = useState<Set<string>>(new Set());
  const [refreshing, setRefreshing] = useState(false);
  const mountedRef = useRef(true);
  const { t } = useTranslation();

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    if (isMockBrowserMode()) {
      setSnapshot(null);
      return;
    }
    try {
      const [toolsResult, envResult, mcpResult] = await Promise.allSettled([
        scanTools(),
        checkEnvSetup(),
        listServers(),
      ]);

      const tools: ToolConfig[] =
        toolsResult.status === "fulfilled" && toolsResult.value.success
          ? toolsResult.value.tools
          : [];
      const env: EnvSetupStatus | null = envResult.status === "fulfilled" ? envResult.value : null;
      const servers: McpServerInfo[] = mcpResult.status === "fulfilled" ? mcpResult.value : [];

      const missingEssentialTools = tools.filter(
        (t) =>
          (t.tier === "essential" || t.tier === "recommended") &&
          (t as ToolConfig & { installed?: boolean }).installed === false
      ).length;

      const failingMcpServers = servers.filter(
        (s) => s.enabled && (s.status === "disconnected" || s.status === "error")
      ).length;

      if (mountedRef.current) {
        setSnapshot({
          missingEssentialTools,
          runtimes: buildRuntimeStatuses(env),
          failingMcpServers,
          totalMcpServers: servers.length,
        });
      }
    } catch (err) {
      logger.warn("[SetupHealthBanner] health refresh failed:", err);
    }
  }, []);

  useEffect(() => {
    refresh();
    const id = window.setInterval(refresh, HEALTH_REFRESH_MS);
    return () => window.clearInterval(id);
  }, [refresh]);

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    await refresh();
    if (mountedRef.current) setRefreshing(false);
  }, [refresh]);

  const handleInstallRuntime = useCallback(
    async (runtimeKey: string) => {
      if (installingRuntime) return;
      setInstallingRuntime(runtimeKey);
      try {
        const appSettings = await getSettings().catch(() => null);
        const proxyUrl =
          (appSettings as { network?: { proxy_url?: string } })?.network?.proxy_url || undefined;
        const result = await installRuntime(runtimeKey, proxyUrl);
        if (result.success && mountedRef.current) {
          setRuntimeDone((prev) => new Set(prev).add(runtimeKey));
          setSnapshot((prev) => {
            if (!prev) return prev;
            return {
              ...prev,
              runtimes: buildRuntimeStatuses(result.env_status),
            };
          });
        }
      } catch (err) {
        logger.warn(`[SetupHealthBanner] install ${runtimeKey} failed:`, err);
      } finally {
        if (mountedRef.current) setInstallingRuntime(null);
      }
    },
    [installingRuntime]
  );

  if (dismissed || !snapshot) return null;

  const missingRuntimes = snapshot.runtimes.filter((r) => !r.installed);
  const hasIssues =
    snapshot.missingEssentialTools > 0 ||
    missingRuntimes.length > 0 ||
    snapshot.failingMcpServers > 0;

  if (!hasIssues) return null;

  return (
    <div className="w-full border-b border-amber-500/15 bg-gradient-to-r from-amber-500/[0.06] to-transparent px-4 py-3 flex-shrink-0">
      <div className="flex items-start gap-3">
        <div className="mt-0.5 flex-shrink-0 rounded-md bg-amber-500/10 p-1.5">
          <AlertTriangle className="w-4 h-4 text-amber-400" />
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center justify-between mb-3">
            <h3 className="text-xs font-semibold text-amber-300/90 uppercase tracking-wider">
              {t("setupHealth.title")}
            </h3>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={handleRefresh}
                disabled={refreshing}
                className="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-foreground/70 transition-colors rounded-md px-1.5 py-0.5 hover:bg-foreground/5 disabled:opacity-40"
              >
                <RefreshCw className={cn("w-3 h-3", refreshing && "animate-spin")} />
              </button>
              <button
                type="button"
                onClick={() => setDismissed(true)}
                className="flex items-center gap-1 text-[10px] text-muted-foreground/50 hover:text-foreground/70 transition-colors rounded-md px-1.5 py-0.5 hover:bg-foreground/5"
              >
                <X className="w-3 h-3" />
                {t("setupHealth.dismiss")}
              </button>
            </div>
          </div>
          <ul className="space-y-2">
            {/* ── Missing tools ── */}
            {snapshot.missingEssentialTools > 0 && (
              <li className="flex items-center justify-between gap-3 px-3 py-2.5 rounded-lg bg-background/50 border border-border/30 hover:border-border/50 transition-colors">
                <div className="flex items-center gap-2.5 min-w-0">
                  <Hammer className="w-4 h-4 text-amber-400/80 flex-shrink-0" />
                  <div className="min-w-0">
                    <div className="text-[12px] text-foreground/90 font-medium truncate">
                      {t("setupHealth.toolsMissing", { count: snapshot.missingEssentialTools })}
                    </div>
                    <div className="text-[10px] text-muted-foreground/50 truncate">
                      {t("setupHealth.toolsMissingDetail")}
                    </div>
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => dispatchNavigate("tool-manager")}
                  className="flex items-center gap-0.5 px-2.5 py-1.5 rounded-md text-[11px] font-medium text-accent hover:bg-accent/10 active:bg-accent/15 transition-colors flex-shrink-0"
                >
                  {t("setupHealth.goToolManager")}
                  <ChevronRight className="w-3 h-3" />
                </button>
              </li>
            )}

            {/* ── Missing runtimes with inline install ── */}
            {missingRuntimes.length > 0 && (
              <li className="px-3 py-2.5 rounded-lg bg-background/50 border border-border/30">
                <div className="flex items-center gap-2.5 mb-2">
                  <ServerCog className="w-4 h-4 text-amber-400/80 flex-shrink-0" />
                  <div className="text-[12px] text-foreground/90 font-medium">
                    {t("setupHealth.runtimesMissing", { count: missingRuntimes.length })}
                  </div>
                </div>
                <div className="space-y-1.5 ml-6.5 pl-[26px]">
                  {missingRuntimes.map((rt) => {
                    const isInstalling = installingRuntime === rt.key;
                    const isDone = runtimeDone.has(rt.key);
                    return (
                      <div key={rt.key} className="flex items-center justify-between gap-2">
                        <span className="text-[11px] text-muted-foreground/70">{rt.label}</span>
                        <button
                          type="button"
                          onClick={() => handleInstallRuntime(rt.key)}
                          disabled={!!installingRuntime || isDone}
                          className={cn(
                            "flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-medium transition-colors",
                            isDone
                              ? "text-emerald-400 bg-emerald-400/10"
                              : "text-accent hover:bg-accent/10 active:bg-accent/15 disabled:opacity-40 disabled:cursor-not-allowed"
                          )}
                        >
                          {isDone ? (
                            <>
                              <Check className="w-3 h-3" />
                              {t("common.installed")}
                            </>
                          ) : isInstalling ? (
                            <>
                              <Loader2 className="w-3 h-3 animate-spin" />
                              {t("common.loading")}
                            </>
                          ) : (
                            <>
                              <Download className="w-3 h-3" />
                              {t("common.install")}
                            </>
                          )}
                        </button>
                      </div>
                    );
                  })}
                </div>
              </li>
            )}

            {/* ── Failing MCP servers ── */}
            {snapshot.failingMcpServers > 0 && (
              <li className="flex items-center justify-between gap-3 px-3 py-2.5 rounded-lg bg-background/50 border border-border/30 hover:border-border/50 transition-colors">
                <div className="flex items-center gap-2.5 min-w-0">
                  <Plug className="w-4 h-4 text-amber-400/80 flex-shrink-0" />
                  <div className="min-w-0">
                    <div className="text-[12px] text-foreground/90 font-medium truncate">
                      {t("setupHealth.mcpFailing", { count: snapshot.failingMcpServers })}
                    </div>
                    <div className="text-[10px] text-muted-foreground/50 truncate">
                      {t("setupHealth.mcpFailingDetail", { total: snapshot.totalMcpServers })}
                    </div>
                  </div>
                </div>
                <button
                  type="button"
                  onClick={handleRefresh}
                  disabled={refreshing}
                  className="flex items-center gap-1 px-2.5 py-1.5 rounded-md text-[11px] font-medium text-accent hover:bg-accent/10 active:bg-accent/15 transition-colors flex-shrink-0 disabled:opacity-40"
                >
                  <RefreshCw className={cn("w-3 h-3", refreshing && "animate-spin")} />
                  {t("setupHealth.recheck")}
                </button>
              </li>
            )}
          </ul>
        </div>
      </div>
    </div>
  );
}
