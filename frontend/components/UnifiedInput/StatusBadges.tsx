import { memo, useCallback, useEffect, useRef, useState } from "react";
import { Bug, Gauge, Server } from "lucide-react";
import { SiOpentelemetry } from "react-icons/si";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { type ApiRequestStatsSnapshot, getApiRequestStats } from "@/lib/ai";
import { logger } from "@/lib/logger";
import { formatRelativeTime } from "@/lib/time";
import * as mcp from "@/lib/mcp";
import { cn } from "@/lib/utils";
import { isMockBrowserMode } from "@/mocks";
import type { TelemetryStats } from "@/lib/settings";

function formatTokenCountDetailed(tokens: number): string {
  return tokens.toLocaleString();
}

function formatUptime(startedAtMs: number): string {
  const now = Date.now();
  const elapsedMs = now - startedAtMs;
  if (elapsedMs < 0) return "0s";
  const seconds = Math.floor(elapsedMs / 1000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  if (hours > 0) return `${hours}h ${minutes % 60}m`;
  if (minutes > 0) return `${minutes}m ${seconds % 60}s`;
  return `${seconds}s`;
}

/* ── Context Usage Badge ── */

interface ContextUsageBadgeProps {
  utilization: number;
  usedTokens: number;
  maxTokens: number;
}

export const ContextUsageBadge = memo(function ContextUsageBadge({ utilization, usedTokens, maxTokens }: ContextUsageBadgeProps) {
  if (maxTokens <= 0) {
    return (
      <button type="button" title="Context: not available"
        className="h-6 px-2 gap-1.5 text-xs font-medium rounded-lg flex items-center text-muted-foreground/70 border border-[var(--border-subtle)]/60 bg-card/30">
        <Gauge className="size-icon-status-bar" />
        <span>0%</span>
      </button>
    );
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" title={`Context: ${Math.round(utilization * 100)}% used`}
          className={cn(
            "h-6 px-2 gap-1.5 text-xs font-medium rounded-lg flex items-center cursor-pointer transition-all duration-200",
            utilization < 0.7 && "bg-[#9ece6a]/10 text-[#9ece6a] hover:bg-[#9ece6a]/20 border border-[#9ece6a]/20 hover:border-[#9ece6a]/30",
            utilization >= 0.7 && utilization < 0.85 && "bg-[#e0af68]/10 text-[#e0af68] hover:bg-[#e0af68]/20 border border-[#e0af68]/20 hover:border-[#e0af68]/30",
            utilization >= 0.85 && "bg-[#f7768e]/10 text-[#f7768e] hover:bg-[#f7768e]/20 border border-[#f7768e]/20 hover:border-[#f7768e]/30"
          )}>
          <Gauge className="size-icon-status-bar" />
          <span>{Math.round(utilization * 100)}%</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-auto min-w-[200px] p-3 bg-card/95 backdrop-blur-sm border-[var(--border-medium)] shadow-lg">
        <div className="text-xs font-medium text-muted-foreground mb-2">Context Window Usage</div>
        <div className="font-mono text-xs space-y-1">
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Used</span>
            <span className="text-foreground">{formatTokenCountDetailed(usedTokens)}</span>
          </div>
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Max</span>
            <span className="text-foreground">{formatTokenCountDetailed(maxTokens)}</span>
          </div>
          <div className="border-t border-[var(--border-subtle)] my-1.5" />
          <div className="flex justify-between gap-4">
            <span className="text-muted-foreground">Utilization</span>
            <span className={cn("font-medium",
              utilization < 0.7 && "text-[#9ece6a]",
              utilization >= 0.7 && utilization < 0.85 && "text-[#e0af68]",
              utilization >= 0.85 && "text-[#f7768e]"
            )}>
              {Math.round(utilization * 100)}%
            </span>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
});

/* ── Langfuse Badge ── */

interface LangfuseBadgeProps {
  telemetryStats: TelemetryStats | null;
  onRefresh: () => void;
}

export const LangfuseBadge = memo(function LangfuseBadge({ telemetryStats, onRefresh }: LangfuseBadgeProps) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" title="Langfuse tracing enabled"
          className="ml-2 h-6 px-2 gap-1.5 text-xs font-medium rounded-lg flex items-center bg-[#7c3aed]/10 text-[#7c3aed] hover:bg-[#7c3aed]/20 border border-[#7c3aed]/20 hover:border-[#7c3aed]/30 transition-all duration-200 cursor-pointer"
          onClick={onRefresh}>
          <SiOpentelemetry className="size-icon-status-bar" />
          {telemetryStats && telemetryStats.spans_ended > 0 && (
            <span className="tabular-nums">{telemetryStats.spans_ended}</span>
          )}
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-auto min-w-[200px] p-3 bg-card/95 backdrop-blur-sm border-[var(--border-medium)] shadow-lg">
        <div className="text-xs font-medium text-muted-foreground mb-2">Langfuse Tracing</div>
        {telemetryStats ? (
          <div className="font-mono text-xs space-y-1">
            <div className="flex justify-between gap-4">
              <span className="text-muted-foreground">Spans Started</span>
              <span className="text-foreground tabular-nums">{telemetryStats.spans_started.toLocaleString()}</span>
            </div>
            <div className="flex justify-between gap-4">
              <span className="text-muted-foreground">Spans Queued</span>
              <span className="text-foreground tabular-nums">{telemetryStats.spans_ended.toLocaleString()}</span>
            </div>
            <div className="border-t border-[var(--border-subtle)] my-1.5" />
            <div className="flex justify-between gap-4">
              <span className="text-muted-foreground">Uptime</span>
              <span className="text-foreground">{formatUptime(telemetryStats.started_at)}</span>
            </div>
          </div>
        ) : (
          <div className="text-xs text-muted-foreground">Stats not available</div>
        )}
      </PopoverContent>
    </Popover>
  );
});

