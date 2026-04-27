import { enableMapSet } from "immer";
import { create } from "zustand";
import { devtools } from "zustand/middleware";
import { immer } from "zustand/middleware/immer";
import { type PaneId, type PaneNode, type SplitDirection, type TabLayout } from "@/lib/pane-utils";
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
    { name: "golish" },
  ),
);

// Side-effect subscription: sync ZAP sidecar when active project changes.
// Decoupled from the pure-state setCurrentProject action.
let _prevProjectPath: string | null = null;
useStore.subscribe((state) => {
  const curPath = state.currentProjectPath;
  if (curPath === _prevProjectPath) return;
  const prev = _prevProjectPath;
  _prevProjectPath = curPath;

  if (prev && prev !== curPath) {
    useStore.getState().setZapRunning(false);
    import("@/lib/pentest/zap-api").then(({ zapStop, zapUpdateProject }) => {
      zapStop(prev)
        .catch(() => {})
        .then(() => zapUpdateProject(curPath).catch(() => {}));
    });
  } else {
    import("@/lib/pentest/zap-api").then(({ zapUpdateProject }) => {
      zapUpdateProject(curPath).catch(() => {});
    });
  }
});

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

export { clearConversation, restoreSession } from "./actions";

import { installDevTools } from "./dev-mock";
installDevTools();
