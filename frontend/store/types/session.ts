import type { StageRunRow, StageRunSummary } from "@/components/Engagement/StageRunOrgRows";
import type { ReasoningEffort } from "@/lib/ai";
import type { RetiredPlan, TaskPlan } from "./plan";

export type SessionMode = "terminal" | "agent";
export type InputMode = "terminal" | "agent" | "auto";
export type RenderMode = "timeline" | "fullterm";
export type AiStatus = "disconnected" | "initializing" | "ready" | "error";
export type TabType = "terminal" | "settings" | "home";
export type AgentMode = "default" | "auto-approve" | "planning";
/**
 * Execution mode id matching one of the policies registered in the
 * backend's `ExecutionModeRegistry`. Today this is `"chat" | "task"`,
 * but the type is `string` to forward-allow new modes (`plan`,
 * `debug`, …) without churning every consumer.
 *
 * Use the discriminator constants `EXECUTION_MODE_CHAT` /
 * `EXECUTION_MODE_TASK` for comparisons against the two modes that
 * have hard-coded UI behaviour today.
 */
export type ExecutionMode = string;
export const EXECUTION_MODE_CHAT = "chat";
export const EXECUTION_MODE_TASK = "task";

export type StdinWaitDetector =
  | "yn_choice"
  | "password"
  | "powershell_choice"
  | "continue"
  | "generic_prompt";

export interface AiConfig {
  provider: string;
  model: string;
  status: AiStatus;
  errorMessage?: string;
  reasoningEffort?: ReasoningEffort;
  vertexConfig?: {
    workspace: string;
    credentialsPath: string;
    projectId: string;
    location: string;
  };
}

export type DetailViewMode = "timeline" | "tool-detail" | "sub-agent-detail";

/**
 * A stage-run's live per-org fan-out (设计 2026-06-13-stage-run-fanout). Holds the
 * per-org rows + summary + stage config for {@link StageRunOrgRows}, which renders
 * inside the standard tool-call detail pane when the selected tool is `stage_run`.
 * `requestId` ties this run to its `stage_run` tool execution so the detail only
 * shows these rows on the matching tool row.
 */
export interface SessionStageRun {
  rows: StageRunRow[];
  summary: StageRunSummary;
  stageLabel: string;
  roleLabel: string;
  coverageAxis: string[];
  /** The `stage_run` tool call's requestId this run belongs to, if known. */
  requestId?: string;
}

/**
 * Refresh-only hint emitted by Candidate review harness traces. The review
 * panel always reloads authoritative decisions/barrier state through IPC.
 */
export interface CandidateReviewHint {
  operationId: string;
  waveRunId: string;
  status: string;
  resumeVersion: number;
  refreshVersion: number;
}

/**
 * Refresh-only pointer from a Reporting harness trace. The report component
 * must reload the authoritative read model through IPC; no report content or
 * gate truth is stored here.
 */
export interface ReportingReadModelHint {
  operationId: string;
  refreshVersion: number;
}

export interface InteractiveModeState {
  active: boolean;
  command: string | null;
  detector: StdinWaitDetector;
  enteredAt: number;
}

export interface Session {
  id: string;
  logicalTerminalId?: string;
  tabType?: TabType;
  name: string;
  workingDirectory: string;
  createdAt: string;
  mode: SessionMode;
  inputMode?: InputMode;
  agentMode?: AgentMode;
  executionMode?: ExecutionMode;
  renderMode?: RenderMode;
  customName?: string;
  processName?: string;
  virtualEnv?: string | null;
  aiConfig?: AiConfig;
  plan?: TaskPlan;
  planMessageId?: string | null;
  retiredPlans?: RetiredPlan[];
  /**
   * Per-harness-stage plan buckets (task mode, design 2026-06-04). Each
   * `update_plan` tagged with a `stage_id` lands in its own bucket so the UI
   * renders one card per stage instead of a single ever-growing list.
   * Chat / non-harness planning leaves this undefined and uses `plan` above.
   */
  plansByStage?: Record<string, TaskPlan>;
  /** Order stages first appeared, so the per-stage cards render in run order. */
  stageOrder?: string[];
  /**
   * Stages whose authoritative evidence gate PASSED (backend `stage_passed`
   * TaskProgress, emitted from `consume_gate_outcome`). Drives the per-stage
   * card's "completed" state instead of the model's self-reported todo statuses,
   * so a stage only reads as done once the deterministic gate accepts it.
   */
  passedStages?: string[];
  detailViewMode?: DetailViewMode;
  toolDetailRequestIds?: string[] | null;
  /**
   * Live per-org progress for this session's current `stage_run` tool call.
   * Rendered inside the standard tool-call detail pane (ToolCallDetailView) for
   * the matching `stage_run` tool row.
   */
  stageRun?: SessionStageRun | null;
  /**
   * Request-scoped stage-run snapshots. A chat can contain multiple `stage_run`
   * tool calls after an interrupt/continue, so rows must be keyed by the tool
   * requestId instead of one session-wide mutable slot.
   */
  stageRuns?: Record<string, SessionStageRun>;
  candidateReviewHint?: CandidateReviewHint;
  reportingReadModelHint?: ReportingReadModelHint;
  interactiveMode?: InteractiveModeState | null;
}
