import { useCallback, useRef } from "react";
import {
  type AgentMode,
  initAiSession,
  restoreAiConversation,
  sendPromptSession,
  setAgentMode,
  setExecutionMode as setExecutionModeBackend,
  setUseAgents as setUseAgentsBackend,
  shutdownAiSession,
} from "@/lib/ai";
import { getSettings } from "@/lib/settings";
import { useStore } from "@/store";
import { buildProviderConfig } from "../providerConfig";

type SelectedModel = { model: string; provider: string } | null;

interface UseChatSessionInitOptions {
  selectedModel: SelectedModel;
  chatExecutionModeRef: React.MutableRefObject<"chat" | "task">;
  chatUseSubAgentsRef: React.MutableRefObject<boolean>;
  setChatExecutionMode: (mode: "chat" | "task") => void;
  setChatUseSubAgents: (val: boolean) => void;
  updateConv: (convId: string, update: Record<string, unknown>) => void;
}

export function useChatSessionInit(opts: UseChatSessionInitOptions) {
  const {
    selectedModel,
    chatExecutionModeRef,
    chatUseSubAgentsRef,
    setChatExecutionMode,
    setChatUseSubAgents,
    updateConv,
  } = opts;

  const generateTitleRef = useRef<((convId: string, firstMsg: string) => void) | null>(null);

  const generateTitle = useCallback(
    async (convId: string, firstMessage: string) => {
      if (!selectedModel?.model || !selectedModel?.provider) return;
      const titleSessionId = `title-gen-${convId}`;
      try {
        const settings = await getSettings();
        const titleWorkspace = useStore.getState().currentProjectPath || ".";
        const providerConfig = buildProviderConfig(
          selectedModel.provider,
          selectedModel.model,
          titleWorkspace,
          settings,
        );
        if (!providerConfig) return;
        await initAiSession(titleSessionId, providerConfig);
        const title = await sendPromptSession(
          titleSessionId,
          `Generate a concise 3-5 word title for this chat message. Output ONLY the title, nothing else. No quotes, no punctuation at the end.\n\nMessage: "${firstMessage.slice(0, 200)}"`,
        );
        const cleaned = title
          .trim()
          .replace(/^["']|["']$/g, "")
          .slice(0, 40);
        if (cleaned) {
          useStore.getState().updateConversation(convId, { title: cleaned });
        }
      } catch {
        // Title generation failed silently
      } finally {
        shutdownAiSession(titleSessionId).catch(() => {});
      }
    },
    [selectedModel],
  );
  generateTitleRef.current = generateTitle;

  const initializeSession = useCallback(
    async (conv: { id: string; aiSessionId: string; aiInitialized: boolean }) => {
      if (conv.aiInitialized) return true;
      if (!selectedModel?.model || !selectedModel?.provider) return false;

      try {
        const settings = await getSettings();
        const workspace = useStore.getState().currentProjectPath || ".";
        const providerConfig = buildProviderConfig(
          selectedModel.provider,
          selectedModel.model,
          workspace,
          settings,
        );
        if (!providerConfig) return false;

        await initAiSession(conv.aiSessionId, providerConfig);

        const existingMessages = useStore.getState().conversations[conv.id]?.messages ?? [];
        if (existingMessages.length > 0) {
          const pairs: [string, string][] = existingMessages
            .filter((m) => m.role === "user" || m.role === "assistant")
            .map((m) => [m.role, m.content] as [string, string]);
          if (pairs.length > 0) {
            try {
              await restoreAiConversation(conv.aiSessionId, pairs);
            } catch {
              // Restore failed silently
            }
          }
        }

        const savedMode = useStore.getState().approvalMode || "ask";
        const backendMode: AgentMode = savedMode === "run-all" ? "auto-approve" : "default";
        await setAgentMode(conv.aiSessionId, backendMode).catch(() => {});

        const storeState = useStore.getState();
        const termIds = storeState.conversationTerminals[conv.id] ?? [];
        let restoredExecMode: "chat" | "task" = chatExecutionModeRef.current;
        let restoredUseAgents = chatUseSubAgentsRef.current;
        for (const tid of termIds) {
          const sess = storeState.sessions[tid];
          if (sess?.executionMode === "task") restoredExecMode = "task";
          if (sess?.useAgents) restoredUseAgents = true;
        }

        if (restoredExecMode !== "chat") {
          await setExecutionModeBackend(conv.aiSessionId, restoredExecMode).catch(() => {});
          setChatExecutionMode(restoredExecMode);
        }
        if (restoredUseAgents) {
          await setUseAgentsBackend(conv.aiSessionId, true).catch(() => {});
          setChatUseSubAgents(true);
        }

        updateConv(conv.id, { aiInitialized: true });
        return true;
      } catch (err) {
        console.error("[AIChatPanel] Failed to initialize AI session:", err);
        return false;
      }
    },
    [selectedModel, updateConv, chatExecutionModeRef, chatUseSubAgentsRef, setChatExecutionMode, setChatUseSubAgents],
  );

  return { initializeSession, generateTitle, generateTitleRef };
}
