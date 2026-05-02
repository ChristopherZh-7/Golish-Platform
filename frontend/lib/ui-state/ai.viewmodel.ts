/**
 * AI View Model — UI-ready derived state for AI chat components.
 *
 * Provides selector-friendly helpers to transform raw AI state
 * into shapes that React components can render directly.
 */

export interface TokenUsageSummary {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  estimatedCostUsd: number | null;
}

export function computeTokenUsage(
  stats: { prompt_tokens?: number; completion_tokens?: number; model?: string } | null
): TokenUsageSummary {
  if (!stats) {
    return { promptTokens: 0, completionTokens: 0, totalTokens: 0, estimatedCostUsd: null };
  }
  const prompt = stats.prompt_tokens ?? 0;
  const completion = stats.completion_tokens ?? 0;
  return {
    promptTokens: prompt,
    completionTokens: completion,
    totalTokens: prompt + completion,
    estimatedCostUsd: null,
  };
}

export type AgentActivityStatus =
  | "idle"
  | "thinking"
  | "streaming"
  | "tool_calling"
  | "waiting_approval";

export function deriveAgentStatus(flags: {
  isResponding: boolean;
  isThinking: boolean;
  hasPendingApproval: boolean;
  hasActiveToolCalls: boolean;
}): AgentActivityStatus {
  if (flags.hasPendingApproval) return "waiting_approval";
  if (flags.hasActiveToolCalls) return "tool_calling";
  if (flags.isThinking) return "thinking";
  if (flags.isResponding) return "streaming";
  return "idle";
}
