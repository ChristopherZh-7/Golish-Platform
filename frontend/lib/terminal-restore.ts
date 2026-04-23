/**
 * Shared terminal restore / teardown logic.
 *
 * Extracts the duplicated restoreTerminalForConv / batch-restore patterns
 * from AIChatPanel into a single reusable module.  Both the boot-time and
 * project-switch restore paths consume this.
 *
 * `disposeAllRuntimeTerminals` handles the reverse direction — tearing down
 * all live PTY processes, xterm instances, and session-related store state
 * so a clean project switch can proceed without leaking resources.
 */

import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import { getAllLeafPanes } from "@/lib/pane-utils";
import type { PersistedTerminalData, PersistedTimelineBlock } from "@/lib/workspace-storage";
import { useStore } from "@/store";

type CreateTerminalFn = (
  workingDirectory?: string,
  skipConversationLink?: boolean,
  scrollback?: string,
  logicalTerminalId?: string,
) => Promise<string | null>;

/** Restore persisted timeline blocks into a runtime session's timeline (idempotent). */
function restoreTimelineBlocks(
  blocks: PersistedTimelineBlock[] | undefined,
  targetSessionId: string,
) {
  if (!blocks || blocks.length === 0) return;
  useStore.setState((state) => {
    const existing = state.timelines[targetSessionId];
    const existingIds = new Set(existing?.map((b) => b.id));

    if (!state.timelines[targetSessionId]) state.timelines[targetSessionId] = [];
    for (const block of blocks) {
      if (existingIds.has(block.id)) continue;
      const sanitized = { ...block } as any;
      if (sanitized.type === "pipeline_progress" && sanitized.data?.status === "running") {
        sanitized.data = { ...sanitized.data, status: "interrupted" };
        if (Array.isArray(sanitized.data.steps)) {
          sanitized.data.steps = sanitized.data.steps.map((s: any) =>
            s.status === "running" || s.status === "pending" ? { ...s, status: "interrupted" } : s,
          );
        }
      }
      if (sanitized.type === "ai_tool_execution" && sanitized.data?.status === "running") {
        sanitized.data = { ...sanitized.data, status: "interrupted" };
      }
      if (sanitized.type === "sub_agent_activity" && sanitized.data?.status === "running") {
        sanitized.data = { ...sanitized.data, status: "interrupted" };
      }
      if (sanitized.type === "command") {
        sanitized.data = { ...sanitized.data, sessionId: targetSessionId };
      }
      state.timelines[targetSessionId].push(sanitized);
    }
  });
}

/**
 * Restore a single terminal for a conversation.
 * If a terminal already exists for this conversation, patches its state in-place.
 * Otherwise creates a new PTY terminal via the provided factory.
 */
export async function restoreTerminalForConv(
  convId: string,
  termInfo: PersistedTerminalData,
  isActiveConv: boolean,
  createTerminalTab: CreateTerminalFn,
): Promise<void> {
  const existing = useStore.getState().conversationTerminals[convId] ?? [];
  if (existing.length > 0) {
    const existingTermId = existing[0];
    if (termInfo.logicalTerminalId) {
      useStore.setState((s) => {
        const sess = s.sessions[existingTermId];
        if (sess) sess.logicalTerminalId = termInfo.logicalTerminalId!;
      });
    }
    if (termInfo.scrollback && TerminalInstanceManager.has(existingTermId)) {
      const inst = TerminalInstanceManager.get(existingTermId);
      if (inst) inst.terminal.write(termInfo.scrollback);
    } else if (termInfo.scrollback) {
      TerminalInstanceManager.setPendingScrollback(existingTermId, termInfo.scrollback);
    }
    if (termInfo.customName)
      useStore.getState().setCustomTabName(existingTermId, termInfo.customName);
    if (termInfo.planJson) {
      useStore.getState().setPlan(existingTermId, termInfo.planJson as any);
      useStore.getState().setExecutionMode(existingTermId, "task");
      useStore.getState().setUseAgents(existingTermId, true);
    }
    restoreTimelineBlocks(termInfo.timelineBlocks, existingTermId);
    return;
  }

  const termId = await createTerminalTab(
    termInfo.workingDirectory,
    true,
    termInfo.scrollback,
    termInfo.logicalTerminalId,
  );
  if (!termId) return;
  useStore.getState().addTerminalToConversation(convId, termId);
  if (isActiveConv) useStore.getState().setActiveSession(termId);
  if (termInfo.customName)
    useStore.getState().setCustomTabName(termId, termInfo.customName);
  if (termInfo.planJson) {
    useStore.getState().setPlan(termId, termInfo.planJson as any);
    useStore.getState().setExecutionMode(termId, "task");
    useStore.getState().setUseAgents(termId, true);
  }
  restoreTimelineBlocks(termInfo.timelineBlocks, termId);
}

