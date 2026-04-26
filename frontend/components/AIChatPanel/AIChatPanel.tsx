import {
  ArrowUp,
  ChevronDown,
  Cpu,
  Image,
  MessageSquare,
  Square,
  Users,
  Wrench,
  X,
  Zap,
} from "lucide-react";
import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";
import { formatModelName, PROVIDER_GROUPS } from "@/lib/models";
import { cn } from "@/lib/utils";
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
  TaskPlanCard,
  WorkflowProgress,
} from "./ChatSubComponents";
import { SubAgentSummaryBar } from "./SubAgentSummaryBar";
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
    textareaRef, userScrolledUpRef, streamingMsgRef,
    chatExecutionModeRef: modes.chatExecutionModeRef,
    taskInProgressRef,
    initializeSession: sessionInit.initializeSession,
    buildPentestSystemPrompt: buildSystemPrompt,
    createTerminalTab, t,
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

  const { activeAiSessionId, taskPlan, planTargetIdx, retiredPlansByMsg } =
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
                const retiredHere = retiredPlansByMsg.get(msg.id);
                return (
                  <React.Fragment key={msg.id}>
                    {retiredHere && retiredHere.length > 0 && retiredHere.map((rp, ri) => (
                      <TaskPlanCard key={`retired-${rp.retiredAt ?? ri}`} plan={rp} retired />
                    ))}
                    <MessageBlock
                      message={msg}
                      taskPlan={isPlanTarget ? taskPlan : null}
                      planTextOffset={isPlanTarget ? planTextOffsetRef.current : null}
                      terminalId={taskPlan ? activeAiSessionId : null}
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
              <SubAgentSummaryBar />
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
              {/* Execution mode dropdown */}
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button type="button" className={cn(
                    "flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium transition-colors",
                    modes.chatExecutionMode === "task"
                      ? "bg-[var(--ansi-magenta)]/10 text-[var(--ansi-magenta)] hover:bg-[var(--ansi-magenta)]/20"
                      : "bg-muted text-foreground hover:bg-[var(--bg-hover)]"
                  )}>
                    {modes.chatExecutionMode === "task" ? <Zap className="w-3 h-3" /> : <MessageSquare className="w-3 h-3" />}
                    {modes.chatExecutionMode === "task" ? "Task" : "Chat"}
                    <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" side="top" className="bg-card border-[var(--border-medium)] min-w-[220px]">
                  <DropdownMenuItem onClick={() => { modes.handleExecutionModeChange("chat"); modes.handleAgentModeChange("default"); }} className={cn("text-xs cursor-pointer flex items-start gap-2 py-2.5", modes.chatExecutionMode === "chat" ? "text-accent bg-[var(--accent-dim)]" : "text-foreground hover:text-accent")}>
                    <MessageSquare className="w-4 h-4 mt-0.5 shrink-0" />
                    <div className="flex flex-col"><span className="font-medium">Chat</span><span className="text-[10px] text-muted-foreground">Conversational assistant with tools</span></div>
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => { modes.handleExecutionModeChange("task"); modes.handleAgentModeChange("auto-approve"); }} className={cn("text-xs cursor-pointer flex items-start gap-2 py-2.5", modes.chatExecutionMode === "task" ? "text-[var(--ansi-magenta)] bg-[var(--ansi-magenta)]/10" : "text-foreground hover:text-accent")}>
                    <Zap className="w-4 h-4 mt-0.5 shrink-0" />
                    <div className="flex flex-col"><span className="font-medium">Task</span><span className="text-[10px] text-muted-foreground">Auto: plan → execute → refine → report</span></div>
                  </DropdownMenuItem>
                  <DropdownMenuSeparator className="bg-[var(--border-medium)]" />
                  <DropdownMenuItem onSelect={(e) => { e.preventDefault(); modes.handleToggleSubAgents(); }} className="text-xs cursor-pointer flex items-center gap-2 py-2">
                    <Users className={cn("w-4 h-4 shrink-0", modes.chatUseSubAgents ? "text-[var(--ansi-green)]" : "text-muted-foreground")} />
                    <div className="flex flex-col flex-1"><span className="font-medium">Sub-Agents</span><span className="text-[10px] text-muted-foreground">{modes.chatUseSubAgents ? "Enabled" : "Disabled"}</span></div>
                    <div className={cn("w-7 h-4 rounded-full transition-colors duration-200 flex items-center shrink-0", modes.chatUseSubAgents ? "bg-[var(--ansi-green)]/30 justify-end" : "bg-muted justify-start")}>
                      <div className={cn("w-3 h-3 rounded-full mx-0.5 transition-colors duration-200", modes.chatUseSubAgents ? "bg-[var(--ansi-green)]" : "bg-muted-foreground/50")} />
                    </div>
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>

              {/* Model selector dropdown */}
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button type="button" className="flex items-center gap-1 px-2 py-1 rounded-md text-[11px] text-accent hover:bg-[var(--bg-hover)] transition-colors">
                    <Cpu className="w-3 h-3" />
                    {modelDisplay}
                    <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" side="top" className="bg-card border-[var(--border-medium)] min-w-[200px] max-h-[400px] overflow-y-auto">
                  {(() => {
                    const filtered = PROVIDER_GROUPS.filter((g) => configuredProviders.has(g.provider));
                    if (filtered.length === 0) {
                      return (
                        <div className="px-3 py-4 text-center">
                          <p className="text-xs text-muted-foreground">{t("ai.noProviders", "No providers configured")}</p>
                          <p className="text-[10px] text-muted-foreground/60 mt-1">{t("ai.configureInSettings", "Configure API keys in Settings → Providers")}</p>
                        </div>
                      );
                    }
                    return filtered.map((group, gi) => (
                      <div key={group.provider}>
                        {gi > 0 && <DropdownMenuSeparator />}
                        <div className="px-2 py-1 text-[10px] text-muted-foreground uppercase tracking-wide">{group.providerName}</div>
                        {group.models.map((model) => {
                          const isSelected = currentModel === model.id && (currentProvider === group.provider || currentProvider === "anthropic_vertex");
                          return (
                            <DropdownMenuItem key={`${group.provider}-${model.id}-${model.reasoningEffort ?? ""}`} onClick={() => handleModelSelect(model.id, group.provider)} className={cn("text-xs cursor-pointer", isSelected ? "text-accent bg-[var(--accent-dim)]" : "text-foreground hover:text-accent")}>
                              {model.name}
                            </DropdownMenuItem>
                          );
                        })}
                      </div>
                    ));
                  })()}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
            <div className="flex items-center gap-1">
              {/* Context usage ring */}
              <div className="relative group" title={contextUsage ? `${(contextUsage.utilization * 100).toFixed(1)}% · ${(contextUsage.totalTokens / 1000).toFixed(1)}K / ${(contextUsage.maxTokens / 1000).toFixed(0)}K context used` : "No context data"}>
                <svg className="w-5 h-5 -rotate-90" viewBox="0 0 20 20">
                  <circle cx="10" cy="10" r="8" fill="none" stroke="currentColor" strokeWidth="2" className="text-muted-foreground/20" />
                  <circle cx="10" cy="10" r="8" fill="none" strokeWidth="2" strokeLinecap="round" strokeDasharray={`${(contextUsage?.utilization ?? 0) * 50.27} 50.27`} className={cn("transition-all duration-300", !contextUsage ? "text-muted-foreground/30" : contextUsage.utilization > 0.9 ? "text-red-400" : contextUsage.utilization > 0.7 ? "text-[#e0af68]" : "text-accent")} stroke="currentColor" />
                </svg>
                <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 rounded bg-popover border border-border/30 text-[10px] text-popover-foreground whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-50">
                  {contextUsage ? `${(contextUsage.utilization * 100).toFixed(1)}% · ${(contextUsage.totalTokens / 1000).toFixed(1)}K / ${(contextUsage.maxTokens / 1000).toFixed(0)}K context used` : "Context usage unavailable"}
                </div>
              </div>
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
