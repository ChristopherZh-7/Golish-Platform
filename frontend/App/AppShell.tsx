import type React from "react";
import { Suspense, useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { listServers } from "@/lib/api/mcp";
import { isWindows } from "@/lib/env";
import { checkEnvSetup, scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { cn } from "@/lib/utils";
import { isMockBrowserMode } from "@/mocks";
import { ActivityBar, type ActivityView } from "../components/ActivityBar/ActivityBar";
import { AIChatPanel } from "../components/AIChatPanel/AIChatPanel";
import { CommandPalette, type PageRoute } from "../components/CommandPalette";
import { SETUP_BANNER_NAVIGATE_EVENT } from "../components/HomeView/SetupHealthBanner";
import { PaneContainer } from "../components/PaneContainer";
import { SidecarNotifications } from "../components/Sidecar";
import { TerminalLayer } from "../components/Terminal";
import { WindowControls } from "../components/WindowControls/WindowControls";
import { useCreateTerminalTab } from "../hooks/useCreateTerminalTab";
import { TerminalPortalProvider } from "../hooks/useTerminalPortal";
import { useStore } from "../store";
import { useAppState } from "../store/selectors";
import { createNewConversation } from "../store/slices/conversation";
import { AppErrorFallback, AppLoadingSkeleton } from "./components/AppLoadingSkeleton";
import { SplitColumn, SplitDropZone } from "./components/SplitColumn";
import type { ActivityViewControls } from "./hooks/useActivityViewControls";
import { useSplitTabDrag } from "./hooks/useSplitTabDrag";
import {
  AuditLogPanelView,
  ComponentTestbed,
  ContextPanel,
  DashboardPanelView,
  FileEditorSidebarPanel,
  FindingsPanelView,
  KeyboardShortcutsHelp,
  MethodologyPanelView,
  QuickOpenDialog,
  RecordingsPanelView,
  SessionBrowser,
  SettingsContent,
  SettingsDialog,
  SettingsNav,
  SidecarPanel,
  TargetPanelView,
  ToolManagerView,
  VulnIntelPanelView,
  WikiPanelView,
  WordlistPanelView,
} from "./lazyRegistry";

const FULLSCREEN_OVERLAYS: Array<{
  view: NonNullable<ActivityView>;
  Component: React.LazyExoticComponent<React.ComponentType>;
  innerClassName?: string;
}> = [
  { view: "toolManage", Component: ToolManagerView },
  { view: "wiki", Component: WikiPanelView },
  { view: "targets", Component: TargetPanelView },
  { view: "methodology", Component: MethodologyPanelView, innerClassName: "relative" },
  { view: "dashboard", Component: DashboardPanelView },
  { view: "findings", Component: FindingsPanelView },
  { view: "auditLog", Component: AuditLogPanelView },
  { view: "wordlists", Component: WordlistPanelView },
  { view: "vulnIntel", Component: VulnIntelPanelView },
];

export interface AppShellProps {
  // Lifecycle status
  isLoading: boolean;
  error: string | null;

  // Routing
  currentPage: PageRoute;
  setCurrentPage: (page: PageRoute) => void;
  activityView: ActivityView;
  setActivityView: React.Dispatch<React.SetStateAction<ActivityView>>;
  activityControls: ActivityViewControls;
  visitedViews: Set<string>;

  // Dialog / panel / bottom-terminal state subscribed directly inside
  // AppShell via `useStore` (dropped from props in 2026-05 prop-drilling
  // reform — reduces shellProps fan-out by 16 fields).

  // Right split column
  rightPanelTabs: string[];
  rightActiveTab: string | null;
  setRightActiveTab: React.Dispatch<React.SetStateAction<string | null>>;
  rightPanelWidth: number;
  showSplitDropZone: boolean;
  setShowMergeDropZone: React.Dispatch<React.SetStateAction<boolean>>;
  splitDragGhost: { x: number; y: number; name: string } | null;
  setSplitDragGhost: React.Dispatch<
    React.SetStateAction<{ x: number; y: number; name: string } | null>
  >;
  closeRightTab: (tabId?: string) => void;
  handlePanelResizeStart: (e: React.PointerEvent) => void;

  // Tab + session actions
  handleNewTab: () => void;
  handleToggleMode: () => void;
  handleClearConversation: () => Promise<void>;
  handleToggleFullTerminal: () => void;
  handleRestoreSession: (identifier: string) => Promise<void>;
  handleOpenHistory: () => void;

  // Pane actions (only what JSX needs)
  handleSplitPane: (direction: "vertical" | "horizontal") => Promise<void>;
  handleClosePane: () => Promise<void>;

  // Panel actions
  openContextPanel: () => void;
  openSettingsTab: () => void;
  toggleFileEditorPanel: () => void;
  handleContextPanelOpenChange: (open: boolean) => void;
  handleFileEditorPanelOpenChange: (open: boolean) => void;
  handleSidecarPanelOpenChange: (open: boolean) => void;
}

function CenterSessionIndicator() {
  const session = useStore((s) => {
    if (!s.activeSessionId) return null;
    return s.sessions[s.activeSessionId] ?? null;
  });
  const convTitle = useStore((s) => {
    const convId = s.activeConversationId;
    if (!convId) return null;
    return s.conversations[convId]?.title ?? null;
  });

  if (!session) return null;

  const dirName = session.workingDirectory?.split(/[/\\]/).pop() || "";
  const displayName =
    convTitle || session.customName || session.processName || dirName || "Terminal";

  return (
    <div className="h-[34px] flex items-center px-3 gap-2 border-b border-border/20 flex-shrink-0 select-none">
      <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 flex-shrink-0" />
      <span className="text-[11px] font-medium text-foreground/70 truncate">{displayName}</span>
      {dirName && displayName !== dirName && (
        <span className="text-[10px] text-foreground/40 truncate ml-auto font-mono">{dirName}</span>
      )}
    </div>
  );
}

export function AppShell(props: AppShellProps) {
  const {
    isLoading,
    error,
    currentPage,
    setCurrentPage,
    activityView,
    setActivityView,
    activityControls,
    visitedViews,
    rightPanelTabs,
    rightActiveTab,
    setRightActiveTab,
    rightPanelWidth,
    showSplitDropZone,
    setShowMergeDropZone,
    splitDragGhost,
    setSplitDragGhost,
    closeRightTab,
    handlePanelResizeStart,
    handleNewTab,
    handleToggleMode,
    handleClearConversation,
    handleToggleFullTerminal,
    handleRestoreSession,
    handleOpenHistory,
    handleSplitPane,
    handleClosePane,
    openContextPanel,
    openSettingsTab,
    toggleFileEditorPanel,
    handleContextPanelOpenChange,
    handleFileEditorPanelOpenChange,
    handleSidecarPanelOpenChange,
  } = props;

  // Store subscriptions owned by the shell (presentational reads only)
  const { activeSessionId, focusedWorkingDirectory: workingDirectory, tabLayouts } = useAppState();
  const contextPanelOpen = useStore((state) => state.contextPanelOpen);
  const fileEditorPanelOpen = useStore((state) => state.fileEditorPanelOpen);
  const sidecarPanelOpen = useStore((state) => state.sidecarPanelOpen);
  const isOnHomeTab = useStore((s) => s.homeTabId !== null && s.activeSessionId === s.homeTabId);
  const chatPanelVisible = useStore((s) => s.chatPanelVisible);
  const uiScale = useStore((s) => s.displaySettings.uiScale);
  // Dialog / panel / bottom-terminal state — moved here from props in
  // the 2026-05 prop-drilling reform.
  const commandPaletteOpen = useStore((s) => s.commandPaletteOpen);
  const setCommandPaletteOpen = useStore((s) => s.setCommandPaletteOpen);
  const quickOpenDialogOpen = useStore((s) => s.quickOpenDialogOpen);
  const setQuickOpenDialogOpen = useStore((s) => s.setQuickOpenDialogOpen);
  const settingsOpen = useStore((s) => s.settingsDialogOpen);
  const setSettingsOpen = useStore((s) => s.setSettingsDialogOpen);
  const settingsSection = useStore((s) => s.settingsSection);
  const setSettingsSection = useStore((s) => s.setSettingsSection);
  const shortcutsHelpOpen = useStore((s) => s.shortcutsHelpOpen);
  const setShortcutsHelpOpen = useStore((s) => s.setShortcutsHelpOpen);
  const recordingsPanelOpen = useStore((s) => s.recordingsPanelOpen);
  const setRecordingsPanelOpen = useStore((s) => s.setRecordingsPanelOpen);
  const sessionBrowserOpen = useStore((s) => s.sessionBrowserOpen);
  const setSessionBrowserOpen = useStore((s) => s.setSessionBrowserOpen);
  const bottomTerminalOpen = useStore((s) => s.bottomTerminalOpen);

  const { createTerminalTab } = useCreateTerminalTab();
  const splitDrag = useSplitTabDrag({ setShowMergeDropZone, setSplitDragGhost, closeRightTab });

  const [toolIssueCount, setToolIssueCount] = useState(0);
  const [runtimeIssueCount, setRuntimeIssueCount] = useState(0);
  const [mcpIssueCount, setMcpIssueCount] = useState(0);
  const refreshEnvHealth = useCallback(async () => {
    if (isMockBrowserMode() || isOnHomeTab) return;
    try {
      const [toolsResult, envResult, mcpResult] = await Promise.allSettled([
        scanTools(),
        checkEnvSetup(),
        listServers(),
      ]);
      let toolCount = 0;
      let rtCount = 0;
      let mcpCount = 0;
      if (toolsResult.status === "fulfilled" && toolsResult.value.success) {
        toolCount = toolsResult.value.tools.filter(
          (t: ToolConfig) =>
            (t.tier === "essential" || t.tier === "recommended") &&
            (t as ToolConfig & { installed?: boolean }).installed === false
        ).length;
      }
      if (envResult.status === "fulfilled") {
        const env = envResult.value;
        // Homebrew is macOS/Linux-only — don't count it as a missing runtime on Windows.
        if (!isWindows() && !env.homebrew_installed) rtCount++;
        if (!env.conda_installed) rtCount++;
        if (!env.nvm_installed) rtCount++;
        if (!env.java_installed) rtCount++;
        if (!env.ruby_installed) rtCount++;
        if (!env.pgvector_installed) rtCount++;
      }
      if (mcpResult.status === "fulfilled") {
        mcpCount = mcpResult.value.filter(
          (s) => s.enabled && (s.status === "disconnected" || s.status === "error")
        ).length;
      }
      setToolIssueCount(toolCount);
      setRuntimeIssueCount(rtCount);
      setMcpIssueCount(mcpCount);
    } catch {
      /* ignore */
    }
  }, [isOnHomeTab]);
  useEffect(() => {
    refreshEnvHealth();
    const id = setInterval(refreshEnvHealth, 60_000);
    return () => clearInterval(id);
  }, [refreshEnvHealth]);

  // Forward Home Tab's `<SetupHealthBanner />` "Go to Tool Manager" / "Go to
  // Settings" clicks into the activity-view machinery owned by `useAppRouting`.
  // The banner cannot reach `activityControls` directly because the activity
  // view is local React state in `App.tsx`, so we bridge via window events.
  useEffect(() => {
    const handler = (e: Event) => {
      const target = (e as CustomEvent<string>).detail;
      if (target === "tool-manager") {
        activityControls.toggleView("toolManage");
      } else if (target === "settings") {
        setSettingsOpen(true);
      }
    };
    window.addEventListener(SETUP_BANNER_NAVIGATE_EVENT, handler);
    return () => window.removeEventListener(SETUP_BANNER_NAVIGATE_EVENT, handler);
  }, [activityControls, setSettingsOpen]);

  const hasSplit = rightPanelTabs.length > 0;

  if (isLoading) return <AppLoadingSkeleton />;
  if (error) return <AppErrorFallback error={error} />;

  // Render component testbed page
  if (currentPage === "testbed") {
    return (
      <>
        <Suspense fallback={<div className="h-screen w-screen bg-background" />}>
          <ComponentTestbed />
        </Suspense>
        <CommandPalette
          open={commandPaletteOpen}
          onOpenChange={setCommandPaletteOpen}
          currentPage={currentPage}
          onNavigate={setCurrentPage}
          activeSessionId={activeSessionId}
          onNewTab={handleNewTab}
          onToggleMode={handleToggleMode}
          onClearConversation={handleClearConversation}
          onToggleFullTerminal={handleToggleFullTerminal}
          onOpenSessionBrowser={handleOpenHistory}
          onOpenSettings={openSettingsTab}
        />
        <Suspense fallback={null}>
          <SessionBrowser
            open={sessionBrowserOpen}
            onOpenChange={setSessionBrowserOpen}
            onSessionRestore={handleRestoreSession}
          />
        </Suspense>
        <Suspense fallback={null}>
          <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
        </Suspense>
      </>
    );
  }

  const renderLeftPanel = () => {
    switch (activityView) {
      case "settings":
        return (
          <Suspense fallback={null}>
            <SettingsNav
              activeSection={settingsSection}
              onSectionChange={setSettingsSection}
              envIssueCount={runtimeIssueCount}
              mcpIssueCount={mcpIssueCount}
            />
          </Suspense>
        );
      default:
        return null;
    }
  };

  return (
    <TerminalPortalProvider>
      <div
        className="bg-background flex flex-col overflow-hidden app-bg-layered"
        data-bottom-terminal={bottomTerminalOpen ? "open" : "closed"}
        style={{
          zoom: uiScale,
          width: `calc(100vw / ${uiScale})`,
          height: `calc(100vh / ${uiScale})`,
        }}
      >
        {/* Window drag region — macOS traffic lights (left) / Windows custom controls (right) */}
        <div
          className={cn(
            "w-full titlebar-drag flex-shrink-0 flex items-center",
            isWindows() ? "h-[32px]" : "h-[38px]"
          )}
          data-tauri-drag-region
        >
          <div className="flex-1" data-tauri-drag-region />
          <WindowControls />
        </div>

        {/* Content - floating panels */}
        <div className="flex-1 flex overflow-hidden gap-2 px-2 pb-2 min-h-0 relative">
          {/* Activity Bar - instant hide when viewing the home tab */}
          <div className={cn("flex-shrink-0 overflow-hidden", isOnHomeTab ? "w-0" : "w-[48px]")}>
            <ActivityBar
              activeView={activityView}
              onViewChange={setActivityView}
              terminalOpen={bottomTerminalOpen}
              onToggleTerminal={activityControls.toggleBottomTerminal}
              onOpenSettings={() => setSettingsOpen(true)}
              toolIssueCount={toolIssueCount}
              settingsIssueCount={runtimeIssueCount + mcpIssueCount}
            />
          </div>

          {/* Left panel - only shown for settings view */}
          <div
            className={cn(
              "flex-shrink-0 h-full rounded-xl bg-card overflow-hidden panel-float",
              activityView === "settings" ? "w-[220px]" : "w-0 pointer-events-none -mr-2"
            )}
          >
            {renderLeftPanel()}
          </div>

          {/* Settings view - overlays center+right area */}
          {visitedViews.has("settings") && (
            <div
              className={cn(
                "absolute inset-0 left-[284px] flex transition-opacity duration-150 ease-out px-2 pb-2 pt-0",
                activityView === "settings"
                  ? "opacity-100 pointer-events-auto z-10"
                  : "opacity-0 pointer-events-none z-0"
              )}
            >
              <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
                <Suspense fallback={null}>
                  <SettingsContent activeSection={settingsSection} />
                </Suspense>
              </div>
            </div>
          )}

          {/* Fullscreen activity view overlays */}
          {FULLSCREEN_OVERLAYS.map(
            ({ view, Component, innerClassName }) =>
              visitedViews.has(view) && (
                <div
                  key={view}
                  className={cn(
                    "absolute inset-0 left-[64px] flex transition-opacity duration-150 ease-out pr-2 pb-2 pt-0",
                    activityView === view
                      ? "opacity-100 pointer-events-auto z-10"
                      : "opacity-0 pointer-events-none z-0"
                  )}
                >
                  <div
                    className={cn(
                      "flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float",
                      innerClassName
                    )}
                  >
                    <Suspense fallback={null}>
                      <Component />
                    </Suspense>
                  </div>
                </div>
              )
          )}

          {/* Normal view - center + right panels */}
          <div
            className={cn(
              "flex-1 flex gap-1 min-w-0 transition-opacity duration-150 ease-out",
              activityView === "settings" ||
                FULLSCREEN_OVERLAYS.some((o) => o.view === activityView)
                ? "opacity-0 pointer-events-none"
                : "opacity-100 pointer-events-auto"
            )}
          >
            {/* Center - TabBar + Pane content (with optional split) */}
            <div
              className={cn(
                "flex-1 min-w-0 flex gap-2 overflow-hidden relative",
                hasSplit ? "flex-row" : "flex-col"
              )}
            >
              {/* Left column (or full width when no split) */}
              <div
                className={cn(
                  "flex flex-col overflow-hidden rounded-xl bg-card panel-float relative",
                  hasSplit ? "flex-1 min-w-0" : "flex-1"
                )}
              >
                {/* 1:1 model: minimal session indicator instead of TabBar */}
                {!isOnHomeTab && <CenterSessionIndicator />}

                <div className="flex-1 min-h-0 min-w-0 flex overflow-hidden">
                  <div className="flex-1 min-h-0 min-w-0 flex flex-col overflow-hidden relative">
                    {tabLayouts.map(({ tabId, root }) => {
                      const isActive = hasSplit
                        ? tabId === activeSessionId && !rightPanelTabs.includes(tabId)
                        : tabId === activeSessionId;
                      return (
                        <div
                          key={tabId}
                          className={`absolute inset-0 ${isActive ? "" : "invisible pointer-events-none [&_.pane-bottom-terminal]:!hidden"}`}
                        >
                          <PaneContainer node={root} tabId={tabId} />
                        </div>
                      );
                    })}
                    {!activeSessionId && (
                      <div className="flex items-center justify-center h-full">
                        <span className="text-muted-foreground">No active session</span>
                      </div>
                    )}
                  </div>

                  <Suspense fallback={null}>
                    <ContextPanel
                      open={contextPanelOpen}
                      onOpenChange={handleContextPanelOpenChange}
                    />
                  </Suspense>
                  <Suspense fallback={null}>
                    <FileEditorSidebarPanel
                      open={fileEditorPanelOpen}
                      onOpenChange={handleFileEditorPanelOpenChange}
                    />
                  </Suspense>
                </div>
              </div>

              {hasSplit && (
                <SplitColumn
                  rightPanelTabs={rightPanelTabs}
                  rightActiveTab={rightActiveTab}
                  setRightActiveTab={setRightActiveTab}
                  tabLayouts={tabLayouts}
                  closeRightTab={closeRightTab}
                  splitDrag={splitDrag}
                />
              )}
              {showSplitDropZone && <SplitDropZone />}
            </div>

            {/* Resize handle between center and right panels */}
            {!isOnHomeTab && (
              <div className="flex-shrink-0 w-0 relative z-10">
                <div
                  className="absolute inset-y-3 -left-1 w-2 cursor-col-resize hover:bg-accent/20 active:bg-accent/40 transition-colors rounded-full"
                  onPointerDown={handlePanelResizeStart}
                />
              </div>
            )}

            {/* Right sidebar - AI Chat Panel (hide on home tab or when collapsed) */}
            {!isOnHomeTab && chatPanelVisible && (
              <div
                data-right-panel
                className="flex-shrink-0 h-full rounded-xl bg-card overflow-hidden panel-float"
                style={{ width: rightPanelWidth }}
              >
                <AIChatPanel />
              </div>
            )}

            {/* Floating toggle to reopen collapsed chat panel */}
            {!isOnHomeTab && !chatPanelVisible && (
              <button
                type="button"
                onClick={async () => {
                  useStore.getState().setChatPanelVisible(true);
                  if (!useStore.getState().activeConversationId) {
                    const fresh = createNewConversation();
                    useStore.getState().addConversation(fresh);
                    const termId = await createTerminalTab(undefined, true);
                    if (termId) {
                      useStore.getState().addTerminalToConversation(fresh.id, termId);
                      useStore.getState().setActiveSession(termId);
                    }
                  }
                }}
                className="fixed bottom-16 right-4 z-50 flex items-center gap-2 px-3 py-2 rounded-lg bg-primary text-primary-foreground shadow-lg hover:bg-primary/90 transition-colors text-sm font-medium"
                title="Open AI Chat"
              >
                <svg
                  aria-hidden="true"
                  xmlns="http://www.w3.org/2000/svg"
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z" />
                </svg>
                Chat
              </button>
            )}
          </div>
        </div>

        {/* Terminal Layer - renders all Terminal instances via React portals */}
        <TerminalLayer />

        {/* Command Palette */}
        <CommandPalette
          open={commandPaletteOpen}
          onOpenChange={setCommandPaletteOpen}
          currentPage={currentPage}
          onNavigate={setCurrentPage}
          activeSessionId={activeSessionId}
          onNewTab={handleNewTab}
          onToggleMode={handleToggleMode}
          onClearConversation={handleClearConversation}
          onToggleFullTerminal={handleToggleFullTerminal}
          workingDirectory={workingDirectory}
          onOpenSessionBrowser={handleOpenHistory}
          onToggleFileEditorPanel={toggleFileEditorPanel}
          onOpenContextPanel={openContextPanel}
          onOpenSettings={openSettingsTab}
          onSplitPaneRight={() => handleSplitPane("vertical")}
          onSplitPaneDown={() => handleSplitPane("horizontal")}
          onClosePane={handleClosePane}
          onOpenQuickOpen={() => setQuickOpenDialogOpen(true)}
          onOpenBrowser={() => activityControls.toggleView("targets")}
          onOpenSecurity={() => activityControls.toggleView("targets")}
          onToggleToolManager={() => activityControls.toggleView("toolManage")}
          onToggleWiki={() => activityControls.toggleView("wiki")}
          onToggleBottomTerminal={activityControls.toggleBottomTerminal}
          onFocusAiChat={activityControls.focusAiChat}
          onOpenShortcutsHelp={() => setShortcutsHelpOpen(true)}
          onOpenRecordings={() => setRecordingsPanelOpen(true)}
        />

        <Suspense fallback={null}>
          <QuickOpenDialog
            open={quickOpenDialogOpen}
            onOpenChange={setQuickOpenDialogOpen}
            workingDirectory={workingDirectory}
          />
        </Suspense>

        <Suspense fallback={null}>
          <SidecarPanel open={sidecarPanelOpen} onOpenChange={handleSidecarPanelOpenChange} />
        </Suspense>

        <Suspense fallback={null}>
          <SessionBrowser
            open={sessionBrowserOpen}
            onOpenChange={setSessionBrowserOpen}
            onSessionRestore={handleRestoreSession}
          />
        </Suspense>

        <Suspense fallback={null}>
          <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
        </Suspense>

        <Suspense fallback={null}>
          <KeyboardShortcutsHelp open={shortcutsHelpOpen} onOpenChange={setShortcutsHelpOpen} />
        </Suspense>

        {/* Terminal Recordings Panel - overlay */}
        {recordingsPanelOpen && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
            <div className="w-[800px] h-[500px] max-w-[90vw] max-h-[80vh] rounded-xl overflow-hidden shadow-2xl">
              <Suspense fallback={null}>
                <RecordingsPanelView onClose={() => setRecordingsPanelOpen(false)} />
              </Suspense>
            </div>
          </div>
        )}

        <SidecarNotifications />

        {/* Floating ghost tab following cursor during right-panel drag */}
        {splitDragGhost &&
          createPortal(
            <div
              className="fixed z-[9999] pointer-events-none flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-card/95 border border-accent/70 text-foreground text-[11px] font-mono shadow-2xl backdrop-blur-md ring-1 ring-accent/20"
              style={{
                left: splitDragGhost.x,
                top: splitDragGhost.y,
                transform: "translate(-50%, -120%)",
                transition: "box-shadow 0.2s ease",
              }}
            >
              <svg
                aria-hidden="true"
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                className="text-accent flex-shrink-0"
              >
                <polyline points="4 17 10 11 4 5" />
                <line x1="12" y1="19" x2="12" y2="5" />
              </svg>
              <span className="truncate max-w-[120px]">{splitDragGhost.name}</span>
            </div>,
            document.body
          )}
      </div>
    </TerminalPortalProvider>
  );
}
