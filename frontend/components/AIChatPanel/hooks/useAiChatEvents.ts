import { type MutableRefObject, useCallback, useEffect, useRef, useState } from "react";
import {
  type AiEvent,
  isGenerationSuppressedForAiSession,
  isTitleGenSessionId,
  onAiEvent,
  respondToToolApproval,
} from "@/lib/ai";
import { classifyErrorSeverity } from "@/lib/ai/errorSeverity";
import { safeStringify } from "@/lib/text";
import { type ChatMessage, useStore } from "@/store";
import { type AskHumanState, resolveAskHumanInputType } from "../AskHumanInline";
import { readContextUsage, writeContextUsage } from "../contextUsagePersistence";
import { prettyStageName } from "../StageMarker";
import type { WorkflowRunSnapshot } from "../WorkflowProgress";

/** Read the `stage_id` (or `stage`) out of a `submit_stage_deliverable` call's args. */
function parseStageIdFromArgs(args: unknown): string | null {
  try {
    const obj = typeof args === "string" ? JSON.parse(args) : args;
    if (obj && typeof obj === "object") {
      const rec = obj as Record<string, unknown>;
      const sid = rec.stage_id ?? rec.stage;
      if (typeof sid === "string" && sid.trim()) return sid.trim();
    }
  } catch {
    /* non-JSON args — ignore */
  }
  return null;
}

/** Remember the stage a `submit_stage_deliverable` call targets, keyed by request id. */
function rememberSubmitStage(
  map: Map<string, string>,
  toolName: string,
  requestId: string,
  args: unknown
): void {
  if (toolName !== "submit_stage_deliverable") return;
  const sid = parseStageIdFromArgs(args);
  if (sid) map.set(requestId, sid);
}

