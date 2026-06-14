/**
 * Store slices barrel export.
 *
 * This module exports all slice creators and their associated types
 * for composition in the main store.
 */

// AI slice
export {
  type AiActions,
  type AiSlice,
  type AiState,
  createAiSlice,
  initialAiState,
  selectActiveToolCalls,
  selectAiConfig,
  selectIsAgentResponding,
  selectIsAgentThinking,
} from "./ai";
// App-shell slice (root-level app state: focus, project, chat-panel…)
export {
  type AppShellActions,
  type AppShellSlice,
  type AppShellState,
  createAppShellSlice,
  initialAppShellState,
} from "./app-shell";
// Appearance slice
export {
  type AppearanceActions,
  type AppearanceSlice,
  type AppearanceState,
  createAppearanceSlice,
  type DisplaySettings,
  defaultDisplaySettings,
  initialAppearanceState,
  selectDisplaySettings,
} from "./appearance";
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
// Conversation slice
export {
  type ChatConversation,
  type ChatMessage,
  type ChatToolCall,
  type ConversationActions,
  type ConversationSlice,
  type ConversationState,
  createConversationSlice,
  createNewConversation,
  initialConversationState,
  selectActiveConversation,
  selectActiveConversationTerminals,
  selectAllConversations,
  selectConversationTerminals,
} from "./conversation";
// Dialog slice
export {
  createDialogSlice,
  type DialogActions,
  type DialogSlice,
  type DialogState,
  initialDialogState,
} from "./dialog";
// HITL slice
export {
  createHitlSlice,
  type HitlActions,
  type HitlSlice,
  type HitlState,
  initialHitlState,
  selectApprovalMode,
  selectPendingAskHuman,
  selectPendingToolApproval,
} from "./hitl";
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

// Pane slice
export {
  createPaneSlice,
  initialPaneState,
  type PaneActions,
  type PaneSlice,
  type PaneState,
  selectPaneMoveState,
  selectTabLayout,
} from "./pane";
// Panel slice
export {
  createPanelSlice,
  initialPanelState,
  type PanelActions,
  type PanelSlice,
  type PanelState,
  selectContextPanelOpen,
  selectFileEditorPanelOpen,
  selectSessionBrowserOpen,
  selectSidecarPanelOpen,
} from "./panel";
// Session slice
export {
  _drainOutputBuffer,
  _drainOutputBufferSize,
  createSessionSlice,
  initialSessionState,
  type SessionActions,
  type SessionSlice,
  type SessionState,
  selectActiveSessionId,
  selectSession,
  selectTabOrder,
} from "./session";
// Types
export type { ImmerSet, SliceCreator, StateGet } from "./types";
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
