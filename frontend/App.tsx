import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { cn } from "@/lib/utils";
import { getProjectPath } from "@/lib/projects";
import { logger } from "@/lib/logger";
import { ActivityBar, type ActivityView } from "./components/ActivityBar/ActivityBar";
import { CommandPalette, type PageRoute } from "./components/CommandPalette";
import { PaneContainer } from "./components/PaneContainer";
import { AIChatPanel } from "./components/AIChatPanel/AIChatPanel";
import { SidecarNotifications } from "./components/Sidecar";
import { TabBar } from "./components/TabBar";
import { TerminalLayer } from "./components/Terminal";
import { Skeleton } from "./components/ui/skeleton";
import {
  createKeyboardHandler,
  useKeyboardHandlerContext,
} from "./hooks/useKeyboardHandlerContext";
import { usePaneControls } from "./hooks/usePaneControls";

// Lazy loaded components - these are not needed on initial render
// and can be loaded on-demand to reduce initial bundle size
const FileEditorSidebarPanel = lazy(() =>
  import("./components/FileEditorSidebar").then((m) => ({
    default: m.FileEditorSidebarPanel,
  }))
);
const GitPanel = lazy(() => import("./components/GitPanel").then((m) => ({ default: m.GitPanel })));
const SessionBrowser = lazy(() =>
  import("./components/SessionBrowser/SessionBrowser").then((m) => ({
    default: m.SessionBrowser,
  }))
);
const SettingsDialog = lazy(() =>
  import("./components/Settings").then((m) => ({ default: m.SettingsDialog }))
);
const SettingsNav = lazy(() =>
  import("./components/Settings").then((m) => ({ default: m.SettingsNav }))
);
const SettingsContent = lazy(() =>
  import("./components/Settings").then((m) => ({ default: m.SettingsContent }))
);
const ToolManagerView = lazy(() =>
  import("./components/ToolManager/ToolManager").then((m) => ({
    default: m.ToolManager,
  }))
);
const WikiPanelView = lazy(() =>
  import("./components/WikiPanel/WikiPanel").then((m) => ({
    default: m.WikiPanel,
  }))
);
const TargetPanelView = lazy(() =>
  import("./components/TargetPanel/TargetPanel").then((m) => ({
    default: m.TargetPanel,
  }))
);
const TopologyPanelView = lazy(() =>
  import("./components/TopologyView/TopologyView").then((m) => ({
    default: m.TopologyView,
  }))
);
const MethodologyPanelView = lazy(() =>
  import("./components/MethodologyPanel/MethodologyPanel").then((m) => ({
    default: m.MethodologyPanel,
  }))
);
const DashboardPanelView = lazy(() =>
  import("./components/DashboardPanel/DashboardPanel").then((m) => ({
    default: m.DashboardPanel,
  }))
);
const FindingsPanelView = lazy(() =>
  import("./components/FindingsPanel/FindingsPanel").then((m) => ({
    default: m.FindingsPanel,
  }))
);
const PipelinePanelView = lazy(() =>
  import("./components/PipelinePanel/PipelinePanel").then((m) => ({
    default: m.PipelinePanel,
  }))
);
const AuditLogPanelView = lazy(() =>
  import("./components/AuditLogPanel/AuditLogPanel").then((m) => ({
    default: m.AuditLogPanel,
  }))
);
const WordlistPanelView = lazy(() =>
  import("./components/WordlistPanel/WordlistPanel").then((m) => ({
    default: m.WordlistPanel,
  }))
);
const VulnIntelPanelView = lazy(() =>
  import("./components/VulnIntelPanel/VulnIntelPanel").then((m) => ({
    default: m.VulnIntelPanel,
  }))
);
const RecordingsPanelView = lazy(() =>
  import("./components/Terminal/RecordingsPanel").then((m) => ({
    default: m.RecordingsPanel,
  }))
);
const ContextPanel = lazy(() =>
  import("./components/Sidecar/ContextPanel").then((m) => ({
    default: m.ContextPanel,
  }))
);
const SidecarPanel = lazy(() =>
  import("./components/Sidecar/SidecarPanel").then((m) => ({
    default: m.SidecarPanel,
  }))
);
const ComponentTestbed = lazy(() =>
  import("./pages/ComponentTestbed").then((m) => ({
    default: m.ComponentTestbed,
  }))
);
const QuickOpenDialog = lazy(() =>
  import("./components/QuickOpenDialog").then((m) => ({
    default: m.QuickOpenDialog,
  }))
);
const KeyboardShortcutsHelp = lazy(() =>
  import("./components/KeyboardShortcutsHelp/KeyboardShortcutsHelp").then((m) => ({
    default: m.KeyboardShortcutsHelp,
  }))
);

