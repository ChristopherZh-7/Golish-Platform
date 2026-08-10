/**
 * PaneLeaf - Individual pane content renderer.
 * Displays either UnifiedTimeline+UnifiedInput (timeline mode) or Terminal (fullterm mode).
 * Handles focus management and visual indicators.
 *
 * Terminal rendering is handled via React portals (see TerminalLayer) to prevent
 * unmount/remount when pane structure changes during splits.
 *
 * HomeView and SettingsTabContent are lazy-loaded to improve initial bundle size
 * and load performance. These tab types are less frequently used than the default
 * terminal view, so deferring their load is beneficial.
 *
 * Performance: Uses usePaneLeafState selector to subscribe only to relevant state,
 * preventing re-renders when unrelated session or layout properties change.
 */

import React, { lazy, Suspense, useCallback, useEffect } from "react";
import { ToolCallDetailView } from "@/components/ToolCallDetailView/ToolCallDetailView";
import { UnifiedInput } from "@/components/UnifiedInput";
import { UnifiedTimeline } from "@/components/UnifiedTimeline";
import { ContextMenuTrigger } from "@/components/ui/context-menu";
import { countLeafPanes } from "@/lib/pane-utils";
import type { PaneId } from "@/store";
import { usePendingCommand, useStore } from "@/store";
import { usePaneLeafState } from "@/store/selectors/pane-leaf";
import { PaneContextMenu } from "./PaneContextMenu";
import { PaneMoveOverlay } from "./PaneMoveOverlay";

// Phase B GridTerminal is lazy-loaded so non-fullterm sessions don't
// pay for the React + CSS bundle until they actually need it.
const GridTerminal = lazy(() =>
  import("@/components/GridTerminal").then((m) => ({ default: m.GridTerminal }))
);

// Lazy-load tab-specific components to reduce initial bundle size
// HomeView (~50KB) and SettingsTabContent (~80KB) are only needed when
// the user opens those specific tab types
const HomeView = lazy(() => import("@/components/HomeView").then((m) => ({ default: m.HomeView })));
const SettingsTabContent = lazy(() =>
  import("@/components/Settings/SettingsTabContent").then((m) => ({
    default: m.SettingsTabContent,
  }))
);
// Loading fallback component for lazy-loaded tab content
function TabLoadingFallback() {
  return (
    <div className="h-full w-full flex items-center justify-center">
      <div className="animate-pulse text-muted-foreground">Loading...</div>
    </div>
  );
}

interface PaneLeafProps {
  paneId: PaneId;
  sessionId: string;
  tabId: string;
}

