/**
 * Domain types for the Golish store.
 *
 * Types are organized by domain concern:
 *   plan.ts       — Task plan types (PlanStep, TaskPlan, etc.)
 *   session.ts    — Session & AI config types
 *   tool-call.ts  — Tool call & approval types
 *   streaming.ts  — Streaming block types
 *   workflow.ts   — Workflow execution types
 *   sub-agent.ts  — Sub-agent types
 *   message.ts    — Command block, agent message, AI tool execution
 *   timeline.ts   — Unified timeline block union type
 */

export type { ApprovalPattern, ReasoningEffort } from "@/lib/ai";
export type { RiskLevel } from "@/lib/tools";
export type { BackgroundJob, BackgroundJobOrigin, BackgroundRunMeta } from "./background-job";
export type {
  AgentMessage,
  AiToolExecution,
  CommandBlock,
  PendingCommand,
} from "./message";
export type { PlanStep, PlanSummary, RetiredPlan, StepStatus, TaskPlan } from "./plan";
export type {
  AgentMode,
  AiConfig,
  AiStatus,
  CandidateReviewHint,
  DetailViewMode,
  ExecutionMode,
  InputMode,
  InteractiveModeState,
  RenderMode,
  ReportingReadModelHint,
  Session,
  SessionMode,
  SessionStageRun,
  StdinWaitDetector,
  TabType,
} from "./session";
export type {
  CompactionResult,
  FinalizedStreamingBlock,
  StreamingBlock,
} from "./streaming";
export type { ActiveSubAgent, SubAgentEntry, SubAgentToolCall } from "./sub-agent";

export type { UnifiedBlock } from "./timeline";
export type {
  ActiveToolCall,
  AskHumanRequest,
  ToolCall,
  ToolCallSource,
} from "./tool-call";
export type { ActiveWorkflow, WorkflowStatus, WorkflowStep } from "./workflow";
