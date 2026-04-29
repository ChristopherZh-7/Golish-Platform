import { ArrowUp, Image, Square, Wrench, X } from "lucide-react";
import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";
import { formatModelName } from "@/lib/models";
import { cn } from "@/lib/utils";
import { ExecutionModePicker } from "./ExecutionModePicker";
import { ChatModelSelector } from "./ChatModelSelector";
import { ContextUsageRing } from "./ContextUsageRing";
import { type ChatMessage, useStore } from "@/store";
import { useAiChatInit } from "./hooks/useAiChatInit";
import { useChatSessionInit } from "./hooks/useChatSessionInit";
import { useChatSend } from "./hooks/useChatSend";
import { useChatModes } from "./hooks/useChatModes";
import { useChatConversationOps } from "./hooks/useChatConversationOps";
import { useChatHotkeys } from "./hooks/useChatHotkeys";
import { useAiChatEvents } from "./hooks/useAiChatEvents";
import { useTaskPlanState } from "./hooks/useTaskPlanState";
import { useChatAutoScroll } from "./useChatAutoScroll";
import { ConversationTabs } from "./ConversationTabs";
import {
  AskHumanInline,
  CompactionNotice,
  WorkflowProgress,
} from "./ChatSubComponents";
import { MessageBlock } from "./MessageBlock";
import { buildPentestSystemPrompt } from "./pentestSystemPrompt";


