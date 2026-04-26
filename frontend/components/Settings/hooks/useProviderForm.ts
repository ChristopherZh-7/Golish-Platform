import { useCallback, useEffect, useState } from "react";
import { logger } from "@/lib/logger";
import { getProviders, type ProviderInfo } from "@/lib/model-registry";
import type { AiSettings, OpenRouterProviderPreferences } from "@/lib/settings";

export type ProviderSettingsKey = keyof Pick<
  AiSettings,
  | "vertex_ai"
  | "vertex_gemini"
  | "openrouter"
  | "anthropic"
  | "openai"
  | "ollama"
  | "gemini"
  | "groq"
  | "xai"
  | "zai_sdk"
  | "nvidia"
>;

export interface ProviderConfig {
  id: ProviderSettingsKey;
  name: string;
  icon: string;
  description: string;
  getConfigured: (settings: AiSettings) => boolean;
}

function isProviderConfigured(id: ProviderSettingsKey, settings: AiSettings): boolean {
  switch (id) {
    case "anthropic":
      return !!settings.anthropic.api_key;
    case "gemini":
      return !!settings.gemini.api_key;
    case "groq":
      return !!settings.groq.api_key;
    case "ollama":
      return !!settings.ollama.base_url;
    case "openai":
      return !!settings.openai.api_key;
    case "openrouter":
      return !!settings.openrouter.api_key;
    case "vertex_ai":
      return !!(settings.vertex_ai.credentials_path || settings.vertex_ai.project_id);
    case "vertex_gemini":
      return !!(settings.vertex_gemini.credentials_path || settings.vertex_gemini.project_id);
    case "xai":
      return !!settings.xai.api_key;
    case "zai_sdk":
      return !!settings.zai_sdk?.api_key;
    case "nvidia":
      return !!settings.nvidia?.api_key;
    default:
      return false;
  }
}

function providerToSettingsKey(provider: string): ProviderSettingsKey | null {
  const mapping: Record<string, ProviderSettingsKey> = {
    vertex_ai: "vertex_ai",
    vertex_gemini: "vertex_gemini",
    anthropic: "anthropic",
    openai: "openai",
    openrouter: "openrouter",
    ollama: "ollama",
    gemini: "gemini",
    groq: "groq",
    xai: "xai",
    zai_sdk: "zai_sdk",
    nvidia: "nvidia",
  };
  return mapping[provider] ?? null;
}

function toProviderConfig(info: ProviderInfo): ProviderConfig | null {
  const settingsKey = providerToSettingsKey(info.provider);
  if (!settingsKey) return null;

  return {
    id: settingsKey,
    name: info.name,
    icon: info.icon,
    description: info.description,
    getConfigured: (settings: AiSettings) => isProviderConfigured(settingsKey, settings),
  };
}

export const FALLBACK_PROVIDERS: ProviderConfig[] = [
  { id: "anthropic", name: "Anthropic", icon: "🔶", description: "Direct Claude API access", getConfigured: (s) => !!s.anthropic.api_key },
  { id: "gemini", name: "Gemini", icon: "💎", description: "Google AI models via API", getConfigured: (s) => !!s.gemini.api_key },
  { id: "groq", name: "Groq", icon: "⚡", description: "Ultra-fast LLM inference", getConfigured: (s) => !!s.groq.api_key },
  { id: "ollama", name: "Ollama", icon: "🦙", description: "Run models locally on your machine", getConfigured: (s) => !!s.ollama.base_url },
  { id: "openai", name: "OpenAI", icon: "⚪", description: "GPT models via OpenAI API", getConfigured: (s) => !!s.openai.api_key },
  { id: "openrouter", name: "OpenRouter", icon: "🔀", description: "Access multiple models via one API", getConfigured: (s) => !!s.openrouter.api_key },
  { id: "vertex_ai", name: "Vertex AI", icon: "🔷", description: "Claude models via Google Cloud", getConfigured: (s) => !!(s.vertex_ai.credentials_path || s.vertex_ai.project_id) },
  { id: "vertex_gemini", name: "Vertex AI Gemini", icon: "💎", description: "Gemini models via Google Cloud", getConfigured: (s) => !!(s.vertex_gemini.credentials_path || s.vertex_gemini.project_id) },
  { id: "xai", name: "xAI", icon: "𝕏", description: "Grok models from xAI", getConfigured: (s) => !!s.xai.api_key },
  { id: "zai_sdk", name: "Z.AI SDK", icon: "🤖", description: "Z.AI native SDK (GLM models)", getConfigured: (s) => !!s.zai_sdk?.api_key },
  { id: "nvidia", name: "NVIDIA NIM", icon: "🟢", description: "NVIDIA NIM inference (OpenAI-compatible)", getConfigured: (s) => !!s.nvidia?.api_key },
];

