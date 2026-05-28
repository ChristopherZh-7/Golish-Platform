/**
 * Settings View Model — UI-ready derived state for settings panels.
 *
 * Transforms raw GolishSettings into shapes optimized for rendering
 * (e.g., provider lists, feature flags, validation states).
 */

import type { GolishSettings, ProviderVisibility } from "../settings/types";

export interface ProviderCard {
  id: string;
  label: string;
  enabled: boolean;
  configured: boolean;
}

export function deriveProviderCards(
  settings: GolishSettings,
  visibility: ProviderVisibility
): ProviderCard[] {
  const entries: ProviderCard[] = [
    {
      id: "openai",
      label: "OpenAI",
      enabled: visibility.openai,
      configured: !!settings.ai.openai.api_key,
    },
    {
      id: "anthropic",
      label: "Anthropic",
      enabled: visibility.anthropic,
      configured: !!settings.ai.anthropic.api_key,
    },
    {
      id: "gemini",
      label: "Google Gemini",
      enabled: visibility.gemini,
      configured: !!settings.ai.gemini.api_key,
    },
    {
      id: "deepseek",
      label: "DeepSeek",
      enabled: visibility.deepseek,
      configured: !!settings.ai.deepseek?.api_key,
    },
    {
      id: "xiaomi",
      label: "Xiaomi MiMo",
      enabled: visibility.xiaomi,
      configured: !!settings.ai.xiaomi?.api_key,
    },
    {
      id: "vertex_ai",
      label: "Vertex AI (Claude)",
      enabled: visibility.vertex_ai,
      configured: !!settings.ai.vertex_ai.project_id,
    },
    {
      id: "vertex_gemini",
      label: "Vertex Gemini",
      enabled: visibility.vertex_gemini,
      configured: !!settings.ai.vertex_gemini?.project_id,
    },
    {
      id: "ollama",
      label: "Ollama",
      enabled: visibility.ollama,
      configured: true,
    },
    {
      id: "groq",
      label: "Groq",
      enabled: visibility.groq,
      configured: !!settings.ai.groq.api_key,
    },
    {
      id: "xai",
      label: "xAI (Grok)",
      enabled: visibility.xai,
      configured: !!settings.ai.xai.api_key,
    },
    {
      id: "openrouter",
      label: "OpenRouter",
      enabled: visibility.openrouter,
      configured: !!settings.ai.openrouter.api_key,
    },
  ];
  return entries;
}

export function countConfiguredProviders(cards: ProviderCard[]): number {
  return cards.filter((c) => c.enabled && c.configured).length;
}
