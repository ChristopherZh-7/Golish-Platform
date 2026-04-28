import { logger } from "@/lib/logger";
import type { PersistedTerminalData, PersistedTimelineBlock } from "@/lib/workspace-storage";
import { useStore } from "./index";
import type { AgentMessage, TabType } from "./store-types";

export interface CloseTabAndCleanupOptions {
  /** Tab type — controls whether PTY/AI cleanup runs and whether a replacement terminal is created. */
  tabType: TabType;
  /**
   * Factory used to create a replacement terminal so the 1:1 conversation:terminal binding is preserved.
   * Caller passes the same `createTerminalTab` they obtain from `useCreateTerminalTab`.
   */
  createTerminalTab: (
    workingDirectory?: string,
    skipConversationLink?: boolean
  ) => Promise<string | null>;
}

/**
 * Fully close a tab and run the appropriate cleanup.
 *
 * For terminal tabs this:
 *  1. Shuts down the AI session and destroys the PTY for every pane in the tab.
 *  2. Disposes terminal/live-terminal manager instances for those sessions.
 *  3. Removes the terminal from its conversation mapping.
 *  4. Removes all frontend tab/session state via the existing `closeTab` slice action.
 *  5. Creates a replacement terminal — either inside the same conversation (to preserve the
 *     1:1 conversation:terminal binding) or a fresh one in the user's last cwd if no terminal
 *     tabs would be left otherwise.
 *
 * For non-terminal tab types only frontend state cleanup runs.
 */
export async function closeTabAndCleanup(
  tabId: string,
  { tabType, createTerminalTab }: CloseTabAndCleanupOptions
): Promise<void> {
  if (tabType === "terminal") {
    try {
      const sessionIds = useStore.getState().getTabSessionIds(tabId);
      const idsToCleanup = sessionIds.length > 0 ? sessionIds : [tabId];

      const [
        { shutdownAiSession },
        { ptyDestroy },
        { TerminalInstanceManager, liveTerminalManager },
      ] = await Promise.all([import("@/lib/ai"), import("@/lib/tauri"), import("@/lib/terminal")]);

      await Promise.all(
        idsToCleanup.map(async (sessionId) => {
          try {
            await shutdownAiSession(sessionId);
          } catch (err) {
            logger.error(`Failed to shutdown AI session ${sessionId}:`, err);
          }
          try {
            await ptyDestroy(sessionId);
          } catch (err) {
            logger.error(`Failed to destroy PTY ${sessionId}:`, err);
          }
          TerminalInstanceManager.dispose(sessionId);
          liveTerminalManager.dispose(sessionId);
        })
      );
    } catch (err) {
      logger.error(`Error during terminal cleanup for tab ${tabId}:`, err);
    }
  }

  const store = useStore.getState();
  const convId = store.getConversationForTerminal(tabId);
  const oldDir = store.sessions[tabId]?.workingDirectory;
  if (convId) {
    store.removeTerminalFromConversation(convId, tabId);
  }

  store.closeTab(tabId);

  if (tabType !== "terminal") return;

  if (convId) {
    const newId = await createTerminalTab(oldDir, true);
    if (newId) {
      const s = useStore.getState();
      s.addTerminalToConversation(convId, newId);
      s.setActiveSession(newId);
    }
    return;
  }

  const s = useStore.getState();
  const hasTerminals = s.tabOrder.some(
    (id) => (s.sessions[id]?.tabType ?? "terminal") === "terminal"
  );
  if (!hasTerminals) {
    const newId = await createTerminalTab();
    if (newId) {
      s.setActiveSession(newId);
    }
  }
}

export interface OpenProjectOptions {
  /**
   * Factory that creates a terminal in the project's working directory once workspace
   * state has been restored. Caller passes the same `createTerminalTab` they obtain
   * from `useCreateTerminalTab`.
   */
  createTerminalTab: (
    workingDirectory?: string,
    skipConversationLink?: boolean
  ) => Promise<string | null>;
}

/**
 * Open a project: dispose runtime terminals, restore conversations + terminals from the
 * conversation DB (with legacy workspace-storage fallback), and create at least one
 * terminal so the UI transitions away from the Home view.
 *
 * Returns the new terminal session id, or `null` if creation failed (or an exception was
 * caught — see logger output).
 *
 * Extracted from `HomeView.handleOpenProject` so File→Open, command-palette
 * "Switch project", and other entry points can reuse the same restoration flow without
 * duplicating ~90 lines of cross-module orchestration.
 */
