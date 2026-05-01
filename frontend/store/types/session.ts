import type { ReasoningEffort } from "@/lib/ai";
import type { RetiredPlan, TaskPlan } from "./plan";

export type SessionMode = "terminal" | "agent";
export type InputMode = "terminal" | "agent" | "auto";
export type RenderMode = "timeline" | "fullterm";
export type AiStatus = "disconnected" | "initializing" | "ready" | "error";
export type TabType = "terminal" | "settings" | "home" | "browser" | "security";
export type AgentMode = "default" | "auto-approve" | "planning";
export type ExecutionMode = "chat" | "task";

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
  useAgents?: boolean;
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
}
