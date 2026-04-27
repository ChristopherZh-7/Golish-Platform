import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type React from "react";
import { useEffect, useState } from "react";
import { logger } from "@/lib/logger";
import { useAiEvents } from "../../hooks/useAiEvents";
import { useCreateTerminalTab } from "../../hooks/useCreateTerminalTab";
import { usePipelineEvents } from "../../hooks/usePipelineEvents";
import { useTauriEvents } from "../../hooks/useTauriEvents";
import { createDbAutoSaver, loadFromDb, markDbLoadSucceeded } from "../../lib/conversation-db-sync";
import { notify } from "../../lib/notify";
import { updateConfig as updatePentestConfig } from "../../lib/pentest/api";
import { getSettings } from "../../lib/settings";
import { initSystemNotifications, listenForSettingsUpdates } from "../../lib/systemNotifications";
import { shellIntegrationInstall, shellIntegrationStatus } from "../../lib/tauri";
import {
  getLastProjectName,
  loadWorkspaceState,
  toChatConversation,
} from "../../lib/workspace-storage";
import { useStore } from "../../store";
import { useFileEditorSidebarStore } from "../../store/file-editor-sidebar";
import { createNewConversation } from "../../store/slices/conversation";
import { useCredentialCapture } from "./useCredentialCapture";
import { useTabSplitEvents } from "./useTabSplitEvents";

interface UseAppLifecycleProps {
  setRightPanelTabs: React.Dispatch<React.SetStateAction<string[]>>;
  setRightActiveTab: React.Dispatch<React.SetStateAction<string | null>>;
  setShowSplitDropZone: React.Dispatch<React.SetStateAction<boolean>>;
}

/**
 * Owns every long-lived side-effect attached to the App shell:
 *
 *  - workspace bootstrapping (project restore, terminal creation, DB load)
 *  - Tauri / AI / pipeline event subscriptions
 *  - DB auto-save + pentest config sync
 *  - system notifications + window focus / visibility tracking
 *  - tab split / detach / credential / recording event listeners
 *  - native menu event subscriptions
 *  - file-editor sidebar store sync
 *  - UI scale CSS variable
 *
 * Returns the `isLoading` / `error` flags consumed by the loading and error
 * fallback states in the AppShell.
 */
