import { useCallback, useEffect, useRef, useState } from "react";
import { type AiEvent, onAiEvent, respondToToolApproval } from "@/lib/ai";
import { type ChatMessage, useStore } from "@/store";
import type { AskHumanState, WorkflowRunSnapshot } from "./ChatSubComponents";

/**
 * Local UI state owned by the AI-event subscription.
 *
 * The Tauri backend pushes a single stream of `AiEvent`s for every active
 * AI session.  This hook owns:
 *
 *  - the streaming message ref (used by `handleStop` to clear partial output)
 *  - the task-in-progress flag (drives stop-button visibility for task mode)
 *  - the per-event UI slices that are *not* part of the global Zustand store
 *    (pending approval prompt, ask-human prompt, active workflow card,
 *    compaction notice, context-usage ring).
 *
 * Plan-event side effects mutate the global store via `setPlan`; only the
 * local `planTextOffsetRef` / `planMessageIdRef` / `retiredCountRef` book
 * keeping refs are kept here because they're scoped to one chat panel
 * lifetime.
 */
export interface PendingApprovalState {
  requestId: string;
  sessionId: string;
  toolName: string;
  args: Record<string, unknown>;
  riskLevel: string;
}

export interface CompactionUiState {
  active: boolean;
  tokensBefore?: number;
}

export interface ContextUsageState {
  utilization: number;
  totalTokens: number;
  maxTokens: number;
}

export interface UseChatAiEventsResult {
  // Per-event UI state
  pendingApproval: PendingApprovalState | null;
  pendingApprovalRef: React.MutableRefObject<PendingApprovalState | null>;
  askHumanRequest: AskHumanState | null;
  activeWorkflow: WorkflowRunSnapshot | null;
  compactionState: CompactionUiState | null;
  contextUsage: ContextUsageState | null;

  // Plan-tracking refs (consumed by JSX to align retired plan cards)
  planTextOffsetRef: React.MutableRefObject<number | null>;
  planMessageIdRef: React.MutableRefObject<string | null>;
  retiredCountRef: React.MutableRefObject<number>;

  // Streaming-state refs (consumed by handleStop / handleSend)
  streamingMsgRef: React.MutableRefObject<string | null>;
  taskInProgressRef: React.MutableRefObject<boolean>;

  // Setters needed by callers (handleSend, conv-switch effect, etc.)
  setAskHumanRequest: React.Dispatch<React.SetStateAction<AskHumanState | null>>;
  setCompactionState: React.Dispatch<React.SetStateAction<CompactionUiState | null>>;

  // Actions wired by the panel
  handleToolApprove: (requestId: string) => void;
  handleToolDeny: (requestId: string) => void;
  handleAskHumanSubmit: (response: string) => Promise<void>;
  handleAskHumanSkip: () => Promise<void>;
}

interface UseChatAiEventsOptions {
  /**
   * Called when the *first* user message of a conversation finishes
   * streaming so the panel can auto-generate a short title.  Passed as a
   * ref to avoid invalidating the AI-event listener subscription on every
   * render.
   */
  generateTitleRef: React.MutableRefObject<((convId: string, firstMsg: string) => void) | null>;
}

