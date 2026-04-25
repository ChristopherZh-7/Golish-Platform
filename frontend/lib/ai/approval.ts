import { invoke } from "@tauri-apps/api/core";
import type {
  AgentMode,
  ApprovalDecision,
  ApprovalPattern,
  ProjectSettings,
  ToolApprovalConfig,
} from "./types";

export async function getApprovalPatterns(): Promise<ApprovalPattern[]> {
  return invoke("get_approval_patterns");
}

export async function getToolApprovalPattern(toolName: string): Promise<ApprovalPattern | null> {
  return invoke("get_tool_approval_pattern", { toolName });
}

export async function getHitlConfig(): Promise<ToolApprovalConfig> {
  return invoke("get_hitl_config");
}

export async function setHitlConfig(config: ToolApprovalConfig): Promise<void> {
  return invoke("set_hitl_config", { config });
}

export async function addToolAlwaysAllow(toolName: string): Promise<void> {
  return invoke("add_tool_always_allow", { toolName });
}

export async function removeToolAlwaysAllow(toolName: string): Promise<void> {
  return invoke("remove_tool_always_allow", { toolName });
}

export async function resetApprovalPatterns(): Promise<void> {
  return invoke("reset_approval_patterns");
}

export async function respondToToolApproval(
  sessionId: string,
  decision: ApprovalDecision
): Promise<void> {
  return invoke("respond_to_tool_approval", { sessionId, decision });
}

export function calculateApprovalRate(pattern: ApprovalPattern): number {
  if (pattern.total_requests === 0) return 0;
  return pattern.approvals / pattern.total_requests;
}

export function qualifiesForAutoApprove(
  pattern: ApprovalPattern,
  minApprovals = 3,
  threshold = 0.8
): boolean {
  return pattern.approvals >= minApprovals && calculateApprovalRate(pattern) >= threshold;
}

export async function setAgentMode(
  sessionId: string,
  mode: AgentMode,
  workspace?: string
): Promise<void> {
  return invoke("set_agent_mode", { sessionId, mode, workspace });
}

export async function setUseAgents(sessionId: string, enabled: boolean): Promise<void> {
  return invoke("set_use_agents", { sessionId, enabled });
}

export async function setExecutionMode(
  sessionId: string,
  mode: "chat" | "task"
): Promise<void> {
  return invoke("set_execution_mode", { sessionId, mode });
}

export async function getExecutionMode(sessionId: string): Promise<string> {
  return invoke("get_execution_mode", { sessionId });
}

export async function getProjectSettings(workspace: string): Promise<ProjectSettings> {
  return invoke("get_project_settings", { workspace });
}

export async function saveProjectModel(
  workspace: string,
  provider: string,
  model: string
): Promise<void> {
  return invoke("save_project_model", { workspace, provider, model });
}

export async function saveProjectAgentMode(workspace: string, mode: AgentMode): Promise<void> {
  return invoke("save_project_agent_mode", { workspace, mode });
}
