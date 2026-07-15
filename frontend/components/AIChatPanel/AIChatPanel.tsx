import { ArrowUp, Image, LoaderCircle, Square, Wrench, X } from "lucide-react";
import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { ReportReadModelView } from "@/components/Engagement/ReportReadModelView";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";
import { respondToToolApproval } from "@/lib/ai";
import { resetHarnessStageCheckpoint } from "@/lib/api/harness-dev";
import { formatModelName } from "@/lib/models";
import { cn } from "@/lib/utils";
import { type ChatMessage, useStore } from "@/store";
import { AgentStatusIndicator } from "./AgentStatusIndicator";
import { resolveAskHumanInputType } from "./AskHumanInline";
import { clearMatchingPendingAskHuman } from "./askHumanStore";
import { ChatModelSelector } from "./ChatModelSelector";
import {
  AskHumanInline,
  type AskHumanState,
  CompactionNotice,
  WorkflowProgress,
} from "./ChatSubComponents";
import { ContextUsageRing } from "./ContextUsageRing";
import { ConversationTabs } from "./ConversationTabs";
import { activateConversationTerminalFromChat } from "./conversationTerminalActivation";
import { ExecutionModePicker } from "./ExecutionModePicker";
import {
  normalizeExecutionModeId,
  readLastExecutionMode,
  resolveEngine,
} from "./executionModePicker.utils";
import { useAiChatEvents } from "./hooks/useAiChatEvents";
import { useAiChatInit } from "./hooks/useAiChatInit";
import { useChatConversationOps } from "./hooks/useChatConversationOps";
import { useChatHotkeys } from "./hooks/useChatHotkeys";
import { useChatModes } from "./hooks/useChatModes";
import { useChatSend } from "./hooks/useChatSend";
import { useChatSessionInit } from "./hooks/useChatSessionInit";
import { useTaskPlanState } from "./hooks/useTaskPlanState";
import { MessageBlock } from "./MessageBlock";
import { buildPentestSystemPrompt } from "./pentestSystemPrompt";
import { shouldShowChatRestoreLoading } from "./restoreLoadingState";
import { StageMarker } from "./StageMarker";
import { StageProgressBar } from "./StageProgressBar";
import { StageResetMenu } from "./StageResetMenu";
import type { StagePlansViewModel } from "./TaskPlan";
import { useChatAutoScroll } from "./useChatAutoScroll";

const EMPTY_MESSAGES: ChatMessage[] = [];
const TASK_RESUME_PROMPT = "继续跑";

function currentRestartStage(stagePlans: StagePlansViewModel | null): string | null {
  if (!stagePlans || stagePlans.stageOrder.length === 0) return null;
  const passed = new Set(stagePlans.passedStages);
  const activeStage = stagePlans.stageOrder.find(
    (stageId) =>
      !passed.has(stageId) &&
      (stagePlans.plansByStage[stageId]?.steps.some((step) => step.status !== "pending") ?? false)
  );
  if (activeStage) return activeStage;
  return stagePlans.stageOrder.find((stageId) => !passed.has(stageId)) ?? null;
}