export function useChatAiEvents({
  generateTitleRef,
}: UseChatAiEventsOptions): UseChatAiEventsResult {
  const [pendingApproval, setPendingApproval] = useState<PendingApprovalState | null>(null);
  const pendingApprovalRef = useRef<PendingApprovalState | null>(pendingApproval);
  pendingApprovalRef.current = pendingApproval;

  const [askHumanRequest, setAskHumanRequest] = useState<AskHumanState | null>(null);
  const [activeWorkflow, setActiveWorkflow] = useState<WorkflowRunSnapshot | null>(null);
  const [compactionState, setCompactionState] = useState<CompactionUiState | null>(null);
  const [contextUsage, setContextUsage] = useState<ContextUsageState | null>(null);

  const planTextOffsetRef = useRef<number | null>(null);
  const planMessageIdRef = useRef<string | null>(null);
  const retiredCountRef = useRef<number>(0);
  const streamingMsgRef = useRef<string | null>(null);
  const taskInProgressRef = useRef(false);

  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | null = null;

    const setup = async () => {
      try {
        const dispose = await onAiEvent((event: AiEvent) => {
          if (!mounted) return;

          console.debug(
            "[AIChatPanel] AI event received:",
            event.type,
            "session:",
            event.session_id
          );

          const store = useStore.getState();
          let conv = store.getConversationBySessionId(event.session_id);

          // Fallback: check if the active conversation's aiSessionId matches
          if (!conv) {
            const activeConvId = store.activeConversationId;
            const activeConv = activeConvId ? store.conversations[activeConvId] : null;
            if (activeConv?.aiSessionId === event.session_id) {
              conv = activeConv;
            }
          }

          if (!conv) {
            console.debug("[AIChatPanel] No matching conversation for session:", event.session_id);
            return;
          }
          const convId = conv.id;

          switch (event.type) {
            case "started": {
              // Reset plan text offset for new turn
              planTextOffsetRef.current = null;
              const assistantMsg: ChatMessage = {
                id: `ai-${Date.now()}`,
                role: "assistant",
                content: "",
                timestamp: Date.now(),
                isStreaming: true,
              };
              streamingMsgRef.current = assistantMsg.id;
              store.addConversationMessage(convId, assistantMsg);
              store.setConversationStreaming(convId, true);
              break;
            }

            case "text_delta": {
              store.appendMessageDelta(convId, event.delta);
              break;
            }

            case "tool_request":
            case "tool_auto_approved": {
              store.addMessageToolCall(convId, {
                name: event.tool_name,
                args:
                  typeof event.args === "string" ? event.args : JSON.stringify(event.args, null, 2),
                requestId: event.request_id,
              });
              break;
            }

            case "tool_approval_request": {
              store.addMessageToolCall(convId, {
                name: event.tool_name,
                args:
                  typeof event.args === "string" ? event.args : JSON.stringify(event.args, null, 2),
                requestId: event.request_id,
              });

              const currentMode = useStore.getState().approvalMode || "ask";
              if (currentMode === "run-all") {
                respondToToolApproval(event.session_id, {
                  request_id: event.request_id,
                  approved: true,
                  remember: false,
                  always_allow: false,
                }).catch(console.error);
              } else {
                setPendingApproval({
                  requestId: event.request_id,
                  sessionId: event.session_id,
                  toolName: event.tool_name,
                  args: event.args as Record<string, unknown>,
                  riskLevel: event.risk_level ?? "medium",
                });
              }
              break;
            }

            case "tool_result": {
              const resultStr =
                typeof event.result === "string"
                  ? event.result
                  : JSON.stringify(event.result, null, 2);
              store.updateMessageToolResult(convId, event.tool_name, resultStr, event.success);
              if (pendingApprovalRef.current?.requestId === event.request_id) {
                setPendingApproval(null);
              }
              break;
            }

            case "reasoning": {
              store.appendMessageThinking(convId, event.content);
              break;
            }

            case "completed": {
              store.finalizeStreamingMessage(convId, event.response, event.reasoning);
              streamingMsgRef.current = null;
              // Keep the stop button visible during task-mode execution:
              // finalizeStreamingMessage clears isStreaming, but the Tauri call
              // is still pending so re-assert streaming until it returns.
              if (taskInProgressRef.current) {
                store.setConversationStreaming(convId, true);
              }
              // Auto-generate title after first exchange
              const freshConv = store.conversations[convId];
              if (freshConv) {
                const userMsgs = freshConv.messages.filter((m) => m.role === "user");
                if (
                  userMsgs.length === 1 &&
                  freshConv.title ===
                    userMsgs[0].content.slice(0, 30) +
                      (userMsgs[0].content.length > 30 ? "..." : "")
                ) {
                  generateTitleRef.current?.(convId, userMsgs[0].content);
                }
              }
              break;
            }

            case "context_warning": {
              setContextUsage({
                utilization: event.utilization,
                totalTokens: event.total_tokens,
                maxTokens: event.max_tokens,
              });
              break;
            }

            case "error": {
              taskInProgressRef.current = false;
              store.setMessageError(convId, event.message);
              streamingMsgRef.current = null;
              break;
            }

            // AskHuman events
            case "ask_human_request": {
              setAskHumanRequest({
                requestId: event.request_id,
                sessionId: event.session_id,
                question: event.question,
                inputType: (event.input_type || "freetext") as AskHumanState["inputType"],
                options: event.options ?? [],
                context: event.context ?? "",
              });
              break;
            }

            // Workflow events
            case "workflow_started": {
              setActiveWorkflow({
                id: event.workflow_id,
                name: event.workflow_name,
                currentStep: "",
                stepIndex: 0,
                totalSteps: 0,
                completedSteps: [],
                status: "running",
              });
              break;
            }
            case "workflow_step_started": {
              setActiveWorkflow((prev) =>
                prev?.id === event.workflow_id
                  ? {
                      ...prev,
                      currentStep: event.step_name,
                      stepIndex: event.step_index,
                      totalSteps: event.total_steps,
                    }
                  : prev
              );
              break;
            }
            case "workflow_step_completed": {
              setActiveWorkflow((prev) =>
                prev?.id === event.workflow_id
                  ? {
                      ...prev,
                      completedSteps: [
                        ...prev.completedSteps,
                        {
                          name: event.step_name,
                          output: event.output ?? undefined,
                          durationMs: event.duration_ms,
                        },
                      ],
                    }
                  : prev
              );
              break;
            }
            case "workflow_completed": {
              setActiveWorkflow((prev) =>
                prev?.id === event.workflow_id
                  ? {
                      ...prev,
                      status: "completed" as const,
                      totalDurationMs: event.total_duration_ms,
                    }
                  : prev
              );
              break;
            }
            case "workflow_error": {
              setActiveWorkflow((prev) =>
                prev?.id === event.workflow_id
                  ? { ...prev, status: "error" as const, error: event.error }
                  : prev
              );
              break;
            }

            // Plan events
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
                useStore.getState().setPlan(
                  termId,
                  {
                    version: event.version,
                    steps: event.steps,
                    summary: event.summary,
                    explanation: event.explanation ?? null,
                    updated_at: new Date().toISOString(),
                  },
                  prevMsgId,
                  newMsgId
                );

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

            // Compaction events
            case "compaction_started": {
              setCompactionState({ active: true, tokensBefore: event.tokens_before });
              break;
            }
            case "compaction_completed": {
              setCompactionState({ active: false, tokensBefore: event.tokens_before });
              setTimeout(() => setCompactionState(null), 5000);
              break;
            }
            case "compaction_failed": {
              setCompactionState(null);
              store.setMessageError(convId, `Context compaction failed: ${event.error}`);
              break;
            }
          }
        });

        if (mounted) {
          unlisten = dispose;
        } else {
          dispose();
        }
      } catch {
        // AI backend not available
      }
    };

    setup();

    return () => {
      mounted = false;
      unlisten?.();
      unlisten = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleToolApprove = useCallback((requestId: string) => {
    const pa = pendingApprovalRef.current;
    if (!pa) return;
    respondToToolApproval(pa.sessionId, {
      request_id: requestId,
      approved: true,
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
      remember: false,
      always_allow: false,
    }).catch(console.error);
    setPendingApproval(null);
  }, []);

  const handleAskHumanSubmit = useCallback(
    async (response: string) => {
      if (!askHumanRequest) return;
      try {
        await respondToToolApproval(askHumanRequest.sessionId, {
          request_id: askHumanRequest.requestId,
          approved: true,
          reason: response,
          remember: false,
          always_allow: false,
        });
      } catch (err) {
        console.error("[AIChatPanel] Failed to respond to ask_human:", err);
      }
      setAskHumanRequest(null);
    },
    [askHumanRequest]
  );

  const handleAskHumanSkip = useCallback(async () => {
    if (!askHumanRequest) return;
    try {
      await respondToToolApproval(askHumanRequest.sessionId, {
        request_id: askHumanRequest.requestId,
        approved: false,
        reason: undefined,
        remember: false,
        always_allow: false,
      });
    } catch (err) {
      console.error("[AIChatPanel] Failed to skip ask_human:", err);
    }
    setAskHumanRequest(null);
  }, [askHumanRequest]);

  return {
    pendingApproval,
    pendingApprovalRef,
    askHumanRequest,
    activeWorkflow,
    compactionState,
    contextUsage,
    planTextOffsetRef,
    planMessageIdRef,
    retiredCountRef,
    streamingMsgRef,
    taskInProgressRef,
    setAskHumanRequest,
    setCompactionState,
    handleToolApprove,
    handleToolDeny,
    handleAskHumanSubmit,
    handleAskHumanSkip,
  };
}
