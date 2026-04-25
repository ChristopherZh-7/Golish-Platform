import { enableMapSet } from "immer";
import { create } from "zustand";
import { devtools } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import { findPaneById, getAllLeafPanes, type PaneId, type PaneNode, type SplitDirection, type TabLayout } from "@/lib/pane-utils";
import {
  type AiSlice,
  type AppearanceSlice,
  type ChatConversation,
  type ChatMessage,
  type ChatToolCall,
  type ContextMetrics,
  type ContextSlice,
  type ConversationSlice,
  createAiSlice,
  createAppearanceSlice,
  createContextSlice,
  createConversationSlice,
  createGitSlice,
  createHitlSlice,
  createNotificationSlice,
  createPanelSlice,
  createPaneSlice,
  createSessionSlice,
  createWorkflowSlice,
  type GitSlice,
  type HitlSlice,
  type Notification,
  type NotificationSlice,
  type NotificationType,
  type PanelSlice,
  type PaneSlice,
  selectActiveConversation,
  selectActiveConversationTerminals,
  selectAllConversations,
  selectContextMetrics,
  type SessionSlice,
  type WorkflowSlice,
} from "./slices";
import type { ActiveToolCall, StreamingBlock, UnifiedBlock } from "./store-types";

// Re-export all domain types from store-types
export type {
  ActiveSubAgent,
  ActiveToolCall,
  ActiveWorkflow,
  AgentMessage,
  AiConfig,
  AiToolExecution,
  AskHumanRequest,
  CommandBlock,
  CompactionResult,
  DetailViewMode,
  FinalizedStreamingBlock,
  PendingCommand,
  PipelineExecution,
  PipelineStepExecution,
  PipelineStepStatus,
  PipelineSubTarget,
  Session,
  StreamingBlock,
  SubAgentEntry,
  SubAgentToolCall,
  TaskPlan,
  ToolCall,
  ToolCallSource,
  UnifiedBlock,
  WorkflowStep,
} from "./store-types";
export type {
  AgentMode,
  AiStatus,
  ApprovalPattern,
  ExecutionMode,
  InputMode,
  PlanStep,
  PlanSummary,
  ReasoningEffort,
  RenderMode,
  RiskLevel,
  SessionMode,
  StepStatus,
  TabType,
  WorkflowStatus,
} from "./store-types";
// Re-export pane types from the single source of truth
export type { PaneId, PaneNode, SplitDirection, TabLayout };
// Re-export conversation types
export type { ChatConversation, ChatMessage, ChatToolCall };
// Re-export slice types
export type { ContextMetrics, Notification, NotificationType };

// Re-export session-slice helpers used by other modules.
export { _drainOutputBuffer, _drainOutputBufferSize } from "./slices/session";

// Enable Immer support for Set and Map (needed for processedToolRequests)
enableMapSet();

/**
 * GolishState aggregates every slice plus the small handful of root-level
 * fields that don't belong to a single slice (app focus, workspace bootstrap,
 * project info, chat-panel toggle).
 */
interface GolishState
  extends AppearanceSlice,
    ContextSlice,
    ConversationSlice,
    GitSlice,
    NotificationSlice,
    PanelSlice,
    SessionSlice,
    AiSlice,
    WorkflowSlice,
    PaneSlice,
    HitlSlice {
  // App focus/visibility state
  appIsFocused: boolean;
  appIsVisible: boolean;

  // Terminal restore loading state
  terminalRestoreInProgress: boolean;
  setTerminalRestoreInProgress: (inProgress: boolean) => void;

  workspaceDataReady: boolean;
  setWorkspaceDataReady: (ready: boolean) => void;

  zapRunning: boolean;
  setZapRunning: (running: boolean) => void;

  pendingTerminalRestoreData: Record<string, import("@/lib/workspace-storage").PersistedTerminalData[]> | null;
  setPendingTerminalRestoreData: (
    data: Record<string, import("@/lib/workspace-storage").PersistedTerminalData[]> | null,
  ) => void;

  chatPanelVisible: boolean;
  setChatPanelVisible: (visible: boolean) => void;
  toggleChatPanel: () => void;

  // Project (kept at root for now — has external side-effects on ZAP sidecar).
  currentProjectName: string | null;
  currentProjectPath: string | null;
  setCurrentProject: (name: string | null, path?: string | null) => void;

  // App focus/visibility actions
  setAppIsFocused: (focused: boolean) => void;
  setAppIsVisible: (visible: boolean) => void;
}

