import { useCallback, useState } from "react";
import { AppShell, type AppShellProps } from "./App/AppShell";
import { useAppRouting } from "./App/hooks/useAppRouting";
import { useGlobalShortcuts } from "./App/hooks/useGlobalShortcuts";
import { useAppLifecycle } from "./App/hooks/useAppLifecycle";
import { useLayoutManager } from "./App/hooks/useLayoutManager";
import { useCreateTerminalTab } from "./hooks/useCreateTerminalTab";
import { usePaneControls } from "./hooks/usePaneControls";
import { ThemeProvider } from "./hooks/useTheme";
import { notify } from "./lib/notify";
import { clearConversation, restoreSession, useStore } from "./store";
import { useFileEditorSidebarStore } from "./store/file-editor-sidebar";
import { useAppState } from "./store/selectors";

export function App() {
  // Store state (focused selectors only)
  const { activeSessionId } = useAppState();
  const setInputMode = useStore((state) => state.setInputMode);
  const setRenderMode = useStore((state) => state.setRenderMode);
  const openSettingsTab = useStore((state) => state.openSettingsTab);

  // Panel state from store (replaces local useState)
  const gitPanelOpen = useStore((state) => state.gitPanelOpen);
  const sessionBrowserOpen = useStore((state) => state.sessionBrowserOpen);
  const openGitPanel = useStore((state) => state.openGitPanel);
  const openContextPanel = useStore((state) => state.openContextPanel);
  const toggleFileEditorPanel = useStore((state) => state.toggleFileEditorPanel);
  const closePanels = useStore((state) => state.closePanels);
  const setSessionBrowserOpen = useStore((state) => state.setSessionBrowserOpen);

  const { createTerminalTab } = useCreateTerminalTab();
  const { handleSplitPane, handleClosePane, handleNavigatePane } = usePaneControls(activeSessionId);

  // Local UI dialog state
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [quickOpenDialogOpen, setQuickOpenDialogOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsSection, setSettingsSection] = useState("environment");
  const [bottomTerminalOpen, setBottomTerminalOpen] = useState(true);
  const [shortcutsHelpOpen, setShortcutsHelpOpen] = useState(false);
  const [recordingsPanelOpen, setRecordingsPanelOpen] = useState(false);

  // Right split column state + handlers
  const rightSplit = useLayoutManager();

  // Routing state (activity view + page route)
  const { currentPage, setCurrentPage, activityView, setActivityView, visitedViews } =
    useAppRouting();

  // Lifecycle / startup side effects + isLoading / error flags
  const { isLoading, error } = useAppLifecycle({
    setRightPanelTabs: rightSplit.setRightPanelTabs,
    setRightActiveTab: rightSplit.setRightActiveTab,
    setShowSplitDropZone: rightSplit.setShowSplitDropZone,
  });

  // Handle toggle mode from command palette (cycles: terminal → agent → auto → terminal)
  const handleToggleMode = useCallback(() => {
    if (activeSessionId) {
      const currentSession = useStore.getState().sessions[activeSessionId];
      const current = currentSession?.inputMode ?? "terminal";
      const newMode = current === "terminal" ? "agent" : current === "agent" ? "auto" : "terminal";
      setInputMode(activeSessionId, newMode);
    }
  }, [activeSessionId, setInputMode]);

  const handleNewTab = useCallback(() => createTerminalTab(), [createTerminalTab]);

  // Wire global keyboard shortcuts (refs pattern, listener installed once)
  useGlobalShortcuts({
    gitPanelOpen,
    handleNewTab,
    handleToggleMode,
    openContextPanel,
    openGitPanel,
    toggleFileEditorPanel,
    openSettingsTab,
    handleSplitPane,
    handleClosePane,
    handleNavigatePane,
    openBrowserTab: () => setActivityView((v) => (v === "targets" ? null : "targets")),
    openSecurityTab: () => setActivityView((v) => (v === "targets" ? null : "targets")),
    toggleToolManager: () => setActivityView((v) => (v === "toolManage" ? null : "toolManage")),
    toggleWiki: () => setActivityView((v) => (v === "wiki" ? null : "wiki")),
    toggleBottomTerminal: () => {
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
    },
    focusAiChat: () => {
      setActivityView(null);
      requestAnimationFrame(() => {
        document.querySelector<HTMLTextAreaElement>("[data-ai-chat-input]")?.focus();
      });
    },
    setCommandPaletteOpen,
    setQuickOpenDialogOpen,
    setSidecarPanelOpen: (open) => (open ? useStore.getState().openSidecarPanel() : closePanels()),
    setShortcutsHelpOpen,
  });

  const handleClearConversation = useCallback(async () => {
    if (activeSessionId) {
      await clearConversation(activeSessionId);
      notify.success("Conversation cleared");
    }
  }, [activeSessionId]);

  const handleToggleFullTerminal = useCallback(() => {
    if (activeSessionId) {
      const currentRenderMode =
        useStore.getState().sessions[activeSessionId]?.renderMode ?? "timeline";
      setRenderMode(activeSessionId, currentRenderMode === "fullterm" ? "timeline" : "fullterm");
    }
  }, [activeSessionId, setRenderMode]);

  const handleRestoreSession = useCallback(
    async (identifier: string) => {
      if (!activeSessionId) return notify.error("No active session to restore into");
      try {
        await restoreSession(activeSessionId, identifier);
        notify.success("Session restored");
      } catch (e) {
        notify.error(`Failed to restore session: ${e}`);
      }
    },
    [activeSessionId]
  );

  const handleOpenHistory = useCallback(() => setSessionBrowserOpen(true), [setSessionBrowserOpen]);

  // Panel onOpenChange callbacks (open via specific opener, close via shared closePanels)
  const handleGitPanelOpenChange = useCallback(
    (open: boolean) => (open ? openGitPanel() : closePanels()),
    [openGitPanel, closePanels]
  );
  const handleContextPanelOpenChange = useCallback(
    (open: boolean) => (open ? openContextPanel() : closePanels()),
    [openContextPanel, closePanels]
  );
  const handleFileEditorPanelOpenChange = useCallback(
    (open: boolean) => {
      if (open) useStore.getState().openFileEditorPanel();
      else {
        closePanels();
        useFileEditorSidebarStore.getState().setOpen(false);
      }
    },
    [closePanels]
  );
  const handleSidecarPanelOpenChange = useCallback(
    (open: boolean) => (open ? useStore.getState().openSidecarPanel() : closePanels()),
    [closePanels]
  );

  const shellProps: AppShellProps = {
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
    rightPanelTabs: rightSplit.rightPanelTabs,
    rightActiveTab: rightSplit.rightActiveTab,
    setRightActiveTab: rightSplit.setRightActiveTab,
    rightPanelWidth: rightSplit.rightPanelWidth,
    showSplitDropZone: rightSplit.showSplitDropZone,
    setShowMergeDropZone: rightSplit.setShowMergeDropZone,
    splitDragGhost: rightSplit.splitDragGhost,
    setSplitDragGhost: rightSplit.setSplitDragGhost,
    splitDragRef: rightSplit.splitDragRef,
    closeRightTab: rightSplit.closeRightTab,
    handlePanelResizeStart: rightSplit.handlePanelResizeStart,
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
  };

  return <AppShell {...shellProps} />;
}

function AppWithTheme() {
  return (
    <ThemeProvider defaultThemeId="golish">
      <App />
    </ThemeProvider>
  );
}

export default AppWithTheme;
