/**
 * InputStatusRow - Status elements for per-pane display.
 * Contains mode toggle, model selector, token usage, and context metrics.
 * This component was extracted from StatusBar to support multi-pane layouts.
 */

import { Bot, Terminal } from "lucide-react";
import { memo } from "react";
import { useProviderSettings } from "@/hooks/useProviderSettings";
import { cn } from "@/lib/utils";
import { isMockBrowserMode } from "@/mocks";
import { useContextMetrics, useInputMode, useSessionAiConfig, useStore } from "@/store";
import { selectDisplaySettings } from "@/store/slices";
import { ModelSelectorBadge } from "./ModelSelector";
import { ContextUsageBadge, DebugPopover, LangfuseBadge, McpServersBadge } from "./StatusBadges";

interface InputStatusRowProps {
  sessionId: string;
}

export const InputStatusRow = memo(function InputStatusRow({ sessionId }: InputStatusRowProps) {
  const aiConfig = useSessionAiConfig(sessionId);
  const status = aiConfig?.status ?? "disconnected";
  const errorMessage = aiConfig?.errorMessage;
  const inputMode = useInputMode(sessionId);
  const setInputMode = useStore((state) => state.setInputMode);
  const display = useStore(selectDisplaySettings);

  const hideAiItems =
    (display.hideAiSettingsInShellMode && inputMode === "terminal") ||
    status === "disconnected";

  const sessionWorkingDirectory = useStore((state) => state.sessions[sessionId]?.workingDirectory);
  const contextMetrics = useContextMetrics(sessionId);

  const [providerSettings, refreshProviderSettings] = useProviderSettings();
  const { langfuseActive, telemetryStats } = providerSettings;

  return (
    <div className="flex items-center justify-between px-3 py-1 text-xs text-muted-foreground">
      {/* Left side */}
      <div className="flex items-center">
        {/* Mode segmented control */}
        {status === "disconnected" ? (
          <div className="p-0.5 border border-transparent rounded-lg">
            <button type="button" aria-label="Terminal mode" title="Terminal"
              className="h-6 w-6 flex items-center justify-center rounded-md bg-accent/15 text-accent shadow-[0_0_8px_rgba(var(--accent-rgb),0.3)]">
              <Terminal className="size-icon-status-bar" />
            </button>
          </div>
        ) : !display.showInputModeToggle && inputMode !== "auto" ? (
          <div className="p-0.5 border border-transparent rounded-lg">
            <button type="button"
              aria-label={inputMode === "terminal" ? "Terminal mode" : "AI mode"}
              title={inputMode === "terminal" ? "Terminal" : "AI"}
              className="h-6 w-6 flex items-center justify-center rounded-md bg-accent/15 text-accent shadow-[0_0_8px_rgba(var(--accent-rgb),0.3)]">
              {inputMode === "terminal" ? <Terminal className="size-icon-status-bar" /> : <Bot className="size-icon-status-bar" />}
            </button>
          </div>
        ) : (
          <div className="flex items-center rounded-lg bg-muted/50 p-0.5 border border-[var(--border-subtle)]/50">
            <button type="button"
              aria-label={inputMode === "terminal" ? "Switch to Auto mode" : "Switch to Terminal mode"}
              title="Terminal"
              onClick={() => setInputMode(sessionId, inputMode === "terminal" ? "auto" : "terminal")}
              className={cn(
                "h-6 w-6 flex items-center justify-center rounded-md transition-all duration-200",
                inputMode === "terminal"
                  ? "bg-accent/15 text-accent shadow-[0_0_8px_rgba(var(--accent-rgb),0.3)]"
                  : "text-muted-foreground hover:text-foreground hover:bg-muted"
              )}>
              <Terminal className="size-icon-status-bar" />
            </button>
            <button type="button"
              aria-label={inputMode === "agent" ? "Switch to Auto mode" : "Switch to AI mode"}
              title="AI"
              onClick={() => setInputMode(sessionId, inputMode === "agent" ? "auto" : "agent")}
              className={cn(
                "h-6 w-6 flex items-center justify-center rounded-md transition-all duration-200",
                inputMode === "agent"
                  ? "bg-accent/15 text-accent shadow-[0_0_8px_rgba(var(--accent-rgb),0.3)]"
                  : "text-muted-foreground hover:text-foreground hover:bg-muted"
              )}>
              <Bot className="size-icon-status-bar" />
            </button>
          </div>
        )}

        {/* Model selector badge (hidden - moved to AI Chat Panel) */}
        {display.showStatusBadge && (
          <div className="ml-2" style={{ display: "none" }} data-visible={String(!hideAiItems)}>
            <div className="shrink-0 flex items-center gap-2">
              <div className="h-4 w-px bg-[var(--border-medium)]" />
              <ModelSelectorBadge sessionId={sessionId} />
            </div>
          </div>
        )}

        {/* Context utilization indicator */}
        {display.showContextUsage && (
          <div className="ui-fade-width ml-2" data-visible={String(!hideAiItems)}>
            <div className="shrink-0">
              <ContextUsageBadge
                utilization={contextMetrics.utilization}
                usedTokens={contextMetrics.usedTokens}
                maxTokens={contextMetrics.maxTokens}
              />
            </div>
          </div>
        )}

        {/* Langfuse tracing indicator */}
        {langfuseActive && (
          <LangfuseBadge telemetryStats={telemetryStats} onRefresh={refreshProviderSettings} />
        )}

        {/* MCP servers indicator */}
        {display.showMcpBadge && (
          <div className="ui-fade-width ml-2" data-visible={String(!hideAiItems)}>
            <div className="shrink-0">
              <McpServersBadge sessionId={sessionId} sessionWorkingDirectory={sessionWorkingDirectory} />
            </div>
          </div>
        )}

        {/* Debug panel (dev only) */}
        {import.meta.env.DEV && !isMockBrowserMode() && (
          <DebugPopover sessionId={sessionId} />
        )}
      </div>

      {/* Right side */}
      <div className="flex items-center gap-2">
        {isMockBrowserMode() ? (
          <span className="text-[var(--ansi-yellow)] text-[11px] truncate max-w-[200px]">Browser only mode</span>
        ) : (
          status === "error" && errorMessage && (
            <span className="text-destructive text-[11px] truncate max-w-[200px]">({errorMessage})</span>
          )
        )}
      </div>
    </div>
  );
});
