import {
  ArrowUp,
  ChevronDown,
  Clock,
  Cpu,
  Image,
  MessageSquare,
  Plus,
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
  setExecutionMode as setExecutionModeBackend,
  setUseAgents as setUseAgentsBackend,
  shutdownAiSession,
} from "@/lib/ai";
import { logger } from "@/lib/logger";
import { formatModelName, PROVIDER_GROUPS } from "@/lib/models";
import { scanTools } from "@/lib/pentest/api";
import type { ToolConfig } from "@/lib/pentest/types";
import { getSettings } from "@/lib/settings";
import { TerminalInstanceManager } from "@/lib/terminal/TerminalInstanceManager";
import { cn } from "@/lib/utils";
import { convDelete } from "@/lib/conversation-db";
import { restoreBatchTerminals } from "@/lib/terminal-restore";
import { getAllLeafPanes } from "@/lib/pane-utils";
import { type ChatMessage, useStore } from "@/store";
import { createNewConversation } from "@/store/slices/conversation";
import {
  AskHumanInline,
  type AskHumanState,
  CompactionNotice,
  type TaskPlanState,
  WorkflowProgress,
  type WorkflowState,
} from "./ChatSubComponents";
import { AgentSummaryBar } from "./AgentSummaryBar";
import { MessageBlock } from "./MessageBlock";


