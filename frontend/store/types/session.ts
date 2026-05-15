import type { ReasoningEffort } from "@/lib/ai";
import type { RetiredPlan, TaskPlan } from "./plan";

export type SessionMode = "terminal" | "agent";
export type InputMode = "terminal" | "agent" | "auto";
export type RenderMode = "timeline" | "fullterm";
export type AiStatus = "disconnected" | "initializing" | "ready" | "error";
export type TabType = "terminal" | "settings" | "home" | "browser" | "security";
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
 * Identifier of the heuristic that detected a stdin prompt. Mirrors
 * `StdinWaitKind::as_event_str` in
 * `backend/crates/golish-pty/src/manager/stdin_wait_detector.rs`. Used by
 * `UnifiedInput` to tweak the placeholder / banner text per pattern.
 */
export type StdinWaitDetector =
  | "yn_choice"
  | "password"
  | "powershell_choice"
  | "continue"
  | "generic_prompt";

/**
 * Set when the backend's `stdin_wait_detector` reports that the running
 * command is blocking on stdin. While `active` is true, the bottom
 * `UnifiedInput` repurposes itself: Enter writes the bytes to the
 * running PTY (instead of starting a new command), the placeholder
 * changes to "回复 $command…", and a small banner explains what's
 * happening.
 *
 * Cleared when:
 *  - the running command emits `command_end`, or
 *  - the user presses Esc to leave interactive mode, or
 *  - the session is destroyed.
 */
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
  gitBranch?: string | null;
  aiConfig?: AiConfig;
  plan?: TaskPlan;
  planMessageId?: string | null;
  retiredPlans?: RetiredPlan[];
  detailViewMode?: DetailViewMode;
  toolDetailRequestIds?: string[] | null;
  interactiveMode?: InteractiveModeState | null;
}