export const AIChatPanel = memo(function AIChatPanel() {
  const { t } = useTranslation();

  // ── Store selectors ──────────────────────────────────────────────────
  const conversations = useStore(
    useShallow((s) => s.conversationOrder.map((id) => s.conversations[id]).filter(Boolean))
  );
  const activeConvId = useStore((s) => s.activeConversationId);
  const activeConv = useStore((s) =>
    s.activeConversationId ? (s.conversations[s.activeConversationId] ?? null) : null
  );
  const storeAskHumanRequest = useStore((s) => {
    const convId = s.activeConversationId;
    if (!convId) return null;
    const aiSessionId = s.conversations[convId]?.aiSessionId;
    const terminalId = s.conversationTerminals[convId]?.[0];
    if (aiSessionId) {
      const pending = s.pendingAskHuman[aiSessionId] ?? null;
      if (pending) return pending;
    }
    if (terminalId) return s.pendingAskHuman[terminalId] ?? null;
    return null;
  });
  const activeConversationTerminalId = useStore((s) => {
    const convId = s.activeConversationId;
    return convId ? (s.conversationTerminals[convId]?.[0] ?? null) : null;
  });
  const reportingReadModelHint = useStore((s) => {
    const convId = s.activeConversationId;
    if (!convId) return null;
    const terminalId = s.conversationTerminals[convId]?.[0];
    const terminalHint = terminalId ? s.sessions[terminalId]?.reportingReadModelHint : undefined;
    if (terminalHint) return terminalHint;
    const aiSessionId = s.conversations[convId]?.aiSessionId;
    return aiSessionId ? (s.sessions[aiSessionId]?.reportingReadModelHint ?? null) : null;
  });
  const messages = activeConv?.messages ?? EMPTY_MESSAGES;
  const isStreaming = activeConv?.isStreaming ?? false;
  const activeSessionId = useStore((s) => s.activeSessionId);
  const workspaceDataReady = useStore((s) => s.workspaceDataReady);
  const pendingTerminalRestoreData = useStore((s) => s.pendingTerminalRestoreData);

  const storeAiModel = useStore((s) => s.selectedAiModel);
  const [selectedModel, setSelectedModel] = useState<{ model: string; provider: string } | null>(
    storeAiModel
  );
  useEffect(() => {
    if (storeAiModel) setSelectedModel(storeAiModel);
  }, [storeAiModel]);
  const modelDisplay = selectedModel?.model ? formatModelName(selectedModel.model) : "No Model";

  // ── Local UI state ───────────────────────────────────────────────────
  const [showHistory, setShowHistory] = useState(false);
  const [input, setInput] = useState("");
  const [imageAttachments, setImageAttachments] = useState<
    Array<{ data: string; mediaType: string; name: string }>
  >([]);
  const [taskResumeBusy, setTaskResumeBusy] = useState(false);

  // ── Refs ──────────────────────────────────────────────────────────────
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const streamingMsgRef = useRef<string | null>(null);
  const taskInProgressRef = useRef(false);

  // ── Composed hooks ───────────────────────────────────────────────────
  const { createTerminalTab } = useCreateTerminalTab();
  const { pentestTools, configuredProviders } = useAiChatInit(createTerminalTab);
  const { messagesContainerRef, userScrolledUpRef } = useChatAutoScroll(messages);

  const modes = useChatModes();

  const storeApprovalMode = useStore((s) => s.approvalMode);
  useEffect(() => {
    if (storeApprovalMode)
      modes.setApprovalMode(storeApprovalMode === "run-all" ? "run-all" : "ask");
  }, [storeApprovalMode, modes.setApprovalMode]); // eslint-disable-line react-hooks/exhaustive-deps

  const updateConv = useStore.getState().updateConversation;

  const sessionInit = useChatSessionInit({
    selectedModel,
    chatExecutionModeRef: modes.chatExecutionModeRef,
    setChatExecutionMode: modes.setChatExecutionMode,
    updateConv,
  });

  const buildSystemPrompt = useCallback(
    () => buildPentestSystemPrompt(pentestTools),
    [pentestTools]
  );

  const { handleSend, handleStop } = useChatSend({
    input,
    setInput,
    isStreaming,
    activeConvId,
    imageAttachments,
    setImageAttachments,
    textareaRef: textareaRef as React.MutableRefObject<HTMLTextAreaElement | null>,
    userScrolledUpRef,
    streamingMsgRef,
    chatExecutionModeRef: modes.chatExecutionModeRef,
    taskInProgressRef,
    initializeSession: sessionInit.initializeSession,
    buildPentestSystemPrompt: buildSystemPrompt,
    createTerminalTab,
    t: ((key: string, fallback?: string) => t(key, fallback ?? key)) as (
      key: string,
      fallback?: string
    ) => string,
  });

  const { handleNewChat, handleCloseTab } = useChatConversationOps(createTerminalTab);
  const { handleKeyDown, handleTextareaInput } = useChatHotkeys({
    textareaRef,
    onSend: handleSend,
  });

  // ── AI events + plan state (extracted hooks) ─────────────────────────
  const {
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
  } = useAiChatEvents({
    activeConvId,
    streamingMsgRef,
    taskInProgressRef,
    modes: {
      setPendingApproval: modes.setPendingApproval,
      pendingApprovalRef: modes.pendingApprovalRef,
    },
    generateTitleRef: sessionInit.generateTitleRef,
  });

  const { activeAiSessionId, taskPlan, stagePlans, planTargetIdx } = useTaskPlanState(
    messages,
    planMessageIdRef
  );
  const askHumanSessionId =
    storeAskHumanRequest?.sessionId ??
    activeConv?.aiSessionId ??
    activeConversationTerminalId ??
    "";
  const visibleAskHumanRequest =
    askHumanRequest ??
    (storeAskHumanRequest && askHumanSessionId
      ? {
          requestId: storeAskHumanRequest.requestId,
          sessionId: askHumanSessionId,
          question: storeAskHumanRequest.question,
          rawInputType: storeAskHumanRequest.rawInputType ?? storeAskHumanRequest.inputType,
          inputType: resolveAskHumanInputType(
            storeAskHumanRequest.rawInputType ?? storeAskHumanRequest.inputType,
            storeAskHumanRequest.options
          ),
          options: storeAskHumanRequest.options,
          context: storeAskHumanRequest.context,
        }
      : null);
  const clearStoreAskHumanRequest = useCallback(
    (request: Pick<AskHumanState, "requestId" | "sessionId">) => {
      clearMatchingPendingAskHuman(useStore.getState(), request, [
        activeConvId,
        activeConv?.aiSessionId,
        activeConversationTerminalId,
        storeAskHumanRequest?.sessionId,
      ]);
    },
    [
      activeConvId,
      activeConv?.aiSessionId,
      activeConversationTerminalId,
      storeAskHumanRequest?.sessionId,
    ]
  );
  const handleVisibleAskHumanSubmit = useCallback(
    async (response: string) => {
      if (askHumanRequest) {
        try {
          await handleAskHumanSubmit(response);
        } finally {
          clearStoreAskHumanRequest(askHumanRequest);
        }
        return;
      }
      if (!visibleAskHumanRequest) return;
      try {
        await respondToToolApproval(visibleAskHumanRequest.sessionId, {
          request_id: visibleAskHumanRequest.requestId,
          approved: true,
          reason: response,
          remember: false,
          always_allow: false,
        });
      } finally {
        clearStoreAskHumanRequest(visibleAskHumanRequest);
      }
    },
    [askHumanRequest, clearStoreAskHumanRequest, handleAskHumanSubmit, visibleAskHumanRequest]
  );
  const handleVisibleAskHumanSkip = useCallback(async () => {
    if (askHumanRequest) {
      try {
        await handleAskHumanSkip();
      } finally {
        clearStoreAskHumanRequest(askHumanRequest);
      }
      return;
    }
    if (!visibleAskHumanRequest) return;
    try {
      await respondToToolApproval(visibleAskHumanRequest.sessionId, {
        request_id: visibleAskHumanRequest.requestId,
        approved: false,
        reason: null,
        remember: false,
        always_allow: false,
      });
    } finally {
      clearStoreAskHumanRequest(visibleAskHumanRequest);
    }
  }, [askHumanRequest, clearStoreAskHumanRequest, handleAskHumanSkip, visibleAskHumanRequest]);

  // Sticky "you-are-here" bar visibility. Reveal it only once the inline roadmap
  // (`[data-stage-roadmap]`) has fully scrolled out of view ABOVE the viewport —
  // so it never duplicates the roadmap during planning and snaps in Cursor-style
  // as the user scrolls past it. An IntersectionObserver on the roadmap element is
  // cheaper and jitter-free vs a scroll listener.
  const hasStagePlans = !!stagePlans && stagePlans.stageOrder.length > 0;
  const [roadmapScrolledPast, setRoadmapScrolledPast] = useState(false);
  useEffect(() => {
    const container = messagesContainerRef.current;
    if (!hasStagePlans || !container) {
      setRoadmapScrolledPast(false);
      return;
    }
    const roadmap = container.querySelector("[data-stage-roadmap]");
    if (!roadmap) {
      setRoadmapScrolledPast(false);
      return;
    }
    const observer = new IntersectionObserver(
      ([entry]) => {
        // Out of view AND above the viewport top = scrolled past the whole roadmap.
        const above = entry.boundingClientRect.top < (entry.rootBounds?.top ?? 0);
        setRoadmapScrolledPast(!entry.isIntersecting && above);
      },
      { root: container, threshold: 0 }
    );
    observer.observe(roadmap);
    return () => observer.disconnect();
  }, [hasStagePlans, planTargetIdx, messages.length, messagesContainerRef]);

  // ── Conversation switch: activate terminal + restore execution mode ──
  const terminalRestoreInProgress = useStore((s) => s.terminalRestoreInProgress);
  useEffect(() => {
    if (!activeConvId) return;
    if (terminalRestoreInProgress || useStore.getState().terminalRestoreInProgress) return;
    activateConversationTerminalFromChat(activeConvId, {
      setChatExecutionMode: modes.setChatExecutionMode,
      // Fresh tab (no terminals yet): reopen in the last-remembered mode rather
      // than always snapping back to Chat.
      emptyExecutionMode: readLastExecutionMode,
    });
  }, [activeConvId, terminalRestoreInProgress, modes.setChatExecutionMode]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Handlers ─────────────────────────────────────────────────────────
  const handleModelSelect = useCallback((modelId: string, provider: string) => {
    const sel = { model: modelId, provider };
    setSelectedModel(sel);
    useStore.getState().setSelectedAiModel(sel);
  }, []);

  const handleImageUpload = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files) return;
    for (const file of Array.from(files)) {
      if (!file.type.startsWith("image/")) continue;
      const reader = new FileReader();
      reader.onload = () => {
        const base64 = (reader.result as string).split(",")[1];
        if (base64)
          setImageAttachments((prev) => [
            ...prev,
            { data: base64, mediaType: file.type, name: file.name },
          ]);
      };
      reader.readAsDataURL(file);
    }
    e.target.value = "";
  }, []);

  const handleConvNewChat = useCallback(async () => {
    await handleNewChat();
    setInput("");
    setShowHistory(false);
    requestAnimationFrame(() => textareaRef.current?.focus());
  }, [handleNewChat]);

  const handleConvSelect = useCallback((convId: string) => {
    useStore.getState().setActiveConversation(convId);
    setShowHistory(false);
    requestAnimationFrame(() => textareaRef.current?.focus());
  }, []);

  // ── Derived data ─────────────────────────────────────────────────────
  const currentModel = selectedModel?.model ?? "";
  const currentProvider = selectedModel?.provider ?? "";
  const activeRestartStage = useMemo(() => currentRestartStage(stagePlans), [stagePlans]);
  const handleStageReset = useCallback(
    async (stage: string) => {
      if (!activeConv || isStreaming || taskResumeBusy || !stage) return;
      setTaskResumeBusy(true);
      try {
        // Full reset: rewind + purge the selected stage's discovered facts so the
        // re-test starts clean, then resume the task from that stage.
        await resetHarnessStageCheckpoint({
          mode: "restart_from_stage_purge",
          sessionId: activeConv.aiSessionId,
          stage,
        });
        if (resolveEngine(modes.chatExecutionMode) !== "task") {
          const taskMode = normalizeExecutionModeId("task");
          const changed = await modes.handleExecutionModeChange(taskMode);
          if (!changed) throw new Error("后端未接受 Task execution profile");
        }
        await handleSend(TASK_RESUME_PROMPT);
      } catch (cause) {
        const message = cause instanceof Error ? cause.message : String(cause);
        useStore.getState().setMessageError(activeConv.id, `重置阶段失败: ${message}`);
        setInput(TASK_RESUME_PROMPT);
        requestAnimationFrame(() => textareaRef.current?.focus());
      } finally {
        setTaskResumeBusy(false);
      }
    },
    [
      activeConv,
      handleSend,
      isStreaming,
      modes.chatExecutionMode,
      modes.chatExecutionModeRef,
      modes.handleExecutionModeChange,
      taskResumeBusy,
    ]
  );
  const showTaskResumeButton = import.meta.env.DEV && modes.chatExecutionMode !== "chat";
  const showRestoreLoading = shouldShowChatRestoreLoading({
    workspaceDataReady,
    terminalRestoreInProgress,
    pendingTerminalRestoreData,
    activeSessionId,
  });

  const stablePendingApproval = useMemo(
    () =>
      modes.pendingApproval
        ? { requestId: modes.pendingApproval.requestId, toolName: modes.pendingApproval.toolName }
        : null,
    [modes.pendingApproval?.requestId, modes.pendingApproval?.toolName, modes.pendingApproval]
  );

  // Bridge the blank gap between sending and the first assistant bubble: the
  // orchestrator plans (task/profile modes) before emitting `started`, so the
  // conversation streams with no streaming assistant message to host the
  // in-bubble status line. Suppress while a human prompt/approval is pending.
  const lastMessage = messages[messages.length - 1];
  const showPreparing =
    isStreaming &&
    !(lastMessage?.role === "assistant" && !!lastMessage.isStreaming) &&
    !askHumanRequest &&
    !stablePendingApproval;

  // ── Render ───────────────────────────────────────────────────────────
  return (
    <div className="flex flex-col h-full">
      {/* Tab Bar */}
      <ConversationTabs
        conversations={conversations}
        activeConvId={activeConvId}
        showHistory={showHistory}
        onSelect={handleConvSelect}
        onClose={handleCloseTab}
        onNewChat={handleConvNewChat}
        onToggleHistory={() => setShowHistory((v) => !v)}
      />

      {/* History panel */}
      {showHistory && (
        <div className="flex-1 overflow-y-auto overflow-x-hidden border-b border-[var(--border-subtle)]">
          <div className="px-3 py-2">
            <span className="text-[11px] text-muted-foreground uppercase tracking-wider font-semibold">
              {t("ai.historyTitle")}
            </span>
          </div>
          {conversations.filter((c) => c.messages.length > 0).length === 0 ? (
            <div className="flex items-center justify-center py-8">
              <span className="text-[12px] text-muted-foreground/50">{t("ai.noHistory")}</span>
            </div>
          ) : (
            conversations
              .filter((c) => c.messages.length > 0)
              .sort((a, b) => b.createdAt - a.createdAt)
              .map((conv) => (
                <button
                  key={conv.id}
                  type="button"
                  className={cn(
                    "w-full text-left px-3 py-2 text-[12px] hover:bg-[var(--bg-hover)] transition-colors",
                    conv.id === activeConvId
                      ? "text-foreground bg-[var(--bg-hover)]"
                      : "text-muted-foreground"
                  )}
                  onClick={() => handleConvSelect(conv.id)}
                >
                  <div className="truncate">{conv.title}</div>
                  <div className="text-[10px] text-muted-foreground/50 mt-0.5">
                    {new Date(conv.createdAt).toLocaleDateString()} · {conv.messages.length}{" "}
                    {t("ai.messages")}
                  </div>
                </button>
              ))
          )}
        </div>
      )}

      {/* Messages */}
      {!showHistory && (
        <div className="relative flex-1 min-h-0 flex flex-col">
          {hasStagePlans && messages.length > 0 && roadmapScrolledPast && (
            <StageProgressBar
              stagePlans={stagePlans!}
              isRunning={isStreaming}
              className="absolute top-0 left-0 right-0 z-20"
            />
          )}
          <div ref={messagesContainerRef} className="flex-1 overflow-y-auto overflow-x-hidden">
            {messages.length === 0 && !reportingReadModelHint && showRestoreLoading ? (
              <div className="flex flex-col items-center justify-center h-full select-none gap-3 text-center px-6">
                <LoaderCircle className="w-5 h-5 text-accent/70 animate-spin" />
                <div className="space-y-1">
                  <p className="text-[13px] text-foreground/80">{t("ai.loadingWorkspace")}</p>
                  <p className="text-[11px] text-muted-foreground/55">
                    {t("ai.loadingWorkspaceDetail")}
                  </p>
                </div>
              </div>
            ) : messages.length === 0 && !reportingReadModelHint ? (
              <div className="flex flex-col items-center justify-center h-full select-none gap-4">
                <div className="flex items-center gap-1.5">
                  {[0, 1, 2].map((i) => (
                    <div
                      key={i}
                      className="w-1.5 h-1.5 rounded-full bg-accent/40 typing-dot"
                      style={{ animationDelay: `${i * 0.2}s` }}
                    />
                  ))}
                </div>
                <p className="text-[13px] text-muted-foreground/70">{t("ai.placeholder")}</p>
                {pentestTools.length > 0 && (
                  <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground/50">
                    <Wrench className="w-3 h-3" />
                    <span>
                      {pentestTools.length} {t("ai.toolsAvailable", "tools available")}
                    </span>
                  </div>
                )}
              </div>
            ) : (
              <div>
                {messages.map((msg, msgIdx) => {
                  if (msg.role === "system") {
                    return <StageMarker key={msg.id} message={msg} />;
                  }
                  const isPlanTarget = msgIdx === planTargetIdx;
                  return (
                    <React.Fragment key={msg.id}>
                      <MessageBlock
                        message={msg}
                        taskPlan={isPlanTarget ? taskPlan : null}
                        stagePlans={isPlanTarget ? stagePlans : null}
                        planTextOffset={isPlanTarget ? planTextOffsetRef.current : null}
                        terminalId={activeAiSessionId}
                        pendingApproval={stablePendingApproval}
                        approvalMode={modes.approvalMode}
                        onApprovalModeChange={modes.handleApprovalModeChange}
                        onApprove={modes.handleToolApprove}
                        onApproveAlways={modes.handleToolApproveAlways}
                        onDeny={modes.handleToolDeny}
                      />
                    </React.Fragment>
                  );
                })}

                {reportingReadModelHint && (
                  <div className="px-4 py-3">
                    {/* Harness traces only locate/refresh this panel. The component
                        always reloads report truth through the scoped IPC API. */}
                    <ReportReadModelView
                      key={reportingReadModelHint.operationId}
                      operationId={reportingReadModelHint.operationId}
                      refreshVersion={reportingReadModelHint.refreshVersion}
                    />
                  </div>
                )}

                {activeWorkflow && <WorkflowProgress workflow={activeWorkflow} />}
                {compactionState && (
                  <CompactionNotice
                    active={compactionState.active}
                    tokensBefore={compactionState.tokensBefore}
                  />
                )}
                {visibleAskHumanRequest && (
                  <AskHumanInline
                    key={visibleAskHumanRequest.requestId}
                    request={visibleAskHumanRequest}
                    onSubmit={handleVisibleAskHumanSubmit}
                    onSkip={handleVisibleAskHumanSkip}
                    autoResolve={modes.approvalMode === "run-all"}
                    fallbackOrgId={lastDiscoverOrgId}
                    minOwnershipPercent={lastDiscoverThreshold}
                  />
                )}
                {showPreparing && (
                  <div className="px-4 py-3">
                    {/* Cursor-style "Planning" status while the orchestrator plans,
                      before the first streamed bubble exists. Reuses the same dot+
                      shimmer as the in-bubble status footer. */}
                    <AgentStatusIndicator phase="planning" />
                  </div>
                )}
                <div ref={messagesEndRef} />
              </div>
            )}
          </div>
        </div>
      )}

      {/* Input Area */}
      <div className="p-3 flex-shrink-0">
        <div className="rounded-lg border border-[var(--border-subtle)] bg-background overflow-hidden focus-within:border-muted-foreground/30 transition-colors">
          {imageAttachments.length > 0 && (
            <div className="flex items-center gap-1.5 px-3 pt-2 flex-wrap">
              {imageAttachments.map((img, i) => (
                <div key={`${img.name}-${i}`} className="relative group">
                  <img
                    src={`data:${img.mediaType};base64,${img.data}`}
                    alt={img.name}
                    className="w-12 h-12 rounded-md object-cover border border-border/30"
                  />
                  <button
                    type="button"
                    onClick={() => setImageAttachments((prev) => prev.filter((_, j) => j !== i))}
                    className="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-destructive text-destructive-foreground flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                  >
                    <X className="w-2.5 h-2.5" />
                  </button>
                </div>
              ))}
            </div>
          )}
          <textarea
            ref={textareaRef}
            data-ai-chat-input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            onInput={handleTextareaInput}
            placeholder={t("ai.inputPlaceholder")}
            rows={1}
            wrap="soft"
            className={cn(
              "w-full bg-transparent border-none outline-none resize-none",
              "text-[13px] text-foreground placeholder:text-muted-foreground/40",
              "leading-relaxed max-h-[160px] px-3 pt-2.5 pb-1.5",
              "ai-chat-input-textarea"
            )}
          />
          {/* Bottom toolbar */}
          <div className="flex items-center justify-between px-2.5 pb-2">
            <div className="flex items-center gap-1.5">
              <ExecutionModePicker
                chatExecutionMode={modes.chatExecutionMode}
                onExecutionModeChange={modes.handleExecutionModeChange}
                onAgentModeChange={modes.handleAgentModeChange}
              />
              <ChatModelSelector
                modelDisplay={modelDisplay}
                currentModel={currentModel}
                currentProvider={currentProvider}
                configuredProviders={configuredProviders}
                onModelSelect={handleModelSelect}
              />
            </div>
            <div className="flex items-center gap-1">
              <ContextUsageRing contextUsage={contextUsage} />
              {showTaskResumeButton && hasStagePlans && (
                <StageResetMenu
                  stageOrder={stagePlans!.stageOrder}
                  passedStages={stagePlans!.passedStages}
                  currentStage={activeRestartStage}
                  disabled={!activeConv || isStreaming || taskResumeBusy}
                  busy={taskResumeBusy}
                  onReset={(stage) => void handleStageReset(stage)}
                />
              )}
              <button
                type="button"
                title={t("ai.uploadImage")}
                className="h-6 w-6 flex items-center justify-center rounded text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
                onClick={() => fileInputRef.current?.click()}
              >
                <Image className="w-3.5 h-3.5" />
              </button>
              <input
                ref={fileInputRef}
                type="file"
                accept="image/*"
                multiple
                className="hidden"
                onChange={handleImageUpload}
              />
              {isStreaming ? (
                <button
                  type="button"
                  title="Stop"
                  onClick={handleStop}
                  className="h-6 w-6 flex items-center justify-center rounded bg-destructive/20 text-destructive hover:bg-destructive/30 transition-colors"
                >
                  <Square className="w-3 h-3" />
                </button>
              ) : (
                <button
                  type="button"
                  title={input.trim() ? t("ai.send") : ""}
                  onClick={() => void handleSend()}
                  disabled={!input.trim()}
                  className={cn(
                    "h-6 w-6 flex items-center justify-center rounded transition-colors",
                    input.trim()
                      ? "bg-accent text-accent-foreground hover:bg-accent/80 cursor-pointer"
                      : "bg-muted text-muted-foreground cursor-default"
                  )}
                >
                  <ArrowUp className="w-3.5 h-3.5" />
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
});