const EMPTY_MESSAGES: ChatMessage[] = [];


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
  const messages = activeConv?.messages ?? EMPTY_MESSAGES;
  const isStreaming = activeConv?.isStreaming ?? false;

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
    if (storeApprovalMode) modes.setApprovalMode(storeApprovalMode as "ask" | "allowlist" | "run-all");
  }, [storeApprovalMode]); // eslint-disable-line react-hooks/exhaustive-deps

  const updateConv = useStore.getState().updateConversation;

  const sessionInit = useChatSessionInit({
    selectedModel,
    chatExecutionModeRef: modes.chatExecutionModeRef,
    chatUseSubAgentsRef: modes.chatUseSubAgentsRef,
    setChatExecutionMode: modes.setChatExecutionMode,
    setChatUseSubAgents: modes.setChatUseSubAgents,
    updateConv,
  });

  const buildSystemPrompt = useCallback(
    () => buildPentestSystemPrompt(pentestTools),
    [pentestTools]
  );

  const { handleSend, handleStop } = useChatSend({
    input, setInput, isStreaming, activeConvId,
    imageAttachments, setImageAttachments,
    textareaRef: textareaRef as React.MutableRefObject<HTMLTextAreaElement | null>, userScrolledUpRef, streamingMsgRef,
    chatExecutionModeRef: modes.chatExecutionModeRef,
    taskInProgressRef,
    initializeSession: sessionInit.initializeSession,
    buildPentestSystemPrompt: buildSystemPrompt,
    createTerminalTab,
    t: ((key: string, fallback?: string) => t(key, fallback ?? key)) as (key: string, fallback?: string) => string,
  });

  const { handleNewChat, handleCloseTab } = useChatConversationOps(createTerminalTab);
  const { handleKeyDown, handleTextareaInput } = useChatHotkeys({ textareaRef, onSend: handleSend });

  // ── AI events + plan state (extracted hooks) ─────────────────────────
  const {
    contextUsage, askHumanRequest, activeWorkflow, compactionState,
    planTextOffsetRef, planMessageIdRef,
    handleAskHumanSubmit, handleAskHumanSkip,
  } = useAiChatEvents({
    activeConvId, streamingMsgRef, taskInProgressRef,
    modes: { setPendingApproval: modes.setPendingApproval, pendingApprovalRef: modes.pendingApprovalRef },
    generateTitleRef: sessionInit.generateTitleRef,
  });

  const { activeAiSessionId, taskPlan, planTargetIdx } =
    useTaskPlanState(messages, planMessageIdRef);

  // ── Conversation switch: activate terminal + restore execution mode ──
  const terminalRestoreInProgress = useStore((s) => s.terminalRestoreInProgress);
  useEffect(() => {
    if (!activeConvId) return;
    if (terminalRestoreInProgress || useStore.getState().terminalRestoreInProgress) return;
    const store = useStore.getState();
    const terminals = store.conversationTerminals[activeConvId];
    if (terminals && terminals.length > 0) {
      const firstTerminal = terminals[0];
      if (store.sessions[firstTerminal] && store.activeSessionId !== firstTerminal) {
        store.setActiveSession(firstTerminal);
      }
      for (const tid of terminals) {
        if (store.sessions[tid]?.executionMode === "task") {
          modes.setChatExecutionMode("task");
          break;
        }
      }
      const hasAgents = terminals.some((tid) => store.sessions[tid]?.useAgents);
      if (hasAgents !== modes.chatUseSubAgents) modes.setChatUseSubAgents(hasAgents);
    } else {
      modes.setChatExecutionMode("chat");
    }
  }, [activeConvId, terminalRestoreInProgress]); // eslint-disable-line react-hooks/exhaustive-deps

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
        if (base64) setImageAttachments((prev) => [...prev, { data: base64, mediaType: file.type, name: file.name }]);
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
  }, []);

  // ── Derived data ─────────────────────────────────────────────────────
  const currentModel = selectedModel?.model ?? "";
  const currentProvider = selectedModel?.provider ?? "";

  const stablePendingApproval = useMemo(
    () => modes.pendingApproval ? { requestId: modes.pendingApproval.requestId, toolName: modes.pendingApproval.toolName } : null,
    [modes.pendingApproval?.requestId, modes.pendingApproval?.toolName]
  );

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
                  key={conv.id} type="button"
                  className={cn(
                    "w-full text-left px-3 py-2 text-[12px] hover:bg-[var(--bg-hover)] transition-colors",
                    conv.id === activeConvId ? "text-foreground bg-[var(--bg-hover)]" : "text-muted-foreground"
                  )}
                  onClick={() => handleConvSelect(conv.id)}
                >
                  <div className="truncate">{conv.title}</div>
                  <div className="text-[10px] text-muted-foreground/50 mt-0.5">
                    {new Date(conv.createdAt).toLocaleDateString()} · {conv.messages.length} {t("ai.messages")}
                  </div>
                </button>
              ))
          )}
        </div>
      )}

      {/* Messages */}
      {!showHistory && (
        <div ref={messagesContainerRef} className="flex-1 overflow-y-auto overflow-x-hidden">
          {messages.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full select-none gap-4">
              <div className="flex items-center gap-1.5">
                {[0, 1, 2].map((i) => (
                  <div key={i} className="w-1.5 h-1.5 rounded-full bg-accent/40 typing-dot" style={{ animationDelay: `${i * 0.2}s` }} />
                ))}
              </div>
              <p className="text-[13px] text-muted-foreground/70">{t("ai.placeholder")}</p>
              {pentestTools.length > 0 && (
                <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground/50">
                  <Wrench className="w-3 h-3" />
                  <span>{pentestTools.length} {t("ai.toolsAvailable", "tools available")}</span>
                </div>
              )}
            </div>
          ) : (
            <div>
              {messages.map((msg, msgIdx) => {
                const isPlanTarget = msgIdx === planTargetIdx;
                return (
                  <React.Fragment key={msg.id}>
                    <MessageBlock
                      message={msg}
                      taskPlan={isPlanTarget ? taskPlan : null}
                      planTextOffset={isPlanTarget ? planTextOffsetRef.current : null}
                      terminalId={activeAiSessionId}
                      pendingApproval={stablePendingApproval}
                      approvalMode={modes.approvalMode}
                      onApprovalModeChange={modes.handleApprovalModeChange}
                      onApprove={modes.handleToolApprove}
                      onDeny={modes.handleToolDeny}
                    />
                  </React.Fragment>
                );
              })}

              {activeWorkflow && <WorkflowProgress workflow={activeWorkflow} />}
              {compactionState && <CompactionNotice active={compactionState.active} tokensBefore={compactionState.tokensBefore} />}
              {askHumanRequest && <AskHumanInline request={askHumanRequest} onSubmit={handleAskHumanSubmit} onSkip={handleAskHumanSkip} />}
              <div ref={messagesEndRef} />
            </div>
          )}
        </div>
      )}

      {/* Input Area */}
      <div className="p-3 flex-shrink-0">
        <div className="rounded-lg border border-[var(--border-subtle)] bg-background overflow-hidden focus-within:border-muted-foreground/30 transition-colors">
          {imageAttachments.length > 0 && (
            <div className="flex items-center gap-1.5 px-3 pt-2 flex-wrap">
              {imageAttachments.map((img, i) => (
                <div key={`${img.name}-${i}`} className="relative group">
                  <img src={`data:${img.mediaType};base64,${img.data}`} alt={img.name} className="w-12 h-12 rounded-md object-cover border border-border/30" />
                  <button type="button" onClick={() => setImageAttachments((prev) => prev.filter((_, j) => j !== i))} className="absolute -top-1 -right-1 w-4 h-4 rounded-full bg-destructive text-destructive-foreground flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity">
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
            className={cn(
              "w-full bg-transparent border-none outline-none resize-none",
              "text-[13px] text-foreground placeholder:text-muted-foreground/40",
              "leading-relaxed max-h-[160px] px-3 pt-2.5 pb-1.5"
            )}
          />
          {/* Bottom toolbar */}
          <div className="flex items-center justify-between px-2.5 pb-2">
            <div className="flex items-center gap-1.5">
              <ExecutionModePicker
                chatExecutionMode={modes.chatExecutionMode}
                chatUseSubAgents={modes.chatUseSubAgents}
                onExecutionModeChange={modes.handleExecutionModeChange}
                onAgentModeChange={modes.handleAgentModeChange}
                onToggleSubAgents={modes.handleToggleSubAgents}
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
              <button type="button" title={t("ai.uploadImage")} className="h-6 w-6 flex items-center justify-center rounded text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors" onClick={() => fileInputRef.current?.click()}>
                <Image className="w-3.5 h-3.5" />
              </button>
              <input ref={fileInputRef} type="file" accept="image/*" multiple className="hidden" onChange={handleImageUpload} />
              {isStreaming ? (
                <button type="button" title="Stop" onClick={handleStop} className="h-6 w-6 flex items-center justify-center rounded bg-destructive/20 text-destructive hover:bg-destructive/30 transition-colors">
                  <Square className="w-3 h-3" />
                </button>
              ) : (
                <button type="button" title={input.trim() ? t("ai.send") : ""} onClick={handleSend} disabled={!input.trim()} className={cn("h-6 w-6 flex items-center justify-center rounded transition-colors", input.trim() ? "bg-accent text-accent-foreground hover:bg-accent/80 cursor-pointer" : "bg-muted text-muted-foreground cursor-default")}>
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
