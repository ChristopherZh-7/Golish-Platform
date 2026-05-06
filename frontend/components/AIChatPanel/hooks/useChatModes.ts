import { useCallback, useRef, useState } from "react";
import {
  type AgentMode,
  respondToToolApproval,
  setAgentMode,
  setExecutionMode as setExecutionModeBackend,
} from "@/lib/ai";
import { flushDbSave } from "@/lib/conversation-db-sync";
import { useStore } from "@/store";

type ApprovalMode = "ask" | "allowlist" | "run-all";

export function useChatModes() {
  const [chatAgentMode, setChatAgentMode] = useState<AgentMode>("default");
  const [chatExecutionMode, setChatExecutionMode] = useState<string>("chat");

  const chatExecutionModeRef = useRef<string>(chatExecutionMode);
  chatExecutionModeRef.current = chatExecutionMode;

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
      // Sub-agent dispatch is now unconditional across modes — see the
      // `SUB_AGENTS_ALWAYS_ON` note at the top of this hook. Switching
      // execution mode no longer needs to flip a per-conversation flag.
    },
    [chatExecutionMode]
  );

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
    chatExecutionModeRef,
    approvalMode,
    setApprovalMode,
    pendingApproval,
    setPendingApproval,
    pendingApprovalRef,
    handleApprovalModeChange,
    handleAgentModeChange,
    handleExecutionModeChange,
    handleToolApprove,
    handleToolDeny,
  };
}
