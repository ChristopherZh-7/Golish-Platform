import { useCallback, useRef, useState } from "react";
import {
  type AgentMode,
  respondToToolApproval,
  setAgentMode,
  setExecutionMode as setExecutionModeBackend,
} from "@/lib/ai";
import { flushDbSave } from "@/lib/conversation-db-sync";
import { useStore } from "@/store";
import {
  normalizeExecutionModeId,
  readLastExecutionMode,
  writeLastExecutionMode,
} from "../executionModePicker.utils";

type ApprovalMode = "ask" | "run-all";

export function useChatModes() {
  const [chatAgentMode, setChatAgentMode] = useState<AgentMode>("default");
  // Seed from the remembered engine so a fresh panel reopens in the last mode.
  const [chatExecutionMode, setChatExecutionMode] = useState<string>(() => readLastExecutionMode());

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
    async (mode: string): Promise<boolean> => {
      const nextMode = normalizeExecutionModeId(mode);
      if (nextMode === chatExecutionMode) return true;
      const storeState = useStore.getState();
      const activeConvId = storeState.activeConversationId;
      const conv = activeConvId ? storeState.conversations[activeConvId] : null;

      // An initialized bridge already owns an execution profile. Treat its
      // acknowledgement as the commit point: UI/store/localStorage must not
      // move first and leave a stale backend profile behind on failure.
      if (conv?.aiInitialized) {
        try {
          await setExecutionModeBackend(conv.aiSessionId, nextMode);
        } catch (cause) {
          const message = cause instanceof Error ? cause.message : String(cause);
          storeState.setMessageError(conv.id, `Failed to switch execution profile: ${message}`);
          return false;
        }
      }

      chatExecutionModeRef.current = nextMode;
      setChatExecutionMode(nextMode);
      // Persist the explicit choice so new tabs / sessions reopen in it.
      writeLastExecutionMode(nextMode);
      if (activeConvId) {
        const termIds = storeState.conversationTerminals[activeConvId] ?? [];
        for (const tid of termIds) storeState.setExecutionMode(tid, nextMode);
      }
      flushDbSave().catch(console.warn);
      // Sub-agent dispatch is now unconditional across modes — see the
      // `SUB_AGENTS_ALWAYS_ON` note at the top of this hook. Switching
      // execution mode no longer needs to flip a per-conversation flag.
      return true;
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

  // Cursor-style "allow this command": approve now AND remember so this tool
  // auto-runs next time (backend adds it to the always-allow / learned set).
  const handleToolApproveAlways = useCallback((requestId: string) => {
    const pa = pendingApprovalRef.current;
    if (!pa) return;
    respondToToolApproval(pa.sessionId, {
      request_id: requestId,
      approved: true,
      reason: null,
      remember: true,
      always_allow: true,
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
    handleToolApproveAlways,
    handleToolDeny,
  };
}
