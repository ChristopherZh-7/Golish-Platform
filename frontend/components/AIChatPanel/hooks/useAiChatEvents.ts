import { type MutableRefObject, useCallback, useEffect, useRef, useState } from "react";
import {
  type AiEvent,
  isGenerationSuppressedForAiSession,
  isTitleGenSessionId,
  onAiEvent,
  respondToToolApproval,
} from "@/lib/ai";
import { classifyErrorSeverity } from "@/lib/ai/errorSeverity";
import {
  batchConversationThinking,
  runRealtimeBatchFlush,
  runRealtimeBatchFlushForConversation,
} from "@/lib/ai/streaming-buffer";
import { safeStringify } from "@/lib/text";
import { type ChatMessage, useStore } from "@/store";
import { type AskHumanState, resolveAskHumanInputType } from "../AskHumanInline";
import { readContextUsage, writeContextUsage } from "../contextUsagePersistence";
import { prettyStageName } from "../StageMarker";
import type { WorkflowRunSnapshot } from "../WorkflowProgress";

/** Pull a UUID `organization_id` out of a tool call's args (string-encoded JSON
 * or an already-parsed object). Returns null when absent or not a UUID, so a
 * mangled/placeholder value never poisons the unit_review fallback. */
function extractOrgId(args: unknown): string | null {
  let obj: unknown = args;
  if (typeof args === "string") {
    try {
      obj = JSON.parse(args);
    } catch {
      return null;
    }
  }
  if (!obj || typeof obj !== "object") return null;
  const raw = (obj as Record<string, unknown>).organization_id;
  if (typeof raw !== "string") return null;
  const id = raw.trim();
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id) ? id : null;
}

/** Pull the `min_ownership_percent` (number or "51"/"51%" string) the agent
 * passed to `recon_discover_subsidiaries`, so the unit_review table can hide
 * sub-threshold candidates instead of making the user hand-delete them. Null
 * when absent/unparseable (→ no filtering). */
