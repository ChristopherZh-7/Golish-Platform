import { open } from "@tauri-apps/plugin-shell";
import {
  AlertCircle,
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronRight,
  Download,
  ExternalLink,
  Loader2,
  Plug,
  PlugZap,
  RefreshCw,
  Server,
  Wrench,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent } from "@/components/ui/collapsible";
import { setupBuiltinMcp } from "@/lib/api/mcp";
import { onEvent } from "@/lib/events";
import { logger } from "@/lib/logger";
import * as mcp from "@/lib/mcp";
import { notify } from "@/lib/notify";
import { runTauriUnlistenFromPromise } from "@/lib/run-tauri-unlisten";

interface McpSettingsProps {
  workspacePath?: string;
}

export function McpSettings({ workspacePath }: McpSettingsProps) {
  const { t } = useTranslation();
  const [servers, setServers] = useState<mcp.McpServerInfo[]>([]);
  const [tools, setTools] = useState<mcp.McpToolInfo[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [expandedServers, setExpandedServers] = useState<Set<string>>(new Set());
  const [connectingServers, setConnectingServers] = useState<Set<string>>(new Set());
  const [disconnectingServers, setDisconnectingServers] = useState<Set<string>>(new Set());
  const [settingUpServers, setSettingUpServers] = useState<Set<string>>(new Set());

  // Load servers and tools
  const loadData = useCallback(async () => {
    setIsLoading(true);
    try {
      const serverList = await mcp.listServers(workspacePath);
      setServers(serverList);

      try {
        const toolList = await mcp.listTools();
        setTools(toolList);
      } catch {
        // MCP manager not yet initialized - that's OK
        setTools([]);
      }
    } catch (err) {
      logger.error("Failed to load MCP servers:", err);
      notify.error(t("mcp.loadFailed"));
    } finally {
      setIsLoading(false);
    }
  }, [workspacePath]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Listen for MCP background initialization events and auto-refresh
  useEffect(() => {
    const unlisten = onEvent("mcp-event", (payload) => {
      if (payload.type === "ready") {
        loadData();
      } else if (payload.type === "error") {
        logger.error("[mcp-event] Error:", payload.message);
      }
    });

    return () => {
      runTauriUnlistenFromPromise(unlisten);
    };
  }, [loadData]);

  // Connect to a server
  const handleConnect = useCallback(
    async (serverName: string) => {
      setConnectingServers((prev) => new Set(prev).add(serverName));
      try {
        await mcp.connect(serverName);
        notify.success(t("mcp.connectedTo", { name: serverName }));
        await loadData();
      } catch (err) {
        logger.error(`Failed to connect to ${serverName}:`, err);
        notify.error(
          err instanceof Error ? err.message : t("mcp.connectFailed", { name: serverName })
        );
      } finally {
        setConnectingServers((prev) => {
          const next = new Set(prev);
          next.delete(serverName);
          return next;
        });
      }
    },
    [loadData]
  );

  // Disconnect from a server
  const handleDisconnect = useCallback(
    async (serverName: string) => {
      setDisconnectingServers((prev) => new Set(prev).add(serverName));
      try {
        await mcp.disconnect(serverName);
        notify.success(t("mcp.disconnectedFrom", { name: serverName }));
        await loadData();
      } catch (err) {
        logger.error(`Failed to disconnect from ${serverName}:`, err);
        notify.error(
          err instanceof Error ? err.message : t("mcp.disconnectFailed", { name: serverName })
        );
      } finally {
        setDisconnectingServers((prev) => {
          const next = new Set(prev);
          next.delete(serverName);
          return next;
        });
      }
    },
    [loadData]
  );

  const handleSetup = useCallback(
    async (serverName: string) => {
      setSettingUpServers((prev) => new Set(prev).add(serverName));
      try {
        const result = await setupBuiltinMcp(serverName, workspacePath);
        if (result.success) {
          notify.success(t("mcp.setupComplete", { name: serverName }));
          await loadData();
        } else {
          notify.error(result.message);
        }
      } catch (err) {
        logger.error(`Failed to setup ${serverName}:`, err);
        notify.error(
          err instanceof Error ? err.message : t("mcp.setupFailed", { name: serverName })
        );
      } finally {
        setSettingUpServers((prev) => {
          const next = new Set(prev);
          next.delete(serverName);
          return next;
        });
      }
    },
    [loadData, workspacePath, t]
  );

  // Toggle server expansion
  const toggleExpanded = useCallback((serverName: string) => {
    setExpandedServers((prev) => {
      const next = new Set(prev);
      if (next.has(serverName)) {
        next.delete(serverName);
      } else {
        next.add(serverName);
      }
      return next;
    });
  }, []);

  // Get tools for a specific server
  const getToolsForServer = useCallback(
    (serverName: string) => {
      return tools.filter((t) => t.serverName === serverName);
    },
    [tools]
  );

  // Render status indicator
  const renderStatus = (status: mcp.McpServerStatus, error?: string | null) => {
    switch (status) {
      case "connected":
        return (
          <div className="flex items-center gap-1.5">
            <Check className="w-3.5 h-3.5 text-green-500" />
            <span className="text-xs text-green-600">{t("mcp.status.connected")}</span>
          </div>
        );
      case "connecting":
        return (
          <div className="flex items-center gap-1.5">
            <Loader2 className="w-3.5 h-3.5 text-blue-500 animate-spin" />
            <span className="text-xs text-blue-600">{t("mcp.status.connecting")}</span>
          </div>
        );
      case "error":
        return (
          <div
            className="flex items-center gap-1.5"
            title={error || t("mcp.status.connectionError")}
          >
            <AlertCircle className="w-3.5 h-3.5 text-red-500" />
            <span className="text-xs text-red-600 truncate max-w-[150px]">
              {error || t("common.error")}
            </span>
          </div>
        );
      default:
        return (
          <div className="flex items-center gap-1.5">
            <div className="w-2 h-2 rounded-full bg-muted-foreground/50" />
            <span className="text-xs text-muted-foreground">{t("mcp.status.disconnected")}</span>
          </div>
        );
    }
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-8">
        <Loader2 className="w-6 h-6 text-muted-foreground animate-spin" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <h3 className="text-sm font-medium text-foreground">{t("mcp.title")}</h3>
          <p className="text-xs text-muted-foreground">{t("mcp.description")}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={loadData} title={t("common.refresh")}>
            <RefreshCw className="w-4 h-4" />
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => open("https://modelcontextprotocol.io/servers")}
          >
            <ExternalLink className="w-4 h-4 mr-2" />
            {t("mcp.browseServers")}
          </Button>
        </div>
      </div>

      {/* Config location info */}
      <div className="text-xs text-muted-foreground bg-[var(--bg-secondary)] rounded-md px-3 py-2 border border-[var(--border-subtle)]">
        <p>
          {t("mcp.configPrefix")} <code className="text-accent">~/.golish/mcp.json</code>{" "}
          {t("mcp.configMiddle")}{" "}
          <code className="text-accent">&lt;project&gt;/.golish/mcp.json</code>{" "}
          {t("mcp.configSuffix")}
        </p>
      </div>

      {/* Node.js warning for built-in servers */}
      {servers.some(
        (s) => (s as mcp.McpServerInfo & { setupStatus?: string }).setupStatus === "needs_node"
      ) && (
        <div className="flex items-center gap-2 px-3 py-2.5 rounded-lg border border-amber-500/20 bg-amber-500/5">
          <AlertTriangle className="w-3.5 h-3.5 text-amber-400 flex-shrink-0" />
          <span className="text-[11px] text-amber-300/90">{t("mcp.nodeRequired")}</span>
        </div>
      )}

      {/* Server list */}
      {servers.length === 0 ? (
        <div className="text-center py-8 text-muted-foreground text-sm">
          <Server className="w-8 h-8 mx-auto mb-3 opacity-50" />
          <p>{t("mcp.empty")}</p>
          <p className="mt-1 text-xs">
            {t("mcp.emptyHintPrefix")} <code>~/.golish/mcp.json</code> {t("mcp.emptyHintSuffix")}
          </p>
        </div>
      ) : (
        <div className="space-y-2">
          {servers.map((server) => {
            const isConnecting = connectingServers.has(server.name);
            const isDisconnecting = disconnectingServers.has(server.name);
            const isExpanded = expandedServers.has(server.name);
            const serverTools = getToolsForServer(server.name);
            const isConnected = server.status === "connected";
            const isDisabled = !server.enabled;

            return (
              <div
                key={server.name}
                className={`rounded-lg border bg-[var(--bg-secondary)] ${
                  isDisabled
                    ? "border-[var(--border-subtle)] opacity-60"
                    : "border-[var(--border-medium)]"
                }`}
              >
                {/* Server header */}
                <div className="flex items-center justify-between px-4 py-3">
                  <div className="flex items-center gap-3 flex-1 min-w-0">
                    {/* Expand/collapse button (only if connected with tools) */}
                    {isConnected && serverTools.length > 0 ? (
                      <button
                        type="button"
                        onClick={() => toggleExpanded(server.name)}
                        className="p-0.5 hover:bg-[var(--bg-hover)] rounded"
                      >
                        {isExpanded ? (
                          <ChevronDown className="w-4 h-4 text-muted-foreground" />
                        ) : (
                          <ChevronRight className="w-4 h-4 text-muted-foreground" />
                        )}
                      </button>
                    ) : (
                      <div className="w-5" />
                    )}

                    {/* Server info */}
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium text-foreground truncate">
                          {server.name}
                        </span>
                        <Badge variant="outline" className="text-[10px] px-1.5 py-0">
                          {server.transport}
                        </Badge>
                        {server.source === "builtin" && (
                          <Badge
                            variant="secondary"
                            className="text-[10px] px-1.5 py-0 bg-blue-500/15 text-blue-400 border-blue-500/30"
                          >
                            {t("mcp.source.builtin")}
                          </Badge>
                        )}
                        {server.source === "project" && (
                          <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
                            {t("mcp.source.project")}
                          </Badge>
                        )}
                        {isDisabled && (
                          <Badge variant="secondary" className="text-[10px] px-1.5 py-0">
                            {t("mcp.status.disabled")}
                          </Badge>
                        )}
                      </div>
                      <div className="mt-1 flex items-center gap-3">
                        {renderStatus(server.status, server.error)}
                        {isConnected && serverTools.length > 0 && (
                          <span className="text-xs text-muted-foreground">
                            {t("mcp.toolCount", { count: serverTools.length })}
                          </span>
                        )}
                      </div>
                    </div>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-2">
                    {isConnected ? (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleDisconnect(server.name)}
                        disabled={isDisconnecting}
                        className="text-muted-foreground hover:text-foreground"
                      >
                        {isDisconnecting ? (
                          <Loader2 className="w-4 h-4 animate-spin" />
                        ) : (
                          <Plug className="w-4 h-4" />
                        )}
                        <span className="ml-2">{t("mcp.disconnect")}</span>
                      </Button>
                    ) : (server as mcp.McpServerInfo & { setupStatus?: string }).setupStatus ===
                      "needs_build" ? (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => handleSetup(server.name)}
                        disabled={settingUpServers.has(server.name)}
                        className="text-accent border-accent/30"
                      >
                        {settingUpServers.has(server.name) ? (
                          <Loader2 className="w-4 h-4 animate-spin" />
                        ) : (
                          <Download className="w-4 h-4" />
                        )}
                        <span className="ml-2">
                          {settingUpServers.has(server.name)
                            ? t("common.loading")
                            : t("common.install")}
                        </span>
                      </Button>
                    ) : (server as mcp.McpServerInfo & { setupStatus?: string }).setupStatus ===
                      "needs_node" ? (
                      <span className="text-[10px] text-amber-400">{t("mcp.needsNode")}</span>
                    ) : (
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => handleConnect(server.name)}
                        disabled={isConnecting || isDisabled}
                      >
                        {isConnecting ? (
                          <Loader2 className="w-4 h-4 animate-spin" />
                        ) : (
                          <PlugZap className="w-4 h-4" />
                        )}
                        <span className="ml-2">{t("mcp.connect")}</span>
                      </Button>
                    )}
                  </div>
                </div>

                {/* Expanded tools list */}
                {isConnected && serverTools.length > 0 && (
                  <Collapsible open={isExpanded}>
                    <CollapsibleContent>
                      <div className="border-t border-[var(--border-subtle)] px-4 py-3 bg-[var(--bg-primary)]">
                        <div className="text-xs font-medium text-muted-foreground mb-2 flex items-center gap-1.5">
                          <Wrench className="w-3.5 h-3.5" />
                          {t("mcp.availableTools")}
                        </div>
                        <div className="space-y-1.5">
                          {serverTools.map((tool) => (
                            <div
                              key={tool.name}
                              className="text-xs py-1.5 px-2 rounded bg-[var(--bg-secondary)] border border-[var(--border-subtle)]"
                            >
                              <div className="font-mono text-foreground">{tool.toolName}</div>
                              {tool.description && (
                                <div className="text-muted-foreground mt-0.5 line-clamp-2">
                                  {tool.description}
                                </div>
                              )}
                            </div>
                          ))}
                        </div>
                      </div>
                    </CollapsibleContent>
                  </Collapsible>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
