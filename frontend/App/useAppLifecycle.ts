import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type React from "react";
import { useEffect, useState } from "react";
import { logger } from "@/lib/logger";
import { getProjectPath } from "@/lib/projects";
import { useAiEvents } from "../hooks/useAiEvents";
import { useCreateTerminalTab } from "../hooks/useCreateTerminalTab";
import { usePipelineEvents } from "../hooks/usePipelineEvents";
import { useTauriEvents } from "../hooks/useTauriEvents";
import { createDbAutoSaver, loadFromDb, markDbLoadSucceeded } from "../lib/conversation-db-sync";
import { notify } from "../lib/notify";
import { updateConfig as updatePentestConfig } from "../lib/pentest/api";
import { getSettings } from "../lib/settings";
import { initSystemNotifications, listenForSettingsUpdates } from "../lib/systemNotifications";
import { shellIntegrationInstall, shellIntegrationStatus } from "../lib/tauri";
import {
  getLastProjectName,
  loadWorkspaceState,
  toChatConversation,
} from "../lib/workspace-storage";
import { useStore } from "../store";
import { useFileEditorSidebarStore } from "../store/file-editor-sidebar";
import { createNewConversation } from "../store/slices/conversation";

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
          const { getProjectConfig } = await import("../lib/projects");
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

  // Handle tab split events from TabBar
  // Original effect in App.tsx had empty deps; setters arriving as props are stable
  // React useState setters so [] is still correct here. Preserved verbatim per
  // refactor constraint: do not change useEffect dependency arrays.
  // biome-ignore lint/correctness/useExhaustiveDependencies: preserve original empty deps; setters are stable
  useEffect(() => {
    const handleSplitTab = (e: Event) => {
      const tabId = (e as CustomEvent<string>).detail;
      setRightPanelTabs((prev) => {
        if (prev.includes(tabId)) return prev;
        const s = useStore.getState();
        if (s.activeSessionId === tabId) {
          const other = s.tabOrder.find(
            (id) =>
              id !== tabId &&
              !prev.includes(id) &&
              (s.sessions[id]?.tabType ?? "terminal") !== "home"
          );
          if (other) s.setActiveSession(other);
        }
        return [...prev, tabId];
      });
      setRightActiveTab(tabId);
      setShowSplitDropZone(false);
    };
    const handleUnsplitTab = () => {
      setRightPanelTabs([]);
      setRightActiveTab(null);
    };
    const handleDragHint = (e: Event) => setShowSplitDropZone((e as CustomEvent<boolean>).detail);
    const handleToolOutput = async (e: Event) => {
      if (localStorage.getItem("golish-auto-detect-output") === "false") return;
      const { command, output } = (
        e as CustomEvent<{ command: string; output: string; sessionId: string }>
      ).detail;
      try {
        const detected = await invoke<{
          tool_id: string;
          tool_name: string;
          output_config: {
            format: string;
            produces: string[];
            patterns: unknown[];
            fields: Record<string, string>;
            detect?: string;
          };
        } | null>("output_detect_tool", { command, rawOutput: output });
        if (!detected) return;
        const parsed = await invoke<{
          items: { data_type: string; fields: Record<string, string> }[];
        }>("output_parse", {
          rawOutput: output,
          config: detected.output_config,
          toolId: detected.tool_id,
          toolName: detected.tool_name,
        });
        if (!parsed.items.length) return;
        const pp = getProjectPath();
        const produces = detected.output_config.produces;

        if (produces.includes("vulnerability")) {
          const vulnItems = parsed.items
            .filter((it) => it.data_type === "vulnerability")
            .map((it) => it.fields);
          if (vulnItems.length > 0) {
            const added = await invoke<number>("findings_import_parsed", {
              items: vulnItems,
              toolName: detected.tool_name,
              projectPath: pp,
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
      const { tabId, screenX, screenY } = (
        e as CustomEvent<{ tabId: string; screenX: number; screenY: number }>
      ).detail;
      const s = useStore.getState();
      const session = s.sessions[tabId];
      if (!session) return;
      const tabType = session.tabType ?? "terminal";

      if (tabType === "security") {
        const pseudoId = `security-all-${Date.now()}`;
        try {
          await invoke("create_detached_window", {
            sessionId: pseudoId,
            tabType: "security-all",
            title: "Security — Detached",
            x: screenX - 50,
            y: screenY - 20,
            width: 1000.0,
            height: 700.0,
          });
          notify.info("Security detached to floating window");
        } catch (err) {
          logger.error("[App] detach security tab failed:", err);
        }
        return;
      }

      if (tabType !== "terminal") return;

      const title =
        session.customName ||
        session.processName ||
        session.workingDirectory?.split(/[/\\]/).pop() ||
        "Terminal";

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
        const detached = JSON.parse(localStorage.getItem("golish-detached-tabs") || "{}");
        detached[tabId] = { title, tabType };
        try {
          localStorage.setItem("golish-detached-tabs", JSON.stringify(detached));
        } catch {
          /* ignore */
        }

        const other = s.tabOrder.find(
          (id) => id !== tabId && (s.sessions[id]?.tabType ?? "terminal") !== "home"
        );
        if (other) s.setActiveSession(other);
        notify.info(`"${title}" detached to floating window`);
      } catch (err) {
        logger.error("[App] detach tab failed:", err);
      }
    };

    const handleDetachSecurityTab = async (e: Event) => {
      const { tabId, screenX, screenY } = (
        e as CustomEvent<{ tabId: string; screenX: number; screenY: number }>
      ).detail;
      const tabLabels: Record<string, string> = {
        history: "HTTP History",
        sitemap: "Site Map",
        scanner: "Scanner",
        repeater: "Repeater",
        alerts: "Alerts",
        audit: "Audit Log",
        passive: "Passive Scan",
        vault: "Credential Vault",
      };
      const title = tabLabels[tabId] || tabId;
      const pseudoId = `security-${tabId}-${Date.now()}`;

      try {
        await invoke("create_detached_window", {
          sessionId: pseudoId,
          tabType: `security-${tabId}`,
          title: `${title} — Detached`,
          x: screenX - 50,
          y: screenY - 20,
          width: 900.0,
          height: 600.0,
        });
        notify.info(`"${title}" detached to floating window`);
      } catch (err) {
        logger.error("[App] detach security tab failed:", err);
      }
    };

    window.addEventListener("split-tab-right", handleSplitTab);
    window.addEventListener("unsplit-tab", handleUnsplitTab);
    window.addEventListener("tab-drag-split-hint", handleDragHint);
    window.addEventListener("detach-tab", handleDetachTab);
    window.addEventListener("detach-security-tab", handleDetachSecurityTab);
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
      try {
        localStorage.setItem("golish-detached-tabs", JSON.stringify(detached));
      } catch {
        /* ignore */
      }
      notify.info("Detached window closed — tab restored");
    }).then((fn) => {
      unlistenDetachedClose = fn;
    });

    return () => {
      window.removeEventListener("split-tab-right", handleSplitTab);
      window.removeEventListener("unsplit-tab", handleUnsplitTab);
      window.removeEventListener("tab-drag-split-hint", handleDragHint);
      window.removeEventListener("detach-tab", handleDetachTab);
      window.removeEventListener("detach-security-tab", handleDetachSecurityTab);
      window.removeEventListener("tool-output-completed", handleToolOutput);
      window.removeEventListener("recording-saved", handleRecordingSaved);
      unlistenDetachedClose?.();
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

  // Auto-capture credentials detected by ZAP proxy and save/update vault
  useEffect(() => {
    const knownEntries = new Map<string, { id: string; valueHash: string }>();
    let unlisten: (() => void) | null = null;

    interface DetectedCredential {
      source_url: string;
      host: string;
      cred_type: string;
      username: string | null;
      value: string;
      field_name: string;
      zap_message_id: number;
    }

    interface VaultEntry {
      id: string;
      name: string;
      tags: string[];
    }

    const credTypeMap: Record<string, string> = {
      password: "password",
      bearer_token: "token",
      basic_auth: "password",
      api_key: "api_key",
      access_token: "token",
      refresh_token: "token",
      session_cookie: "cookie",
      o_auth_client_secret: "api_key",
    };

    function hashValue(v: string): string {
      let h = 0;
      for (let i = 0; i < v.length; i++) h = ((h << 5) - h + v.charCodeAt(i)) | 0;
      return h.toString(36);
    }

    (async () => {
      try {
        const existing = await invoke<VaultEntry[]>("vault_list", {
          projectPath: getProjectPath(),
        });
        if (Array.isArray(existing)) {
          for (const e of existing) {
            if (e.tags?.includes("auto-captured")) {
              knownEntries.set(e.name, { id: e.id, valueHash: "" });
            }
          }
        }
      } catch {
        /* vault might not be ready yet */
      }

      unlisten = await listen<DetectedCredential>("credential-detected", async (event) => {
        const cred = event.payload;
        const name = `${cred.host} - ${cred.field_name}`;
        const newHash = hashValue(cred.value);
        const existing = knownEntries.get(name);

        if (existing && existing.valueHash === newHash) return;

        const entryType = credTypeMap[cred.cred_type] || "other";

        try {
          if (existing) {
            await invoke("vault_update", {
              id: existing.id,
              value: cred.value,
              username: cred.username || null,
              notes: `Auto-captured from ${cred.source_url} (updated)`,
              projectPath: getProjectPath(),
            });
            knownEntries.set(name, { id: existing.id, valueHash: newHash });
            notify.info("Credential updated", {
              message: `${cred.host} — ${cred.field_name}`,
            });
          } else {
            const added = await invoke<{ id: string }>("vault_add", {
              name,
              entryType,
              value: cred.value,
              username: cred.username || null,
              notes: `Auto-captured from ${cred.source_url}`,
              project: cred.host,
              tags: ["auto-captured", "zap"],
              sourceUrl: cred.source_url,
              projectPath: getProjectPath(),
            });
            knownEntries.set(name, { id: added.id, valueHash: newHash });
            notify.success("Credential captured", {
              message: `${cred.host} — ${cred.field_name} (${cred.cred_type})`,
            });
          }
        } catch (e) {
          console.error("[CredAutoCapture] Failed to save credential:", e);
        }
      });
    })();

    return () => {
      unlisten?.();
    };
  }, []);

  return { isLoading, error };
}