function extractMinOwnership(args: unknown): number | null {
  let obj: unknown = args;
  if (typeof args === "string") {
    try {
      obj = JSON.parse(args);
    } catch {
      return null;
    }
  }
  if (!obj || typeof obj !== "object") return null;
  const raw = (obj as Record<string, unknown>).min_ownership_percent;
  if (typeof raw === "number") return Number.isFinite(raw) ? raw : null;
  if (typeof raw === "string") {
    const cleaned = raw.trim().replace(/%$/, "");
    if (!cleaned) return null;
    const n = Number(cleaned);
    return Number.isFinite(n) ? n : null;
  }
  return null;
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
  // The org id the engine actually ran subsidiary discovery for, captured from
  // the `recon_discover_subsidiaries` tool call (a validated required arg). The
  // unit_review box falls back to this when the model fails to thread a usable
  // organization_id into the ask_human context (mimo's textual tool calls mangle
  // it — see the DB-sourced-candidates flow). One company per session ⇒ this is
  // unambiguous.
  const [lastDiscoverOrgId, setLastDiscoverOrgId] = useState<string | null>(null);
  // Ownership threshold from the last subsidiary discovery, so the unit_review
  // table only lists candidates meeting it (the user asked for "≥51%", not "all").
  const [lastDiscoverThreshold, setLastDiscoverThreshold] = useState<number | null>(null);
  const [activeWorkflow, setActiveWorkflow] = useState<WorkflowRunSnapshot | null>(null);
  const [compactionState, setCompactionState] = useState<{
    active: boolean;
    tokensBefore?: number;
  } | null>(null);

  const planTextOffsetRef = useRef<number | null>(null);
  const planMessageIdRef = useRef<string | null>(null);
  const retiredCountRef = useRef<number>(0);
  const unlistenRef = useRef<(() => void) | null>(null);
  // convId → last stage_id we emitted a "Stage complete" marker for, so a stage
  // re-passed (gate accepts twice) doesn't spam duplicate milestones.
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
              runRealtimeBatchFlushForConversation(convId);
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
              runRealtimeBatchFlushForConversation(convId);
              store.addMessageToolCall(convId, {
                name: event.tool_name,
                args:
                  typeof event.args === "string" ? event.args : JSON.stringify(event.args, null, 2),
                requestId: event.request_id,
              });
              if (event.tool_name === "recon_discover_subsidiaries") {
                const orgId = extractOrgId(event.args);
                if (orgId) setLastDiscoverOrgId(orgId);
                const threshold = extractMinOwnership(event.args);
                if (threshold != null) setLastDiscoverThreshold(threshold);
              }
              break;
            }
            case "tool_approval_request": {
              runRealtimeBatchFlushForConversation(convId);
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
              runRealtimeBatchFlushForConversation(convId);
              const resultStr =
                typeof event.result === "string" ? event.result : safeStringify(event.result);
              store.updateMessageToolResult(
                convId,
                event.tool_name,
                resultStr,
                event.success,
                event.request_id
              );
              // NB: the "Stage complete" milestone is NOT driven from here. A
              // `submit_stage_deliverable` `accepted` is only the structural
              // preview; the milestone fires off the backend's authoritative
              // `stage_passed` TaskProgress (see the `task_progress` case).
              if (modes.pendingApprovalRef.current?.requestId === event.request_id)
                modes.setPendingApproval(null);
              break;
            }
            case "tool_background_completed": {
              store.updateMessageToolResultByJobId(
                convId,
                event.job_id,
                safeStringify({
                  status: event.status,
                  job_id: event.job_id,
                  command: event.command,
                  exit_code: event.exit_code ?? null,
                  stdout: event.stdout_tail,
                  stderr: event.stderr_tail,
                  duration_ms: event.duration_ms,
                  backgrounded_completed: true,
                }),
                event.status === "done"
              );
              break;
            }
            case "reasoning":
              batchConversationThinking(convId, event.content);
              break;
            case "completed": {
              runRealtimeBatchFlushForConversation(convId);
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
              // Authoritative stage pass: the backend emits this from
              // `consume_gate_outcome` only when the deterministic evidence gate
              // ACCEPTS the stage. THIS — not the structural
              // `submit_stage_deliverable` preview — drives the prominent "Stage
              // complete" milestone + the per-stage card's completed state, so
              // completion shows only after real evidence is validated. `message`
              // carries the stage id.
              if (s === "stage_passed") {
                const stageId = event.message;
                if (stageId && lastStageRef.current.get(convId) !== stageId) {
                  lastStageRef.current.set(convId, stageId);
                  store.addConversationStageMarker(convId, {
                    kind: "stage_completed",
                    label: `Stage complete: ${prettyStageName(stageId)}`,
                    status: "finished",
                  });
                  // Per-stage plan state (stageOrder / plansByStage / passedStages)
                  // is keyed by the resolved STORE session id — the terminal/PTY id.
                  // The ai-events registry writes plans under that id (`useAiEvents`
                  // maps the aiSessionId → termId for conversation-mode sessions), so
                  // the `passedStages` write MUST use the same resolution. The raw
                  // `event.session_id` is the aiSessionId, whose `sessions[…]` row
                  // doesn't exist for conversation-mode chats — writing there would
                  // silently no-op and leave the stage stuck on "starting…".
                  const stageStoreSessionId = store.sessions[event.session_id]
                    ? event.session_id
                    : (store.conversationTerminals[convId]?.[0] ?? event.session_id);
                  store.markStagePassed(stageStoreSessionId, stageId);
                }
                break;
              }
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
              if (
                s === "finished" ||
                s === "waiting_approval" ||
                s === "waiting_target_scope" ||
                s === "reporting"
              ) {
                const label =
                  s === "finished"
                    ? "Task complete"
                    : s === "waiting_approval"
                      ? "Waiting for approval"
                      : s === "waiting_target_scope"
                        ? "Review scan targets"
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
              // The run is ending — any pending ask_human box is now stale, so
              // clear it instead of leaving it dangling with no way to resolve.
              setAskHumanRequest(null);
              break;
            case "ask_human_request": {
              const askOptions = event.options ?? [];
              setAskHumanRequest({
                requestId: event.request_id,
                sessionId: event.session_id,
                question: event.question,
                rawInputType: event.input_type ?? "",
                inputType: resolveAskHumanInputType(event.input_type, askOptions),
                options: askOptions,
                context: event.context ?? "",
              });
              break;
            }
            case "ask_human_response":
              setAskHumanRequest(null);
              break;
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
              // Per-stage plan updates (stage_id present) are routed into the
              // per-stage buckets by the ai-events service handler. Skip the
              // legacy single-card path here so a harness run doesn't ALSO render
              // a duplicate InlinePlanCard alongside the per-stage StageRow cards.
              if (event.stage_id) break;
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
      runRealtimeBatchFlush();
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
    // Drop the captured discover org on conversation switch so one chat's
    // engagement subject never leaks into another's unit_review fallback.
    setLastDiscoverOrgId(null);
    setLastDiscoverThreshold(null);
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
    lastDiscoverOrgId,
    lastDiscoverThreshold,
    activeWorkflow,
    compactionState,
    planTextOffsetRef,
    planMessageIdRef,
    handleAskHumanSubmit,
    handleAskHumanSkip,
  };
}
