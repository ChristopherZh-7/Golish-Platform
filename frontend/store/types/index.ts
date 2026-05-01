/**
 * Domain types for the Golish store.
 *
 * Types are organized by domain concern:
 *   plan.ts       — Task plan types (PlanStep, TaskPlan, etc.)
 *   session.ts    — Session & AI config types
 *   pipeline.ts   — Pipeline execution types
 *   tool-call.ts  — Tool call & approval types
 *   streaming.ts  — Streaming block types
 *   workflow.ts   — Workflow execution types
 *   sub-agent.ts  — Sub-agent types
 *   message.ts    — Command block, agent message, AI tool execution
 *   timeline.ts   — Unified timeline block union type
 */

export type { StepStatus, PlanStep, PlanSummary, TaskPlan, RetiredPlan } from "./plan";

export type {
  SessionMode,
  InputMode,
  RenderMode,
  AiStatus,
  TabType,
  AgentMode,
  ExecutionMode,
  AiConfig,
  DetailViewMode,
  Session,
} from "./session";

export type {
  PipelineStepStatus,
  PipelineSubTarget,
  PipelineStepExecution,
  PipelineExecution,
} from "./pipeline";

export type {
  ToolCallSource,
  ToolCall,
  ActiveToolCall,
  AskHumanRequest,
} from "./tool-call";

export type {
  StreamingBlock,
  FinalizedStreamingBlock,
  CompactionResult,
} from "./streaming";

export type { WorkflowStatus, WorkflowStep, ActiveWorkflow } from "./workflow";

export type { SubAgentToolCall, SubAgentEntry, ActiveSubAgent } from "./sub-agent";

export type {
  CommandBlock,
  AgentMessage,
  AiToolExecution,
  PendingCommand,
} from "./message";

export type { UnifiedBlock } from "./timeline";

export type { ApprovalPattern, ReasoningEffort } from "@/lib/ai";
export type { RiskLevel } from "@/lib/tools";