export const useStore = create<GolishState>()(
  devtools(
    immer((set, get, _store) => ({
      // Slices
      ...createAppearanceSlice(set, get),
      ...createContextSlice(set, get),
      ...createConversationSlice(set, get),
      ...createGitSlice(set, get),
      ...createNotificationSlice(set, get),
      ...createPanelSlice(set, get),
      ...createSessionSlice(set, get),
      ...createAiSlice(set, get),
      ...createWorkflowSlice(set, get),
      ...createPaneSlice(set, get),
      ...createHitlSlice(set, get),

      // Root-level state
      appIsFocused: true,
      appIsVisible: true,

      terminalRestoreInProgress: false,
      setTerminalRestoreInProgress: (inProgress: boolean) =>
        set((state) => {
          state.terminalRestoreInProgress = inProgress;
        }),

      workspaceDataReady: false,
      setWorkspaceDataReady: (ready: boolean) =>
        set((state) => {
          state.workspaceDataReady = ready;
        }),

      zapRunning: false,
      setZapRunning: (running: boolean) =>
        set((state) => {
          state.zapRunning = running;
        }),

      pendingTerminalRestoreData: null,
      setPendingTerminalRestoreData: (data) =>
        set((state) => {
          state.pendingTerminalRestoreData = data as any;
        }),

      chatPanelVisible: true,
      setChatPanelVisible: (visible) =>
        set((state) => {
          state.chatPanelVisible = visible;
        }),
      toggleChatPanel: () =>
        set((state) => {
          state.chatPanelVisible = !state.chatPanelVisible;
        }),

      currentProjectName: null,
      currentProjectPath: null,
      setCurrentProject: (name, path) => {
        const prevPath = get().currentProjectPath;
        set((state) => {
          state.currentProjectName = name;
          state.currentProjectPath = path ?? null;
        });
        if (prevPath && prevPath !== (path ?? null)) {
          set((state) => {
            state.zapRunning = false;
          });
          import("@/lib/pentest/zap-api").then(({ zapStop, zapUpdateProject }) => {
            zapStop(prevPath)
              .catch(() => {})
              .then(() => {
                zapUpdateProject(path ?? null).catch(() => {});
              });
          });
        } else {
          import("@/lib/pentest/zap-api").then(({ zapUpdateProject }) => {
            zapUpdateProject(path ?? null).catch(() => {});
          });
        }
      },

      setAppIsFocused: (focused) =>
        set((state) => {
          state.appIsFocused = focused;
        }),

      setAppIsVisible: (visible) =>
        set((state) => {
          state.appIsVisible = visible;
        }),
    })),
    { name: "golish" },
  ),
);

// Stable empty arrays to avoid re-render loops (declared once to prevent recreation)
// Frozen to ensure immutability and prevent accidental mutations
const EMPTY_TIMELINE = Object.freeze([]) as unknown as UnifiedBlock[];
const EMPTY_TOOL_CALLS = Object.freeze([]) as unknown as ActiveToolCall[];
const EMPTY_STREAMING_BLOCKS = Object.freeze([]) as unknown as StreamingBlock[];

// Import derived selectors for Single Source of Truth pattern
import { memoizedSelectAgentMessages, memoizedSelectCommandBlocks } from "@/lib/timeline/selectors";

// Selectors
export const useActiveSession = () =>
  useStore((state) => {
    const id = state.activeSessionId;
    return id ? state.sessions[id] : null;
  });

/**
 * Get command blocks for a session.
 * Derives from the unified timeline (Single Source of Truth).
 */
export const useSessionBlocks = (sessionId: string) =>
  useStore((state) => memoizedSelectCommandBlocks(sessionId, state.timelines[sessionId]));

export const useTerminalClearRequest = (sessionId: string) =>
  useStore((state) => state.terminalClearRequest[sessionId] ?? 0);

export const usePendingCommand = (sessionId: string) =>
  useStore((state) => state.pendingCommand[sessionId]);

export const useSessionMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.mode ?? "terminal");

/**
 * Get agent messages for a session.
 * Derives from the unified timeline (Single Source of Truth).
 */
export const useAgentMessages = (sessionId: string) =>
  useStore((state) => memoizedSelectAgentMessages(sessionId, state.timelines[sessionId]));

export const useAgentStreaming = (sessionId: string) =>
  useStore((state) => state.agentStreaming[sessionId] ?? "");

export const useAgentInitialized = (sessionId: string) =>
  useStore((state) => state.agentInitialized[sessionId] ?? false);

export const usePendingToolApproval = (sessionId: string) =>
  useStore((state) => state.pendingToolApproval[sessionId] ?? null);

export const usePendingAskHuman = (sessionId: string) =>
  useStore((state) => state.pendingAskHuman[sessionId] ?? null);