/* ── MCP Servers Badge ── */

interface McpServersBadgeProps {
  sessionId: string;
  sessionWorkingDirectory: string | undefined;
}

export const McpServersBadge = memo(function McpServersBadge({ sessionId, sessionWorkingDirectory }: McpServersBadgeProps) {
  const [mcpServers, setMcpServers] = useState<mcp.McpServerInfo[]>([]);
  const [mcpTools, setMcpTools] = useState<mcp.McpToolInfo[]>([]);

  useEffect(() => {
    if (isMockBrowserMode()) return;
    const loadMcpData = async () => {
      try {
        const servers = await mcp.listServers(sessionWorkingDirectory);
        setMcpServers(servers ?? []);
        if (sessionId) {
          try { setMcpTools(await mcp.listTools()); } catch { setMcpTools([]); }
        }
      } catch (err) {
        logger.error("Failed to load MCP servers:", err);
      }
    };
    loadMcpData();
  }, [sessionId, sessionWorkingDirectory]);

  const connectedMcpServers = mcpServers.filter((s) => s.status === "connected");
  if (mcpServers.length === 0) return null;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button type="button" title="MCP Servers"
          className={cn(
            "h-6 px-2 gap-1.5 text-xs font-medium rounded-lg flex items-center transition-all duration-200 cursor-pointer",
            connectedMcpServers.length > 0
              ? "bg-[#22d3ee]/10 text-[#22d3ee] hover:bg-[#22d3ee]/20 border border-[#22d3ee]/20 hover:border-[#22d3ee]/30"
              : "bg-muted/50 text-muted-foreground hover:bg-muted/70 border border-[var(--border-subtle)]"
          )}>
          <Server className="size-icon-status-bar" />
          <span className="tabular-nums">{connectedMcpServers.length}/{mcpServers.length}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-auto min-w-[240px] p-3 bg-card/95 backdrop-blur-sm border-[var(--border-medium)] shadow-lg">
        <div className="text-xs font-medium text-muted-foreground mb-2">MCP Servers</div>
        <div className="space-y-2">
          {mcpServers.map((server) => {
            const serverTools = mcpTools.filter((t) => t.serverName === server.name);
            return (
              <div key={server.name} className="text-xs">
                <div className="flex items-center justify-between gap-3">
                  <span className="font-medium text-foreground truncate max-w-[140px]">{server.name}</span>
                  <span className={cn("text-[10px] px-1.5 py-0.5 rounded",
                    server.status === "connected" && "bg-green-500/10 text-green-500",
                    server.status === "connecting" && "bg-blue-500/10 text-blue-500",
                    server.status === "error" && "bg-red-500/10 text-red-500",
                    server.status === "disconnected" && "bg-muted text-muted-foreground"
                  )}>
                    {server.status}
                  </span>
                </div>
                {server.status === "connected" && serverTools.length > 0 && (
                  <div className="text-muted-foreground mt-0.5 pl-2">{serverTools.length} tool{serverTools.length !== 1 ? "s" : ""}</div>
                )}
                {server.status === "error" && server.error && (
                  <div className="text-red-400 mt-0.5 pl-2 truncate max-w-[200px]" title={server.error}>{server.error}</div>
                )}
              </div>
            );
          })}
        </div>
        {mcpTools.length > 0 && (
          <>
            <div className="border-t border-[var(--border-subtle)] my-2" />
            <div className="flex justify-between text-xs">
              <span className="text-muted-foreground">Total Tools</span>
              <span className="text-foreground tabular-nums">{mcpTools.length}</span>
            </div>
          </>
        )}
      </PopoverContent>
    </Popover>
  );
});

