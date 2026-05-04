/**
 * AI Service — domain-level API for AI agent operations.
 *
 * Aggregates session management, prompt sending, tool execution,
 * and persistence into a single cohesive service.
 *
 * Components should import from here instead of calling invoke() directly.
 */

import type {
  ProviderConfig,
  SessionAiConfigInfo,
  SubAgentInfo,
  ToolDefinition,
  WorkflowInfo,
} from "../ai/types";
import { invoke } from "../transport";

/**
 * Initialise the legacy single AI agent through the unified
 * `init_ai_agent` IPC.
 *
 * Prefer the per-session `initSession` flow below; this wrapper is kept
 * for the headless / dev paths that still target the legacy bridge.
 */
export async function initAgent(config: ProviderConfig): Promise<void> {
  return invoke("init_ai_agent", { config });
}

export async function shutdownAgent(): Promise<void> {
  return invoke("shutdown_ai_agent");
}

export async function isInitialized(): Promise<boolean> {
  return invoke("is_ai_initialized");
}

export async function sendPrompt(prompt: string): Promise<string> {
  return invoke("send_ai_prompt", { prompt });
}

export async function cancelGeneration(sessionId: string): Promise<void> {
  return invoke("cancel_ai_generation", { sessionId });
}

export async function clearConversation(): Promise<void> {
  return invoke("clear_ai_conversation");
}

export async function getAvailableTools(): Promise<ToolDefinition[]> {
  return invoke("get_available_tools");
}

export async function getAvailableWorkflows(): Promise<WorkflowInfo[]> {
  return invoke("list_workflows");
}

export async function getAvailableSubAgents(): Promise<SubAgentInfo[]> {
  return invoke("list_sub_agents");
}

export async function getSessionConfig(sessionId: string): Promise<SessionAiConfigInfo | null> {
  return invoke("get_session_ai_config", { sessionId });
}

export async function initSession(params: {
  sessionId: string;
  workspace: string;
  provider: string;
  model: string;
  apiKey?: string;
}): Promise<void> {
  return invoke("init_ai_session", params);
}

export async function shutdownSession(sessionId: string): Promise<void> {
  return invoke("shutdown_ai_session", { sessionId });
}

export async function sendPromptSession(sessionId: string, prompt: string): Promise<string> {
  return invoke("send_ai_prompt_session", { sessionId, prompt });
}

export async function signalFrontendReady(sessionId: string): Promise<void> {
  return invoke("signal_frontend_ready", { sessionId });
}
