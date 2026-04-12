import {
  AlertCircle,
  ArrowRight,
  ArrowUp,
  Bot,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Clock,
  Code2,
  Cpu,
  Image,
  Loader2,
  Plus,
  Search,
  Square,
  Wrench,
  X,
  XCircle,
} from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { Markdown } from "@/components/Markdown";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useCreateTerminalTab } from "@/hooks/useCreateTerminalTab";
import {
  type AgentMode,
  type AiEvent,
  createTextPayload,
  initAiSession,
  onAiEvent,
  type ProviderConfig,
  respondToToolApproval,
  restoreAiConversation,
  sendPromptSession,
  sendPromptWithAttachments,
  setAgentMode,
  shutdownAiSession,
} from "@/lib/ai";
import { formatModelName, PROVIDER_GROUPS } from "@/lib/models";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { getSettings } from "@/lib/settings";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import { cn } from "@/lib/utils";
import type { PersistedTerminalData } from "@/lib/workspace-storage";
import { type ChatMessage, useStore } from "@/store";
import { createNewConversation } from "@/store/slices/conversation";
import {
  AskHumanInline,
  type AskHumanState,
  CollapsibleToolCall,
  CompactionNotice,
  TaskPlanCard,
  type TaskPlanState,
  ThinkingBlock,
  ToolCallSummary,
  WorkflowProgress,
  type WorkflowState,
} from "./ChatSubComponents";

const STORAGE_KEY = "golish-pentest-conversations";
const TERMINAL_DATA_KEY = "golish-pentest-conv-terminals";
const MAX_STORED_CONVS = 50;
const MAX_SCROLLBACK_CHARS = 100_000;
const MAX_BLOCK_OUTPUT_CHARS = 50_000;
const MAX_SAVED_BLOCKS = 50;

type PersistedTerminal = PersistedTerminalData;
const EMPTY_MESSAGES: ChatMessage[] = [];

const EMPTY_SUB_AGENTS: never[] = [];

const AGENT_ICON_MAP: Record<string, typeof Bot> = {
  coder: Code2,
  researcher: Search,
  explorer: Search,
};

function getAgentIcon(name: string): typeof Bot {
  const lower = name.toLowerCase();
  for (const [key, icon] of Object.entries(AGENT_ICON_MAP)) {
    if (lower.includes(key)) return icon;
  }
  return Bot;
}