// Timeline selectors
export const useSessionTimeline = (sessionId: string) =>
  useStore((state) => state.timelines[sessionId] ?? EMPTY_TIMELINE);

export const useInputMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.inputMode ?? "terminal");

export const useAgentMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.agentMode ?? "default");

export const useUseAgents = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.useAgents ?? true);

export const useExecutionMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.executionMode ?? "chat");

export const useRenderMode = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.renderMode ?? "timeline");

export const useGitBranch = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.gitBranch ?? null);

export const useGitStatus = (sessionId: string) =>
  useStore((state) => state.gitStatus[sessionId] ?? null);
export const useGitStatusLoading = (sessionId: string) =>
  useStore((state) => state.gitStatusLoading[sessionId] ?? false);
export const useGitCommitMessage = (sessionId: string) =>
  useStore((state) => state.gitCommitMessage[sessionId] ?? "");

// Active tool calls selector
export const useActiveToolCalls = (sessionId: string) =>
  useStore((state) => state.activeToolCalls[sessionId] ?? EMPTY_TOOL_CALLS);

// Streaming blocks selector
export const useStreamingBlocks = (sessionId: string) =>
  useStore((state) => state.streamingBlocks[sessionId] ?? EMPTY_STREAMING_BLOCKS);

// Streaming text length selector (for auto-scroll triggers)
export const useStreamingTextLength = (sessionId: string) =>
  useStore((state) => state.agentStreaming[sessionId]?.length ?? 0);

// AI config selector (global - for backwards compatibility)
export const useAiConfig = () => useStore((state) => state.aiConfig);

// Per-session AI config selector
export const useSessionAiConfig = (sessionId: string) =>
  useStore((state) => state.sessions[sessionId]?.aiConfig);

// Agent thinking selector
export const useIsAgentThinking = (sessionId: string) =>
  useStore((state) => state.isAgentThinking[sessionId] ?? false);

// Agent responding selector (true when agent is actively responding, from started to completed)
export const useIsAgentResponding = (sessionId: string) =>
  useStore((state) => state.isAgentResponding[sessionId] ?? false);

// Extended thinking content selectors
export const useThinkingContent = (sessionId: string) =>
  useStore((state) => state.thinkingContent[sessionId] ?? "");

export const useIsThinkingExpanded = (sessionId: string) =>
  useStore((state) => state.isThinkingExpanded[sessionId] ?? true);

// Context metrics selector (uses slice selector)
export const useContextMetrics = (sessionId: string) =>
  useStore((state) => selectContextMetrics(state, sessionId));

// Conversation selectors
export const useActiveConversation = () => useStore((state) => selectActiveConversation(state));

export const useAllConversations = () => useStore((state) => selectAllConversations(state));

export const useActiveConversationTerminals = () =>
  useStore((state) => selectActiveConversationTerminals(state));

/**
 * Get the session ID of the currently focused pane.
 * Falls back to tabId if no layout exists (backward compatibility).
 *
 * Uses a module-level cache keyed on (tabId, focusedPaneId) to avoid
 * running findPaneById() on every store mutation. This prevents App
 * from re-rendering unless the focused pane actually changes.
 */
let _focusedSessionCache: {
  tabId: string | null;
  focusedPaneId: string | null;
  result: string | null;
} | null = null;

export const useFocusedSessionId = (tabId: string | null) =>
  useStore((state) => {
    if (!tabId) return null;
    const layout = state.tabLayouts[tabId];
    if (!layout) return tabId;

    if (
      _focusedSessionCache &&
      _focusedSessionCache.tabId === tabId &&
      _focusedSessionCache.focusedPaneId === layout.focusedPaneId
    ) {
      return _focusedSessionCache.result;
    }

    const pane = findPaneById(layout.root, layout.focusedPaneId);
    const result = pane?.type === "leaf" ? pane.sessionId : tabId;
    _focusedSessionCache = { tabId, focusedPaneId: layout.focusedPaneId, result };
    return result;
  });

/**
 * Get the owning tab ID for a given session ID.
 * A session may be the tab's root session, or a pane session within the tab.
 * Returns the tab ID (root session ID) that contains this session.
 * Returns null if the session is not found in any tab.
 */
export function getOwningTabId(sessionId: string): string | null {
  const state = useStore.getState();

  if (state.tabLayouts[sessionId]) {
    return sessionId;
  }

  for (const [tabId, layout] of Object.entries(state.tabLayouts)) {
    const panes = getAllLeafPanes(layout.root);
    if (panes.some((pane) => pane.sessionId === sessionId)) {
      return tabId;
    }
  }

  return null;
}

export { clearConversation, restoreSession } from "./actions";

import { installDevTools } from "./dev-mock";
installDevTools();
