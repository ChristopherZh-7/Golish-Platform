import { invoke } from "./client";

export interface ContextSummary {
  total_tokens: number;
  max_tokens: number;
  utilization: number;
  messages: number;
}

export interface TokenUsageStats {
  total_input: number;
  total_output: number;
  total_tokens: number;
  budget: number;
  utilization: number;
}

export interface ContextTrimConfig {
  enabled: boolean;
  threshold: number;
  target: number;
}

export async function getContextSummary(sessionId: string): Promise<ContextSummary> {
  return invoke<ContextSummary>("get_context_summary", { sessionId });
}

export async function getTokenUsageStats(sessionId: string): Promise<TokenUsageStats> {
  return invoke<TokenUsageStats>("get_token_usage_stats", { sessionId });
}

export async function getTokenAlertLevel(sessionId: string): Promise<string> {
  return invoke<string>("get_token_alert_level", { sessionId });
}

export async function getContextUtilization(sessionId: string): Promise<number> {
  return invoke<number>("get_context_utilization", { sessionId });
}

export async function getRemainingTokens(sessionId: string): Promise<number> {
  return invoke<number>("get_remaining_tokens", { sessionId });
}

export async function resetContextManager(sessionId: string): Promise<void> {
  return invoke("reset_context_manager", { sessionId });
}

export async function getContextTrimConfig(sessionId: string): Promise<ContextTrimConfig> {
  return invoke<ContextTrimConfig>("get_context_trim_config", { sessionId });
}

export async function isContextManagementEnabled(sessionId: string): Promise<boolean> {
  return invoke<boolean>("is_context_management_enabled", { sessionId });
}

export interface LoopProtectionConfig {
  enabled: boolean;
  max_consecutive_errors: number;
  max_tool_repeats: number;
  cooldown_ms: number;
}

export interface LoopDetectorStats {
  consecutive_errors: number;
  tool_repeat_counts: Record<string, number>;
  is_in_cooldown: boolean;
}

export async function getLoopProtectionConfig(sessionId: string): Promise<LoopProtectionConfig> {
  return invoke<LoopProtectionConfig>("get_loop_protection_config", { sessionId });
}

export async function setLoopProtectionConfig(
  sessionId: string,
  config: LoopProtectionConfig
): Promise<void> {
  return invoke("set_loop_protection_config", { sessionId, config });
}

export async function getLoopDetectorStats(sessionId: string): Promise<LoopDetectorStats> {
  return invoke<LoopDetectorStats>("get_loop_detector_stats", { sessionId });
}

export async function isLoopDetectionEnabled(sessionId: string): Promise<boolean> {
  return invoke<boolean>("is_loop_detection_enabled", { sessionId });
}

export async function disableLoopDetection(sessionId: string): Promise<void> {
  return invoke("disable_loop_detection", { sessionId });
}

export async function enableLoopDetection(sessionId: string): Promise<void> {
  return invoke("enable_loop_detection", { sessionId });
}

export async function resetLoopDetector(sessionId: string): Promise<void> {
  return invoke("reset_loop_detector", { sessionId });
}