function AgentSummaryBar() {
  const [isExpanded, setIsExpanded] = useState(false);
  const wasRunningRef = useRef(false);
  const activeSessionId = useStore((s) => s.activeSessionId);
  const subAgents = useStore((s) =>
    activeSessionId ? (s.activeSubAgents[activeSessionId] ?? EMPTY_SUB_AGENTS) : EMPTY_SUB_AGENTS
  );

  const running = subAgents.filter((a) => a.status === "running").length;
  const allDone = running === 0 && subAgents.length > 0;

  useEffect(() => {
    if (running > 0) {
      wasRunningRef.current = true;
    } else if (wasRunningRef.current && allDone) {
      const timer = setTimeout(() => setIsExpanded(false), 800);
      return () => clearTimeout(timer);
    }
  }, [running, allDone]);

  if (subAgents.length === 0) return null;

  const completed = subAgents.filter((a) => a.status === "completed").length;
  const errored = subAgents.filter((a) => a.status === "error").length;
  const totalDurationMs = subAgents.reduce((sum, a) => sum + (a.durationMs ?? 0), 0);

  const scrollToAgent = (parentRequestId: string) => {
    const el = document.querySelector(`[data-agent-block="sub-agent-${parentRequestId}"]`) as HTMLElement | null;
    if (!el) return;
    el.scrollIntoView({ behavior: "smooth", block: "center" });
    el.style.transition = "box-shadow 0.3s ease, background-color 0.3s ease";
    el.style.boxShadow = "0 0 0 2px var(--accent), 0 0 16px 2px rgba(var(--accent-rgb, 80 200 180), 0.3)";
    el.style.backgroundColor = "rgba(var(--accent-rgb, 80 200 180), 0.08)";
    setTimeout(() => {
      el.style.boxShadow = "";
      el.style.backgroundColor = "";
      setTimeout(() => { el.style.transition = ""; }, 300);
    }, 2000);
  };

  return (
    <div className="mx-4 my-1.5 rounded-md bg-muted/20 text-[11px] text-muted-foreground overflow-hidden">
      <button
        type="button"
        onClick={() => setIsExpanded((v) => !v)}
        className="w-full flex items-center gap-3 px-3 py-1.5 hover:bg-muted/30 transition-colors"
      >
        <div className="flex items-center gap-1">
          <Bot className="w-3 h-3" />
          <span className="font-medium">
            {subAgents.length} agent{subAgents.length > 1 ? "s" : ""}
          </span>
        </div>
        {running > 0 && (
          <div className="flex items-center gap-1">
            <Loader2 className="w-2.5 h-2.5 animate-spin text-accent" />
            <span>{running} active</span>
          </div>
        )}
        {completed > 0 && (
          <div className="flex items-center gap-1">
            <CheckCircle2 className="w-2.5 h-2.5 text-green-500" />
            <span>{completed} done</span>
          </div>
        )}
        {errored > 0 && (
          <div className="flex items-center gap-1">
            <XCircle className="w-2.5 h-2.5 text-destructive" />
            <span>{errored} failed</span>
          </div>
        )}
        <div className="ml-auto flex items-center gap-1.5">
          {totalDurationMs > 0 && (
            <span className="text-[10px] text-muted-foreground/60">
              {(totalDurationMs / 1000).toFixed(1)}s total
            </span>
          )}
          {isExpanded ? (
            <ChevronUp className="w-3 h-3 text-muted-foreground/40" />
          ) : (
            <ChevronDown className="w-3 h-3 text-muted-foreground/40" />
          )}
        </div>
      </button>

      <div
        className="grid transition-[grid-template-rows] duration-300 ease-in-out"
        style={{ gridTemplateRows: isExpanded ? "1fr" : "0fr" }}
      >
        <div className="overflow-hidden border-t border-border/30 px-3 py-1">
          {subAgents.map((agent) => {
            const AgentIcon = getAgentIcon(agent.agentName);
            return (
              <div
                key={agent.parentRequestId}
                className="flex items-center gap-2 py-1 text-[10px]"
              >
                <AgentIcon className="w-3 h-3 flex-shrink-0 text-muted-foreground/60" />
                <span className="flex-1 truncate">
                  {agent.agentName || agent.agentId}
                </span>
                {agent.status === "running" && (
                  <Loader2 className="w-2.5 h-2.5 animate-spin text-accent" />
                )}
                {agent.status === "completed" && (
                  <CheckCircle2 className="w-2.5 h-2.5 text-green-500" />
                )}
                {agent.status === "error" && (
                  <XCircle className="w-2.5 h-2.5 text-destructive" />
                )}
                {agent.durationMs !== undefined && (
                  <span className="text-muted-foreground/40">
                    {(agent.durationMs / 1000).toFixed(1)}s
                  </span>
                )}
                <button
                  type="button"
                  onClick={(e) => { e.stopPropagation(); scrollToAgent(agent.parentRequestId); }}
                  className="p-0.5 hover:bg-accent/30 rounded transition-colors"
                  title="Scroll to agent card"
                >
                  <ArrowRight className="w-2.5 h-2.5 text-muted-foreground/40 hover:text-accent" />
                </button>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function MessageBlock({
  message,
  pendingApproval,
  onApprove,
  onDeny,
  approvalMode,
  onApprovalModeChange,
  taskPlan,
  planTextOffset,
}: {
  message: ChatMessage;
  pendingApproval?: { requestId: string; toolName: string } | null;
  onApprove?: (requestId: string) => void;
  onDeny?: (requestId: string) => void;
  approvalMode?: string;
  onApprovalModeChange?: (mode: "ask" | "allowlist" | "run-all") => void;
  taskPlan?: TaskPlanState | null;
  planTextOffset?: number | null;
}) {
  const isUser = message.role === "user";

  return (
    <div className={cn("px-4 py-3", !isUser && "bg-[var(--bg-hover)]")}>
      <div className="text-[11px] text-muted-foreground mb-1.5 font-medium">
        {isUser ? "You" : "Golish AI"}
      </div>

      {!isUser && message.thinking && (
        <ThinkingBlock
          content={message.thinking}
          isActive={!!message.isStreaming && !message.content}
        />
      )}

      {(() => {
        if (message.error) {
          return (
            <div className="flex items-start gap-2 text-[13px] text-destructive">
              <AlertCircle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
              <span>{message.error}</span>
            </div>
          );
        }

        const hasToolCalls = !isUser && message.toolCalls && message.toolCalls.length > 0;
        const tcOffset = message.toolCallsContentOffset;
        const shouldSplitForTools = hasToolCalls && tcOffset != null && tcOffset >= 0;

        const visibleCalls = hasToolCalls
          ? message.toolCalls!.filter(
              (tc) =>
                tc.success !== undefined ||
                (pendingApproval == null && tc.success === undefined)
            )
          : [];
        const callIds = visibleCalls
          .map((tc) => tc.requestId)
          .filter((id): id is string => !!id);

        const pendingCalls = hasToolCalls && pendingApproval
          ? message.toolCalls!.filter(
              (tc) => tc.name === pendingApproval.toolName && tc.success === undefined
            )
          : [];

        const renderToolCards = () => (
          <>
            {pendingCalls.length > 0 && (
              <div className="mt-2 space-y-1.5">
                {pendingCalls.map((tc, i) => (
                  <CollapsibleToolCall
                    key={`${tc.name}-${i}`}
                    tc={tc}
                    approval={pendingApproval}
                    onApprove={onApprove}
                    onDeny={onDeny}
                    approvalMode={approvalMode}
                    onApprovalModeChange={onApprovalModeChange}
                  />
                ))}
              </div>
            )}
            {visibleCalls.length > 0 && (
              <ToolCallSummary toolCalls={visibleCalls} requestIds={callIds} />
            )}
          </>
        );

        if (shouldSplitForTools) {
          const textBefore = message.content.slice(0, tcOffset);
          const textAfter = message.content.slice(tcOffset);
          return (
            <>
              {textBefore && (
                <div className="text-[12px] text-foreground leading-[1.55]">
                  <Markdown content={textBefore} />
                </div>
              )}
              {!isUser && taskPlan && <TaskPlanCard plan={taskPlan} />}
              {renderToolCards()}
              {textAfter.trim() && (
                <div className="text-[12px] text-foreground leading-[1.55] mt-2">
                  <Markdown content={textAfter} />
                </div>
              )}
            </>
          );
        }

        if (!isUser && taskPlan && planTextOffset != null && planTextOffset > 0) {
          return (
            <>
              <div className="text-[12px] text-foreground leading-[1.55]">
                <Markdown content={message.content.slice(0, planTextOffset) || (message.isStreaming ? "..." : "")} />
              </div>
              <TaskPlanCard plan={taskPlan} />
              {message.content.length > planTextOffset && (
                <div className="text-[12px] text-foreground leading-[1.55]">
                  <Markdown content={message.content.slice(planTextOffset)} />
                </div>
              )}
              {renderToolCards()}
            </>
          );
        }

        return (
          <>
            <div className="text-[12px] text-foreground leading-[1.55]">
              <Markdown content={message.content || (message.isStreaming ? "..." : "")} />
            </div>
            {!isUser && taskPlan && <TaskPlanCard plan={taskPlan} />}
            {renderToolCards()}
          </>
        );
      })()}

      {message.isStreaming && (
        <div className="flex items-center gap-1 mt-1">
          <Loader2 className="w-3 h-3 animate-spin text-accent" />
        </div>
      )}
    </div>
  );
}

export const AIChatPanel = memo(function AIChatPanel() {
  const { t } = useTranslation();

  // Store state - use useShallow for array selectors to prevent infinite re-render loop
  const conversations = useStore(
    useShallow((s) => s.conversationOrder.map((id) => s.conversations[id]).filter(Boolean))
  );
  const activeConvId = useStore((s) => s.activeConversationId);
  const activeConv = useStore((s) =>
    s.activeConversationId ? (s.conversations[s.activeConversationId] ?? null) : null
  );
  const messages = activeConv?.messages ?? EMPTY_MESSAGES;
  const isStreaming = activeConv?.isStreaming ?? false;

  const [pendingApproval, setPendingApproval] = useState<{
    requestId: string;
    sessionId: string;
    toolName: string;
    args: Record<string, unknown>;
    riskLevel: string;
  } | null>(null);

  type ApprovalMode = "ask" | "allowlist" | "run-all";
  const [approvalMode, setApprovalMode] = useState<ApprovalMode>(() => {
    try {
      return (localStorage.getItem("golish-approval-mode") as ApprovalMode) || "ask";
    } catch {
      return "ask";
    }
  });

  const [contextUsage, setContextUsage] = useState<{
    utilization: number;
    totalTokens: number;
    maxTokens: number;
  } | null>(null);

  // AskHuman state
  const [askHumanRequest, setAskHumanRequest] = useState<AskHumanState | null>(null);

  // Workflow state
  const [activeWorkflow, setActiveWorkflow] = useState<WorkflowState | null>(null);

  // Task plan state
  const [taskPlan, setTaskPlan] = useState<TaskPlanState | null>(null);
  // Text offset at which the plan card was first created (for inline positioning)
  const planTextOffsetRef = useRef<number | null>(null);

  // Context compaction state
  const [compactionState, setCompactionState] = useState<{
    active: boolean;
    tokensBefore?: number;
  } | null>(null);

  // Image attachments for sending with prompt
  const [imageAttachments, setImageAttachments] = useState<
    Array<{ data: string; mediaType: string; name: string }>
  >([]);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Store actions
  const {
    addConversation,
    removeConversation: removeConv,
    setActiveConversation,
    updateConversation: updateConv,
    addConversationMessage,
    finalizeStreamingMessage,
    setMessageError,
    setConversationStreaming,
  } = useStore.getState();

  const { createTerminalTab } = useCreateTerminalTab();

  // Local UI state
  const [showHistory, setShowHistory] = useState(false);
  const [input, setInput] = useState("");
  const [pentestTools, setPentestTools] = useState<ToolConfig[]>([]);
  const [configuredProviders, setConfiguredProviders] = useState<Set<string>>(new Set());
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const messagesContainerRef = useRef<HTMLDivElement>(null);
  const chatAtBottomRef = useRef(true);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const tabsRef = useRef<HTMLDivElement>(null);
  const unlistenRef = useRef<(() => void) | null>(null);
  const streamingMsgRef = useRef<string | null>(null);
  const generateTitleRef = useRef<((convId: string, firstMsg: string) => void) | null>(null);
  const restoringTerminalsRef = useRef(false);
  const termRestoreRanRef = useRef(false);
  const pendingTermRestoreRef = useRef<Record<string, PersistedTerminal[]>>({});
  const termSaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const workspaceDataReady = useStore((s) => s.workspaceDataReady);

  // Load saved conversations into store on mount — merge with any already loaded by workspace auto-saver
  // Then restore per-conversation terminals (with scrollback) from localStorage
  // Wait for workspaceDataReady so that App init has hydrated localStorage from workspace.json
  useEffect(() => {
    if (!workspaceDataReady) return;
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return;
      const parsed = JSON.parse(raw) as Array<{
        id: string;
        title: string;
        messages: ChatMessage[];
        createdAt: number;
        aiSessionId: string;
      }>;
      const store = useStore.getState();
      const existing = store.conversations;
      for (const c of parsed.filter((c) => c.messages.length > 0)) {
        const ex = existing[c.id];
        if (ex && ex.messages.length >= c.messages.length) continue;
        const conv = {
          ...c,
          aiSessionId: c.aiSessionId || c.id,
          aiInitialized: false,
          isStreaming: false,
          messages: c.messages.map((m) => ({ ...m, isStreaming: false })),
        };
        if (ex) {
          store.updateConversation(c.id, { messages: conv.messages, title: conv.title });
        } else {
          store.addConversation(conv);
        }
      }
    } catch {
      /* ignore */
    }

    // Restore one terminal per conversation (1:1 model)
    (async () => {
      try {
        if (termRestoreRanRef.current) return;
        termRestoreRanRef.current = true;
        const termRaw = localStorage.getItem(TERMINAL_DATA_KEY);
        if (!termRaw) return;
        restoringTerminalsRef.current = true;
        useStore.getState().setTerminalRestoreInProgress(true);
        const termData = JSON.parse(termRaw) as Record<string, PersistedTerminal[]>;
        const store = useStore.getState();
        const activeId = store.activeConversationId;

        // Helper: restore a single terminal for a conversation (first saved terminal only)
        const restoreTerminalForConv = async (
          convId: string,
          termInfo: PersistedTerminal,
          isActiveConv: boolean
        ) => {
          const existing = useStore.getState().conversationTerminals[convId] ?? [];
          if (existing.length > 0) {
            const existingTermId = existing[0];
            if (termInfo.scrollback && TerminalInstanceManager.has(existingTermId)) {
              const inst = TerminalInstanceManager.get(existingTermId);
              if (inst) inst.terminal.write(termInfo.scrollback);
            } else if (termInfo.scrollback) {
              TerminalInstanceManager.setPendingScrollback(existingTermId, termInfo.scrollback);
            }
            if (termInfo.customName)
              useStore.getState().setCustomTabName(existingTermId, termInfo.customName);
            if (termInfo.timelineBlocks?.length) {
              const blocksToRestore = termInfo.timelineBlocks;
              const tid = existingTermId;
              useStore.setState((state) => {
                if (!state.timelines[tid]) state.timelines[tid] = [];
                for (const block of blocksToRestore) {
                  if (block.type === "command") {
                    state.timelines[tid].push({
                      ...block,
                      data: { ...block.data, sessionId: tid },
                    });
                  } else {
                    state.timelines[tid].push(block as any);
                  }
                }
              });
            }
            return;
          }
          const termId = await createTerminalTab(
            termInfo.workingDirectory,
            true,
            termInfo.scrollback
          );
          if (!termId) return;
          useStore.getState().addTerminalToConversation(convId, termId);
          if (isActiveConv) useStore.getState().setActiveSession(termId);
          if (termInfo.customName)
            useStore.getState().setCustomTabName(termId, termInfo.customName);
          if (termInfo.timelineBlocks?.length) {
            const blocksToRestore = termInfo.timelineBlocks;
            const tid = termId;
            useStore.setState((state) => {
              if (!state.timelines[tid]) state.timelines[tid] = [];
              for (const block of blocksToRestore) {
                if (block.type === "command") {
                  state.timelines[tid].push({ ...block, data: { ...block.data, sessionId: tid } });
                } else {
                  state.timelines[tid].push(block as any);
                }
              }
            });
          }
        };

        // Eagerly restore terminal for the active conversation
        if (activeId && termData[activeId]?.[0] && store.conversations[activeId]) {
          await restoreTerminalForConv(activeId, termData[activeId][0], true);
        }

        useStore.getState().setTerminalRestoreInProgress(false);
        const savedActiveSessionId = useStore.getState().activeSessionId;

        // Pre-create terminals for other conversations in the background
        const otherConvs = Object.entries(termData).filter(
          ([convId, terminals]) =>
            convId !== activeId && store.conversations[convId] && terminals.length > 0
        );
        for (const [convId, savedTerms] of otherConvs) {
          const existing = useStore.getState().conversationTerminals[convId] ?? [];
          if (existing.length > 0) continue;
          await restoreTerminalForConv(convId, savedTerms[0], false);
        }

        if (savedActiveSessionId && useStore.getState().activeSessionId !== savedActiveSessionId) {
          useStore.getState().setActiveSession(savedActiveSessionId);
        }
      } catch (e) {
        console.warn("[AIChatPanel] Failed to restore conversation terminals:", e);
        useStore.getState().setTerminalRestoreInProgress(false);
      } finally {
        restoringTerminalsRef.current = false;
      }
    })();
  }, [createTerminalTab, workspaceDataReady]);

  // Persist conversations to localStorage immediately on every change
  useEffect(() => {
    if (conversations.length === 0) {
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify([]));
        localStorage.setItem(TERMINAL_DATA_KEY, JSON.stringify({}));
      } catch {
        /* ignore */
      }
      return;
    }
    try {
      const toSave = conversations
        .filter((c) => c.messages.length > 0)
        .slice(-MAX_STORED_CONVS)
        .map((c) => ({
          ...c,
          aiInitialized: false,
          isStreaming: false,
          messages: c.messages.map((m: ChatMessage) => ({ ...m, isStreaming: false })),
        }));
      localStorage.setItem(STORAGE_KEY, JSON.stringify(toSave));

      // Skip terminal data save while restore is in progress (prevents overwriting scrollback)
      if (restoringTerminalsRef.current) return;

      // Debounce terminal data serialization (expensive: serializes all xterm buffers)
      if (termSaveTimerRef.current) clearTimeout(termSaveTimerRef.current);
      termSaveTimerRef.current = setTimeout(() => {
        try {
          const store = useStore.getState();
          let existingTermData: Record<string, PersistedTerminal[]> = {};
          try {
            const raw = localStorage.getItem(TERMINAL_DATA_KEY);
            if (raw) existingTermData = JSON.parse(raw);
          } catch {
            /* ignore */
          }
          const termData: Record<string, PersistedTerminal[]> = {};
          for (const c of toSave) {
            const termIds = store.conversationTerminals[c.id] ?? [];
            if (termIds.length === 0) {
              if (existingTermData[c.id]) termData[c.id] = existingTermData[c.id];
              continue;
            }
            const terminals: PersistedTerminal[] = [];
            for (let i = 0; i < termIds.length; i++) {
              const tid = termIds[i];
              const session = store.sessions[tid];
              if (!session?.workingDirectory) continue;
              let scrollback = TerminalInstanceManager.serialize(tid);
              if (scrollback.length > MAX_SCROLLBACK_CHARS)
                scrollback = scrollback.slice(-MAX_SCROLLBACK_CHARS);
              if (!scrollback && existingTermData[c.id]?.[i]?.scrollback)
                scrollback = existingTermData[c.id][i].scrollback;
              const customName = session.customName || existingTermData[c.id]?.[i]?.customName;
              const blocks: Array<import("@/lib/workspace-storage").PersistedTimelineBlock> = [];
              const timeline = store.timelines[tid];
              if (timeline) {
                for (const block of timeline) {
                  if (block.type === "command") {
                    let output = block.data.output;
                    if (output.length > MAX_BLOCK_OUTPUT_CHARS)
                      output = output.slice(-MAX_BLOCK_OUTPUT_CHARS);
                    blocks.push({
                      id: block.id,
                      type: "command",
                      timestamp: block.timestamp,
                      data: { ...block.data, output },
                    });
                  } else if (block.type === "pipeline_progress") {
                    blocks.push({
                      id: block.id,
                      type: "pipeline_progress",
                      timestamp: block.timestamp,
                      data: block.data,
                    });
                  } else if (block.type === "ai_tool_execution") {
                    const data = { ...block.data };
                    if (data.streamingOutput && data.streamingOutput.length > MAX_BLOCK_OUTPUT_CHARS) {
                      data.streamingOutput = data.streamingOutput.slice(-MAX_BLOCK_OUTPUT_CHARS);
                    }
                    blocks.push({ id: block.id, type: "ai_tool_execution", timestamp: block.timestamp, data });
                  } else if (block.type === "sub_agent_activity") {
                    const data = { ...block.data };
                    if (data.streamingText && data.streamingText.length > MAX_BLOCK_OUTPUT_CHARS) {
                      data.streamingText = data.streamingText.slice(-MAX_BLOCK_OUTPUT_CHARS);
                    }
                    blocks.push({ id: block.id, type: "sub_agent_activity", timestamp: block.timestamp, data, batchId: block.batchId });
                  }
                }
              }
              const entry: PersistedTerminal = {
                workingDirectory: session.workingDirectory,
                scrollback,
                customName,
              };
              entry.timelineBlocks = blocks.slice(-MAX_SAVED_BLOCKS);
              if (
                !entry.timelineBlocks.length &&
                existingTermData[c.id]?.[i]?.timelineBlocks?.length
              ) {
                entry.timelineBlocks = existingTermData[c.id][i].timelineBlocks;
              }
              terminals.push(entry);
            }
            if (terminals.length > 0) {
              termData[c.id] = terminals;
            } else if (existingTermData[c.id]) {
              termData[c.id] = existingTermData[c.id];
            }
          }
          localStorage.setItem(TERMINAL_DATA_KEY, JSON.stringify(termData));
        } catch {
          /* ignore */
        }
      }, 500);
    } catch {
      /* ignore */
    }
  }, [conversations]);

  const [selectedModel, setSelectedModel] = useState<{ model: string; provider: string } | null>(
    () => {
      try {
        const saved = localStorage.getItem("golish-pentest-ai-model");
        return saved ? JSON.parse(saved) : null;
      } catch {
        return null;
      }
    }
  );
  const modelDisplay = selectedModel?.model ? formatModelName(selectedModel.model) : "No Model";

  // Generate a short title for a conversation using the AI
  const generateTitle = useCallback(
    async (convId: string, firstMessage: string) => {
      if (!selectedModel?.model || !selectedModel?.provider) return;
      const titleSessionId = `title-gen-${convId}`;
      try {
        const settings = await getSettings();
        const titleWorkspace = useStore.getState().currentProjectPath || ".";
        const { model, provider } = selectedModel;
        let providerConfig: ProviderConfig;
        switch (provider) {
          case "anthropic":
            providerConfig = {
              provider: "anthropic",
              workspace: titleWorkspace,
              model,
              api_key: settings.ai.anthropic?.api_key || "",
            };
            break;
          case "openai":
            providerConfig = {
              provider: "openai",
              workspace: titleWorkspace,
              model,
              api_key: settings.ai.openai?.api_key || "",
            };
            break;
          case "openrouter":
            providerConfig = {
              provider: "openrouter",
              workspace: titleWorkspace,
              model,
              api_key: settings.ai.openrouter?.api_key || "",
            };
            break;
          case "gemini":
            providerConfig = {
              provider: "gemini",
              workspace: titleWorkspace,
              model,
              api_key: settings.ai.gemini?.api_key || "",
            };
            break;
          case "groq":
            providerConfig = {
              provider: "groq",
              workspace: titleWorkspace,
              model,
              api_key: settings.ai.groq?.api_key || "",
            };
            break;
          case "nvidia":
            providerConfig = {
              provider: "nvidia",
              workspace: titleWorkspace,
              model,
              api_key: settings.ai.nvidia?.api_key || "",
            };
            break;
          case "ollama":
            providerConfig = { provider: "ollama", workspace: titleWorkspace, model };
            break;
          default:
            return;
        }
        await initAiSession(titleSessionId, providerConfig);
        const title = await sendPromptSession(
          titleSessionId,
          `Generate a concise 3-5 word title for this chat message. Output ONLY the title, nothing else. No quotes, no punctuation at the end.\n\nMessage: "${firstMessage.slice(0, 200)}"`
        );
        const cleaned = title
          .trim()
          .replace(/^["']|["']$/g, "")
          .slice(0, 40);
        if (cleaned) {
          useStore.getState().updateConversation(convId, { title: cleaned });
        }
      } catch {
        // Title generation failed silently - keep existing title
      } finally {
        shutdownAiSession(titleSessionId).catch(() => {});
      }
    },
    [selectedModel]
  );
  generateTitleRef.current = generateTitle;

  // Load available pentest tools on mount
  useEffect(() => {
    scanTools()
      .then((result) => {
        if (result.success) {
          setPentestTools(result.tools.filter((t) => t.installed));
        }
      })
      .catch(() => {});
  }, []);


  // Load configured providers from settings
  useEffect(() => {
    const loadProviders = () => {
      getSettings()
        .then((settings) => {
          const configured = new Set<string>();
          const ai = settings.ai;
          if (ai.anthropic?.api_key) configured.add("anthropic");
          if (ai.openai?.api_key) configured.add("openai");
          if (ai.openrouter?.api_key) configured.add("openrouter");
          if (ai.gemini?.api_key) configured.add("gemini");
          if (ai.groq?.api_key) configured.add("groq");
          if (ai.xai?.api_key) configured.add("xai");
          if (ai.zai_sdk?.api_key) configured.add("zai_sdk");
          if (ai.nvidia?.api_key) configured.add("nvidia");
          if (ai.vertex_ai?.credentials_path || ai.vertex_ai?.project_id)
            configured.add("vertex_ai");
          if (ai.vertex_gemini?.credentials_path || ai.vertex_gemini?.project_id)
            configured.add("vertex_gemini");
          configured.add("ollama");
          setConfiguredProviders(configured);
        })
        .catch(() => {});
    };

    loadProviders();
    window.addEventListener("settings-updated", loadProviders);
    return () => window.removeEventListener("settings-updated", loadProviders);
  }, []);

  // Set up AI event listener
  useEffect(() => {
    let mounted = true;

    const setup = async () => {
      try {
        const unlisten = await onAiEvent((event: AiEvent) => {
          if (!mounted) return;

          console.debug(
            "[AIChatPanel] AI event received:",
            event.type,
            "session:",
            event.session_id
          );

          const store = useStore.getState();
          const conv = store.getConversationBySessionId(event.session_id);
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

              const currentMode = localStorage.getItem("golish-approval-mode") || "ask";
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
              break;
            }

            case "reasoning": {
              store.appendMessageThinking(convId, event.content);
              break;
            }

            case "completed": {
              store.finalizeStreamingMessage(convId, event.response, event.reasoning);
              streamingMsgRef.current = null;
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
              // Record text offset on first plan creation for inline positioning
              if (planTextOffsetRef.current === null) {
                const currentConv = useStore.getState().conversations[convId];
                const lastMsg = currentConv?.messages?.[currentConv.messages.length - 1];
                if (lastMsg?.role === "assistant") {
                  planTextOffsetRef.current = (lastMsg.content || "").length;
                }
              }
              setTaskPlan({
                version: event.version,
                steps: event.steps,
                summary: event.summary,
              });
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
          unlistenRef.current = unlisten;
        } else {
          unlisten();
        }
      } catch {
        // AI backend not available
      }
    };

    setup();

    return () => {
      mounted = false;
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, []);

  // Auto-scroll tabs to show the active tab
  useEffect(() => {
    if (tabsRef.current) {
      const activeTab = tabsRef.current.querySelector(`[data-conv-id="${activeConvId}"]`);
      activeTab?.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
    }
  }, [activeConvId]);

  // When switching conversations, activate its terminal or lazily restore from saved data
  useEffect(() => {
    if (!activeConvId) return;
    const store = useStore.getState();
    const terminals = store.conversationTerminals[activeConvId];
    if (terminals && terminals.length > 0) {
      const firstTerminal = terminals[0];
      if (store.sessions[firstTerminal] && store.activeSessionId !== firstTerminal) {
        store.setActiveSession(firstTerminal);
      }
      return;
    }
    // Lazy restore: create ALL terminals in parallel when user switches to this conversation
    const pendingTerminals = pendingTermRestoreRef.current[activeConvId];
    if (pendingTerminals) {
      delete pendingTermRestoreRef.current[activeConvId];
      (async () => {
        restoringTerminalsRef.current = true;
        try {
          const createdIds = await Promise.all(
            pendingTerminals.map((p) => createTerminalTab(p.workingDirectory, true, p.scrollback))
          );
          for (let i = 0; i < createdIds.length; i++) {
            const termId = createdIds[i];
            if (!termId) continue;
            const pending = pendingTerminals[i];
            useStore.getState().addTerminalToConversation(activeConvId, termId);
            if (i === 0) useStore.getState().setActiveSession(termId);
            if (pending.customName)
              useStore.getState().setCustomTabName(termId, pending.customName);
            if (pending.timelineBlocks?.length) {
              const blocksToRestore = pending.timelineBlocks;
              const tid = termId;
              useStore.setState((state) => {
                if (!state.timelines[tid]) state.timelines[tid] = [];
                for (const block of blocksToRestore) {
                  if (block.type === "command") {
                    state.timelines[tid].push({
                      ...block,
                      data: { ...block.data, sessionId: tid },
                    });
                  } else {
                    state.timelines[tid].push(block as any);
                  }
                }
              });
            }
          }
        } finally {
          restoringTerminalsRef.current = false;
        }
      })();
    }
  }, [activeConvId, createTerminalTab]);

  // Custom scrollbar state
  const [tabsHovered, setTabsHovered] = useState(false);
  const [scrollThumb, setScrollThumb] = useState({ left: 0, width: 0, visible: false });
  const thumbDragRef = useRef<{ startX: number; startScroll: number } | null>(null);

  const updateScrollThumb = useCallback(() => {
    const el = tabsRef.current;
    if (!el) return;
    const hasOverflow = el.scrollWidth > el.clientWidth + 1;
    if (!hasOverflow) {
      setScrollThumb({ left: 0, width: 0, visible: false });
      return;
    }
    const ratio = el.clientWidth / el.scrollWidth;
    const thumbWidth = Math.max(ratio * 100, 10);
    const scrollRange = el.scrollWidth - el.clientWidth;
    const thumbLeft = scrollRange > 0 ? (el.scrollLeft / scrollRange) * (100 - thumbWidth) : 0;
    setScrollThumb({ left: thumbLeft, width: thumbWidth, visible: true });
  }, []);

  useEffect(() => {
    const el = tabsRef.current;
    if (!el) return;
    updateScrollThumb();
    el.addEventListener("scroll", updateScrollThumb, { passive: true });
    const observer = new ResizeObserver(updateScrollThumb);
    observer.observe(el);
    return () => {
      el.removeEventListener("scroll", updateScrollThumb);
      observer.disconnect();
    };
  }, [updateScrollThumb, conversations.length]);

  // Mouse wheel -> horizontal scroll
  useEffect(() => {
    const el = tabsRef.current;
    if (!el) return;
    const handler = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
        e.preventDefault();
        el.scrollLeft += e.deltaY;
      }
    };
    el.addEventListener("wheel", handler, { passive: false });
    return () => el.removeEventListener("wheel", handler);
  }, []);

  const handleThumbDragStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const el = tabsRef.current;
    if (!el) return;
    thumbDragRef.current = { startX: e.clientX, startScroll: el.scrollLeft };
    const onMove = (ev: MouseEvent) => {
      if (!thumbDragRef.current || !tabsRef.current) return;
      const trackEl = tabsRef.current;
      const dx = ev.clientX - thumbDragRef.current.startX;
      const trackWidth = trackEl.clientWidth;
      const scrollRange = trackEl.scrollWidth - trackEl.clientWidth;
      trackEl.scrollLeft = thumbDragRef.current.startScroll + (dx / trackWidth) * scrollRange;
    };
    const onUp = () => {
      thumbDragRef.current = null;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  }, []);

  // Track user intent for auto-scroll: only wheel/touch events can set/clear this
  const userScrolledUpRef = useRef(false);

  useEffect(() => {
    const container = messagesContainerRef.current;
    if (!container) return;

    const isAtBottom = () => {
      const { scrollTop, scrollHeight, clientHeight } = container;
      return scrollHeight - scrollTop - clientHeight < 80;
    };

    // Only wheel events control the userScrolledUp flag — scroll events from
    // programmatic scrollTop assignment must NOT accidentally re-enable auto-scroll
    const handleWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) {
        userScrolledUpRef.current = true;
      } else if (e.deltaY > 0) {
        requestAnimationFrame(() => {
          if (isAtBottom()) userScrolledUpRef.current = false;
        });
      }
    };

    const handleScroll = () => {
      chatAtBottomRef.current = isAtBottom();
    };

    container.addEventListener("wheel", handleWheel, { passive: true });
    container.addEventListener("scroll", handleScroll, { passive: true });
    return () => {
      container.removeEventListener("wheel", handleWheel);
      container.removeEventListener("scroll", handleScroll);
    };
  }, []);

  // Auto-scroll: only when user hasn't deliberately scrolled up
  useEffect(() => {
    if (!userScrolledUpRef.current) {
      const container = messagesContainerRef.current;
      if (container) {
        container.scrollTop = container.scrollHeight;
      }
    }
  }, [messages]);

  const handleNewChat = useCallback(async () => {
    const conv = createNewConversation();
    addConversation(conv);
    const termId = await createTerminalTab(undefined, true);
    if (termId) {
      useStore.getState().addTerminalToConversation(conv.id, termId);
      useStore.getState().setActiveSession(termId);
    }
    setInput("");
    setShowHistory(false);
    requestAnimationFrame(() => {
      textareaRef.current?.focus();
    });
  }, [addConversation, createTerminalTab]);

  const handleCloseTab = useCallback(
    (convId: string, e: React.MouseEvent) => {
      e.stopPropagation();
      const storeBefore = useStore.getState();
      const conv = storeBefore.conversations[convId];
      if (conv?.aiInitialized) {
        shutdownAiSession(conv.aiSessionId).catch(() => {});
      }

      // Close all terminals belonging to this conversation
      const terminalIds = storeBefore.conversationTerminals[convId] ?? [];
      for (const termId of terminalIds) {
        storeBefore.closeTab(termId);
      }

      removeConv(convId);

      // Re-read store AFTER removal to get updated state
      const storeAfter = useStore.getState();
      if (storeAfter.conversationOrder.length === 0) {
        const fresh = createNewConversation();
        addConversation(fresh);
        void createTerminalTab(undefined, true).then((termId) => {
          if (termId) {
            useStore.getState().addTerminalToConversation(fresh.id, termId);
            useStore.getState().setActiveSession(termId);
          }
        });
      }
    },
    [removeConv, addConversation, createTerminalTab]
  );

  const handleModelSelect = useCallback((modelId: string, provider: string) => {
    const sel = { model: modelId, provider };
    setSelectedModel(sel);
    try {
      localStorage.setItem("golish-pentest-ai-model", JSON.stringify(sel));
    } catch {}
  }, []);

  const buildPentestSystemPrompt = useCallback(() => {
    const store = useStore.getState();
    const convId = store.activeConversationId;

    // Build terminal context
    let terminalContext = "";
    if (convId) {
      const terminalIds = store.conversationTerminals[convId] ?? [];
      if (terminalIds.length > 0) {
        const terminalDescs = terminalIds
          .map((id, idx) => {
            const session = store.sessions[id];
            if (!session) return null;
            const dir = session.workingDirectory || "(unknown)";
            const name = session.customName || session.processName || `Terminal ${idx + 1}`;
            return `  - [${id}] "${name}" (cwd: ${dir})`;
          })
          .filter(Boolean)
          .join("\n");

        terminalContext = `\n\nYour managed terminals:\n${terminalDescs}`;
      }
    }

    // Build pentest tools context
    let toolContext = "";
    if (pentestTools.length > 0) {
      const toolDescs = pentestTools
        .map((t) => {
          const params = (
            t as unknown as {
              params?: Array<{ label: string; flag: string; type: string; description?: string }>;
            }
          ).params;
          const paramStr = params
            ? params
                .map(
                  (p) =>
                    `  - ${p.flag || "(positional)"} ${p.label} (${p.type})${p.description ? `: ${p.description}` : ""}`
                )
                .join("\n")
            : "";
          return `### ${t.name} (${t.runtime})\n${t.description}\n${paramStr}`;
        })
        .join("\n\n");

      toolContext = `\n\nAvailable installed pentest tools:\n${toolDescs}`;
    }

    return `You are Golish AI, a penetration testing and terminal assistant. You help security professionals plan and execute security assessments, and you have direct control over terminal sessions.

## Core Capabilities

### Terminal Control
You can execute any shell command using the run_pty_cmd tool:
- run_pty_cmd: Execute a shell command and return stdout/stderr/exit_code
  - "command" (required): The shell command to execute
  - "cwd" (optional): Working directory path (use terminal cwd from context below)
  - "timeout" (optional): Timeout in seconds (default: 120)

Use run_pty_cmd for all command execution needs: running tools, checking system state, installing packages, network operations, file manipulation, etc.${terminalContext}

### Penetration Testing Tools
- pentest_list_tools: List all available pentest tools, their skills, and skill documents
- pentest_run: Execute a specific pentest tool by name with arguments
- pentest_read_skill: Read a skill document (Markdown) for detailed tool usage instructions${toolContext}

### File Operations
- read_file: Read file contents
- write_file / create_file: Write or create files
- edit_file: Make targeted edits to existing files
- delete_file: Delete a file
- list_files / list_directory: Browse directory contents
- grep_file: Search for patterns in files

## Guidelines
- When the user asks to run a command, use run_pty_cmd with the appropriate working directory from your managed terminals.
- When running pentest tools specifically, prefer pentest_run as it handles runtime resolution (Python envs, Java paths, etc.) automatically.
- Use pentest_read_skill to learn detailed usage patterns before running unfamiliar tools.
- For long-running commands, set an appropriate timeout.
- Always report command output and exit codes to the user.`;
  }, [pentestTools]);

  const initializeSession = useCallback(
    async (conv: { id: string; aiSessionId: string; aiInitialized: boolean }) => {
      if (conv.aiInitialized) return true;

      if (!selectedModel?.model || !selectedModel?.provider) {
        return false;
      }

      try {
        const settings = await getSettings();
        const workspace = useStore.getState().currentProjectPath || ".";
        const { model, provider } = selectedModel;
        let providerConfig: ProviderConfig;

        switch (provider) {
          case "anthropic":
            providerConfig = {
              provider: "anthropic",
              workspace,
              model,
              api_key: settings.ai.anthropic.api_key || "",
            };
            break;
          case "openai":
            providerConfig = {
              provider: "openai",
              workspace,
              model,
              api_key: settings.ai.openai.api_key || "",
            };
            break;
          case "openrouter":
            providerConfig = {
              provider: "openrouter",
              workspace,
              model,
              api_key: settings.ai.openrouter.api_key || "",
            };
            break;
          case "gemini":
            providerConfig = {
              provider: "gemini",
              workspace,
              model,
              api_key: settings.ai.gemini.api_key || "",
            };
            break;
          case "groq":
            providerConfig = {
              provider: "groq",
              workspace,
              model,
              api_key: settings.ai.groq.api_key || "",
            };
            break;
          case "xai":
            providerConfig = {
              provider: "xai",
              workspace,
              model,
              api_key: settings.ai.xai.api_key || "",
            };
            break;
          case "zai_sdk":
            providerConfig = {
              provider: "zai_sdk",
              workspace,
              model,
              api_key: settings.ai.zai_sdk.api_key || "",
            };
            break;
          case "nvidia":
            providerConfig = {
              provider: "nvidia",
              workspace,
              model,
              api_key: settings.ai.nvidia.api_key || "",
            };
            break;
          case "vertex_ai":
            providerConfig = {
              provider: "vertex_ai",
              workspace,
              model,
              credentials_path: settings.ai.vertex_ai.credentials_path ?? "",
              project_id: settings.ai.vertex_ai.project_id ?? "",
              location: settings.ai.vertex_ai.location ?? "us-east5",
            };
            break;
          case "vertex_gemini":
            providerConfig = {
              provider: "vertex_gemini",
              workspace,
              model,
              credentials_path: settings.ai.vertex_gemini.credentials_path ?? "",
              project_id: settings.ai.vertex_gemini.project_id ?? "",
              location: settings.ai.vertex_gemini.location ?? "us-east5",
            };
            break;
          case "ollama":
            providerConfig = { provider: "ollama", workspace, model };
            break;
          default:
            return false;
        }

        await initAiSession(conv.aiSessionId, providerConfig);

        // Restore conversation history so the AI retains context from previous messages
        const existingMessages = useStore.getState().conversations[conv.id]?.messages ?? [];
        if (existingMessages.length > 0) {
          const pairs: [string, string][] = existingMessages
            .filter((m) => m.role === "user" || m.role === "assistant")
            .map((m) => [m.role, m.content] as [string, string]);
          if (pairs.length > 0) {
            try {
              await restoreAiConversation(conv.aiSessionId, pairs);
              console.debug(
                "[AIChatPanel] Restored",
                pairs.length,
                "messages for session",
                conv.aiSessionId
              );
            } catch (restoreErr) {
              console.warn("[AIChatPanel] Failed to restore conversation history:", restoreErr);
            }
          }
        }

        // Sync the stored approval/agent mode to the backend
        const savedMode = localStorage.getItem("golish-approval-mode") || "ask";
        const backendMode: AgentMode = savedMode === "run-all" ? "auto-approve" : "default";
        await setAgentMode(conv.aiSessionId, backendMode).catch(console.warn);

        updateConv(conv.id, { aiInitialized: true });
        return true;
      } catch (err) {
        console.error("[AIChatPanel] Failed to initialize AI session:", err);
        return false;
      }
    },
    [selectedModel, updateConv]
  );

  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text || isStreaming) return;

    if (!activeConvId) return;
    const conv = useStore.getState().conversations[activeConvId];
    if (!conv) return;

    const userMsg: ChatMessage = {
      id: `user-${Date.now()}`,
      role: "user",
      content: text,
      timestamp: Date.now(),
    };

    // Update title on first message
    const newTitle =
      conv.title === "New Chat" ? text.slice(0, 30) + (text.length > 30 ? "..." : "") : conv.title;

    addConversationMessage(conv.id, userMsg);
    if (newTitle !== conv.title) {
      updateConv(conv.id, { title: newTitle });
    }
    setInput("");
    if (textareaRef.current) textareaRef.current.style.height = "auto";
    userScrolledUpRef.current = false;

    // Ensure the conversation has a linked terminal tab and it's active
    const storeNow = useStore.getState();
    const convTerminals = storeNow.conversationTerminals[conv.id] ?? [];
    let activeTermId: string | null = null;
    if (convTerminals.length === 0) {
      // No terminal linked — prefer adopting the currently active terminal over creating a new one
      const currentActive = storeNow.activeSessionId;
      if (currentActive && storeNow.sessions[currentActive]) {
        // Check this terminal isn't already owned by another conversation
        const ownerConv = storeNow.getConversationForTerminal(currentActive);
        if (!ownerConv || ownerConv === conv.id) {
          activeTermId = currentActive;
          storeNow.addTerminalToConversation(conv.id, currentActive);
        }
      }
      // If no existing terminal could be adopted, create a new one
      if (!activeTermId) {
        try {
          activeTermId = await createTerminalTab(undefined, true);
          if (activeTermId) {
            useStore.getState().addTerminalToConversation(conv.id, activeTermId);
          }
        } catch (e) {
          console.warn("[AIChatPanel] Failed to create terminal for conversation:", e);
        }
      }
    } else {
      activeTermId = convTerminals[0];
      if (storeNow.sessions[activeTermId] && storeNow.activeSessionId !== activeTermId) {
        storeNow.setActiveSession(activeTermId);
      }
    }

    // Explicitly sync active terminal to backend before AI processes the prompt
    if (activeTermId) {
      try {
        const { setActiveTerminalSession } = await import("@/lib/tauri");
        await setActiveTerminalSession(activeTermId);
      } catch {
        /* ignore */
      }
    }

    // Initialize session if needed
    const initialized = await initializeSession(conv);
    if (!initialized) {
      setMessageError(
        conv.id,
        t("ai.noModelSelected", "Please select a model first (bottom-left dropdown)")
      );
      return;
    }

    // Prepend system context with pentest tools info on first message
    let prompt = text;
    if (conv.messages.length === 0) {
      const systemPrompt = buildPentestSystemPrompt();
      if (systemPrompt) {
        prompt = `[System Context]\n${systemPrompt}\n\n[User Message]\n${text}`;
      }
    }

    try {
      setConversationStreaming(conv.id, true);
      console.debug("[AIChatPanel] Sending prompt to session:", conv.aiSessionId);

      if (imageAttachments.length > 0) {
        const payload = createTextPayload(prompt);
        for (const img of imageAttachments) {
          payload.parts.push({
            type: "image",
            data: img.data,
            media_type: img.mediaType,
          });
        }
        await sendPromptWithAttachments(conv.aiSessionId, payload);
        setImageAttachments([]);
      } else {
        await sendPromptSession(conv.aiSessionId, prompt);
      }
      // Safety timeout: if streaming is still active after 30s with no new content, reset.
      // Some models don't send a proper stop signal.
      const convId = conv.id;
      let lastMsgLength = 0;
      let idleChecks = 0;
      const checkInterval = setInterval(() => {
        const s = useStore.getState();
        const c = s.conversations[convId];
        if (!c?.isStreaming) {
          clearInterval(checkInterval);
          return;
        }
        const currentLength = c.messages[c.messages.length - 1]?.content?.length ?? 0;
        if (currentLength === lastMsgLength) {
          idleChecks++;
          if (idleChecks >= 3) {
            console.warn("[AIChatPanel] Idle timeout: resetting stuck streaming for", convId);
            s.finalizeStreamingMessage(convId);
            clearInterval(checkInterval);
          }
        } else {
          lastMsgLength = currentLength;
          idleChecks = 0;
        }
      }, 5_000);
    } catch (err) {
      const errMsg = err instanceof Error ? err.message : String(err);
      setMessageError(conv.id, errMsg);
    }
  }, [
    input,
    isStreaming,
    activeConvId,
    initializeSession,
    buildPentestSystemPrompt,
    updateConv,
    addConversationMessage,
    setConversationStreaming,
    setMessageError,
    t,
  ]);

  // AskHuman handlers
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

  // Image attachment handler
  const handleImageUpload = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files) return;
    for (const file of Array.from(files)) {
      if (!file.type.startsWith("image/")) continue;
      const reader = new FileReader();
      reader.onload = () => {
        const base64 = (reader.result as string).split(",")[1];
        if (base64) {
          setImageAttachments((prev) => [
            ...prev,
            { data: base64, mediaType: file.type, name: file.name },
          ]);
        }
      };
      reader.readAsDataURL(file);
    }
    e.target.value = "";
  }, []);

  const handleApprovalModeChange = useCallback(
    (mode: ApprovalMode) => {
      setApprovalMode(mode);
      localStorage.setItem("golish-approval-mode", mode);
      if (!activeConv) return;
      const backendMode: AgentMode = mode === "run-all" ? "auto-approve" : "default";
      setAgentMode(activeConv.aiSessionId, backendMode).catch(console.error);
    },
    [activeConv]
  );

  const handleStop = useCallback(() => {
    if (!activeConv) return;
    shutdownAiSession(activeConv.aiSessionId).catch(() => {});
    streamingMsgRef.current = null;
    finalizeStreamingMessage(activeConv.id);
    updateConv(activeConv.id, { aiInitialized: false });
  }, [activeConv, finalizeStreamingMessage, updateConv]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend]
  );

  const handleTextareaInput = useCallback(() => {
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
      textareaRef.current.style.height = `${Math.min(textareaRef.current.scrollHeight, 160)}px`;
    }
  }, []);

  const currentModel = selectedModel?.model ?? "";
  const currentProvider = selectedModel?.provider ?? "";

  return (
    <div className="flex flex-col h-full">
      {/* Tab Bar */}
      <div
        className="relative flex flex-col flex-shrink-0"
        onMouseEnter={() => setTabsHovered(true)}
        onMouseLeave={() => setTabsHovered(false)}
      >
        <div className="h-[37px] flex items-center px-2 gap-1.5">
          <div
            ref={tabsRef}
            className="flex-1 flex items-center gap-1.5 overflow-x-auto scrollbar-none min-w-0"
          >
            {conversations.map((conv) => (
              <button
                key={conv.id}
                type="button"
                data-conv-id={conv.id}
                className={cn(
                  "group flex items-center gap-1.5 h-[28px] px-3 text-[12px] whitespace-nowrap flex-shrink-0 transition-all rounded-lg",
                  conv.id === activeConvId
                    ? "text-foreground bg-[var(--bg-hover)]"
                    : "text-muted-foreground hover:text-foreground/80"
                )}
                onClick={() => {
                  setActiveConversation(conv.id);
                  setShowHistory(false);
                }}
              >
                {conv.id === activeConvId && (
                  <div className="w-1.5 h-1.5 rounded-full bg-accent/50 flex-shrink-0" />
                )}
                <span className="max-w-[120px] truncate">{conv.title}</span>
                <span
                  className={cn(
                    "w-4 h-4 flex items-center justify-center rounded-full transition-opacity",
                    conv.id === activeConvId
                      ? "opacity-60 hover:opacity-100"
                      : "opacity-0 group-hover:opacity-60 hover:!opacity-100"
                  )}
                  onClick={(e) => handleCloseTab(conv.id, e)}
                  onKeyDown={() => {}}
                  role="button"
                  tabIndex={-1}
                >
                  <X className="w-2.5 h-2.5" />
                </span>
              </button>
            ))}
          </div>
          <div className="flex items-center gap-0.5 flex-shrink-0">
            <button
              type="button"
              title={t("ai.newChat")}
              className="h-6 w-6 flex items-center justify-center rounded-md text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)] transition-colors"
              onClick={handleNewChat}
            >
              <Plus className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              title={t("ai.history")}
              className={cn(
                "h-6 w-6 flex items-center justify-center rounded-md transition-colors",
                showHistory
                  ? "text-foreground bg-[var(--bg-hover)]"
                  : "text-muted-foreground hover:text-foreground hover:bg-[var(--bg-hover)]"
              )}
              onClick={() => setShowHistory((v) => !v)}
            >
              <Clock className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
        {/* Custom scrollbar track */}
        {tabsHovered && scrollThumb.visible && (
          <div className="h-[3px] mx-2">
            <div className="relative h-full w-full">
              {/* biome-ignore lint/a11y/noStaticElementInteractions: scrollbar thumb is drag-only */}
              <div
                className="absolute h-full rounded-full bg-foreground/20 hover:bg-foreground/35 cursor-pointer"
                style={{ left: `${scrollThumb.left}%`, width: `${scrollThumb.width}%` }}
                onMouseDown={handleThumbDragStart}
              />
            </div>
          </div>
        )}
      </div>

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
                  onClick={() => {
                    setActiveConversation(conv.id);
                    setShowHistory(false);
                  }}
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
        <div ref={messagesContainerRef} className="flex-1 overflow-y-auto overflow-x-hidden">
          {messages.length === 0 ? (
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
              <p className="text-[13px] text-muted-foreground/50">{t("ai.placeholder")}</p>
              {pentestTools.length > 0 && (
                <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground/30">
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
                // Show plan card inline on the last assistant message
                const isLastAssistant =
                  msg.role === "assistant" &&
                  !messages.slice(msgIdx + 1).some((m) => m.role === "assistant");
                return (
                  <MessageBlock
                    key={msg.id}
                    message={msg}
                    taskPlan={isLastAssistant ? taskPlan : null}
                    planTextOffset={isLastAssistant ? planTextOffsetRef.current : null}
                    pendingApproval={
                      pendingApproval
                        ? { requestId: pendingApproval.requestId, toolName: pendingApproval.toolName }
                        : null
                    }
                    approvalMode={approvalMode}
                    onApprovalModeChange={handleApprovalModeChange}
                    onApprove={(requestId) => {
                      if (!pendingApproval) return;
                      respondToToolApproval(pendingApproval.sessionId, {
                        request_id: requestId,
                        approved: true,
                        remember: false,
                        always_allow: false,
                      }).catch(console.error);
                      setPendingApproval(null);
                    }}
                    onDeny={(requestId) => {
                      if (!pendingApproval) return;
                      respondToToolApproval(pendingApproval.sessionId, {
                        request_id: requestId,
                        approved: false,
                        remember: false,
                        always_allow: false,
                      }).catch(console.error);
                      setPendingApproval(null);
                    }}
                  />
                );
              })}

              {/* Active Workflow */}
              {activeWorkflow && <WorkflowProgress workflow={activeWorkflow} />}

              {/* Agent Summary */}
              <AgentSummaryBar />

              {/* Context Compaction */}
              {compactionState && (
                <CompactionNotice
                  active={compactionState.active}
                  tokensBefore={compactionState.tokensBefore}
                />
              )}

              {/* AskHuman Dialog */}
              {askHumanRequest && (
                <AskHumanInline
                  request={askHumanRequest}
                  onSubmit={handleAskHumanSubmit}
                  onSkip={handleAskHumanSkip}
                />
              )}

              <div ref={messagesEndRef} />
            </div>
          )}
        </div>
      )}

      {/* Input Area */}
      <div className="p-3 flex-shrink-0">
        <div className="rounded-lg border border-[var(--border-subtle)] bg-background overflow-hidden focus-within:border-muted-foreground/30 transition-colors">
          {/* Image attachment preview */}
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
            className={cn(
              "w-full bg-transparent border-none outline-none resize-none",
              "text-[13px] text-foreground placeholder:text-muted-foreground/40",
              "leading-relaxed max-h-[160px] px-3 pt-2.5 pb-1.5"
            )}
          />
          {/* Bottom toolbar */}
          <div className="flex items-center justify-between px-2.5 pb-2">
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                className="flex items-center gap-1 px-2 py-1 rounded-md bg-muted text-[11px] text-foreground font-medium hover:bg-[var(--bg-hover)] transition-colors"
              >
                <span className="text-accent">∞</span>
                Agent
                <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
              </button>

              {/* Model selector dropdown */}
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    type="button"
                    className="flex items-center gap-1 px-2 py-1 rounded-md text-[11px] text-accent hover:bg-[var(--bg-hover)] transition-colors"
                  >
                    <Cpu className="w-3 h-3" />
                    {modelDisplay}
                    <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent
                  align="start"
                  side="top"
                  className="bg-card border-[var(--border-medium)] min-w-[200px] max-h-[400px] overflow-y-auto"
                >
                  {(() => {
                    const filtered = PROVIDER_GROUPS.filter((group) =>
                      configuredProviders.has(group.provider)
                    );
                    if (filtered.length === 0) {
                      return (
                        <div className="px-3 py-4 text-center">
                          <p className="text-xs text-muted-foreground">
                            {t("ai.noProviders", "No providers configured")}
                          </p>
                          <p className="text-[10px] text-muted-foreground/60 mt-1">
                            {t(
                              "ai.configureInSettings",
                              "Configure API keys in Settings → Providers"
                            )}
                          </p>
                        </div>
                      );
                    }
                    return filtered.map((group, gi) => (
                      <div key={group.provider}>
                        {gi > 0 && <DropdownMenuSeparator />}
                        <div className="px-2 py-1 text-[10px] text-muted-foreground uppercase tracking-wide">
                          {group.providerName}
                        </div>
                        {group.models.map((model) => {
                          const isSelected =
                            currentModel === model.id &&
                            (currentProvider === group.provider ||
                              currentProvider === "anthropic_vertex");
                          return (
                            <DropdownMenuItem
                              key={`${group.provider}-${model.id}-${model.reasoningEffort ?? ""}`}
                              onClick={() => handleModelSelect(model.id, group.provider)}
                              className={cn(
                                "text-xs cursor-pointer",
                                isSelected
                                  ? "text-accent bg-[var(--accent-dim)]"
                                  : "text-foreground hover:text-accent"
                              )}
                            >
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
              <div
                className="relative group"
                title={
                  contextUsage
                    ? `${(contextUsage.utilization * 100).toFixed(1)}% · ${(contextUsage.totalTokens / 1000).toFixed(1)}K / ${(contextUsage.maxTokens / 1000).toFixed(0)}K context used`
                    : "No context data"
                }
              >
                <svg className="w-5 h-5 -rotate-90" viewBox="0 0 20 20">
                  <circle
                    cx="10"
                    cy="10"
                    r="8"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    className="text-muted-foreground/20"
                  />
                  <circle
                    cx="10"
                    cy="10"
                    r="8"
                    fill="none"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeDasharray={`${(contextUsage?.utilization ?? 0) * 50.27} 50.27`}
                    className={cn(
                      "transition-all duration-300",
                      !contextUsage
                        ? "text-muted-foreground/30"
                        : contextUsage.utilization > 0.9
                          ? "text-red-400"
                          : contextUsage.utilization > 0.7
                            ? "text-[#e0af68]"
                            : "text-accent"
                    )}
                    stroke="currentColor"
                  />
                </svg>
                <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 rounded bg-[#1a1b26] border border-[#27293d] text-[10px] text-[#c0caf5] whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-50">
                  {contextUsage
                    ? `${(contextUsage.utilization * 100).toFixed(1)}% · ${(contextUsage.totalTokens / 1000).toFixed(1)}K / ${(contextUsage.maxTokens / 1000).toFixed(0)}K context used`
                    : "Context usage unavailable"}
                </div>
              </div>

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
                  onClick={handleSend}
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
