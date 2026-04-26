import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import { type AiEvent, onAiEvent, respondToToolApproval } from "@/lib/ai";
import { type ChatMessage, useStore } from "@/store";
import type { AskHumanState } from "../AskHumanInline";
import type { WorkflowRunSnapshot } from "../WorkflowProgress";

interface UseAiChatEventsOptions {
  activeConvId: string | null;
  streamingMsgRef: MutableRefObject<string | null>;
  taskInProgressRef: MutableRefObject<boolean>;
  modes: {
    setPendingApproval: (v: any) => void;
    pendingApprovalRef: MutableRefObject<{ requestId: string } | null>;
  };
  generateTitleRef: MutableRefObject<((convId: string, text: string) => void) | null>;
}

export function useAiChatEvents({
  activeConvId,
  streamingMsgRef,
  taskInProgressRef,
  modes,
  generateTitleRef,
}: UseAiChatEventsOptions) {
  const [contextUsage, setContextUsage] = useState<{
    utilization: number; totalTokens: number; maxTokens: number;
  } | null>(null);
  const [askHumanRequest, setAskHumanRequest] = useState<AskHumanState | null>(null);
  const [activeWorkflow, setActiveWorkflow] = useState<WorkflowRunSnapshot | null>(null);
  const [compactionState, setCompactionState] = useState<{
    active: boolean; tokensBefore?: number;
  } | null>(null);

  const planTextOffsetRef = useRef<number | null>(null);
  const planMessageIdRef = useRef<string | null>(null);
  const retiredCountRef = useRef<number>(0);
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let mounted = true;
    const setup = async () => {
      try {
        const unlisten = await onAiEvent((event: AiEvent) => {
          if (!mounted) return;
          const store = useStore.getState();
          let conv = store.getConversationBySessionId(event.session_id);
          if (!conv) {
            const activeConvId2 = store.activeConversationId;
            const activeConv2 = activeConvId2 ? store.conversations[activeConvId2] : null;
            if (activeConv2?.aiSessionId === event.session_id) conv = activeConv2;
          }
          if (!conv) return;
          const convId = conv.id;

          switch (event.type) {
            case "started": {
              planTextOffsetRef.current = null;
              const assistantMsg: ChatMessage = {
                id: `ai-${Date.now()}`, role: "assistant", content: "", timestamp: Date.now(), isStreaming: true,
              };
              streamingMsgRef.current = assistantMsg.id;
              store.addConversationMessage(convId, assistantMsg);
              store.setConversationStreaming(convId, true);
              break;
            }
            case "text_delta":
              store.appendMessageDelta(convId, event.delta);
              break;
            case "tool_request":
            case "tool_auto_approved":
              store.addMessageToolCall(convId, {
                name: event.tool_name,
                args: typeof event.args === "string" ? event.args : JSON.stringify(event.args, null, 2),
                requestId: event.request_id,
              });
              break;
            case "tool_approval_request": {
              store.addMessageToolCall(convId, {
                name: event.tool_name,
                args: typeof event.args === "string" ? event.args : JSON.stringify(event.args, null, 2),
                requestId: event.request_id,
              });
              const currentMode = useStore.getState().approvalMode || "ask";
              if (currentMode === "run-all") {
                respondToToolApproval(event.session_id, {
                  request_id: event.request_id, approved: true, remember: false, always_allow: false,
                }).catch(console.error);
              } else {
                modes.setPendingApproval({
                  requestId: event.request_id, sessionId: event.session_id,
                  toolName: event.tool_name, args: event.args as Record<string, unknown>,
                  riskLevel: event.risk_level ?? "medium",
                });
              }
              break;
            }
            case "tool_result": {
              const resultStr = typeof event.result === "string" ? event.result : JSON.stringify(event.result, null, 2);
              store.updateMessageToolResult(convId, event.tool_name, resultStr, event.success);
              if (modes.pendingApprovalRef.current?.requestId === event.request_id) modes.setPendingApproval(null);
              break;
            }
            case "reasoning":
              store.appendMessageThinking(convId, event.content);
              break;
            case "completed": {
              store.finalizeStreamingMessage(convId, event.response, event.reasoning);
              streamingMsgRef.current = null;
              if (taskInProgressRef.current) store.setConversationStreaming(convId, true);
              const freshConv = store.conversations[convId];
              if (freshConv) {
                const userMsgs = freshConv.messages.filter((m) => m.role === "user");
                if (userMsgs.length === 1 && freshConv.title === userMsgs[0].content.slice(0, 30) + (userMsgs[0].content.length > 30 ? "..." : "")) {
                  generateTitleRef.current?.(convId, userMsgs[0].content);
                }
              }
              break;
            }
            case "context_warning":
              setContextUsage({ utilization: event.utilization, totalTokens: event.total_tokens, maxTokens: event.max_tokens });
              break;
            case "error":
              taskInProgressRef.current = false;
              store.setMessageError(convId, event.message);
              streamingMsgRef.current = null;
              break;
            case "ask_human_request":
              setAskHumanRequest({
                requestId: event.request_id, sessionId: event.session_id,
                question: event.question, inputType: (event.input_type || "freetext") as AskHumanState["inputType"],
                options: event.options ?? [], context: event.context ?? "",
              });
              break;
            case "workflow_started":
              setActiveWorkflow({ id: event.workflow_id, name: event.workflow_name, currentStep: "", stepIndex: 0, totalSteps: 0, completedSteps: [], status: "running" });
              break;
            case "workflow_step_started":
              setActiveWorkflow((p) => p?.id === event.workflow_id ? { ...p, currentStep: event.step_name, stepIndex: event.step_index, totalSteps: event.total_steps } : p);
              break;
            case "workflow_step_completed":
              setActiveWorkflow((p) => p?.id === event.workflow_id ? { ...p, completedSteps: [...p.completedSteps, { name: event.step_name, output: event.output ?? undefined, durationMs: event.duration_ms }] } : p);
              break;
            case "workflow_completed":
              setActiveWorkflow((p) => p?.id === event.workflow_id ? { ...p, status: "completed" as const, totalDurationMs: event.total_duration_ms } : p);
              break;
            case "workflow_error":
              setActiveWorkflow((p) => p?.id === event.workflow_id ? { ...p, status: "error" as const, error: event.error } : p);
              break;
            case "plan_updated": {
              const currentConv = useStore.getState().conversations[convId];
              const lastMsg = currentConv?.messages?.[currentConv.messages.length - 1];
              const termIds = useStore.getState().conversationTerminals[convId];
              const termId = termIds?.[0];
              if (termId) {
                const sess = useStore.getState().sessions[termId];
                const prevMsgId = sess?.planMessageId ?? planMessageIdRef.current;
                const newMsgId = lastMsg?.role === "assistant" ? lastMsg.id : prevMsgId;
                if (planMessageIdRef.current === null && newMsgId) {
                  planMessageIdRef.current = newMsgId;
                  planTextOffsetRef.current = (lastMsg?.content || "").length;
                }
                useStore.getState().setPlan(termId, {
                  version: event.version, steps: event.steps, summary: event.summary,
                  explanation: event.explanation ?? null, updated_at: new Date().toISOString(),
                }, prevMsgId, newMsgId);
                const sessAfter = useStore.getState().sessions[termId];
                if (sessAfter?.retiredPlans?.length && lastMsg?.role === "assistant") {
                  if (sessAfter.retiredPlans.length > (retiredCountRef.current ?? 0)) {
                    planMessageIdRef.current = lastMsg.id;
                    planTextOffsetRef.current = (lastMsg.content || "").length;
                    retiredCountRef.current = sessAfter.retiredPlans.length;
                  }
                }
              }
              break;
            }
            case "compaction_started":
              setCompactionState({ active: true, tokensBefore: event.tokens_before });
              break;
            case "compaction_completed":
              setCompactionState({ active: false, tokensBefore: event.tokens_before });
              setTimeout(() => setCompactionState(null), 5000);
              break;
            case "compaction_failed":
              setCompactionState(null);
              store.setMessageError(convId, `Context compaction failed: ${event.error}`);
              break;
          }
        });
        if (mounted) { unlistenRef.current = unlisten; } else { unlisten(); }
      } catch { /* AI backend not available */ }
    };
    setup();
    return () => { mounted = false; unlistenRef.current?.(); unlistenRef.current = null; };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Reset plan refs on conversation switch
  useEffect(() => {
    planTextOffsetRef.current = null;
    planMessageIdRef.current = null;
  }, [activeConvId]);

  const handleAskHumanSubmit = useCallback(async (response: string) => {
    if (!askHumanRequest) return;
    try {
      await respondToToolApproval(askHumanRequest.sessionId, {
        request_id: askHumanRequest.requestId, approved: true, reason: response, remember: false, always_allow: false,
      });
    } catch (err) { console.error("[AIChatPanel] Failed to respond to ask_human:", err); }
    setAskHumanRequest(null);
  }, [askHumanRequest]);

  const handleAskHumanSkip = useCallback(async () => {
    if (!askHumanRequest) return;
    try {
      await respondToToolApproval(askHumanRequest.sessionId, {
        request_id: askHumanRequest.requestId, approved: false, reason: undefined, remember: false, always_allow: false,
      });
    } catch (err) { console.error("[AIChatPanel] Failed to skip ask_human:", err); }
    setAskHumanRequest(null);
  }, [askHumanRequest]);

  return {
    contextUsage,
    askHumanRequest,
    activeWorkflow,
    compactionState,
    planTextOffsetRef,
    planMessageIdRef,
    handleAskHumanSubmit,
    handleAskHumanSkip,
  };
}