export async function openProject(
  projectName: string,
  rootPath: string,
  { createTerminalTab }: OpenProjectOptions
): Promise<string | null> {
  try {
    const [
      { disposeAllRuntimeTerminals },
      { clearSaveFingerprints, loadFromDb, markDbLoadSucceeded },
      { loadWorkspaceState, setLastProjectName, toChatConversation },
      { createNewConversation },
    ] = await Promise.all([
      import("@/lib/terminal-restore"),
      import("@/lib/conversation-db-sync"),
      import("@/lib/workspace-storage"),
      import("./slices/conversation"),
    ]);

    await disposeAllRuntimeTerminals();
    clearSaveFingerprints();

    useStore.getState().setCurrentProject(projectName, rootPath);
    setLastProjectName(projectName);

    const saved = await loadFromDb(rootPath);
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
        const termRestoreData: Record<string, PersistedTerminalData[]> = {};
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
            timelineBlocks: t.timelineBlocks.map(
              (b) =>
                ({
                  id: b.id,
                  type: b.type,
                  timestamp: b.timestamp ?? new Date().toISOString(),
                  data: b.data,
                  batchId: (b as { batchId?: string }).batchId,
                }) as PersistedTimelineBlock
            ),
          }));
        }
        useStore.getState().setPendingTerminalRestoreData(termRestoreData);
      }
    } else {
      const legacy = await loadWorkspaceState(projectName);

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

        if (legacy.conversationTerminalData) {
          useStore.getState().setPendingTerminalRestoreData(legacy.conversationTerminalData);
        }
      } else {
        const conv = createNewConversation();
        useStore.getState().restoreConversations([conv], [conv.id], conv.id);
      }
    }

    useStore.getState().setWorkspaceDataReady(true);
    markDbLoadSucceeded();

    // Always create at least one terminal so the UI transitions away from Home.
    // If we have saved terminal data, the AIChatPanel effect will restore
    // scrollback/timeline/plan into the terminal created here.
    const activeConv = useStore.getState().activeConversationId;
    const sessionId = await createTerminalTab(rootPath);
    if (sessionId) {
      if (activeConv) {
        useStore.getState().addTerminalToConversation(activeConv, sessionId);
      }
      useStore.getState().setActiveSession(sessionId);
    }
    return sessionId;
  } catch (error) {
    logger.error("Failed to open project:", error);
    return null;
  }
}

export async function clearConversation(sessionId: string): Promise<void> {
  useStore.getState().clearTimeline(sessionId);

  try {
    const { clearAiConversationSession, clearAiConversation } = await import("@/lib/ai");
    try {
      await clearAiConversationSession(sessionId);
    } catch {
      await clearAiConversation(sessionId);
    }
  } catch (error) {
    logger.warn("Failed to clear backend conversation history:", error);
  }
}

export async function restoreSession(sessionId: string, identifier: string): Promise<void> {
  const aiModule = await import("@/lib/ai");
  const { loadAiSession, restoreAiSession, initAiSession, buildProviderConfig } = aiModule;
  const { getSettings } = await import("@/lib/settings");

  const session = await loadAiSession(identifier);
  if (!session) {
    throw new Error(`Session '${identifier}' not found`);
  }

  const settings = await getSettings();
  const workspace = session.workspace_path;

  logger.info(
    `Restoring session (original: ${session.provider}/${session.model}, ` +
      `using current: ${settings.ai.default_provider}/${settings.ai.default_model})`
  );

  const config = await buildProviderConfig(settings, workspace);

  await initAiSession(sessionId, config);

  useStore.getState().setSessionAiConfig(sessionId, {
    provider: settings.ai.default_provider,
    model: settings.ai.default_model,
    status: "ready",
  });

  await restoreAiSession(sessionId, identifier);

  const agentMessages: AgentMessage[] = session.messages
    .filter((msg) => msg.role === "user" || msg.role === "assistant")
    .map((msg, index) => ({
      id: `restored-${identifier}-${index}`,
      sessionId,
      role: msg.role as "user" | "assistant",
      content: msg.content,
      timestamp: index === 0 ? session.started_at : session.ended_at,
      isStreaming: false,
    }));

  useStore.getState().clearTimeline(sessionId);
  useStore.getState().restoreAgentMessages(sessionId, agentMessages);
  useStore.getState().setInputMode(sessionId, "agent");
}