export const PaneLeaf = React.memo(function PaneLeaf({ paneId, sessionId, tabId }: PaneLeafProps) {
  // Use combined selector for efficient state access - only re-renders when
  // specific properties change, not when entire Session/TabLayout objects change
  const { focusedPaneId, renderMode, tabType, sessionExists, sessionName, detailViewMode } =
    usePaneLeafState(tabId, sessionId);
  const terminalRestoreInProgress = useStore((s) => s.terminalRestoreInProgress);

  // Action is stable (doesn't change between renders)
  const focusPane = useStore((state) => state.focusPane);
  // Get pane count - subscribe to a primitive number instead of the full tree object
  const paneCount = useStore((state) => countLeafPanes(state.tabLayouts[tabId]?.root));

  const pendingCommand = usePendingCommand(sessionId);
  // Warp-style architecture: while a command is running, the bottom
  // `UnifiedInput` is fully unmounted and the `RunningCommandCard` in
  // the timeline owns both output streaming AND stdin (keystroke
  // routing to `ptyWrite`, IME composition, paste). When the command
  // ends, the card converts to a `CommandBlock` and the bottom input
  // re-appears in its place. `interactiveMode` (from the stdin_wait
  // detector) is no longer used to keep the bottom input visible —
  // it's now consumed by the card itself to auto-focus its hidden
  // textarea so the user can just type `y` + Enter without a click.
  const interactiveMode = useStore((s) => s.sessions[sessionId]?.interactiveMode ?? null);
  const isInteractiveInputActive = interactiveMode?.active === true;

  const isFocused = focusedPaneId === paneId;
  const showFocusIndicator = isFocused && paneCount > 1;

  const handleFocus = useCallback(() => {
    if (!isFocused) {
      focusPane(tabId, paneId);
    }
  }, [tabId, paneId, isFocused, focusPane]);

  // Window-level Esc fallback. The textarea-scoped Esc handler in
  // `useInputKeyboard.ts` only fires when the textarea is focused —
  // which is exactly the state the running-command branch above used
  // to blur away (see PaneLeaf's `isCommandRunning` opacity branch and
  // `useUnifiedInputState`'s `isProcessRunning` blur effect). Without
  // this fallback, a `stdin_wait` miss leaves the user with no way to
  // recover the input box short of clicking it and pressing Esc. We
  // scope the listener to the focused pane (so multi-pane layouts
  // don't compete) and skip when the textarea is the active element
  // (the React handler is sharper there — it knows about popups, tool
  // mode, etc.).
  useEffect(() => {
    if (!isFocused) return;
    const hasPendingCommand = !!pendingCommand;
    if (!hasPendingCommand && !isInteractiveInputActive) return;

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const active = document.activeElement as HTMLElement | null;
      if (active && active.tagName === "TEXTAREA") return;
      const store = useStore.getState();
      if (isInteractiveInputActive) {
        e.preventDefault();
        store.setInteractiveMode(sessionId, null);
        return;
      }
      if (hasPendingCommand) {
        e.preventDefault();
        store.handlePromptStart(sessionId);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isFocused, pendingCommand, isInteractiveInputActive, sessionId]);

  // Don't render if session doesn't exist
  if (!sessionExists) {
    return (
      <div className="h-full w-full flex items-center justify-center text-muted-foreground">
        Session not found
      </div>
    );
  }

  // Route content based on tab type
  // HomeView and SettingsTabContent are lazy-loaded with Suspense boundaries
  const renderTabContent = () => {
    switch (tabType) {
      case "home":
        return (
          <Suspense fallback={<TabLoadingFallback />}>
            <HomeView />
          </Suspense>
        );
      case "settings":
        return (
          <Suspense fallback={<TabLoadingFallback />}>
            <SettingsTabContent />
          </Suspense>
        );
      default: {
        const fullterm = renderMode === "fullterm";
        return (
          <>
            {/* Phase B GridTerminal — sole TUI renderer since D6.4b.
                Lazy-loaded so non-fullterm panes don't pay for the
                React + CSS bundle until they enter alt-screen. */}
            {fullterm && (
              <div className="flex-1 min-h-0 p-1" onMouseDownCapture={handleFocus}>
                <Suspense fallback={<TabLoadingFallback />}>
                  <GridTerminal sessionId={sessionId} />
                </Suspense>
              </div>
            )}
            {renderMode !== "fullterm" && (
              <>
                <div className="flex-1 min-h-0 min-w-0 flex flex-col overflow-hidden">
                  {terminalRestoreInProgress ? (
                    <div className="h-full flex items-center justify-center">
                      <div className="flex flex-col items-center gap-3 text-muted-foreground">
                        <div className="w-5 h-5 border-2 border-current border-t-transparent rounded-full animate-spin" />
                        <span className="text-sm">Restoring session...</span>
                      </div>
                    </div>
                  ) : detailViewMode === "tool-detail" || detailViewMode === "sub-agent-detail" ? (
                    <ToolCallDetailView sessionId={sessionId} />
                  ) : (
                    <UnifiedTimeline sessionId={sessionId} />
                  )}
                </div>
                {detailViewMode !== "sub-agent-detail" &&
                  detailViewMode !== "tool-detail" &&
                  // Hide ONLY when both (a) a command is still running AND
                  // (b) interactive mode is active — i.e. the user is
                  // actively typing into the RunningCommandCard's capture
                  // textarea. The moment `pendingCommand` clears (command
                  // ended / user Ctrl-C'd / SIGINT) we MUST re-show the
                  // bottom input, even if `interactiveMode` somehow lingers
                  // (race condition between `command_end` and the
                  // `setInteractiveMode(null)` it triggers). Without the
                  // `pendingCommand` guard the user can end up staring at
                  // a card-less, input-less pane after force-quitting a
                  // command.
                  !(isInteractiveInputActive && !!pendingCommand?.command) && (
                    // Bottom input is always mounted (except during
                    // interactive mode — see below) so short commands
                    // (`ls`, `pwd`, …) don't flash it out and back in.
                    //
                    // During Warp-style interactive mode
                    // (`stdin_wait` detector fires while a command is
                    // running) the bottom input is HIDDEN entirely and
                    // `RunningCommandCard` in the timeline takes over
                    // BOTH output streaming AND stdin via its zero-
                    // size offscreen capture textarea — keystrokes
                    // flow directly to the PTY and the user sees their
                    // `y` appear right next to the running program's
                    // `[Y/n]` prompt (just like Warp does). No
                    // separate "回复 cmd" textarea, no duplicated
                    // output rendering.
                    //
                    // Outside interactive mode but with a command
                    // running, the bottom input stays mounted; its
                    // submit handler early-returns inside
                    // `useUnifiedInputState` so an accidental Enter
                    // doesn't ship the buffered text into the PTY.
                    <div
                      className="pane-bottom-terminal origin-bottom"
                      data-input-state="idle"
                      data-interactive="false"
                    >
                      <UnifiedInput sessionId={sessionId} />
                    </div>
                  )}
              </>
            )}
          </>
        );
      }
    }
  };

  // Only show context menu for terminal tabs
  const isTerminal = tabType === "terminal" || tabType === undefined;

  const sectionContent = (
    <section
      className="h-full w-full flex flex-col relative overflow-hidden"
      tabIndex={-1}
      onClick={handleFocus}
      onKeyDown={handleFocus}
      onFocus={handleFocus}
      aria-label={`Pane: ${sessionName || "Terminal"}`}
      data-pane-drop-zone={sessionId}
    >
      {/* Focus indicator overlay - only show when multiple panes exist */}
      {showFocusIndicator && (
        <div
          className="absolute inset-0 pointer-events-none z-50 border border-accent"
          aria-hidden="true"
        />
      )}
      {/* Move overlay - shown when pane move mode is active */}
      {isTerminal && <PaneMoveOverlay paneId={paneId} />}
      {renderTabContent()}
    </section>
  );

  if (isTerminal) {
    return (
      <PaneContextMenu paneId={paneId} sessionId={sessionId} tabId={tabId}>
        <ContextMenuTrigger asChild>{sectionContent}</ContextMenuTrigger>
      </PaneContextMenu>
    );
  }

  return sectionContent;
});