/** A `submit_stage_deliverable` result counts as a stage pass only when accepted. */
function submitResultAccepted(result: unknown): boolean {
  try {
    const obj = typeof result === "string" ? JSON.parse(result) : result;
    return (
      !!obj && typeof obj === "object" && (obj as Record<string, unknown>).status === "accepted"
    );
  } catch {
    return false;
  }
}

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
    utilization: number;
    totalTokens: number;
    maxTokens: number;
  } | null>(null);
  const [askHumanRequest, setAskHumanRequest] = useState<AskHumanState | null>(null);
  const [activeWorkflow, setActiveWorkflow] = useState<WorkflowRunSnapshot | null>(null);
  const [compactionState, setCompactionState] = useState<{
    active: boolean;
    tokensBefore?: number;
  } | null>(null);

  const planTextOffsetRef = useRef<number | null>(null);
  const planMessageIdRef = useRef<string | null>(null);
  const retiredCountRef = useRef<number>(0);
  const unlistenRef = useRef<(() => void) | null>(null);
  // `submit_stage_deliverable` request_id → stage_id, captured at call time so
  // the accepted tool_result can label the stage-complete milestone.
  const submitStageByRequestRef = useRef<Map<string, string>>(new Map());
  // convId → last stage_id we emitted a "Stage complete" marker for, so a stage
  // re-submitted (gate accepts twice) doesn't spam duplicate milestones.
  const lastStageRef = useRef<Map<string, string>>(new Map());

  useEffect(() => {
    let mounted = true;
    const setup = async () => {
      try {
        const unlisten = await onAiEvent((event: AiEvent) => {
          if (!mounted) return;
          if (isTitleGenSessionId(event.session_id)) return;
          const store = useStore.getState();
          let conv = store.getConversationBySessionId(event.session_id);
          if (!conv) {
            const activeConvId2 = store.activeConversationId;
            const activeConv2 = activeConvId2 ? store.conversations[activeConvId2] : null;
            if (activeConv2?.aiSessionId === event.session_id) conv = activeConv2;
          }
          if (!conv) return;
          const convId = conv.id;

          if (isGenerationSuppressedForAiSession(event.session_id)) {
            return;
          }

          switch (event.type) {
            case "started": {
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
            case "text_delta":
              store.appendMessageDelta(convId, event.delta);
              break;
            case "tool_intent_observation":
              store.recordToolIntentObservation(event.session_id, {
                requestId: event.request_id,
                modelWanted: event.tool_name,
                source:
                  event.source === "native_tool_call" ||
                  event.source === "textual_xml" ||
                  event.source === "textual_json" ||
                  event.source === "recovered"
                    ? event.source
                    : "recovered",
                decision:
                  event.decision === "allow" ||
                  event.decision === "require_approval" ||
                  event.decision === "require_human_answer" ||
                  event.decision === "reject"
                    ? event.decision
                    : "reject",
                reason: event.reason ?? undefined,
                rawPreview: event.raw_preview ?? undefined,
              });
              break;
            case "tool_request":
            case "tool_auto_approved": {
              rememberSubmitStage(
                submitStageByRequestRef.current,
                event.tool_name,
                event.request_id,
                event.args
              );
              store.addMessageToolCall(convId, {
                name: event.tool_name,
                args:
                  typeof event.args === "string" ? event.args : JSON.stringify(event.args, null, 2),
                requestId: event.request_id,
              });
              break;
            }
            case "tool_approval_request": {
              rememberSubmitStage(
                submitStageByRequestRef.current,
                event.tool_name,
                event.request_id,
                event.args
              );
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
                  reason: null,
                  remember: false,
                  always_allow: false,
                }).catch(console.error);
              } else {
                modes.setPendingApproval({
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
                typeof event.result === "string" ? event.result : safeStringify(event.result);
              store.updateMessageToolResult(convId, event.tool_name, resultStr, event.success);
              // A stage *passes* when its `submit_stage_deliverable` is accepted —
              // surface that as a distinct, prominent milestone (separate from the
              // per-step "Step complete" markers).
              if (
                event.tool_name === "submit_stage_deliverable" &&
                submitResultAccepted(event.result)
              ) {
                const stageId = submitStageByRequestRef.current.get(event.request_id);
                submitStageByRequestRef.current.delete(event.request_id);
                if (stageId && lastStageRef.current.get(convId) !== stageId) {
                  lastStageRef.current.set(convId, stageId);
                  store.addConversationStageMarker(convId, {
                    kind: "stage_completed",
                    label: `Stage complete: ${prettyStageName(stageId)}`,
                    status: "finished",
                  });
                }
              }
              if (modes.pendingApprovalRef.current?.requestId === event.request_id)
                modes.setPendingApproval(null);
              break;
            }
            case "reasoning":
              store.appendMessageThinking(convId, event.content);
              break;
            case "completed": {
              store.finalizeStreamingMessage(convId, event.response, event.reasoning ?? undefined);
              streamingMsgRef.current = null;
              if (taskInProgressRef.current) store.setConversationStreaming(convId, true);
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
            // Task-mode boundaries: surface them as inline dividers so
            // consecutive stage narrations don't read as one continuous
            // monologue. A finished subtask is a *step* (subdued marker); a
            // whole stage passing its gate is emitted separately as a prominent
            // "Stage complete" milestone from the accepted `submit_stage_deliverable`
            // tool result above.
            case "subtask_completed":
              store.addConversationStageMarker(convId, {
                kind: "subtask_completed",
                label: `Step complete: ${event.title}`,
                title: event.title,
                detail: event.result || undefined,
              });
              break;
            case "task_progress": {
              const s = event.status;
              // `finished` is the authoritative end-of-run signal on the event
              // channel. Without resetting here, `taskInProgressRef` only flips
              // back on the `error` event or when the `send_ai_prompt_session`
              // invoke resolves — but a harness "hold for rework" Interrupt
              // suspends that invoke, so the terminal `completed` re-arms
              // streaming (see the `completed` case) and the "preparing" spinner
              // stays stuck forever. Clearing it here covers blocked + normal end.
              if (s === "finished") {
                taskInProgressRef.current = false;
                store.setConversationStreaming(convId, false);
              }
              // Only meaningful transitions — skip the noisy repeated "running".
              if (s === "finished" || s === "waiting_approval" || s === "reporting") {
                const label =
                  s === "finished"
                    ? "Task complete"
                    : s === "waiting_approval"
                      ? "Waiting for approval"
                      : "Generating report";
                store.addConversationStageMarker(convId, {
                  kind: "task_progress",
                  label,
                  status: s,
                  detail: event.message || undefined,
                });
              }
              break;
            }
            case "task_resumed":
              store.addConversationStageMarker(convId, {
                kind: "task_resumed",
                label: `Task resumed from subtask ${event.subtask_index}/${event.total_subtasks}`,
                status: "resumed",
              });
              break;
            case "context_warning": {
              const usage = {
                utilization: event.utilization,
                totalTokens: event.total_tokens,
                maxTokens: event.max_tokens,
              };
              // Persist per conversation so a refresh / tab switch can restore
              // the ring immediately instead of waiting for the next warning.
              writeContextUsage(convId, usage);
              if (convId === store.activeConversationId) setContextUsage(usage);
              break;
            }
            case "error":
              taskInProgressRef.current = false;
              store.setMessageError(convId, event.message, classifyErrorSeverity(event.message));
              streamingMsgRef.current = null;
              break;
            case "ask_human_request": {
              const askOptions = event.options ?? [];
              setAskHumanRequest({
                requestId: event.request_id,
                sessionId: event.session_id,
                question: event.question,
                inputType: resolveAskHumanInputType(event.input_type, askOptions),
                options: askOptions,
                context: event.context ?? "",
              });
              break;
            }
            case "workflow_started":
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
            case "workflow_step_started":
              setActiveWorkflow((p) =>
                p?.id === event.workflow_id
                  ? {
                      ...p,
                      currentStep: event.step_name,
                      stepIndex: event.step_index,
                      totalSteps: event.total_steps,
                    }
                  : p
              );
              break;
            case "workflow_step_completed":
              setActiveWorkflow((p) =>
                p?.id === event.workflow_id
                  ? {
                      ...p,
                      completedSteps: [
                        ...p.completedSteps,
                        {
                          name: event.step_name,
                          output: event.output ?? undefined,
                          durationMs: Number(event.duration_ms),
                        },
                      ],
                    }
                  : p
              );
              break;
            case "workflow_completed":
              setActiveWorkflow((p) =>
                p?.id === event.workflow_id
                  ? {
                      ...p,
                      status: "completed" as const,
                      totalDurationMs: Number(event.total_duration_ms),
                    }
                  : p
              );
              break;
            case "workflow_error":
              setActiveWorkflow((p) =>
                p?.id === event.workflow_id
                  ? { ...p, status: "error" as const, error: event.error }
                  : p
              );
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
            case "compaction_started":
              setCompactionState({ active: true, tokensBefore: Number(event.tokens_before) });
              break;
            case "compaction_completed":
              setCompactionState({ active: false, tokensBefore: Number(event.tokens_before) });
              setTimeout(() => setCompactionState(null), 5000);
              break;
            case "compaction_failed":
              setCompactionState(null);
              store.setMessageError(convId, `Context compaction failed: ${event.error}`);
              break;
          }
        });
        if (mounted) {
          unlistenRef.current = unlisten;
        } else {
          unlisten();
        }
      } catch {
        /* AI backend not available */
      }
    };
    setup();
    return () => {
      mounted = false;
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, [
    generateTitleRef.current,
    modes.pendingApprovalRef.current?.requestId,
    modes.setPendingApproval,
    streamingMsgRef,
    taskInProgressRef,
  ]); // eslint-disable-line react-hooks/exhaustive-deps

  // Reset plan refs on conversation switch
  useEffect(() => {
    planTextOffsetRef.current = null;
    planMessageIdRef.current = null;
  }, []);

  // Restore the persisted context-usage snapshot for the active conversation so
  // the ring shows the last-known utilization immediately after a refresh or a
  // tab switch, instead of reading "unavailable" until the next warning event.
  useEffect(() => {
    setContextUsage(activeConvId ? readContextUsage(activeConvId) : null);
  }, [activeConvId]);

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
        reason: null,
        remember: false,
        always_allow: false,
      });
    } catch (err) {
      console.error("[AIChatPanel] Failed to skip ask_human:", err);
    }
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
