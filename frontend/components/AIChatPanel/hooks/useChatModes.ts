import { useCallback, useRef, useState } from "react";
import {
  type AgentMode,
  respondToToolApproval,
  setAgentMode,
  setExecutionMode as setExecutionModeBackend,
  setUseAgents as setUseAgentsBackend,
} from "@/lib/ai";
import { flushDbSave } from "@/lib/conversation-db-sync";
import { useStore } from "@/store";

type ApprovalMode = "ask" | "allowlist" | "run-all";

export function useChatModes() {
  const [chatAgentMode, setChatAgentMode] = useState<AgentMode>("default");
  const [chatExecutionMode, setChatExecutionMode] = useState<string>("chat");
  const [chatUseSubAgents, setChatUseSubAgents] = useState(false);

  const chatExecutionModeRef = useRef<string>(chatExecutionMode);
  chatExecutionModeRef.current = chatExecutionMode;
  const chatUseSubAgentsRef = useRef(chatUseSubAgents);
  chatUseSubAgentsRef.current = chatUseSubAgents;

  const [approvalMode, setApprovalMode] = useState<ApprovalMode>("ask");
  const [pendingApproval, setPendingApproval] = useState<{
    requestId: string;
    sessionId: string;
    toolName: string;
    args: Record<string, unknown>;
    riskLevel: string;
  } | null>(null);
  const pendingApprovalRef = useRef(pendingApproval);
  pendingApprovalRef.current = pendingApproval;

  const handleApprovalModeChange = useCallback((mode: ApprovalMode) => {
    setApprovalMode(mode);
    useStore.getState().setApprovalMode(mode);
    const store = useStore.getState();
    const conv = store.activeConversationId
      ? store.conversations[store.activeConversationId]
      : null;
    if (!conv) return;
    const backendMode: AgentMode = mode === "run-all" ? "auto-approve" : "default";
    setAgentMode(conv.aiSessionId, backendMode).catch(console.error);
  }, []);

  const handleAgentModeChange = useCallback(
    (mode: AgentMode) => {
      if (mode === chatAgentMode) return;
      setChatAgentMode(mode);
      const store = useStore.getState();
      const conv = store.activeConversationId
        ? store.conversations[store.activeConversationId]
        : null;
      if (!conv) return;
      setAgentMode(conv.aiSessionId, mode).catch(console.error);
      if (mode === "auto-approve") {
        setApprovalMode("run-all");
        store.setApprovalMode("run-all");
      } else {
        setApprovalMode("ask");
        store.setApprovalMode("ask");
      }
    },
    [chatAgentMode]
  );

  const handleExecutionModeChange = useCallback(
    (mode: string) => {
      if (mode === chatExecutionMode) return;
      setChatExecutionMode(mode);
      const storeState = useStore.getState();
      const activeConvId = storeState.activeConversationId;
      if (activeConvId) {
        const termIds = storeState.conversationTerminals[activeConvId] ?? [];
        for (const tid of termIds) storeState.setExecutionMode(tid, mode);
      }
      flushDbSave().catch(console.warn);
      const conv = activeConvId ? storeState.conversations[activeConvId] : null;
      if (!conv) return;
      if (conv.aiInitialized) {
        setExecutionModeBackend(conv.aiSessionId, mode).catch(console.error);
      }

      // Switching to chat mode forcibly turns sub-agents off so frontend
      // state mirrors the backend-side isolation done by Batch 1
      // (`prepare.rs`: `has_sub_agents = execution_mode.is_task() && use_agents`).
      // Without this auto-clear the picker would still show the green toggle
      // even though the prompt template has been narrowed to single-agent —
      // misleading the user into thinking specialists are reachable.
      if (mode === "chat" && chatUseSubAgents) {
        setChatUseSubAgents(false);
        if (activeConvId) {
          const termIds = storeState.conversationTerminals[activeConvId] ?? [];
          for (const tid of termIds) storeState.setUseAgents(tid, false);
        }
        if (conv.aiInitialized) {
          setUseAgentsBackend(conv.aiSessionId, false).catch(console.error);
        }
      }
    },
    [chatExecutionMode, chatUseSubAgents]
  );

  const handleToggleSubAgents = useCallback(() => {
    // In chat mode the sub-agent toggle is read-only: user must switch to
    // task mode first. Silently ignore the click (UI also paints the
    // toggle as disabled — see ExecutionModePicker.tsx).
    if (chatExecutionMode === "chat") return;

    const newValue = !chatUseSubAgents;
    setChatUseSubAgents(newValue);
    const storeState = useStore.getState();
    const activeConvId = storeState.activeConversationId;
    if (activeConvId) {
      const termIds = storeState.conversationTerminals[activeConvId] ?? [];
      for (const tid of termIds) storeState.setUseAgents(tid, newValue);
    }
    flushDbSave().catch(console.warn);
    const conv = activeConvId ? storeState.conversations[activeConvId] : null;
    if (!conv) return;
    if (conv.aiInitialized) {
      setUseAgentsBackend(conv.aiSessionId, newValue).catch(console.error);
    }
  }, [chatUseSubAgents, chatExecutionMode]);

  const handleToolApprove = useCallback((requestId: string) => {
    const pa = pendingApprovalRef.current;
    if (!pa) return;
    respondToToolApproval(pa.sessionId, {
      request_id: requestId,
      approved: true,
      reason: null,
      remember: false,
      always_allow: false,
    }).catch(console.error);
    setPendingApproval(null);
  }, []);

  const handleToolDeny = useCallback((requestId: string) => {
    const pa = pendingApprovalRef.current;
    if (!pa) return;
    respondToToolApproval(pa.sessionId, {
      request_id: requestId,
      approved: false,
      reason: null,
      remember: false,
      always_allow: false,
    }).catch(console.error);
    setPendingApproval(null);
  }, []);

  return {
    chatAgentMode,
    chatExecutionMode,
    setChatExecutionMode,
    chatUseSubAgents,
    setChatUseSubAgents,
    chatExecutionModeRef,
    chatUseSubAgentsRef,
    approvalMode,
    setApprovalMode,
    pendingApproval,
    setPendingApproval,
    pendingApprovalRef,
    handleApprovalModeChange,
    handleAgentModeChange,
    handleExecutionModeChange,
    handleToggleSubAgents,
    handleToolApprove,
    handleToolDeny,
  };
}