/* ── Debug Popover ── */

interface DebugPopoverProps {
  sessionId: string;
}

export const DebugPopover = memo(function DebugPopover({ sessionId }: DebugPopoverProps) {
  const [debugOpen, setDebugOpen] = useState(false);
  const debugPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [apiRequestStats, setApiRequestStats] = useState<ApiRequestStatsSnapshot | null>(null);
  const [apiRequestStatsError, setApiRequestStatsError] = useState<string | null>(null);

  const refreshApiRequestStats = useCallback(async () => {
    try {
      const stats = await getApiRequestStats(sessionId);
      setApiRequestStats(stats);
      setApiRequestStatsError(null);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes("AI agent not initialized for session") || message.includes("Call init_ai_session first")) {
        setApiRequestStats(null);
        setApiRequestStatsError(null);
        return;
      }
      setApiRequestStatsError(message);
    }
  }, [sessionId]);

  useEffect(() => {
    if (!debugOpen) {
      if (debugPollRef.current) { clearInterval(debugPollRef.current); debugPollRef.current = null; }
      return;
    }
    refreshApiRequestStats();
    debugPollRef.current = setInterval(refreshApiRequestStats, 1500);
    return () => {
      if (debugPollRef.current) { clearInterval(debugPollRef.current); debugPollRef.current = null; }
    };
  }, [debugOpen, refreshApiRequestStats]);

  return (
    <Popover open={debugOpen} onOpenChange={setDebugOpen}>
      <PopoverTrigger asChild>
        <button type="button" title="Debug (This Tab)"
          className="ml-2 h-6 px-2 gap-1.5 text-xs font-medium rounded-lg flex items-center bg-[var(--ansi-yellow)]/10 text-[var(--ansi-yellow)] hover:bg-[var(--ansi-yellow)]/20 border border-[var(--ansi-yellow)]/20 hover:border-[var(--ansi-yellow)]/30 transition-all duration-200 cursor-pointer">
          <Bug className="size-icon-status-bar" />
          <span>Debug</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-[340px] p-3 bg-card/95 backdrop-blur-sm border-[var(--border-medium)] shadow-lg">
        <div className="text-xs font-medium text-muted-foreground mb-1">Debug (This Tab)</div>
        <div className="text-[11px] text-muted-foreground mb-3">LLM API Requests (main + sub-agents)</div>
        {apiRequestStatsError ? (
          <div className="text-xs text-destructive">{apiRequestStatsError}</div>
        ) : apiRequestStats ? (
          (() => {
            const providerEntries = Object.entries(apiRequestStats.providers).sort(([, a], [, b]) => b.requests - a.requests);
            if (providerEntries.length === 0) return <div className="text-xs text-muted-foreground">No requests yet.</div>;
            return (
              <div className="space-y-2">
                <div className="grid grid-cols-[1fr_auto_auto_auto] gap-x-3 text-[10px] uppercase tracking-wide text-muted-foreground">
                  <span>Provider</span>
                  <span className="text-right">Req</span>
                  <span className="text-right">Sent</span>
                  <span className="text-right">Recv</span>
                </div>
                <div className="border-t border-[var(--border-subtle)]" />
                <div className="space-y-1">
                  {providerEntries.map(([name, stats]) => (
                    <div key={name} className="grid grid-cols-[1fr_auto_auto_auto] gap-x-3 text-xs font-mono">
                      <span className="truncate" title={name}>{name}</span>
                      <span className="text-right tabular-nums">{stats.requests}</span>
                      <span className="text-right tabular-nums" title={stats.last_sent_at ? new Date(stats.last_sent_at).toLocaleString() : "—"}>
                        {stats.last_sent_at ? formatRelativeTime(stats.last_sent_at, "localeDate") : "—"}
                      </span>
                      <span className="text-right tabular-nums" title={stats.last_received_at ? new Date(stats.last_received_at).toLocaleString() : "—"}>
                        {stats.last_received_at ? formatRelativeTime(stats.last_received_at, "localeDate") : "—"}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            );
          })()
        ) : (
          <div className="text-xs text-muted-foreground">No requests yet.</div>
        )}
      </PopoverContent>
    </Popover>
  );
});
