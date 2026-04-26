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
import { Skeleton } from "../components/ui/skeleton";
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
import type { ActivityView } from "../components/ActivityBar/ActivityBar";

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

interface SplitDragRef {
  startX: number;
  startY: number;
  dragging: boolean;
  tabId: string | null;
}

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
  setBottomTerminalOpen: React.Dispatch<React.SetStateAction<boolean>>;

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
  splitDragRef: React.MutableRefObject<SplitDragRef>;
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
    setBottomTerminalOpen,
    rightPanelTabs,
    rightActiveTab,
    setRightActiveTab,
    rightPanelWidth,
    showSplitDropZone,
    setShowMergeDropZone,
    splitDragGhost,
    setSplitDragGhost,
    splitDragRef,
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

  const hasSplit = rightPanelTabs.length > 0;

  if (isLoading) {
    return (
      <div className="h-screen w-screen bg-background flex overflow-hidden">
        {/* Skeleton Activity Bar */}
        <div className="w-[48px] flex-shrink-0 bg-background border-r border-[var(--border-subtle)] flex flex-col items-center gap-2 pt-12">
          <Skeleton className="h-8 w-8 rounded-md bg-muted" />
          <Skeleton className="h-8 w-8 rounded-md bg-muted" />
          <Skeleton className="h-8 w-8 rounded-md bg-muted" />
        </div>

        {/* Skeleton left panel */}
        <div className="w-[220px] flex-shrink-0 bg-card border-r border-[var(--border-subtle)] p-3 space-y-3">
          <Skeleton className="h-6 w-16 bg-muted" />
          <Skeleton className="h-7 w-full bg-muted" />
          <Skeleton className="h-4 w-20 bg-muted" />
          <Skeleton className="h-4 w-24 bg-muted" />
        </div>

        {/* Skeleton center */}
        <div className="flex-1 flex flex-col">
          <div className="flex items-center h-[34px] bg-card border-b border-[var(--border-subtle)] pl-2 pr-2 gap-2">
            <Skeleton className="h-5 w-20 bg-muted" />
            <Skeleton className="h-5 w-5 rounded bg-muted" />
          </div>
          <div className="flex-1 p-4 space-y-3">
            <Skeleton className="h-16 w-full bg-muted" />
            <Skeleton className="h-16 w-3/4 bg-muted" />
          </div>
        </div>

        {/* Skeleton right panel */}
        <div className="w-[340px] flex-shrink-0 bg-card border-l border-[var(--border-subtle)] p-3 space-y-3">
          <Skeleton className="h-6 w-16 bg-muted" />
          <Skeleton className="h-20 w-full bg-muted" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center h-screen bg-background">
        <div className="text-[#f7768e] text-lg">Error: {error}</div>
      </div>
    );
  }

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
                setBottomTerminalOpen((v) => !v);
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
                    onSectionChange={setSettingsSection}
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

              {/* Right split column */}
              {hasSplit && (
                <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float animate-in slide-in-from-right-4 fade-in duration-300">
                  <div className="h-[34px] flex items-center border-b border-border/30 flex-shrink-0 overflow-x-auto">
                    {rightPanelTabs.map((rTabId) => {
                      const rSession = useStore.getState().sessions[rTabId];
                      const rName =
                        rSession?.customName ||
                        rSession?.processName ||
                        rSession?.workingDirectory?.split(/[/\\]/).pop() ||
                        "Terminal";
                      const isRightActive = rTabId === rightActiveTab;
                      return (
                        <div
                          key={rTabId}
                          className={cn(
                            "flex items-center gap-1.5 px-3 py-1 h-full relative cursor-grab active:cursor-grabbing select-none text-[11px] font-mono transition-colors",
                            isRightActive
                              ? "bg-muted text-foreground"
                              : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
                          )}
                          onClick={() => setRightActiveTab(rTabId)}
                          onPointerDown={(e) => {
                            if ((e.target as HTMLElement).closest("button")) return;
                            e.preventDefault();
                            splitDragRef.current = {
                              startX: e.clientX,
                              startY: e.clientY,
                              dragging: false,
                              tabId: rTabId,
                            };
                            (e.target as HTMLElement).setPointerCapture(e.pointerId);
                          }}
                          onPointerMove={(e) => {
                            const ref = splitDragRef.current;
                            if (!ref.startX && !ref.startY) return;
                            const dx = e.clientX - ref.startX;
                            const dist = Math.sqrt(dx * dx + (e.clientY - ref.startY) ** 2);
                            if (!ref.dragging && dist > 8) {
                              ref.dragging = true;
                              document.documentElement.classList.add("tab-dragging");
                            }
                            if (ref.dragging) {
                              setShowMergeDropZone(dx < -40);
                              setSplitDragGhost({ x: e.clientX, y: e.clientY, name: rName });
                            }
                          }}
                          onPointerUp={(e) => {
                            const ref = splitDragRef.current;
                            if (ref.dragging && e.clientX - ref.startX < -40) closeRightTab(rTabId);
                            splitDragRef.current = {
                              startX: 0,
                              startY: 0,
                              dragging: false,
                              tabId: null,
                            };
                            setShowMergeDropZone(false);
                            setSplitDragGhost(null);
                            document.documentElement.classList.remove("tab-dragging");
                          }}
                        >
                          {isRightActive && (
                            <span className="absolute bottom-0 left-0 right-0 h-px bg-accent" />
                          )}
                          <span className="truncate max-w-[140px]">{rName}</span>
                          <button
                            className="p-0.5 rounded hover:bg-background/50 text-muted-foreground hover:text-foreground transition-colors flex-shrink-0"
                            onClick={(e) => {
                              e.stopPropagation();
                              closeRightTab(rTabId);
                            }}
                            title="Close split"
                            onPointerDown={(e) => e.stopPropagation()}
                          >
                            <svg
                              width="8"
                              height="8"
                              viewBox="0 0 10 10"
                              fill="none"
                              stroke="currentColor"
                              strokeWidth="1.5"
                              strokeLinecap="round"
                            >
                              <line x1="2" y1="2" x2="8" y2="8" />
                              <line x1="8" y1="2" x2="2" y2="8" />
                            </svg>
                          </button>
                        </div>
                      );
                    })}
                  </div>
                  <div className="flex-1 min-h-0 min-w-0 relative">
                    {rightPanelTabs.map((rTabId) => {
                      const layout = tabLayouts.find((l) => l.tabId === rTabId);
                      if (!layout) return null;
                      return (
                        <div
                          key={rTabId}
                          className={`absolute inset-0 ${rTabId === rightActiveTab ? "visible" : "invisible pointer-events-none"}`}
                        >
                          <PaneContainer node={layout.root} tabId={rTabId} />
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Split drop zone indicator */}
              {showSplitDropZone && (
                <div className="absolute inset-0 z-20 pointer-events-none flex animate-in fade-in duration-200">
                  <div className="flex-1" />
                  <div className="w-1/2 m-2 rounded-xl border-2 border-dashed border-accent/60 bg-accent/8 flex items-center justify-center backdrop-blur-[2px] animate-in slide-in-from-right-2 duration-300">
                    <div className="flex flex-col items-center gap-1.5 animate-pulse">
                      <svg
                        width="28"
                        height="28"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="1.5"
                        strokeLinecap="round"
                        className="text-accent/60"
                      >
                        <rect x="3" y="3" width="18" height="18" rx="2" />
                        <line x1="12" y1="3" x2="12" y2="21" />
                      </svg>
                      <span className="text-xs text-accent/60 font-medium">Split Right</span>
                    </div>
                  </div>
                </div>
              )}
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
            setBottomTerminalOpen((v) => !v);
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
