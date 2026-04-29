import { useCallback, type MutableRefObject } from "react";
import {
  cancelAiGeneration,
  createTextPayload,
  sendPromptSession,
  sendPromptWithAttachments,
  setExecutionMode as setExecutionModeBackend,
} from "@/lib/ai";
import { type ChatMessage, useStore } from "@/store";

type Attachment = { data: string; mediaType: string; name: string };

interface UseChatSendOptions {
  input: string;
  setInput: (v: string) => void;
  isStreaming: boolean;
  activeConvId: string | null;
  imageAttachments: Attachment[];
  setImageAttachments: React.Dispatch<React.SetStateAction<Attachment[]>>;
  textareaRef: MutableRefObject<HTMLTextAreaElement | null>;
  userScrolledUpRef: MutableRefObject<boolean>;
  streamingMsgRef: MutableRefObject<string | null>;
  chatExecutionModeRef: MutableRefObject<"chat" | "task">;
  taskInProgressRef: MutableRefObject<boolean>;
  initializeSession: (conv: { id: string; aiSessionId: string; aiInitialized: boolean }) => Promise<boolean>;
  buildPentestSystemPrompt: () => string;
  createTerminalTab: (dir?: string, skip?: boolean) => Promise<string | null>;
  t: (key: string, fallback?: string) => string;
}

