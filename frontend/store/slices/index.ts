/**
 * Store slices barrel export.
 *
 * This module exports all slice creators and their associated types
 * for composition in the main store.
 */

// Context slice
export {
  type ContextActions,
  type ContextMetrics,
  type ContextSlice,
  type ContextState,
  createContextSlice,
  initialContextState,
  selectCompactionCount,
  selectCompactionError,
  selectContextMetrics,
  selectIsCompacting,
  selectIsSessionDead,
  selectSessionTokenUsage,
} from "./context";

// Git slice
export {
  createGitSlice,
  type GitActions,
  type GitSlice,
  type GitState,
  initialGitState,
  selectGitCommitMessage,
  selectGitStatus,
  selectGitStatusLoading,
} from "./git";

// Notification slice
export {
  createNotificationSlice,
  initialNotificationState,
  type Notification,
  type NotificationActions,
  type NotificationSlice,
  type NotificationState,
  type NotificationType,
  selectNotifications,
  selectNotificationsExpanded,
  selectUnreadNotificationCount,
} from "./notification";

// Appearance slice
export {
  createAppearanceSlice,
  defaultDisplaySettings,
  initialAppearanceState,
  type AppearanceActions,
  type AppearanceSlice,
  type AppearanceState,
  type DisplaySettings,
  selectDisplaySettings,
} from "./appearance";

// Panel slice
export {
  createPanelSlice,
  initialPanelState,
  type PanelActions,
  type PanelSlice,
  type PanelState,
  selectContextPanelOpen,
  selectFileEditorPanelOpen,
  selectGitPanelOpen,
  selectSessionBrowserOpen,
  selectSidecarPanelOpen,
} from "./panel";

// Conversation slice
export {
  createConversationSlice,
  createNewConversation,
  initialConversationState,
  type ChatConversation,
  type ChatMessage,
  type ChatToolCall,
  type ConversationActions,
  type ConversationSlice,
  type ConversationState,
  selectActiveConversation,
  selectActiveConversationTerminals,
  selectAllConversations,
  selectConversationTerminals,
} from "./conversation";

// Session slice
export {
  _drainOutputBuffer,
  _drainOutputBufferSize,
  createSessionSlice,
  initialSessionState,
  selectActiveSessionId,
  selectSession,
  selectTabOrder,
  type SessionActions,
  type SessionSlice,
  type SessionState,
} from "./session";

// AI slice
export {
  createAiSlice,
  initialAiState,
  selectActiveToolCalls,
  selectAiConfig,
  selectIsAgentResponding,
  selectIsAgentThinking,
  type AiActions,
  type AiSlice,
  type AiState,
} from "./ai";

// Workflow slice
export {
  createWorkflowSlice,
  initialWorkflowState,
  selectActiveSubAgents,
  selectActiveWorkflow,
  type WorkflowActions,
  type WorkflowSlice,
  type WorkflowState,
} from "./workflow";

// Pane slice
export {
  createPaneSlice,
  initialPaneState,
  selectPaneMoveState,
  selectTabLayout,
  type PaneActions,
  type PaneSlice,
  type PaneState,
} from "./pane";

// HITL slice
export {
  createHitlSlice,
  initialHitlState,
  selectApprovalMode,
  selectPendingAskHuman,
  selectPendingToolApproval,
  type HitlActions,
  type HitlSlice,
  type HitlState,
} from "./hitl";

// Types
export type { ImmerSet, SliceCreator, StateGet } from "./types";