import { useAiEvents } from "./hooks/useAiEvents";
import { useCreateTerminalTab } from "./hooks/useCreateTerminalTab";
import { useTauriEvents } from "./hooks/useTauriEvents";
import { TerminalPortalProvider } from "./hooks/useTerminalPortal";
import { ThemeProvider } from "./hooks/useTheme";
import { notify } from "./lib/notify";
import { updateConfig as updatePentestConfig } from "./lib/pentest/api";
import { getSettings } from "./lib/settings";
import { initSystemNotifications, listenForSettingsUpdates } from "./lib/systemNotifications";
import { shellIntegrationInstall, shellIntegrationStatus } from "./lib/tauri";
import { clearConversation, restoreSession, useStore } from "./store";
import { createNewConversation } from "./store/slices/conversation";
import {
  createWorkspaceAutoSaver,
  getLastProjectName,
  loadWorkspaceState,
  toChatConversation,
} from "./lib/workspace-storage";
import { useFileEditorSidebarStore } from "./store/file-editor-sidebar";
import { useAppState } from "./store/selectors";

function App() {
  // Get store state using optimized selectors that only subscribe to needed data
  const { activeSessionId, focusedWorkingDirectory: workingDirectory, tabLayouts } = useAppState();

  // Get stable action references (actions are stable by design in Zustand)
  const setInputMode = useStore((state) => state.setInputMode);
  const setRenderMode = useStore((state) => state.setRenderMode);
  const openSettingsTab = useStore((state) => state.openSettingsTab);
  const openHomeTab = useStore((state) => state.openHomeTab);
  const openBrowserTab = useStore((state) => state.openBrowserTab);
  const openSecurityTab = useStore((state) => state.openSecurityTab);

  // Panel state from store (replaces local useState)
  const gitPanelOpen = useStore((state) => state.gitPanelOpen);
  const contextPanelOpen = useStore((state) => state.contextPanelOpen);
  const fileEditorPanelOpen = useStore((state) => state.fileEditorPanelOpen);
  const sidecarPanelOpen = useStore((state) => state.sidecarPanelOpen);
  const sessionBrowserOpen = useStore((state) => state.sessionBrowserOpen);
  const openGitPanel = useStore((state) => state.openGitPanel);
  const openContextPanel = useStore((state) => state.openContextPanel);
  const toggleFileEditorPanel = useStore((state) => state.toggleFileEditorPanel);
  const closePanels = useStore((state) => state.closePanels);
  const setSessionBrowserOpen = useStore((state) => state.setSessionBrowserOpen);

  const { createTerminalTab } = useCreateTerminalTab();
  const { handleSplitPane, handleClosePane, handleNavigatePane } = usePaneControls(activeSessionId);

  const hasProject = useStore((s) => s.currentProjectName !== null);
  const isOnHomeTab = useStore((s) => s.homeTabId !== null && s.activeSessionId === s.homeTabId);
  const hasBrowserTab = useStore((s) => Object.values(s.sessions).some((sess) => sess.tabType === "browser"));
  const hasSecurityTab = useStore((s) => Object.values(s.sessions).some((sess) => sess.tabType === "security"));
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [quickOpenDialogOpen, setQuickOpenDialogOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsSection, setSettingsSection] = useState("environment");
  const [currentPage, setCurrentPage] = useState<PageRoute>("main");
  const [activityView, setActivityView] = useState<ActivityView>(null);
  const [bottomTerminalOpen, setBottomTerminalOpen] = useState(true);
  const [shortcutsHelpOpen, setShortcutsHelpOpen] = useState(false);
  const [recordingsPanelOpen, setRecordingsPanelOpen] = useState(false);
  const [rightPanelTabs, setRightPanelTabs] = useState<string[]>([]);
  const [rightActiveTab, setRightActiveTab] = useState<string | null>(null);
  const [showSplitDropZone, setShowSplitDropZone] = useState(false);
  const [showMergeDropZone, setShowMergeDropZone] = useState(false);
  const [splitDragGhost, setSplitDragGhost] = useState<{ x: number; y: number; name: string } | null>(null);
  const splitDragRef = useRef<{ startX: number; startY: number; dragging: boolean; tabId: string | null }>({ startX: 0, startY: 0, dragging: false, tabId: null });
  const splitTabId = rightActiveTab;
  const hasSplit = rightPanelTabs.length > 0;

  const closeRightTab = useCallback((tabId?: string) => {
    const target = tabId || rightActiveTab;
    if (!target) return;
    setRightPanelTabs((prev) => {
      const next = prev.filter((id) => id !== target);
      if (next.length === 0) {
        setRightActiveTab(null);
      } else if (rightActiveTab === target) {
        setRightActiveTab(next[next.length - 1]);
      }
      return next;
    });
  }, [rightActiveTab]);

  // Subscribe to file editor sidebar store to sync open state
  // This allows openFile() calls from anywhere to open the sidebar
  const fileEditorStoreOpen = useFileEditorSidebarStore((state) => state.open);
  useEffect(() => {
    if (fileEditorStoreOpen && !fileEditorPanelOpen) {
      useStore.getState().openFileEditorPanel();
    }
  }, [fileEditorStoreOpen, fileEditorPanelOpen]);

  // Connect Tauri events to store
  useTauriEvents();

  // Subscribe to AI events for agent mode
  useAiEvents();

  // Handle toggle mode from command palette (cycles: terminal → agent → auto → terminal)
  const handleToggleMode = useCallback(() => {
    if (activeSessionId) {
      const currentSession = useStore.getState().sessions[activeSessionId];
      const current = currentSession?.inputMode ?? "terminal";
      const newMode = current === "terminal" ? "agent" : current === "agent" ? "auto" : "terminal";
      setInputMode(activeSessionId, newMode);
    }
  }, [activeSessionId, setInputMode]);

  // Create a new terminal tab
  const handleNewTab = useCallback(() => {
    createTerminalTab();
  }, [createTerminalTab]);

  useEffect(() => {
    async function init() {
      try {
        const currentSessions = useStore.getState().sessions;
        if (Object.keys(currentSessions).length > 0) {
          logger.info("[App] Sessions already exist, skipping initialization...");
          setIsLoading(false);
          return;
        }

        logger.info("[App] Starting initialization...");

        // #region agent log
        fetch('http://127.0.0.1:7743/ingest/c2581ce7-f104-4330-b1e0-a5f012cf928e',{method:'POST',headers:{'Content-Type':'application/json','X-Debug-Session-Id':'4e8d76'},body:JSON.stringify({sessionId:'4e8d76',location:'App.tsx:init',message:'App init start - checking localStorage state',data:{sessionsCount:Object.keys(useStore.getState().sessions).length,convTerminals:localStorage.getItem('golish-pentest-conv-terminals')?.slice(0,500),convData:localStorage.getItem('golish-pentest-conversations')?.slice(0,200),lastProject:localStorage.getItem('golish-last-project')},timestamp:Date.now(),hypothesisId:'A,C'})}).catch(()=>{});
        // #endregion

        // Create home tab first (always visible, leftmost)
        openHomeTab();

        // Check and install shell integration in the background (non-blocking)
        void (async () => {
          try {
            const status = await shellIntegrationStatus();
            if (status.type === "NotInstalled") {
              notify.info("Installing shell integration...");
              await shellIntegrationInstall();
              notify.success("Shell integration installed! Restart your shell for full features.");
            } else if (status.type === "Outdated") {
              notify.info("Updating shell integration...");
              await shellIntegrationInstall();
              notify.success("Shell integration updated!");
            }
          } catch (e) {
            logger.warn("Shell integration check failed:", e);
          }
        })();

        // Try to restore last project's workspace state
        const lastProject = getLastProjectName();
        if (lastProject) {
          logger.info("[App] Restoring project:", lastProject);
          const { getProjectConfig } = await import("./lib/projects");
          const config = await getProjectConfig(lastProject);

          if (config) {
            useStore.getState().setCurrentProject(lastProject, config.rootPath);

            // Restore conversations
            const saved = await loadWorkspaceState(lastProject);
            // #region agent log
            fetch('http://127.0.0.1:7743/ingest/c2581ce7-f104-4330-b1e0-a5f012cf928e',{method:'POST',headers:{'Content-Type':'application/json','X-Debug-Session-Id':'4e8d76'},body:JSON.stringify({sessionId:'4e8d76',location:'App.tsx:restore',message:'loadWorkspaceState result',data:{hasSaved:!!saved,convCount:saved?.conversations?.length??0,termTabCount:saved?.terminalTabs?.length??0,activeConvId:saved?.activeConversationId,hasTermData:!!saved?.conversationTerminalData,hasAiModel:saved?.aiModel!==undefined,hasApprovalMode:!!saved?.approvalMode},timestamp:Date.now(),hypothesisId:'C'})}).catch(()=>{});
            // #endregion

            // Hydrate localStorage from workspace.json (v2 data)
            if (saved?.conversationTerminalData) {
              localStorage.setItem("golish-pentest-conv-terminals", JSON.stringify(saved.conversationTerminalData));
            }
            if (saved?.aiModel !== undefined) {
              localStorage.setItem("golish-pentest-ai-model", JSON.stringify(saved.aiModel));
            }
            if (saved?.approvalMode) {
              localStorage.setItem("golish-approval-mode", saved.approvalMode);
            }

            if (saved && saved.conversations.length > 0) {
              const restoredConvs = saved.conversations.map(toChatConversation);
              useStore.getState().restoreConversations(
                restoredConvs,
                saved.conversationOrder,
                saved.activeConversationId,
              );
              // #region agent log
              fetch('http://127.0.0.1:7743/ingest/c2581ce7-f104-4330-b1e0-a5f012cf928e',{method:'POST',headers:{'Content-Type':'application/json','X-Debug-Session-Id':'4e8d76'},body:JSON.stringify({sessionId:'4e8d76',location:'App.tsx:afterRestore',message:'After restoreConversations',data:{convIds:Object.keys(useStore.getState().conversations),convTerminals:JSON.stringify(useStore.getState().conversationTerminals).slice(0,300),lsTermData:localStorage.getItem('golish-pentest-conv-terminals')?.slice(0,300)},timestamp:Date.now(),hypothesisId:'E'})}).catch(()=>{});
              // #endregion
            } else {
              const initialConv = createNewConversation();
              useStore.getState().addConversation(initialConv);
            }

            // Terminal tabs are restored per-conversation in AIChatPanel after localStorage merge.
            // Only skip default terminal creation if saved data matches actual conversations.
            const hasConvTerminals = (() => {
              try {
                const raw = localStorage.getItem("golish-pentest-conv-terminals");
                if (!raw) return false;
                const data = JSON.parse(raw) as Record<string, unknown>;
                const convIds = Object.keys(useStore.getState().conversations);
                return convIds.some((id) => id in data);
              } catch { return false; }
            })();

            if (!hasConvTerminals) {
              const wd = saved?.terminalTabs?.[0]?.workingDirectory ?? config.rootPath;
              const termId = await createTerminalTab(wd, true);
              if (termId) {
                const activeConv = useStore.getState().activeConversationId;
                if (activeConv) {
                  useStore.getState().addTerminalToConversation(activeConv, termId);
                }
                useStore.getState().setActiveSession(termId);
              }
            }
          }
        }

        // Only show home tab if no project was restored (no terminals to show)
        if (!useStore.getState().currentProjectName) {
          const homeId = useStore.getState().homeTabId;
          if (homeId) {
            useStore.getState().setActiveSession(homeId);
          }
        }

        setIsLoading(false);
      } catch (e) {
        logger.error("Failed to initialize:", e);
        setError(e instanceof Error ? e.message : String(e));
        setIsLoading(false);
      }
    }

    init();
  }, [openHomeTab, createTerminalTab]);

  // Auto-save workspace state (conversations, chat history, terminal tabs) on changes + window close
  useEffect(() => {
    return createWorkspaceAutoSaver(
      () => useStore.getState().currentProjectName,
      (listener) => useStore.subscribe(listener),
      () => {
        const s = useStore.getState();
        const seen = new Set<string>();
        const terminalTabs = Object.values(s.sessions)
          .filter((sess) => (sess.tabType ?? "terminal") === "terminal")
          .filter((sess) => {
            if (seen.has(sess.workingDirectory)) return false;
            seen.add(sess.workingDirectory);
            return true;
          })
          .map((sess) => ({ workingDirectory: sess.workingDirectory }));
        return {
          conversations: s.conversations,
          conversationOrder: s.conversationOrder,
          activeConversationId: s.activeConversationId,
          terminalTabs,
          conversationTerminals: s.conversationTerminals,
          sessions: s.sessions,
          timelines: s.timelines,
        };
      },
    );
  }, []);

  // Sync frontend network settings (github_token, proxy_url) to backend pentest config on startup.
  // The backend pentest config is in-memory only, so it resets on every restart.
  useEffect(() => {
    getSettings()
      .then((s) => {
        const { proxy_url, github_token } = s.network;
        if (proxy_url || github_token) {
          updatePentestConfig({
            proxy_url: proxy_url || "",
            github_token: github_token || "",
          }).catch((e) => logger.error("[App] pentest config startup sync failed:", e));
        }
      })
      .catch(() => {});
  }, []);

  // Initialize system notifications and app focus/visibility tracking
  useEffect(() => {
    const { setAppIsFocused, setAppIsVisible } = useStore.getState();

    // Initialize notification system with store API
    initSystemNotifications(useStore).catch((error) => {
      logger.error("Failed to initialize system notifications:", error);
    });

    // Listen for settings updates to reactively update notification state
    const unlistenSettings = listenForSettingsUpdates();

    // Track window focus state
    const handleFocus = () => setAppIsFocused(true);
    const handleBlur = () => setAppIsFocused(false);

    // Track document visibility state
    const handleVisibilityChange = () => {
      setAppIsVisible(document.visibilityState === "visible");
    };

    window.addEventListener("focus", handleFocus);
    window.addEventListener("blur", handleBlur);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    // Set initial state
    setAppIsFocused(document.hasFocus());
    setAppIsVisible(document.visibilityState === "visible");

    // Close any stale browser webview on app startup (e.g., after HMR/refresh)
    invoke("pentest_browser_close").catch(() => {});
    const handleBrowserCleanup = () => {
      invoke("pentest_browser_close").catch(() => {});
    };
    window.addEventListener("beforeunload", handleBrowserCleanup);

    return () => {
      unlistenSettings();
      window.removeEventListener("focus", handleFocus);
      window.removeEventListener("blur", handleBlur);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.removeEventListener("beforeunload", handleBrowserCleanup);
    };
  }, []);

  // Handle tab split events from TabBar
  useEffect(() => {
    const handleSplitTab = (e: Event) => {
      const tabId = (e as CustomEvent<string>).detail;
      setRightPanelTabs((prev) => {
        if (prev.includes(tabId)) return prev;
        const s = useStore.getState();
        if (s.activeSessionId === tabId) {
          const other = s.tabOrder.find((id) => id !== tabId && !prev.includes(id) && (s.sessions[id]?.tabType ?? "terminal") !== "home");
          if (other) s.setActiveSession(other);
        }
        return [...prev, tabId];
      });
      setRightActiveTab(tabId);
      setShowSplitDropZone(false);
    };
    const handleUnsplitTab = () => { setRightPanelTabs([]); setRightActiveTab(null); };
    const handleDragHint = (e: Event) => setShowSplitDropZone((e as CustomEvent<boolean>).detail);
    const handleToolOutput = async (e: Event) => {
      if (localStorage.getItem("golish-auto-detect-output") === "false") return;
      const { command, output } = (e as CustomEvent<{ command: string; output: string; sessionId: string }>).detail;
      try {
        const detected = await invoke<{ tool_id: string; tool_name: string; output_config: { format: string; produces: string[]; patterns: unknown[]; fields: Record<string, string>; detect?: string } } | null>(
          "output_detect_tool", { command, rawOutput: output }
        );
        if (!detected) return;
        const parsed = await invoke<{ items: { data_type: string; fields: Record<string, string> }[] }>(
          "output_parse", { rawOutput: output, config: detected.output_config, toolId: detected.tool_id, toolName: detected.tool_name }
        );
        if (!parsed.items.length) return;
        const pp = getProjectPath();
        const produces = detected.output_config.produces;

        if (produces.includes("host") || produces.includes("port")) {
          const data = await invoke<{ nodes: unknown[]; edges: unknown[] }>("topo_parse", { rawOutput: output, projectPath: pp });
          if (data.nodes.length > 0) {
            notify.success(`${detected.tool_name}: ${data.nodes.length} hosts → Topology`);
            await invoke("topo_save", { name: `${detected.tool_name}-${Date.now()}`, data, projectPath: pp });
          }
        }

        if (produces.includes("vulnerability")) {
          const vulnItems = parsed.items
            .filter((it) => it.data_type === "vulnerability")
            .map((it) => it.fields);
          if (vulnItems.length > 0) {
            const added = await invoke<number>("findings_import_parsed", {
              items: vulnItems, toolName: detected.tool_name, projectPath: pp,
            });
            if (added > 0) {
              notify.success(`${detected.tool_name}: ${added} findings imported`);
            }
          }
        }
      } catch {
        /* ignore */
      }
    };
    const handleDetachTab = async (e: Event) => {
      const { tabId, screenX, screenY } = (e as CustomEvent<{ tabId: string; screenX: number; screenY: number }>).detail;
      const s = useStore.getState();
      const session = s.sessions[tabId];
      if (!session) return;
      const tabType = session.tabType ?? "terminal";
      if (tabType !== "terminal") return;

      const title = session.customName
        || session.processName
        || session.workingDirectory?.split(/[/\\]/).pop()
        || "Terminal";

      try {
        await invoke("create_detached_window", {
          sessionId: tabId,
          tabType,
          title: `${title} — Detached`,
          x: screenX - 50,
          y: screenY - 20,
          width: 800.0,
          height: 500.0,
        });
        // Track as detached, switch to another tab
        const detached = JSON.parse(localStorage.getItem("golish-detached-tabs") || "{}");
        detached[tabId] = { title, tabType };
        localStorage.setItem("golish-detached-tabs", JSON.stringify(detached));

        // Switch active session to another available tab
        const other = s.tabOrder.find((id) => id !== tabId && (s.sessions[id]?.tabType ?? "terminal") !== "home");
        if (other) s.setActiveSession(other);
        notify.info(`"${title}" detached to floating window`);
      } catch (err) {
        logger.error("[App] detach tab failed:", err);
      }
    };

    window.addEventListener("split-tab-right", handleSplitTab);
    window.addEventListener("unsplit-tab", handleUnsplitTab);
    window.addEventListener("tab-drag-split-hint", handleDragHint);
    window.addEventListener("detach-tab", handleDetachTab);
    const handleRecordingSaved = () => {
      notify.success("Terminal recording saved");
    };
    window.addEventListener("tool-output-completed", handleToolOutput);
    window.addEventListener("recording-saved", handleRecordingSaved);

    // Listen for detached window close events from Tauri
    let unlistenDetachedClose: (() => void) | null = null;
    listen<{ session_id: string }>("detached-window-closed", (event) => {
      const { session_id } = event.payload;
      const detached = JSON.parse(localStorage.getItem("golish-detached-tabs") || "{}");
      delete detached[session_id];
      localStorage.setItem("golish-detached-tabs", JSON.stringify(detached));
      notify.info("Detached window closed — tab restored");
    }).then((fn) => { unlistenDetachedClose = fn; });

    return () => {
      window.removeEventListener("split-tab-right", handleSplitTab);
      window.removeEventListener("unsplit-tab", handleUnsplitTab);
      window.removeEventListener("tab-drag-split-hint", handleDragHint);
      window.removeEventListener("detach-tab", handleDetachTab);
      window.removeEventListener("tool-output-completed", handleToolOutput);
      window.removeEventListener("recording-saved", handleRecordingSaved);
      unlistenDetachedClose?.();
    };
  }, []);

  // Allow child components (e.g. TargetPanel) to close the activity overlay
  useEffect(() => {
    const handler = () => setActivityView(null);
    window.addEventListener("close-activity-view", handler);
    return () => window.removeEventListener("close-activity-view", handler);
  }, []);

  // Handle native menu events from Tauri backend
  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    (async () => {
      unlisteners.push(await listen("menu-open-project", () => {
        openHomeTab();
      }));
      unlisteners.push(await listen("menu-new-project", () => {
        openHomeTab();
      }));
      unlisteners.push(await listen("menu-settings", () => {
        openSettingsTab();
      }));
    })();
    return () => { unlisteners.forEach((fn) => fn()); };
  }, [openHomeTab, openSettingsTab]);

  // Keyboard shortcuts using refs pattern to avoid recreating the handler on every state change
  const keyboardContextRef = useKeyboardHandlerContext();

  keyboardContextRef.current = {
    ...keyboardContextRef.current,
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
    openBrowserTab: () => { setActivityView(null); openBrowserTab(); },
    openSecurityTab: () => { setActivityView(null); openSecurityTab(); },
    toggleToolManager: () => setActivityView((v) => v === "toolManage" ? null : "toolManage"),
    toggleWiki: () => setActivityView((v) => v === "wiki" ? null : "wiki"),
    toggleBottomTerminal: () => {
      setActivityView(null);
      const s = useStore.getState();
      const currentTabType = s.activeSessionId ? (s.sessions[s.activeSessionId]?.tabType ?? "terminal") : "terminal";
      if (currentTabType !== "terminal") {
        const termTab = s.tabOrder.find((id) => (s.sessions[id]?.tabType ?? "terminal") === "terminal");
        if (termTab) { s.setActiveSession(termTab); return; }
      }
      setBottomTerminalOpen((v) => !v);
    },
    focusAiChat: () => {
      setActivityView(null);
      requestAnimationFrame(() => {
        const el = document.querySelector<HTMLTextAreaElement>('[data-ai-chat-input]');
        el?.focus();
      });
    },
    setCommandPaletteOpen,
    setQuickOpenDialogOpen,
    setSidecarPanelOpen: (open: boolean) => {
      if (open) {
        useStore.getState().openSidecarPanel();
      } else {
        closePanels();
      }
    },
    setShortcutsHelpOpen,
  };

  // Set up the keyboard event listener once
  useEffect(() => {
    const handleKeyDown = createKeyboardHandler(keyboardContextRef);
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [keyboardContextRef]);

  // Handle clear conversation from command palette
  const handleClearConversation = useCallback(async () => {
    if (activeSessionId) {
      await clearConversation(activeSessionId);
      notify.success("Conversation cleared");
    }
  }, [activeSessionId]);

  // Handle toggle full terminal mode from command palette
  const handleToggleFullTerminal = useCallback(() => {
    if (activeSessionId) {
      const currentRenderMode =
        useStore.getState().sessions[activeSessionId]?.renderMode ?? "timeline";
      setRenderMode(activeSessionId, currentRenderMode === "fullterm" ? "timeline" : "fullterm");
    }
  }, [activeSessionId, setRenderMode]);

  // Handle session restore from session browser
  const handleRestoreSession = useCallback(
    async (identifier: string) => {
      if (!activeSessionId) {
        notify.error("No active session to restore into");
        return;
      }
      try {
        await restoreSession(activeSessionId, identifier);
        notify.success("Session restored");
      } catch (error) {
        notify.error(`Failed to restore session: ${error}`);
      }
    },
    [activeSessionId]
  );

  const handleOpenHistory = useCallback(() => setSessionBrowserOpen(true), [setSessionBrowserOpen]);

  // Panel onOpenChange callbacks
  const handleGitPanelOpenChange = useCallback(
    (open: boolean) => {
      if (open) {
        openGitPanel();
      } else {
        closePanels();
      }
    },
    [openGitPanel, closePanels]
  );

  const handleContextPanelOpenChange = useCallback(
    (open: boolean) => {
      if (open) {
        openContextPanel();
      } else {
        closePanels();
      }
    },
    [openContextPanel, closePanels]
  );

  const handleFileEditorPanelOpenChange = useCallback(
    (open: boolean) => {
      if (open) {
        useStore.getState().openFileEditorPanel();
      } else {
        closePanels();
        useFileEditorSidebarStore.getState().setOpen(false);
      }
    },
    [closePanels]
  );

  const handleSidecarPanelOpenChange = useCallback(
    (open: boolean) => {
      if (open) {
        useStore.getState().openSidecarPanel();
      } else {
        closePanels();
      }
    },
    [closePanels]
  );

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
      <div className="flex items-center justify-center h-screen bg-[#1a1b26]">
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
      <div className="h-screen w-screen bg-background flex flex-col overflow-hidden app-bg-layered" data-bottom-terminal={bottomTerminalOpen ? "open" : "closed"}>
        {/* macOS traffic lights + window drag region */}
        <div className="h-[38px] w-full titlebar-drag flex-shrink-0" data-tauri-drag-region />

        {/* Content - floating panels */}
        <div className="flex-1 flex overflow-hidden gap-2 px-2 pb-2 min-h-0 relative">
          {/* Activity Bar - animated hide when viewing the home tab */}
          <div className={cn(
            "flex-shrink-0 transition-all duration-300 ease-out overflow-hidden",
            isOnHomeTab ? "w-0 opacity-0" : "w-[48px] opacity-100"
          )}>
            <ActivityBar
              activeView={activityView}
              onViewChange={setActivityView}
              terminalOpen={bottomTerminalOpen}
              onToggleTerminal={() => {
                setActivityView(null);
                const s = useStore.getState();
                const currentTabType = s.activeSessionId ? (s.sessions[s.activeSessionId]?.tabType ?? "terminal") : "terminal";
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
              onOpenBrowser={() => { setActivityView(null); openBrowserTab(); }}
              onOpenSecurity={() => { setActivityView(null); openSecurityTab(); }}
              browserOpen={hasBrowserTab}
              securityOpen={hasSecurityTab}
            />
          </div>

          {/* Left panel - only shown for settings view */}
          <div className={cn(
            "flex-shrink-0 h-full rounded-xl bg-card overflow-hidden panel-float transition-all duration-200 ease-out",
            activityView === "settings"
              ? "w-[220px] opacity-100"
              : "w-0 opacity-0 pointer-events-none -mr-2"
          )}>
            {renderLeftPanel()}
          </div>

          {/* Settings view - overlays center+right area */}
          <div className={cn(
            "absolute inset-0 left-[284px] flex transition-all duration-200 ease-out px-2 pb-2 pt-0",
            activityView === "settings"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <SettingsContent activeSection={settingsSection} onSectionChange={setSettingsSection} />
              </Suspense>
            </div>
          </div>

          {/* Tool Manager view - overlays entire center+right area */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "toolManage"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <ToolManagerView />
              </Suspense>
            </div>
          </div>

          {/* Wiki view - overlays entire center+right area */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "wiki"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <WikiPanelView />
              </Suspense>
            </div>
          </div>

          {/* Target view - overlays entire center+right area */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "targets"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <TargetPanelView />
              </Suspense>
            </div>
          </div>

          {/* Topology view */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "topology"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float relative">
              <Suspense fallback={null}>
                <TopologyPanelView />
              </Suspense>
            </div>
          </div>

          {/* Methodology view */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "methodology"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float relative">
              <Suspense fallback={null}>
                <MethodologyPanelView />
              </Suspense>
            </div>
          </div>

          {/* Dashboard view */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "dashboard"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <DashboardPanelView />
              </Suspense>
            </div>
          </div>

          {/* Findings view */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "findings"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <FindingsPanelView />
              </Suspense>
            </div>
          </div>

          {/* Pipelines view */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "pipelines"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <PipelinePanelView />
              </Suspense>
            </div>
          </div>

          {/* Audit log view */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "auditLog"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <AuditLogPanelView />
              </Suspense>
            </div>
          </div>

          {/* Wordlists view */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "wordlists"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <WordlistPanelView />
              </Suspense>
            </div>
          </div>

          {/* Vuln intel view */}
          <div className={cn(
            "absolute inset-0 left-[64px] flex transition-all duration-200 ease-out pr-2 pb-2 pt-0",
            activityView === "vulnIntel"
              ? "opacity-100 translate-y-0 pointer-events-auto z-10"
              : "opacity-0 translate-y-1 pointer-events-none z-0",
          )}>
            <div className="flex-1 min-w-0 flex flex-col overflow-hidden rounded-xl bg-card panel-float">
              <Suspense fallback={null}>
                <VulnIntelPanelView />
              </Suspense>
            </div>
          </div>

          {/* Normal view - center + right panels */}
          <div className={cn(
            "flex-1 flex gap-2 min-w-0 transition-all duration-200 ease-out",
            (activityView === "settings" || activityView === "toolManage" || activityView === "wiki" || activityView === "targets" || activityView === "topology" || activityView === "methodology" || activityView === "dashboard" || activityView === "findings" || activityView === "pipelines" || activityView === "auditLog" || activityView === "wordlists" || activityView === "vulnIntel")
              ? "opacity-0 scale-[0.98] pointer-events-none"
              : "opacity-100 scale-100 pointer-events-auto",
          )}>
            {/* Center - TabBar + Pane content (with optional split) */}
            <div className={cn("flex-1 min-w-0 flex gap-2 overflow-hidden relative transition-all duration-300 ease-out", hasSplit ? "flex-row" : "flex-col")}>
              {/* Left column (or full width when no split) */}
              <div className={cn(
                "flex flex-col overflow-hidden rounded-xl bg-card panel-float relative transition-all duration-300 ease-out",
                hasSplit ? "flex-1 min-w-0" : "flex-1",
              )}>
                <div className={cn(
                  "transition-all duration-300 ease-out overflow-hidden",
                  isOnHomeTab ? "max-h-0 opacity-0" : "max-h-[34px] opacity-100"
                )}>
                  <TabBar excludeTabIds={rightPanelTabs} showDropHint={showMergeDropZone} />
                </div>

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
                    <ContextPanel open={contextPanelOpen} onOpenChange={handleContextPanelOpenChange} />
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
                      const rName = rSession?.customName
                        || rSession?.processName
                        || rSession?.workingDirectory?.split(/[/\\]/).pop()
                        || "Terminal";
                      const isRightActive = rTabId === rightActiveTab;
                      return (
                        <div
                          key={rTabId}
                          className={cn(
                            "flex items-center gap-1.5 px-3 py-1 h-full relative cursor-grab active:cursor-grabbing select-none text-[11px] font-mono transition-colors",
                            isRightActive ? "bg-muted text-foreground" : "text-muted-foreground hover:text-foreground hover:bg-muted/50"
                          )}
                          onClick={() => setRightActiveTab(rTabId)}
                          onPointerDown={(e) => {
                            if ((e.target as HTMLElement).closest("button")) return;
                            e.preventDefault();
                            splitDragRef.current = { startX: e.clientX, startY: e.clientY, dragging: false, tabId: rTabId };
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
                            if (ref.dragging && (e.clientX - ref.startX) < -40) closeRightTab(rTabId);
                            splitDragRef.current = { startX: 0, startY: 0, dragging: false, tabId: null };
                            setShowMergeDropZone(false);
                            setSplitDragGhost(null);
                            document.documentElement.classList.remove("tab-dragging");
                          }}
                        >
                          {isRightActive && <span className="absolute bottom-0 left-0 right-0 h-px bg-accent" />}
                          <span className="truncate max-w-[140px]">{rName}</span>
                          <button
                            className="p-0.5 rounded hover:bg-background/50 text-muted-foreground hover:text-foreground transition-colors flex-shrink-0"
                            onClick={(e) => { e.stopPropagation(); closeRightTab(rTabId); }}
                            title="Close split"
                            onPointerDown={(e) => e.stopPropagation()}
                          >
                            <svg width="8" height="8" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
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
                      <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" className="text-accent/60">
                        <rect x="3" y="3" width="18" height="18" rx="2" />
                        <line x1="12" y1="3" x2="12" y2="21" />
                      </svg>
                      <span className="text-xs text-accent/60 font-medium">Split Right</span>
                    </div>
                  </div>
                </div>
              )}
            </div>

            {/* Right sidebar - AI Chat Panel (animated hide on home tab) */}
            <div className={cn(
              "flex-shrink-0 h-full rounded-xl bg-card overflow-hidden panel-float transition-all duration-300 ease-out",
              isOnHomeTab ? "w-0 opacity-0" : "w-[340px] opacity-100"
            )}>
              <AIChatPanel />
            </div>
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
          onOpenBrowser={() => { setActivityView(null); openBrowserTab(); }}
          onOpenSecurity={() => { setActivityView(null); openSecurityTab(); }}
          onToggleToolManager={() => setActivityView((v) => v === "toolManage" ? null : "toolManage")}
          onToggleWiki={() => setActivityView((v) => v === "wiki" ? null : "wiki")}
          onToggleBottomTerminal={() => {
            setActivityView(null);
            const s = useStore.getState();
            const currentTabType = s.activeSessionId ? (s.sessions[s.activeSessionId]?.tabType ?? "terminal") : "terminal";
            if (currentTabType !== "terminal") {
              const termTab = s.tabOrder.find((id) => (s.sessions[id]?.tabType ?? "terminal") === "terminal");
              if (termTab) { s.setActiveSession(termTab); return; }
            }
            setBottomTerminalOpen((v) => !v);
          }}
          onFocusAiChat={() => {
            setActivityView(null);
            requestAnimationFrame(() => {
              document.querySelector<HTMLTextAreaElement>('[data-ai-chat-input]')?.focus();
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
        {splitDragGhost && createPortal(
          <div
            className="fixed z-[9999] pointer-events-none flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-card/95 border border-accent/70 text-foreground text-[11px] font-mono shadow-2xl backdrop-blur-md ring-1 ring-accent/20"
            style={{
              left: splitDragGhost.x,
              top: splitDragGhost.y,
              transform: "translate(-50%, -120%)",
              transition: "box-shadow 0.2s ease",
            }}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className="text-accent flex-shrink-0">
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

function AppWithTheme() {
  const content = (
    <ThemeProvider defaultThemeId="golish">
      <App />
    </ThemeProvider>
  );

  return content;
}

export default AppWithTheme;
