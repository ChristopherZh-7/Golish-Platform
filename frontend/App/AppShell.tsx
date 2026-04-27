import type React from "react";
import { Suspense } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/utils";
import { ActivityBar, type ActivityView } from "../components/ActivityBar/ActivityBar";
import { AIChatPanel } from "../components/AIChatPanel/AIChatPanel";
import { CommandPalette, type PageRoute } from "../components/CommandPalette";
import { PaneContainer } from "../components/PaneContainer";
import { SidecarNotifications } from "../components/Sidecar";
import { TerminalLayer } from "../components/Terminal";
import { useCreateTerminalTab } from "../hooks/useCreateTerminalTab";
import { TerminalPortalProvider } from "../hooks/useTerminalPortal";
import { useStore } from "../store";
import { useAppState } from "../store/selectors";
import { createNewConversation } from "../store/slices/conversation";
import {
  AuditLogPanelView,
  ComponentTestbed,
  ContextPanel,
  DashboardPanelView,
  FileEditorSidebarPanel,
  FindingsPanelView,
  GitPanel,
  KeyboardShortcutsHelp,
  MethodologyPanelView,
  PipelinePanelView,
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
import { AppErrorFallback, AppLoadingSkeleton } from "./components/AppLoadingSkeleton";
import { SplitColumn, SplitDropZone } from "./components/SplitColumn";
import { useSplitTabDrag } from "./hooks/useSplitTabDrag";


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
  { view: "pipelines", Component: PipelinePanelView },
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
  visitedViews: Set<string>;

  // Dialog state
  commandPaletteOpen: boolean;
  setCommandPaletteOpen: (open: boolean) => void;
  quickOpenDialogOpen: boolean;
  setQuickOpenDialogOpen: (open: boolean) => void;
  settingsOpen: boolean;
  setSettingsOpen: (open: boolean) => void;
  settingsSection: string;
  setSettingsSection: (section: string) => void;
  shortcutsHelpOpen: boolean;
  setShortcutsHelpOpen: (open: boolean) => void;
  recordingsPanelOpen: boolean;
  setRecordingsPanelOpen: (open: boolean) => void;
  sessionBrowserOpen: boolean;
  setSessionBrowserOpen: (open: boolean) => void;

  // Bottom terminal
  bottomTerminalOpen: boolean;
  setBottomTerminalOpen: (open: boolean) => void;

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
  handleGitPanelOpenChange: (open: boolean) => void;
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
    visitedViews,
    commandPaletteOpen,
    setCommandPaletteOpen,
    quickOpenDialogOpen,
    setQuickOpenDialogOpen,
    settingsOpen,
    setSettingsOpen,
    settingsSection,
    setSettingsSection,
    shortcutsHelpOpen,
    setShortcutsHelpOpen,
    recordingsPanelOpen,
    setRecordingsPanelOpen,
    sessionBrowserOpen,
    setSessionBrowserOpen,
    bottomTerminalOpen,
    setBottomTerminalOpen: _setBottomTerminalOpen,
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
    handleGitPanelOpenChange,
    handleContextPanelOpenChange,
    handleFileEditorPanelOpenChange,
    handleSidecarPanelOpenChange,
  } = props;

  // Store subscriptions owned by the shell (presentational reads only)
  const { activeSessionId, focusedWorkingDirectory: workingDirectory, tabLayouts } = useAppState();
  const gitPanelOpen = useStore((state) => state.gitPanelOpen);
  const contextPanelOpen = useStore((state) => state.contextPanelOpen);
  const fileEditorPanelOpen = useStore((state) => state.fileEditorPanelOpen);
  const sidecarPanelOpen = useStore((state) => state.sidecarPanelOpen);
  const isOnHomeTab = useStore((s) => s.homeTabId !== null && s.activeSessionId === s.homeTabId);
  const chatPanelVisible = useStore((s) => s.chatPanelVisible);
  const uiScale = useStore((s) => s.displaySettings.uiScale);

  const { createTerminalTab } = useCreateTerminalTab();
  const splitDrag = useSplitTabDrag({ setShowMergeDropZone, setSplitDragGhost, closeRightTab });

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
            <SettingsNav activeSection={settingsSection} onSectionChange={setSettingsSection} />
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
        {/* macOS traffic lights + window drag region */}
        <div className="h-[38px] w-full titlebar-drag flex-shrink-0" data-tauri-drag-region />

        {/* Content - floating panels */}
        <div className="flex-1 flex overflow-hidden gap-2 px-2 pb-2 min-h-0 relative">
          {/* Activity Bar - instant hide when viewing the home tab */}
          <div className={cn("flex-shrink-0 overflow-hidden", isOnHomeTab ? "w-0" : "w-[48px]")}>
            <ActivityBar
              activeView={activityView}
              onViewChange={setActivityView}
              terminalOpen={bottomTerminalOpen}
              onToggleTerminal={() => {
                setActivityView(null);
                const s = useStore.getState();
                const currentTabType = s.activeSessionId
                  ? (s.sessions[s.activeSessionId]?.tabType ?? "terminal")
                  : "terminal";
                if (currentTabType !== "terminal") {
                  const termTab = s.tabOrder.find(
                    (id) => (s.sessions[id]?.tabType ?? "terminal") === "terminal"
                  );
                  if (termTab) {
                    s.setActiveSession(termTab);
                    return;
                  }
                }
                useStore.getState().toggleBottomTerminal();
              }}
              onOpenSettings={() => setSettingsOpen(true)}
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
                  <SettingsContent
                    activeSection={settingsSection}
                  />
                </Suspense>
              </div>
            </div>
          )}

          {/* Fullscreen activity view overlays */}
          {FULLSCREEN_OVERLAYS.map(({ view, Component, innerClassName }) =>
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
                <div className={cn("flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float", innerClassName)}>
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
              activityView === "settings" || FULLSCREEN_OVERLAYS.some((o) => o.view === activityView)
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
                    <GitPanel open={gitPanelOpen} onOpenChange={handleGitPanelOpenChange} />
                  </Suspense>
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
          onOpenBrowser={() => setActivityView((v) => (v === "targets" ? null : "targets"))}
          onOpenSecurity={() => setActivityView((v) => (v === "targets" ? null : "targets"))}
          onToggleToolManager={() =>
            setActivityView((v) => (v === "toolManage" ? null : "toolManage"))
          }
          onToggleWiki={() => setActivityView((v) => (v === "wiki" ? null : "wiki"))}
          onToggleBottomTerminal={() => {
            setActivityView(null);
            const s = useStore.getState();
            const currentTabType = s.activeSessionId
              ? (s.sessions[s.activeSessionId]?.tabType ?? "terminal")
              : "terminal";
            if (currentTabType !== "terminal") {
              const termTab = s.tabOrder.find(
                (id) => (s.sessions[id]?.tabType ?? "terminal") === "terminal"
              );
              if (termTab) {
                s.setActiveSession(termTab);
                return;
              }
            }
            useStore.getState().toggleBottomTerminal();
          }}
          onFocusAiChat={() => {
            setActivityView(null);
            requestAnimationFrame(() => {
              document.querySelector<HTMLTextAreaElement>("[data-ai-chat-input]")?.focus();
            });
          }}
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
