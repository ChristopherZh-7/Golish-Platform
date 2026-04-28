import { invoke } from "@tauri-apps/api/core";
import type {
  ApiRequestStatsSnapshot,
  AuditEntry,
  CommitMessageResponse,
  DbTokenUsageStats,
  MemoryEntry,
  PromptPayload,
  SessionListingInfo,
  SessionSnapshot,
  TaskPlan,
  TextPart,
  ToolCallStats,
  VisionCapabilities,
  AgentFileInfo,
  SkillInfo,
  RuleInfo,
} from "./types";

// ── Session Persistence ──────────────────────

export async function listAiSessions(limit?: number): Promise<SessionListingInfo[]> {
  return invoke("list_ai_sessions", { limit });
}

export async function findAiSession(identifier: string): Promise<SessionListingInfo | null> {
  return invoke("find_ai_session", { identifier });
}

export async function loadAiSession(identifier: string): Promise<SessionSnapshot | null> {
  return invoke("load_ai_session", { identifier });
}

export async function exportAiSessionTranscript(
  identifier: string,
  outputPath: string
): Promise<void> {
  return invoke("export_ai_session_transcript", { identifier, outputPath });
}

export async function setAiSessionPersistence(enabled: boolean, sessionId?: string): Promise<void> {
  return invoke("set_ai_session_persistence", { enabled, sessionId });
}

export async function isAiSessionPersistenceEnabled(sessionId?: string): Promise<boolean> {
  return invoke("is_ai_session_persistence_enabled", { sessionId });
}

export async function finalizeAiSession(sessionId?: string): Promise<string | null> {
  return invoke("finalize_ai_session", { sessionId });
}

export async function restoreAiSession(
  sessionId: string,
  identifier: string
): Promise<SessionSnapshot> {
  return invoke("restore_ai_session", { sessionId, identifier });
}

export async function restoreAiConversation(
  sessionId: string,
  messages: [string, string][]
): Promise<void> {
  return invoke("restore_ai_conversation", { sessionId, messages });
}

// ── Plan ──────────────────────

export async function getPlan(sessionId: string): Promise<TaskPlan> {
  return invoke("get_plan", { sessionId });
}

// ── Vision & Multi-Modal ──────────────────────

export async function getVisionCapabilities(sessionId: string): Promise<VisionCapabilities> {
  return invoke("get_vision_capabilities", { sessionId });
}

export async function getApiRequestStats(sessionId: string): Promise<ApiRequestStatsSnapshot> {
  return invoke("get_api_request_stats", { sessionId });
}

export async function sendPromptWithAttachments(
  sessionId: string,
  payload: PromptPayload
): Promise<string> {
  return invoke("send_ai_prompt_with_attachments", { sessionId, payload });
}

export function createTextPayload(text: string): PromptPayload {
  return { parts: [{ type: "text", text }] };
}

export function hasImages(payload: PromptPayload): boolean {
  return payload.parts.some((part) => part.type === "image");
}

export function extractText(payload: PromptPayload): string {
  return payload.parts
    .filter((part): part is TextPart => part.type === "text")
    .map((part) => part.text)
    .join("\n");
}

// ── Commit Writer ──────────────────────

export async function generateCommitMessage(
  sessionId: string,
  diff: string,
  fileSummary?: string
): Promise<CommitMessageResponse> {
  return invoke("generate_commit_message", { sessionId, diff, fileSummary });
}

// ── Analytics / DB commands ──────────────────────

export async function getToolCallStats(sessionId?: string): Promise<ToolCallStats[]> {
  return invoke("get_tool_call_stats", { sessionId });
}

export async function getDbTokenUsageStats(): Promise<DbTokenUsageStats> {
  return invoke("get_db_token_usage_stats", {});
}

export async function getAuditLog(
  projectPath?: string,
  category?: string,
  limit?: number
): Promise<AuditEntry[]> {
  return invoke("get_audit_log", { projectPath, category, limit });
}

export async function searchMemories(query: string, limit?: number): Promise<MemoryEntry[]> {
  return invoke("search_memories", { query, limit });
}

export async function listRecentMemories(limit?: number): Promise<MemoryEntry[]> {
  return invoke("list_recent_memories", { limit });
}

export async function getMemoryCount(): Promise<number> {
  return invoke("get_memory_count", {});
}

// ── Agent Definition Management ──────────────────────

export async function listAgentDefinitions(workingDirectory?: string): Promise<AgentFileInfo[]> {
  return invoke("list_agent_definitions", { workingDirectory });
}

export async function readAgentPrompt(agentId: string, workingDirectory?: string): Promise<string> {
  return invoke("read_agent_prompt", { agentId, workingDirectory });
}

export async function saveAgentDefinition(params: {
  agentId: string;
  name: string;
  description: string;
  systemPrompt: string;
  allowedTools: string[];
  maxIterations?: number;
  timeoutSecs?: number;
  idleTimeoutSecs?: number;
  readonly?: boolean;
  isBackground?: boolean;
  model?: string;
  temperature?: number;
  maxTokens?: number;
  topP?: number;
  scope?: "global" | "project";
  workingDirectory?: string;
}): Promise<string> {
  return invoke("save_agent_definition", params);
}

export async function deleteAgentDefinition(
  agentId: string,
  workingDirectory?: string
): Promise<void> {
  return invoke("delete_agent_definition", { agentId, workingDirectory });
}

export async function seedAgents(): Promise<number> {
  return invoke("seed_agents");
}

// ── Skill Management ──────────────────────

export async function listSkills(workingDirectory?: string): Promise<SkillInfo[]> {
  return invoke("list_skills", { workingDirectory });
}

export async function readSkillBody(path: string): Promise<string> {
  return invoke("read_skill_body", { path });
}

export async function saveSkill(params: {
  name: string;
  description: string;
  body: string;
  scope?: "global" | "project";
  workingDirectory?: string;
}): Promise<string> {
  return invoke("save_skill", params);
}

export async function deleteSkill(skillPath: string): Promise<void> {
  return invoke("delete_skill", { skillPath });
}

// ── Rule Management ──────────────────────

export async function listRules(workingDirectory?: string): Promise<RuleInfo[]> {
  return invoke("list_rules", { workingDirectory });
}

export async function readRuleBody(rulePath: string): Promise<string> {
  return invoke("read_rule_body", { rulePath });
}

export async function saveRule(params: {
  name: string;
  description: string;
  body: string;
  globs?: string;
  alwaysApply: boolean;
  scope?: "global" | "project";
  workingDirectory?: string;
}): Promise<string> {
  return invoke("save_rule", params);
}

export async function deleteRule(rulePath: string): Promise<void> {
  return invoke("delete_rule", { rulePath });
}