const EMPTY_MESSAGES: ChatMessage[] = [];



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
  const pendingApprovalRef = useRef(pendingApproval);
  pendingApprovalRef.current = pendingApproval;

  type ApprovalMode = "ask" | "allowlist" | "run-all";
  const storeApprovalMode = useStore((s) => s.approvalMode);
  const [approvalMode, setApprovalMode] = useState<ApprovalMode>(
    (storeApprovalMode as ApprovalMode) || "ask"
  );
  useEffect(() => {
    if (storeApprovalMode) setApprovalMode(storeApprovalMode as ApprovalMode);
  }, [storeApprovalMode]);

  const [chatAgentMode, setChatAgentMode] = useState<AgentMode>("default");
  const [chatExecutionMode, setChatExecutionMode] = useState<"chat" | "task">("chat");
  const [chatUseSubAgents, setChatUseSubAgents] = useState(false);

  const [contextUsage, setContextUsage] = useState<{
    utilization: number;
    totalTokens: number;
    maxTokens: number;
  } | null>(null);

  // AskHuman state
  const [askHumanRequest, setAskHumanRequest] = useState<AskHumanState | null>(null);

  // Workflow state
  const [activeWorkflow, setActiveWorkflow] = useState<WorkflowState | null>(null);

  // Task plan: read from store session so it survives tab switches
  const storePlan = useStore((s) => {
    if (!s.activeConversationId) return null;
    const termIds = s.conversationTerminals[s.activeConversationId];
    const termId = termIds?.[0];
    const p = termId ? (s.sessions[termId]?.plan ?? null) : null;
    return p;
  });
  const taskPlan = useMemo<TaskPlanState | null>(
    () => storePlan ? { version: storePlan.version, steps: storePlan.steps, summary: storePlan.summary } : null,
    [storePlan]
  );
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
  const workspaceDataReady = useStore((s) => s.workspaceDataReady);
  const pendingTermData = useStore((s) => s.pendingTerminalRestoreData);

  // Unified terminal restore: fires on both initial boot (App.tsx sets data)
  // and project switch (HomeView sets data).  Clearing the store value
  // synchronously prevents double-processing under React Strict Mode.
  useEffect(() => {
    if (!workspaceDataReady || !pendingTermData) return;
    const data = pendingTermData;
    useStore.getState().setPendingTerminalRestoreData(null);
    void restoreBatchTerminals(data, createTerminalTab);
  }, [pendingTermData, workspaceDataReady, createTerminalTab]);

  // DB auto-saver (createDbAutoSaver in App.tsx) handles all persistence to PostgreSQL.
  // No localStorage mirroring needed.

  const storeAiModel = useStore((s) => s.selectedAiModel);
  const [selectedModel, setSelectedModel] = useState<{ model: string; provider: string } | null>(
    storeAiModel
  );
  useEffect(() => {
    if (storeAiModel) setSelectedModel(storeAiModel);
  }, [storeAiModel]);
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
              if (planTextOffsetRef.current === null) {
                const currentConv = useStore.getState().conversations[convId];
                const lastMsg = currentConv?.messages?.[currentConv.messages.length - 1];
                if (lastMsg?.role === "assistant") {
                  planTextOffsetRef.current = (lastMsg.content || "").length;
                }
              }
              const termIds = useStore.getState().conversationTerminals[convId];
              const termId = termIds?.[0];
              if (termId) {
                useStore.getState().setPlan(termId, {
                  version: event.version,
                  steps: event.steps,
                  summary: event.summary,
                  explanation: event.explanation ?? null,
                  updated_at: new Date().toISOString(),
                });
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
    planTextOffsetRef.current = null;
    if (tabsRef.current) {
      const activeTab = tabsRef.current.querySelector(`[data-conv-id="${activeConvId}"]`);
      activeTab?.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
    }
  }, [activeConvId]);

  // When switching conversations, activate its terminal
  useEffect(() => {
    if (!activeConvId) return;
    const store = useStore.getState();
    const terminals = store.conversationTerminals[activeConvId];
    if (terminals && terminals.length > 0) {
      const firstTerminal = terminals[0];
      if (store.sessions[firstTerminal] && store.activeSessionId !== firstTerminal) {
        store.setActiveSession(firstTerminal);
      }
    }
  }, [activeConvId]);

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

      // Collect ALL session IDs to clean (including split-pane sessions).
      const terminalIds = storeBefore.conversationTerminals[convId] ?? [];
      const allSessionIds: string[] = [];
      for (const termId of terminalIds) {
        const layout = storeBefore.tabLayouts[termId];
        if (layout) {
          for (const pane of getAllLeafPanes(layout.root)) {
            allSessionIds.push(pane.sessionId);
          }
        } else {
          allSessionIds.push(termId);
        }
      }

      // Side effects outside Immer: dispose xterm instances + AI event tracking.
      for (const sid of allSessionIds) {
        TerminalInstanceManager.dispose(sid);
      }
      import("@/hooks/useAiEvents").then(({ resetSessionSequence }) => {
        for (const sid of allSessionIds) resetSessionSequence(sid);
      });

      // Determine the next conversation + terminal to focus BEFORE mutations.
      const remainingOrder = storeBefore.conversationOrder.filter((id) => id !== convId);
      let nextActiveSessionId: string | null = null;
      if (remainingOrder.length > 0) {
        const nextConvId = remainingOrder[remainingOrder.length - 1];
        const nextTerms = storeBefore.conversationTerminals[nextConvId];
        if (nextTerms && nextTerms.length > 0) {
          nextActiveSessionId = nextTerms[0];
        }
      }

      // Batch: remove conversation + terminals + set next active in one update.
      useStore.setState((state) => {
        // Remove terminal tab layouts and all pane session state
        for (const termId of terminalIds) {
          delete state.tabLayouts[termId];
          delete state.tabHasNewActivity[termId];
          const tIdx = state.tabOrder.indexOf(termId);
          if (tIdx !== -1) state.tabOrder.splice(tIdx, 1);
          state.tabActivationHistory = state.tabActivationHistory.filter((id) => id !== termId);
        }
        for (const sid of allSessionIds) {
          delete state.sessions[sid];
          delete state.timelines[sid];
          delete state.pendingCommand[sid];
          delete state.lastSentCommand[sid];
          delete state.agentStreamingBuffer[sid];
          delete state.agentStreaming[sid];
          delete state.streamingBlocks[sid];
          delete state.streamingTextOffset[sid];
          delete state.agentInitialized[sid];
          delete state.isAgentThinking[sid];
          delete state.isAgentResponding[sid];
          delete state.pendingToolApproval[sid];
          delete state.pendingAskHuman[sid];
          delete state.processedToolRequests[sid];
          delete state.activeToolCalls[sid];
          delete state.thinkingContent[sid];
          delete state.isThinkingExpanded[sid];
          delete state.contextMetrics[sid];
          delete state.tabHasNewActivity[sid];
          state.tabActivationHistory = state.tabActivationHistory.filter((id) => id !== sid);
        }

        // Remove conversation
        delete state.conversations[convId];
        delete state.conversationTerminals[convId];
        const orderIdx = state.conversationOrder.indexOf(convId);
        if (orderIdx !== -1) state.conversationOrder.splice(orderIdx, 1);

        if (state.activeConversationId === convId) {
          state.activeConversationId =
            state.conversationOrder.length > 0
              ? state.conversationOrder[state.conversationOrder.length - 1]
              : null;
        }

        // Set next active session atomically — no intermediate home-tab render.
        if (nextActiveSessionId) {
          state.activeSessionId = nextActiveSessionId;
        }
      });

      convDelete(convId).catch((e) => {
        logger.warn("[AIChatPanel] Failed to delete conversation from DB:", e);
      });

      // If no conversations remain, collapse the chat panel instead of creating
      // a new empty conversation. The user can reopen via the floating button.
      if (useStore.getState().conversationOrder.length === 0) {
        useStore.getState().setChatPanelVisible(false);
      }
    },
    []
  );

  const handleModelSelect = useCallback((modelId: string, provider: string) => {
    const sel = { model: modelId, provider };
    setSelectedModel(sel);
    useStore.getState().setSelectedAiModel(sel);
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
- pentest_read_skill: Read a skill document (Markdown) for detailed tool usage instructions
- run_pipeline: Execute a tool pipeline (chain of tools run sequentially with auto-parsing and DB storage)
  - action "list": List all available pipelines (including built-in recon_basic)
  - action "run": Execute a pipeline by ID against one or more targets. Each step's output is automatically parsed and stored to the database. Steps are skipped when they don't apply (e.g. subfinder skips for IP targets).
    - pipeline_id (required): The pipeline to run (e.g. "recon_basic")
    - target: Single target (domain/IP/URL)
    - targets: Array of targets for batch scanning (e.g. ["a.com", "b.com"])${toolContext}

### JavaScript Collection & Analysis
- js_collect: Initial JS collection — fetches page HTML, extracts <script> tags, and downloads JS files with recursive discovery of referenced chunks.
  - "target_url" (required): The URL to collect JS from (e.g. "http://8.138.179.62:8080")
  - "js_urls" (optional): Explicit list of JS URLs. If omitted, auto-discovers from page HTML.
  - "target_id" (optional): Target UUID. Auto-resolved from DB if omitted.
  - Files are saved to .golish/captures/<host>/<port>/js/ and added to SiteMap.
- save_js_analysis: Persist AI-analyzed JS data (routes, endpoints, secrets) to the database.

#### JS Collection Workflow (Follow This Order)
When the user asks to collect JS from a target:
1. Call js_collect(target_url: "...") for initial automated collection
2. Read the saved JS files (use read_file on the paths returned by js_collect) to understand the bundling pattern (Webpack, Vite, custom, etc.)
3. If many chunks are expected but js_collect only found a few, write a custom download script:
   - Analyze the JS content to find chunk loading patterns (e.g., webpack's __webpack_require__.e(), Vite's import(), path concatenation patterns)
   - Write a Node.js or Python script to discover and download ALL remaining chunks
   - Save the script to .golish/scripts/recon/ using write_file
   - Execute it via run_pty_cmd
   - The script should save files to .golish/captures/<host>/<port>/js/
4. Report the total count of collected JS files
NEVER use curl/wget directly for JS collection. Use js_collect first, then scripts if needed.

### Data Management (PostgreSQL Database)
- manage_targets: Manage penetration testing targets in the database
  - action "add": Register a new host, subdomain, IP, or URL as a target
  - action "list": List all current targets and their status
  - action "update_status": Change a target's phase (new → recon → recon_done → scanning → tested)
  - action "update_recon": Attach port/service/technology data to a target
- record_finding: Record a vulnerability or security finding in the database
- credential_vault: Manage discovered credentials
  - action "store": Save a new credential (host, username, password, source)
  - action "get": Retrieve credentials for a specific host
  - action "get_all": List all stored credentials
  - action "delete": Remove a credential entry

### Memory System
- search_memories: Search past observations, techniques, and findings stored across sessions
- store_memory: Save important observations or findings for future reference
- list_memories: List all stored memories

### File Operations
- read_file: Read file contents
- write_file / create_file: Write or create files
- edit_file: Make targeted edits to existing files
- delete_file: Delete a file
- list_files / list_directory: Browse directory contents
- grep_file: Search for patterns in files

## Critical Rules

### Target Verification (MANDATORY)
Before performing ANY operation on a target (scanning, recon, JS collection, exploitation, fuzzing, or any other action):
1. ALWAYS call manage_targets(action: "list") FIRST to check if the target exists in the database
2. If the target ALREADY EXISTS → proceed directly with the operation. Do NOT call manage_targets(action: "add") again.
3. If the target is NOT in the database → use ask_human to ask: "Target [X] is not registered. Do you want to add it before proceeding?" → on confirm, call manage_targets(action: "add", targets: [{"value": "the-target-url-or-host"}])
4. Only THEN proceed with the operation
Never skip this verification. Never call manage_targets(action: "add") without first checking via "list".
CRITICAL: When calling manage_targets(action: "add"), you MUST include the "targets" array with the actual target value. Example: manage_targets(action: "add", targets: [{"value": "http://example.com"}]). Calling add without a targets array WILL fail.

### Tool Selection Rules
- When asked about targets/目标 → use manage_targets (NOT file browsing)
- When asked about vulnerabilities/漏洞/findings → use record_finding
- When asked about credentials/密码/凭证 → use credential_vault
- When asked about past findings/history → use search_memories first
- When asked to scan/recon/enumerate a target → use run_pipeline first (after target verification). Only fall back to pentest_run for individual tool runs when a pipeline is not suitable.
- When asked to collect/download JS files → use js_collect (NOT the js-reverse MCP tools). js_collect downloads all JS files from a target URL to local disk and stores them in the SiteMap.
- Only use js-reverse MCP tools (mcp__js_reverse__*) for dynamic JS debugging and reverse engineering tasks (e.g., intercepting network requests, hooking JS functions, decrypting/signing analysis). Do NOT use them for simple JS file collection.
- NEVER browse the filesystem (.golish/ directory, session files, etc.) to look up target/finding/credential data. Always use the database tools above.

### Scanning Workflow (Follow This Order)
When the user asks to scan, recon, or enumerate a target:
1. **Check targets**: Call manage_targets(action: "list") to see if target is registered
2. **Add if missing**: If not found, use ask_human to ask "Target [X] is not registered. Add it?" → on confirm, call manage_targets(action: "add", targets: [{"value": "the-target"}])
3. **Run pipeline**: Call run_pipeline(action: "run", pipeline_id: "recon_basic", target: "the-target") to execute the full recon chain
4. **Report results**: Summarize what was discovered (subdomains, ports, technologies, directories, etc.)
Do NOT run individual tools one-by-one when a pipeline can handle it. The pipeline automatically parses output and stores data to the database.

## Guidelines
- When the user asks to run a command, use run_pty_cmd with the appropriate working directory from your managed terminals.
- For reconnaissance/scanning tasks, prefer run_pipeline over running individual tools — it handles the full chain with automatic parsing and database storage.
- When running a single pentest tool, prefer pentest_run as it handles runtime resolution (Python envs, Java paths, etc.) automatically.
- Use pentest_read_skill to learn detailed usage patterns before running unfamiliar tools.
- For long-running commands, set an appropriate timeout.
- Always report command output and exit codes to the user.
- After completing a scan or finding something notable, use store_memory to save it for future sessions.
- Perform ONLY what the user explicitly requests. If the user asks to "collect JS", just collect the JS files and report back — do NOT automatically chain additional operations like analysis, fingerprinting, or saving. Only extend the scope if the user says so.
- Only ask the user when target verification requires confirmation or when critical decisions are needed.`;
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
        const savedMode = useStore.getState().approvalMode || "ask";
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
      // Safety timeout: if streaming is idle (no text AND no tool running) for 60s, reset.
      const convId = conv.id;
      let lastMsgLength = 0;
      let lastToolCount = 0;
      let idleChecks = 0;
      const checkInterval = setInterval(() => {
        const s = useStore.getState();
        const c = s.conversations[convId];
        if (!c?.isStreaming) {
          clearInterval(checkInterval);
          return;
        }
        const lastMsg = c.messages[c.messages.length - 1];
        const currentLength = lastMsg?.content?.length ?? 0;
        const currentToolCount = lastMsg?.toolCalls?.length ?? 0;
        const hasPendingTools = lastMsg?.toolCalls?.some(
          (tc) => tc.success === undefined
        ) ?? false;

        if (hasPendingTools) {
          idleChecks = 0;
        } else if (currentLength === lastMsgLength && currentToolCount === lastToolCount) {
          idleChecks++;
          if (idleChecks >= 12) {
            console.warn("[AIChatPanel] Idle timeout: resetting stuck streaming for", convId);
            s.finalizeStreamingMessage(convId);
            clearInterval(checkInterval);
          }
        } else {
          lastMsgLength = currentLength;
          lastToolCount = currentToolCount;
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
      useStore.getState().setApprovalMode(mode);
      const conv = useStore.getState().activeConversationId
        ? useStore.getState().conversations[useStore.getState().activeConversationId!]
        : null;
      if (!conv) return;
      const backendMode: AgentMode = mode === "run-all" ? "auto-approve" : "default";
      setAgentMode(conv.aiSessionId, backendMode).catch(console.error);
    },
    []
  );

  const handleAgentModeChange = useCallback(
    (mode: AgentMode) => {
      if (mode === chatAgentMode) return;
      setChatAgentMode(mode);
      const conv = useStore.getState().activeConversationId
        ? useStore.getState().conversations[useStore.getState().activeConversationId!]
        : null;
      if (!conv) return;
      setAgentMode(conv.aiSessionId, mode).catch(console.error);
      if (mode === "auto-approve") {
        setApprovalMode("run-all");
        useStore.getState().setApprovalMode("run-all");
      } else {
        setApprovalMode("ask");
        useStore.getState().setApprovalMode("ask");
      }
    },
    [chatAgentMode]
  );

  const handleExecutionModeChange = useCallback(
    (mode: "chat" | "task") => {
      if (mode === chatExecutionMode) return;
      setChatExecutionMode(mode);
      const conv = useStore.getState().activeConversationId
        ? useStore.getState().conversations[useStore.getState().activeConversationId!]
        : null;
      if (!conv) return;
      setExecutionModeBackend(conv.aiSessionId, mode).catch(console.error);
    },
    [chatExecutionMode]
  );

  const handleToggleSubAgents = useCallback(() => {
    const newValue = !chatUseSubAgents;
    setChatUseSubAgents(newValue);
    const conv = useStore.getState().activeConversationId
      ? useStore.getState().conversations[useStore.getState().activeConversationId!]
      : null;
    if (!conv) return;
    setUseAgentsBackend(conv.aiSessionId, newValue).catch(console.error);
  }, [chatUseSubAgents]);

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

  const lastAssistantIdx = useMemo(() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      if (messages[i].role === "assistant") return i;
    }
    return -1;
  }, [messages]);

  const planTargetIdx = useMemo(() => {
    for (let i = 0; i < messages.length; i++) {
      const msg = messages[i];
      if (msg.role === "assistant" && msg.toolCalls?.some((tc) => tc.name === "update_plan")) {
        return i;
      }
    }
    return lastAssistantIdx;
  }, [messages, lastAssistantIdx]);

  const stablePendingApproval = useMemo(
    () =>
      pendingApproval
        ? { requestId: pendingApproval.requestId, toolName: pendingApproval.toolName }
        : null,
    [pendingApproval?.requestId, pendingApproval?.toolName]
  );

  const handleToolApprove = useCallback(
    (requestId: string) => {
      const pa = pendingApprovalRef.current;
      if (!pa) return;
      respondToToolApproval(pa.sessionId, {
        request_id: requestId,
        approved: true,
        remember: false,
        always_allow: false,
      }).catch(console.error);
      setPendingApproval(null);
    },
    []
  );

  const handleToolDeny = useCallback(
    (requestId: string) => {
      const pa = pendingApprovalRef.current;
      if (!pa) return;
      respondToToolApproval(pa.sessionId, {
        request_id: requestId,
        approved: false,
        remember: false,
        always_allow: false,
      }).catch(console.error);
      setPendingApproval(null);
    },
    []
  );

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
                const isPlanTarget = msgIdx === planTargetIdx;
                return (
                  <MessageBlock
                    key={msg.id}
                    message={msg}
                    taskPlan={isPlanTarget ? taskPlan : null}
                    planTextOffset={isPlanTarget ? planTextOffsetRef.current : null}
                    pendingApproval={stablePendingApproval}
                    approvalMode={approvalMode}
                    onApprovalModeChange={handleApprovalModeChange}
                    onApprove={handleToolApprove}
                    onDeny={handleToolDeny}
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
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    type="button"
                    className={cn(
                      "flex items-center gap-1 px-2 py-1 rounded-md text-[11px] font-medium transition-colors",
                      chatExecutionMode === "task"
                        ? "bg-[var(--ansi-magenta)]/10 text-[var(--ansi-magenta)] hover:bg-[var(--ansi-magenta)]/20"
                        : "bg-muted text-foreground hover:bg-[var(--bg-hover)]"
                    )}
                  >
                    {chatExecutionMode === "task" ? (
                      <Zap className="w-3 h-3" />
                    ) : (
                      <MessageSquare className="w-3 h-3" />
                    )}
                    {chatExecutionMode === "task" ? "Task" : "Chat"}
                    <ChevronDown className="w-2.5 h-2.5 text-muted-foreground" />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent
                  align="start"
                  side="top"
                  className="bg-card border-[var(--border-medium)] min-w-[220px]"
                >
                  <DropdownMenuItem
                    onClick={() => {
                      handleExecutionModeChange("chat");
                      handleAgentModeChange("default");
                    }}
                    className={cn(
                      "text-xs cursor-pointer flex items-start gap-2 py-2.5",
                      chatExecutionMode === "chat"
                        ? "text-accent bg-[var(--accent-dim)]"
                        : "text-foreground hover:text-accent"
                    )}
                  >
                    <MessageSquare className="w-4 h-4 mt-0.5 shrink-0" />
                    <div className="flex flex-col">
                      <span className="font-medium">Chat</span>
                      <span className="text-[10px] text-muted-foreground">Conversational assistant with tools</span>
                    </div>
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    onClick={() => {
                      handleExecutionModeChange("task");
                      handleAgentModeChange("auto-approve");
                    }}
                    className={cn(
                      "text-xs cursor-pointer flex items-start gap-2 py-2.5",
                      chatExecutionMode === "task"
                        ? "text-[var(--ansi-magenta)] bg-[var(--ansi-magenta)]/10"
                        : "text-foreground hover:text-accent"
                    )}
                  >
                    <Zap className="w-4 h-4 mt-0.5 shrink-0" />
                    <div className="flex flex-col">
                      <span className="font-medium">Task</span>
                      <span className="text-[10px] text-muted-foreground">Auto: plan → execute → refine → report</span>
                    </div>
                  </DropdownMenuItem>
                  <DropdownMenuSeparator className="bg-[var(--border-medium)]" />
                  <DropdownMenuItem
                    onClick={handleToggleSubAgents}
                    className="text-xs cursor-pointer flex items-center gap-2 py-2"
                  >
                    <Users className={cn("w-4 h-4 shrink-0", chatUseSubAgents ? "text-[var(--ansi-green)]" : "text-muted-foreground")} />
                    <div className="flex flex-col flex-1">
                      <span className="font-medium">Sub-Agents</span>
                      <span className="text-[10px] text-muted-foreground">
                        {chatUseSubAgents ? "Enabled" : "Disabled"}
                      </span>
                    </div>
                    <div className={cn(
                      "w-7 h-4 rounded-full transition-colors duration-200 flex items-center shrink-0",
                      chatUseSubAgents ? "bg-[var(--ansi-green)]/30 justify-end" : "bg-muted justify-start"
                    )}>
                      <div className={cn(
                        "w-3 h-3 rounded-full mx-0.5 transition-colors duration-200",
                        chatUseSubAgents ? "bg-[var(--ansi-green)]" : "bg-muted-foreground/50"
                      )} />
                    </div>
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>

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