/**
 * Restore terminals for all conversations from a persisted data map.
 * Active conversation is restored first (eagerly), others follow in background.
 * Guards against concurrent invocations via the store's `terminalRestoreInProgress` flag.
 */
export async function restoreBatchTerminals(
  termData: Record<string, PersistedTerminalData[]>,
  createTerminalTab: CreateTerminalFn,
): Promise<void> {
  if (useStore.getState().terminalRestoreInProgress) return;

  const activeId = useStore.getState().activeConversationId;
  useStore.getState().setTerminalRestoreInProgress(true);

  try {
    if (activeId && termData[activeId]?.[0] && useStore.getState().conversations[activeId]) {
      await restoreTerminalForConv(activeId, termData[activeId][0], true, createTerminalTab);
    }

    useStore.getState().setTerminalRestoreInProgress(false);
    const savedActiveSessionId = useStore.getState().activeSessionId;

    const otherConvs = Object.entries(termData).filter(
      ([convId, terminals]) =>
        convId !== activeId && useStore.getState().conversations[convId] && terminals.length > 0,
    );
    for (const [convId, savedTerms] of otherConvs) {
      const existing = useStore.getState().conversationTerminals[convId] ?? [];
      if (existing.length > 0) continue;
      await restoreTerminalForConv(convId, savedTerms[0], false, createTerminalTab);
    }

    if (savedActiveSessionId && useStore.getState().activeSessionId !== savedActiveSessionId) {
      useStore.getState().setActiveSession(savedActiveSessionId);
    }
  } catch (e) {
    console.warn("[terminal-restore] Failed to restore terminals:", e);
  } finally {
    useStore.getState().setTerminalRestoreInProgress(false);
  }
}

/**
 * Tear down all live terminals before a project switch.
 *
 * Collects every session ID (including split-pane children), then:
 *  1. Shuts down AI sessions (fire-and-forget)
 *  2. Destroys backend PTY processes (fire-and-forget)
 *  3. Disposes xterm.js instances synchronously
 *  4. Purges all session-related store state in a single Immer batch
 *
 * Must be awaited before calling `restoreConversations` so the old
 * project's resources don't leak into the new one.
 */
export async function disposeAllRuntimeTerminals(): Promise<void> {
  const state = useStore.getState();

  const allSessionIds = new Set<string>();
  for (const termIds of Object.values(state.conversationTerminals)) {
    for (const termId of termIds) {
      const layout = state.tabLayouts[termId];
      if (layout) {
        for (const pane of getAllLeafPanes(layout.root)) {
          allSessionIds.add(pane.sessionId);
        }
      } else {
        allSessionIds.add(termId);
      }
    }
  }
  for (const sid of Object.keys(state.sessions)) {
    allSessionIds.add(sid);
  }

  const [{ shutdownAiSession }, { ptyDestroy }] = await Promise.all([
    import("@/lib/ai"),
    import("@/lib/tauri"),
  ]);

  for (const conv of Object.values(state.conversations)) {
    if (conv?.aiInitialized) {
      shutdownAiSession(conv.aiSessionId).catch(() => {});
    }
  }

  for (const sid of allSessionIds) {
    ptyDestroy(sid).catch(() => {});
    TerminalInstanceManager.dispose(sid);
  }

  useStore.setState((s) => {
    for (const sid of allSessionIds) {
      delete s.sessions[sid];
      delete s.timelines[sid];
      delete s.pendingCommand[sid];
      delete s.lastSentCommand[sid];
      delete s.pipelineCommandSource[sid];
      delete s.agentStreamingBuffer[sid];
      delete s.agentStreaming[sid];
      delete s.streamingBlocks[sid];
      delete s.streamingTextOffset[sid];
      delete s.agentInitialized[sid];
      delete s.isAgentThinking[sid];
      delete s.isAgentResponding[sid];
      delete s.pendingToolApproval[sid];
      delete s.pendingAskHuman[sid];
      delete s.processedToolRequests[sid];
      delete s.activeToolCalls[sid];
      delete s.thinkingContent[sid];
      delete s.isThinkingExpanded[sid];
      delete s.contextMetrics[sid];
      delete s.compactionCount[sid];
      delete s.gitStatus[sid];
      delete s.gitStatusLoading[sid];
      delete s.gitCommitMessage[sid];
      delete s.activeWorkflows[sid];
      delete s.workflowHistory[sid];
      delete s.activeSubAgents[sid];
      delete s.subAgentBatchCounter[sid];
      delete s.tabLayouts[sid];
      delete s.tabHasNewActivity[sid];
    }
    s.tabOrder = [];
    s.tabActivationHistory = [];
    s.activeSessionId = null;
  });
}