export function useAppLifecycle({
  setRightPanelTabs,
  setRightActiveTab,
  setShowSplitDropZone,
}: UseAppLifecycleProps) {
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const openHomeTab = useStore((state) => state.openHomeTab);
  const openSettingsTab = useStore((state) => state.openSettingsTab);
  const fileEditorPanelOpen = useStore((state) => state.fileEditorPanelOpen);
  const uiScale = useStore((s) => s.displaySettings.uiScale);

  const { createTerminalTab } = useCreateTerminalTab();

  // Connect Tauri events to store
  useTauriEvents();

  // Subscribe to AI events for agent mode
  useAiEvents();

  // Subscribe to pipeline progress events (bridges Rust pipeline-event → timeline)
  usePipelineEvents();

  // Tab split/detach/tool-output/recording events
  useTabSplitEvents({ setRightPanelTabs, setRightActiveTab, setShowSplitDropZone });

  // Auto-capture credentials detected by ZAP proxy
  useCredentialCapture();

  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--ui-scale", `${uiScale}`);
    return () => {
      root.style.removeProperty("--ui-scale");
    };
  }, [uiScale]);

  // Subscribe to file editor sidebar store to sync open state
  // This allows openFile() calls from anywhere to open the sidebar
  const fileEditorStoreOpen = useFileEditorSidebarStore((state) => state.open);
  useEffect(() => {
    if (fileEditorStoreOpen && !fileEditorPanelOpen) {
      useStore.getState().openFileEditorPanel();
    }
  }, [fileEditorStoreOpen, fileEditorPanelOpen]);

  useEffect(() => {
    let cancelled = false;

    async function init() {
      try {
        const currentSessions = useStore.getState().sessions;
        if (Object.keys(currentSessions).length > 0) {
          logger.info("[App] Sessions already exist, skipping initialization...");
          setIsLoading(false);
          return;
        }

        logger.info("[App] Starting initialization...");

        openHomeTab();

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

        const lastProject = getLastProjectName();
        if (lastProject) {
          logger.info("[App] Restoring project:", lastProject);
          const { getProjectConfig } = await import("@/lib/projects");
          const config = await getProjectConfig(lastProject);
          if (cancelled) return;

          if (config) {
            useStore.getState().setCurrentProject(lastProject, config.rootPath);

            const saved = await loadFromDb(config.rootPath);
            if (cancelled) return;

            if (saved && saved.conversations.length > 0) {
              if (saved.aiModel) useStore.getState().setSelectedAiModel(saved.aiModel);
              if (saved.approvalMode) useStore.getState().setApprovalMode(saved.approvalMode);

              useStore
                .getState()
                .restoreConversations(
                  saved.conversations,
                  saved.conversationOrder,
                  saved.activeConversationId
                );

              if (Object.keys(saved.terminalData).length > 0) {
                const termRestoreData: Record<
                  string,
                  import("@/lib/workspace-storage").PersistedTerminalData[]
                > = {};
                for (const [convId, terminals] of Object.entries(saved.terminalData)) {
                  termRestoreData[convId] = terminals.map((t) => ({
                    logicalTerminalId: t.sessionId,
                    workingDirectory: t.workingDirectory,
                    scrollback: t.scrollback,
                    customName: t.customName ?? undefined,
                    planJson: t.planJson ?? undefined,
                    executionMode: t.executionMode ?? undefined,
                    useAgents: t.useAgents ?? undefined,
                    retiredPlansJson: t.retiredPlansJson ?? undefined,
                    timelineBlocks: t.timelineBlocks.map((b) => ({
                      id: b.id,
                      type: b.type as any,
                      timestamp: b.timestamp ?? new Date().toISOString(),
                      data: b.data as any,
                      batchId: (b as { batchId?: string }).batchId,
                    })),
                  }));
                }
                useStore.getState().setPendingTerminalRestoreData(termRestoreData);
              }

              useStore.getState().setWorkspaceDataReady(true);
              markDbLoadSucceeded();

              const hasConvTerminals = Object.keys(saved.terminalData).length > 0;
              if (!hasConvTerminals) {
                const termId = await createTerminalTab(config.rootPath, true);
                if (cancelled) return;
                if (termId) {
                  const activeConv = useStore.getState().activeConversationId;
                  if (activeConv) {
                    useStore.getState().addTerminalToConversation(activeConv, termId);
                  }
                  useStore.getState().setActiveSession(termId);
                }
              }
            } else {
              const legacy = await loadWorkspaceState(lastProject);
              if (cancelled) return;

              if (legacy?.aiModel) useStore.getState().setSelectedAiModel(legacy.aiModel);
              if (legacy?.approvalMode) useStore.getState().setApprovalMode(legacy.approvalMode);

              if (legacy && legacy.conversations.length > 0) {
                const restoredConvs = legacy.conversations.map(toChatConversation);
                useStore
                  .getState()
                  .restoreConversations(
                    restoredConvs,
                    legacy.conversationOrder,
                    legacy.activeConversationId
                  );
              } else {
                const initialConv = createNewConversation();
                useStore.getState().addConversation(initialConv);
              }

              if (legacy?.conversationTerminalData) {
                useStore.getState().setPendingTerminalRestoreData(legacy.conversationTerminalData);
              }

              useStore.getState().setWorkspaceDataReady(true);
              markDbLoadSucceeded();

              const hasConvTerminals =
                legacy?.conversationTerminalData &&
                Object.keys(legacy.conversationTerminalData).some(
                  (id) => id in useStore.getState().conversations
                );

              if (!hasConvTerminals) {
                const wd = legacy?.terminalTabs?.[0]?.workingDirectory ?? config.rootPath;
                const termId = await createTerminalTab(wd, true);
                if (cancelled) return;
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
        }

        if (cancelled) return;

        if (!useStore.getState().currentProjectName) {
          useStore.getState().setWorkspaceDataReady(true);
          const homeId = useStore.getState().homeTabId;
          if (homeId) {
            useStore.getState().setActiveSession(homeId);
          }
        }

        setIsLoading(false);
      } catch (e) {
        if (cancelled) return;
        logger.error("Failed to initialize:", e);
        setError(e instanceof Error ? e.message : String(e));
        useStore.getState().setWorkspaceDataReady(true);
        setIsLoading(false);
      }
    }

    init();
    return () => {
      cancelled = true;
    };
  }, [openHomeTab, createTerminalTab]);

  // Auto-save conversation state to PostgreSQL on changes + window close
  useEffect(() => {
    return createDbAutoSaver(
      () => useStore.getState().currentProjectPath ?? null,
      (listener) => useStore.subscribe(listener),
      () => {
        const s = useStore.getState();
        return {
          conversations: s.conversations,
          conversationOrder: s.conversationOrder,
          activeConversationId: s.activeConversationId,
          conversationTerminals: s.conversationTerminals,
          sessions: s.sessions,
          timelines: s.timelines,
          selectedAiModel: s.selectedAiModel,
          approvalMode: s.approvalMode,
          terminalRestoreInProgress: s.terminalRestoreInProgress,
          pendingTerminalRestoreData: s.pendingTerminalRestoreData,
        };
      }
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

    initSystemNotifications(useStore).catch((error) => {
      logger.error("Failed to initialize system notifications:", error);
    });

    const unlistenSettings = listenForSettingsUpdates();

    const handleFocus = () => setAppIsFocused(true);
    const handleBlur = () => setAppIsFocused(false);

    const handleVisibilityChange = () => {
      setAppIsVisible(document.visibilityState === "visible");
    };

    window.addEventListener("focus", handleFocus);
    window.addEventListener("blur", handleBlur);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    setAppIsFocused(document.hasFocus());
    setAppIsVisible(document.visibilityState === "visible");

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


  // Handle native menu events from Tauri backend
  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    (async () => {
      unlisteners.push(
        await listen("menu-open-project", () => {
          openHomeTab();
        })
      );
      unlisteners.push(
        await listen("menu-new-project", () => {
          openHomeTab();
        })
      );
      unlisteners.push(
        await listen("menu-settings", () => {
          openSettingsTab();
        })
      );
    })();
    return () => {
      unlisteners.forEach((fn) => fn());
    };
  }, [openHomeTab, openSettingsTab]);


  return { isLoading, error };
}
