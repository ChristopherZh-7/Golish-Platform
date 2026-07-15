import type {
  ActiveSubAgent,
  ActiveToolCall,
  ActiveWorkflow,
  Session,
  TaskPlan,
  UnifiedBlock,
} from "../../store-types";

export interface WorkflowState {
  activeWorkflows: Record<string, ActiveWorkflow | null>;
  workflowHistory: Record<string, ActiveWorkflow[]>;
  activeSubAgents: Record<string, ActiveSubAgent[]>;
  subAgentBatchCounter: Record<string, number>;
}

/**
 * Cross-slice fields accessed by workflow sub-module actions during Immer
 * mutations. Extends WorkflowState with the foreign-slice fields that
 * sub-agent and plan actions read/write.
 */
export interface WorkflowStoreDraft extends WorkflowState {
  sessions: Record<string, Session>;
  timelines: Record<string, UnifiedBlock[]>;
  activeToolCalls?: Record<string, ActiveToolCall[]>;
}

export interface WorkflowActions {
  startWorkflow: (
    sessionId: string,
    workflow: { workflowId: string; workflowName: string; workflowSessionId: string }
  ) => void;
  workflowStepStarted: (
    sessionId: string,
    step: { stepName: string; stepIndex: number; totalSteps: number }
  ) => void;
  workflowStepCompleted: (
    sessionId: string,
    step: { stepName: string; output: string | null; durationMs: number }
  ) => void;
  completeWorkflow: (
    sessionId: string,
    result: { finalOutput: string; totalDurationMs: number }
  ) => void;
  failWorkflow: (sessionId: string, error: { stepName: string | null; error: string }) => void;
  clearActiveWorkflow: (sessionId: string) => void;
  preserveWorkflowToolCalls: (sessionId: string) => void;

  startPromptGeneration: (
    sessionId: string,
    agentId: string,
    parentRequestId: string,
    data: { architectSystemPrompt: string; architectUserMessage: string }
  ) => void;
  completePromptGeneration: (
    sessionId: string,
    agentId: string,
    parentRequestId: string,
    data: { generatedPrompt?: string; success: boolean; durationMs: number }
  ) => void;
  startSubAgent: (
    sessionId: string,
    agent: {
      agentId: string;
      agentName: string;
      parentRequestId: string;
      task: string;
      depth: number;
    }
  ) => void;
  addSubAgentToolCall: (
    sessionId: string,
    parentRequestId: string,
    toolCall: { id: string; name: string; args: Record<string, unknown> }
  ) => void;
  completeSubAgentToolCall: (
    sessionId: string,
    parentRequestId: string,
    toolId: string,
    success: boolean,
    result?: unknown
  ) => void;
  completeBackgroundedSubAgentToolCall: (
    sessionId: string,
    jobId: string,
    success: boolean,
    result?: unknown
  ) => void;
  completeSubAgent: (
    sessionId: string,
    parentRequestId: string,
    result: { response: string; durationMs: number }
  ) => void;
  failSubAgent: (sessionId: string, parentRequestId: string, error: string) => void;
  updateSubAgentStreamingText: (sessionId: string, parentRequestId: string, text: string) => void;
  updateSubAgentThinking: (
    sessionId: string,
    parentRequestId: string,
    text: string,
    timing?: { startedAt: number; endedAt: number }
  ) => void;
  appendSubAgentToolOutput: (sessionId: string, toolId: string, chunk: string) => void;
  clearActiveSubAgents: (sessionId: string) => void;

  setPlan: (
    sessionId: string,
    plan: TaskPlan,
    currentMessageId?: string | null,
    newMessageId?: string | null
  ) => void;
  /**
   * Route a stage-tagged plan update into its per-stage bucket
   * (`plansByStage[stageId]`) instead of the single session plan. Used by
   * task-mode harness stages so each stage renders its own card.
   */
  setStagePlan: (sessionId: string, stageId: string, plan: TaskPlan) => void;
  /**
   * Record a stage whose authoritative evidence gate PASSED. Drives the
   * per-stage card's completed state (not the model's self-reported todos), so a
   * stage shows done only after the deterministic gate accepts it.
   */
  markStagePassed: (sessionId: string, stageId: string) => void;
}

export interface WorkflowSlice extends WorkflowState, WorkflowActions {}

export const initialWorkflowState: WorkflowState = {
  activeWorkflows: {},
  workflowHistory: {},
  activeSubAgents: {},
  subAgentBatchCounter: {},
};
