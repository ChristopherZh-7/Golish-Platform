import { enableMapSet } from "immer";
import { create } from "zustand";
import { devtools } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import type { PaneId, PaneNode, SplitDirection, TabLayout } from "@/lib/pane-utils";
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
  createDialogSlice,
  createGitSlice,
  createHitlSlice,
  createNotificationSlice,
  createPanelSlice,
  createPaneSlice,
  createSessionSlice,
  createWorkflowSlice,
  type DialogSlice,
  type GitSlice,
  type HitlSlice,
  type Notification,
  type NotificationSlice,
  type NotificationType,
  type PanelSlice,
  type PaneSlice,
  type SessionSlice,
  type WorkflowSlice,
} from "./slices";

// Re-export session-slice helpers used by other modules.
export { _drainOutputBuffer, _drainOutputBufferSize } from "./slices/session";
// Re-export all domain types from store-types
export type {
  ActiveSubAgent,
  ActiveToolCall,
  ActiveWorkflow,
  AgentMessage,
  AgentMode,
  AiConfig,
  AiStatus,
  AiToolExecution,
  ApprovalPattern,
  AskHumanRequest,
  CommandBlock,
  CompactionResult,
  DetailViewMode,
  ExecutionMode,
  FinalizedStreamingBlock,
  InputMode,
  PendingCommand,
  PipelineExecution,
  PipelineStepExecution,
  PipelineStepStatus,
  PipelineSubTarget,
  PlanStep,
  PlanSummary,
  ReasoningEffort,
  RenderMode,
  RiskLevel,
  Session,
  SessionMode,
  StepStatus,
  StreamingBlock,
  SubAgentEntry,
  SubAgentToolCall,
  TabType,
  TaskPlan,
  ToolCall,
  ToolCallSource,
  UnifiedBlock,
  WorkflowStatus,
  WorkflowStep,
} from "./store-types";
// Re-export pane types from the single source of truth
// Re-export conversation types
// Re-export slice types
export type {
  ChatConversation,
  ChatMessage,
  ChatToolCall,
  ContextMetrics,
  Notification,
  NotificationType,
  PaneId,
  PaneNode,
  SplitDirection,
  TabLayout,
};

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
    DialogSlice,
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

  pendingTerminalRestoreData: Record<
    string,
    import("@/lib/workspace-storage").PersistedTerminalData[]
  > | null;
  setPendingTerminalRestoreData: (
    data: Record<string, import("@/lib/workspace-storage").PersistedTerminalData[]> | null
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
      ...createDialogSlice(set, get),
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
        set((state) => {
          state.currentProjectName = name;
          state.currentProjectPath = path ?? null;
        });
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
    { name: "golish" }
  )
);

export {
  type CloseTabAndCleanupOptions,
  clearConversation,
  closeTabAndCleanup,
  type OpenProjectOptions,
  openProject,
  restoreSession,
} from "./actions";
// Re-export selector hooks from their dedicated module.
// Keeps store/index.ts focused on slice composition.
export {
  getOwningTabId,
  useActiveConversation,
  useActiveConversationTerminals,
  useActiveSession,
  useActiveToolCalls,
  useAgentInitialized,
  useAgentMessages,
  useAgentMode,
  useAgentStreaming,
  useAiConfig,
  useAllConversations,
  useContextMetrics,
  useExecutionMode,
  useFocusedSessionId,
  useGitBranch,
  useGitCommitMessage,
  useGitStatus,
  useGitStatusLoading,
  useInputMode,
  useIsAgentResponding,
  useIsAgentThinking,
  useIsThinkingExpanded,
  usePendingAskHuman,
  usePendingCommand,
  usePendingToolApproval,
  useRenderMode,
  useSessionAiConfig,
  useSessionBlocks,
  useSessionMode,
  useSessionTimeline,
  useStreamingBlocks,
  useStreamingTextLength,
  useTerminalClearRequest,
  useThinkingContent,
  useUseAgents,
} from "./selectors/store-hooks";

import { installDevTools } from "./dev-mock";

installDevTools();