export const PROVIDER_COLORS: Record<string, { bg: string; border: string; dot: string }> = {
  groq:          { bg: "rgba(245,158,11,0.07)",  border: "rgba(245,158,11,0.35)", dot: "#F59E0B" },
  ollama:        { bg: "rgba(139,92,246,0.07)",   border: "rgba(139,92,246,0.35)", dot: "#8B5CF6" },
  nvidia:        { bg: "rgba(118,185,0,0.07)",    border: "rgba(118,185,0,0.35)",  dot: "#76B900" },
  vertex_ai:     { bg: "rgba(66,133,244,0.07)",   border: "rgba(66,133,244,0.35)", dot: "#4285F4" },
  vertex_gemini: { bg: "rgba(142,117,255,0.07)",  border: "rgba(142,117,255,0.35)",dot: "#8E75FF" },
  anthropic:     { bg: "rgba(217,119,6,0.07)",    border: "rgba(217,119,6,0.35)",  dot: "#D97706" },
  openai:        { bg: "rgba(156,163,175,0.07)",  border: "rgba(156,163,175,0.35)",dot: "#9CA3AF" },
  gemini:        { bg: "rgba(66,133,244,0.07)",   border: "rgba(66,133,244,0.35)", dot: "#4285F4" },
  xai:           { bg: "rgba(156,163,175,0.07)",  border: "rgba(156,163,175,0.35)",dot: "#9CA3AF" },
  zai_sdk:       { bg: "rgba(6,182,212,0.07)",    border: "rgba(6,182,212,0.35)",  dot: "#06B6D4" },
  openrouter:    { bg: "rgba(168,85,247,0.07)",   border: "rgba(168,85,247,0.35)", dot: "#A855F7" },
};

const DEFAULT_COLOR = { bg: "rgba(156,163,175,0.07)", border: "rgba(156,163,175,0.35)", dot: "#9CA3AF" };

export function useProviderForm(settings: AiSettings, onChange: (settings: AiSettings) => void) {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderConfig[]>(FALLBACK_PROVIDERS);

  useEffect(() => {
    getProviders()
      .then((backendProviders) => {
        const configs = backendProviders
          .map(toProviderConfig)
          .filter((p): p is ProviderConfig => p !== null);
        if (configs.length > 0) {
          setProviders(configs);
        }
      })
      .catch((err) => {
        logger.warn("Failed to fetch providers from backend, using fallback:", err);
      });
  }, []);

  const updateProvider = useCallback(<K extends keyof AiSettings>(
    provider: K,
    field: string,
    value: string | boolean | null
  ) => {
    const providerSettings = settings[provider];
    if (typeof providerSettings === "object" && providerSettings !== null) {
      onChange({
        ...settings,
        [provider]: {
          ...providerSettings,
          [field]: typeof value === "boolean" ? value : value || null,
        },
      });
    }
  }, [settings, onChange]);

  const updateOpenRouterPref = useCallback(<K extends keyof OpenRouterProviderPreferences>(
    field: K,
    value: OpenRouterProviderPreferences[K]
  ) => {
    onChange({
      ...settings,
      openrouter: {
        ...settings.openrouter,
        provider_preferences: {
          ...(settings.openrouter.provider_preferences || {}),
          [field]: value,
        },
      },
    });
  }, [settings, onChange]);

  const getShowInSelector = useCallback((providerId: ProviderConfig["id"]): boolean => {
    const providerSettings = settings[providerId];
    if (typeof providerSettings === "object" && "show_in_selector" in providerSettings) {
      return providerSettings.show_in_selector;
    }
    return true;
  }, [settings]);

  const getColor = useCallback((id: string) => PROVIDER_COLORS[id] ?? DEFAULT_COLOR, []);

  const configuredProviders = providers.filter((p) => p.getConfigured(settings));
  const unconfiguredProviders = providers.filter((p) => !p.getConfigured(settings));

  return {
    selectedId,
    setSelectedId,
    providers,
    configuredProviders,
    unconfiguredProviders,
    updateProvider,
    updateOpenRouterPref,
    getShowInSelector,
    getColor,
  };
}
