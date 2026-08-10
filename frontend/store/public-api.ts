/**
 * Barrel of types/selectors/actions that consumers import via `@/store/...`.
 *
 * Kept separate from `store/index.ts` so the latter can stay under ~80 lines
 * (one job: compose slices). All re-exports here are barrel-style and have
 * no runtime cost.
 */

export type { PaneId, PaneNode, SplitDirection, TabLayout } from "@/lib/pane-utils";
export {
  type CloseTabAndCleanupOptions,
  clearConversation,
  closeTabAndCleanup,
  type OpenProjectOptions,
  openProject,
  restoreSession,
} from "./actions";
export * from "./selectors/store-hooks";
export type {
  ChatConversation,
  ChatMessage,
  ChatToolCall,
  ContextMetrics,
  Notification,
  NotificationType,
} from "./slices";
export { _drainOutputBuffer, _drainOutputBufferSize } from "./slices/session";
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
  BackgroundJob,
  BackgroundJobOrigin,
  BackgroundRunMeta,
  CommandBlock,
  CompactionResult,
  DetailViewMode,
  ExecutionMode,
  FinalizedStreamingBlock,
  InputMode,
  InvestigationRefreshHint,
  PendingCommand,
  PlanStep,
  PlanSummary,
  ReasoningEffort,
  RenderMode,
  ReportingReadModelHint,
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