export function useChatSend(opts: UseChatSendOptions) {
  const {
    input, setInput, isStreaming, activeConvId,
    imageAttachments, setImageAttachments,
    textareaRef, userScrolledUpRef, streamingMsgRef,
    chatExecutionModeRef, taskInProgressRef,
    initializeSession, buildPentestSystemPrompt,
    createTerminalTab, t,
  } = opts;

  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text || isStreaming) return;
    if (!activeConvId) return;
    const conv = useStore.getState().conversations[activeConvId];
    if (!conv) return;

    const userMsg: ChatMessage = {
      id: `user-${Date.now()}`,
      role: "user",
      content: text,
      timestamp: Date.now(),
    };

    const newTitle =
      conv.title === "New Chat" ? text.slice(0, 30) + (text.length > 30 ? "..." : "") : conv.title;

    const store = useStore.getState();
    store.addConversationMessage(conv.id, userMsg);
    if (newTitle !== conv.title) store.updateConversation(conv.id, { title: newTitle });
    setInput("");
    if (textareaRef.current) textareaRef.current.style.height = "auto";
    userScrolledUpRef.current = false;

    const storeNow = useStore.getState();
    const convTerminals = storeNow.conversationTerminals[conv.id] ?? [];
    let activeTermId: string | null = null;
    if (convTerminals.length === 0) {
      const currentActive = storeNow.activeSessionId;
      if (currentActive && storeNow.sessions[currentActive]) {
        const ownerConv = storeNow.getConversationForTerminal(currentActive);
        if (!ownerConv || ownerConv === conv.id) {
          activeTermId = currentActive;
          storeNow.addTerminalToConversation(conv.id, currentActive);
        }
      }
      if (!activeTermId) {
        try {
          activeTermId = await createTerminalTab(undefined, true);
          if (activeTermId) {
            useStore.getState().addTerminalToConversation(conv.id, activeTermId);
          }
        } catch {
          // Terminal creation failed
        }
      }
    } else {
      activeTermId = convTerminals[0];
      if (storeNow.sessions[activeTermId] && storeNow.activeSessionId !== activeTermId) {
        storeNow.setActiveSession(activeTermId);
      }
    }

    if (activeTermId) {
      try {
        const { setActiveTerminalSession } = await import("@/lib/tauri");
        await setActiveTerminalSession(activeTermId);
      } catch { /* ignore */ }
    }

    const initialized = await initializeSession(conv);
    if (!initialized) {
      useStore.getState().setMessageError(
        conv.id,
        t("ai.noModelSelected", "Please select a model first (bottom-left dropdown)"),
      );
      return;
    }

    let prompt = text;
    if (conv.messages.length === 0) {
      const systemPrompt = buildPentestSystemPrompt();
      if (systemPrompt) {
        prompt = `[System Context]\n${systemPrompt}\n\n[User Message]\n${text}`;
      }
    }

    try {
      useStore.getState().setConversationStreaming(conv.id, true);
      const isTaskMode = chatExecutionModeRef.current === "task";
      if (isTaskMode) taskInProgressRef.current = true;

      await setExecutionModeBackend(conv.aiSessionId, chatExecutionModeRef.current)
        .catch(() => {});

      if (imageAttachments.length > 0) {
        const payload = createTextPayload(prompt);
        for (const img of imageAttachments) {
          payload.parts.push({ type: "image", data: img.data, media_type: img.mediaType });
        }
        await sendPromptWithAttachments(conv.aiSessionId, payload);
        setImageAttachments([]);
      } else {
        await sendPromptSession(conv.aiSessionId, prompt);
      }

      if (isTaskMode) {
        taskInProgressRef.current = false;
        useStore.getState().finalizeStreamingMessage(conv.id);
      }

      // Idle timeout: reset stuck streaming after 60s of inactivity
      const convId = conv.id;
      let lastMsgLength = 0;
      let lastToolCount = 0;
      let idleChecks = 0;
      const checkInterval = setInterval(() => {
        const s = useStore.getState();
        const c = s.conversations[convId];
        if (!c?.isStreaming) { clearInterval(checkInterval); return; }
        const lastMsg = c.messages[c.messages.length - 1];
        const currentLength = lastMsg?.content?.length ?? 0;
        const currentToolCount = lastMsg?.toolCalls?.length ?? 0;
        const hasPendingTools = lastMsg?.toolCalls?.some((tc) => tc.success === undefined) ?? false;
        if (hasPendingTools) {
          idleChecks = 0;
        } else if (currentLength === lastMsgLength && currentToolCount === lastToolCount) {
          idleChecks++;
          if (idleChecks >= 12) {
            s.finalizeStreamingMessage(convId);
            clearInterval(checkInterval);
          }
        } else {
          lastMsgLength = currentLength;
          lastToolCount = currentToolCount;
          idleChecks = 0;
        }
      }, 5_000);
    } catch (err) {
      taskInProgressRef.current = false;
      const errMsg = err instanceof Error ? err.message : String(err);
      useStore.getState().setMessageError(conv.id, errMsg);
    }
  }, [
    input, isStreaming, activeConvId, initializeSession, buildPentestSystemPrompt,
    imageAttachments, setImageAttachments, setInput, textareaRef, userScrolledUpRef,
    streamingMsgRef, chatExecutionModeRef, taskInProgressRef, createTerminalTab, t,
  ]);

  const handleStop = useCallback(() => {
    if (!activeConvId) return;
    const conv = useStore.getState().conversations[activeConvId];
    if (!conv) return;
    taskInProgressRef.current = false;
    cancelAiGeneration(conv.aiSessionId).catch(() => {});
    streamingMsgRef.current = null;
    const store = useStore.getState();
    store.finalizeStreamingMessage(conv.id);

    const terminals = store.conversationTerminals[conv.id];
    if (terminals) {
      for (const tid of terminals) {
        store.finalizeRunningToolExecutions(tid);
        store.setAgentThinking(tid, false);
        store.setAgentResponding(tid, false);
        store.clearActiveSubAgents(tid);

        const plan = store.sessions[tid]?.plan;
        if (plan && plan.summary.in_progress > 0) {
          const updated = {
            ...plan,
            version: plan.version + 1,
            steps: plan.steps.map((s: { status: string }) =>
              s.status === "in_progress" ? { ...s, status: "cancelled" } : s
            ),
            summary: {
              ...plan.summary,
              in_progress: 0,
            },
          };
          store.setPlan(tid, updated);
        }
      }
    }
  }, [activeConvId, streamingMsgRef, taskInProgressRef]);

  return { handleSend, handleStop };
}
