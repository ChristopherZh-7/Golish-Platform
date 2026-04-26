import { useCallback } from "react";
import { shutdownAiSession } from "@/lib/ai";
import { convDelete } from "@/lib/conversation-db";
import { logger } from "@/lib/logger";
import { getAllLeafPanes } from "@/lib/pane-utils";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import { useStore } from "@/store";
import { createNewConversation } from "@/store/slices/conversation";

type CreateTerminalFn = (dir?: string, skip?: boolean) => Promise<string | null>;

export function useChatConversationOps(createTerminalTab: CreateTerminalFn) {
  const handleNewChat = useCallback(async () => {
    const conv = createNewConversation();
    useStore.getState().addConversation(conv);
    const termId = await createTerminalTab(undefined, true);
    if (termId) {
      useStore.getState().addTerminalToConversation(conv.id, termId);
      useStore.getState().setActiveSession(termId);
    }
    return conv;
  }, [createTerminalTab]);

  const handleCloseTab = useCallback((convId: string, e: React.MouseEvent) => {
    e.stopPropagation();
    const storeBefore = useStore.getState();
    const conv = storeBefore.conversations[convId];
    if (conv?.aiInitialized) {
      shutdownAiSession(conv.aiSessionId).catch(() => {});
    }

    const terminalIds = storeBefore.conversationTerminals[convId] ?? [];
    const allSessionIds: string[] = [];
    for (const termId of terminalIds) {
      const layout = storeBefore.tabLayouts[termId];
      if (layout) {
        for (const pane of getAllLeafPanes(layout.root)) allSessionIds.push(pane.sessionId);
      } else {
        allSessionIds.push(termId);
      }
    }

    import("@/lib/tauri").then(({ ptyDestroy }) => {
      for (const sid of allSessionIds) ptyDestroy(sid).catch(() => {});
    });
    for (const sid of allSessionIds) TerminalInstanceManager.dispose(sid);
    import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
      for (const sid of allSessionIds) resetSessionSequence(sid);
    });

    const remainingOrder = storeBefore.conversationOrder.filter((id) => id !== convId);
    let nextActiveSessionId: string | null = null;
    if (remainingOrder.length > 0) {
      const nextConvId = remainingOrder[remainingOrder.length - 1];
      const nextTerms = storeBefore.conversationTerminals[nextConvId];
      if (nextTerms && nextTerms.length > 0) nextActiveSessionId = nextTerms[0];
    }

    useStore.setState((state) => {
      for (const termId of terminalIds) {
        delete state.tabLayouts[termId];
        delete state.tabHasNewActivity[termId];
        const tIdx = state.tabOrder.indexOf(termId);
        if (tIdx !== -1) state.tabOrder.splice(tIdx, 1);
        state.tabActivationHistory = state.tabActivationHistory.filter((id) => id !== termId);
      }
      for (const sid of allSessionIds) {
        delete state.sessions[sid];
        delete state.timelines[sid];
        delete state.pendingCommand[sid];
        delete state.lastSentCommand[sid];
        delete state.agentStreamingBuffer[sid];
        delete state.agentStreaming[sid];
        delete state.streamingBlocks[sid];
        delete state.streamingTextOffset[sid];
        delete state.agentInitialized[sid];
        delete state.isAgentThinking[sid];
        delete state.isAgentResponding[sid];
        delete state.pendingToolApproval[sid];
        delete state.pendingAskHuman[sid];
        delete state.processedToolRequests[sid];
        delete state.activeToolCalls[sid];
        delete state.thinkingContent[sid];
        delete state.isThinkingExpanded[sid];
        delete state.contextMetrics[sid];
        delete state.tabHasNewActivity[sid];
        state.tabActivationHistory = state.tabActivationHistory.filter((id) => id !== sid);
      }
      delete state.conversations[convId];
      delete state.conversationTerminals[convId];
      const orderIdx = state.conversationOrder.indexOf(convId);
      if (orderIdx !== -1) state.conversationOrder.splice(orderIdx, 1);
      if (state.activeConversationId === convId) {
        state.activeConversationId =
          state.conversationOrder.length > 0
            ? state.conversationOrder[state.conversationOrder.length - 1]
            : null;
      }
      if (nextActiveSessionId) state.activeSessionId = nextActiveSessionId;
    });

    convDelete(convId).catch((e) => {
      logger.warn("[AIChatPanel] Failed to delete conversation from DB:", e);
    });

    if (useStore.getState().conversationOrder.length === 0) {
      useStore.getState().setChatPanelVisible(false);
    }
  }, []);

  return { handleNewChat, handleCloseTab };
}
