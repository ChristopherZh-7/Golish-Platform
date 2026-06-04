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
  interactiveMode?: InteractiveModeState | null;
}
