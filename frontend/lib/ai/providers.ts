import { invoke } from "@/lib/api/client";
import type { GolishSettings, OpenRouterProviderPreferences } from "../settings";
import { resolveProviderOverride } from "./model-overrides";
import type { AiProvider, ProviderConfig, VertexAiEnvConfig } from "./types";

export async function getVertexAiConfig(): Promise<VertexAiEnvConfig> {
  return invoke("get_vertex_ai_config");
}

export async function getOpenAiApiKey(): Promise<string | null> {
  return invoke("get_openai_api_key");
}

export async function getAnthropicApiKey(): Promise<string | null> {
  return invoke("get_anthropic_api_key");
}

function buildOpenRouterProviderPreferencesJson(
  prefs: NonNullable<OpenRouterProviderPreferences>
): Record<string, unknown> {
  const provider: Record<string, unknown> = {};

  if (prefs.order) provider.order = prefs.order;
  if (prefs.only) provider.only = prefs.only;
  if (prefs.ignore) provider.ignore = prefs.ignore;
  if (prefs.allow_fallbacks != null) provider.allow_fallbacks = prefs.allow_fallbacks;
  if (prefs.require_parameters != null) provider.require_parameters = prefs.require_parameters;
  if (prefs.data_collection) provider.data_collection = prefs.data_collection;
  if (prefs.zdr != null) provider.zdr = prefs.zdr;
  if (prefs.sort) provider.sort = prefs.sort;
  if (prefs.preferred_min_throughput != null)
    provider.preferred_min_throughput = prefs.preferred_min_throughput;
  if (prefs.preferred_max_latency != null)
    provider.preferred_max_latency = prefs.preferred_max_latency;

  if (prefs.max_price_prompt != null || prefs.max_price_completion != null) {
    const maxPrice: Record<string, number> = {};
    if (prefs.max_price_prompt != null) maxPrice.prompt = prefs.max_price_prompt;
    if (prefs.max_price_completion != null) maxPrice.completion = prefs.max_price_completion;
    provider.max_price = maxPrice;
  }

  if (prefs.quantizations) provider.quantizations = prefs.quantizations;

  return { provider };
}

export async function buildProviderConfig(
  settings: GolishSettings,
  workspace: string,
  overrides?: { provider?: AiProvider | null; model?: string | null }
): Promise<ProviderConfig> {
  const default_provider = overrides?.provider ?? settings.ai.default_provider;
  const default_model = overrides?.model ?? settings.ai.default_model;
  const model_override = resolveProviderOverride(settings, default_provider, default_model);

  switch (default_provider) {
    case "vertex_ai": {
      const { vertex_ai } = settings.ai;
      if (!vertex_ai.project_id) {
        throw new Error("Vertex AI project_id is required");
      }
      return {
        provider: "vertex_ai",
        workspace,
        credentials_path: vertex_ai.credentials_path || undefined,
        project_id: vertex_ai.project_id,
        location: vertex_ai.location || "us-east5",
        model: default_model,
        model_override,
      };
    }

    case "vertex_gemini": {
      const { vertex_gemini } = settings.ai;
      if (!vertex_gemini.project_id) {
        throw new Error("Vertex Gemini project_id is required");
      }
      return {
        provider: "vertex_gemini",
        workspace,
        credentials_path: vertex_gemini.credentials_path || undefined,
        project_id: vertex_gemini.project_id,
        location: vertex_gemini.location || "us-central1",
        model: default_model,
        model_override,
      };
    }

    case "anthropic": {
      const apiKey = settings.ai.anthropic.api_key || (await getAnthropicApiKey());
      if (!apiKey) throw new Error("Anthropic API key not configured");
      return {
        provider: "anthropic",
        workspace,
        model: default_model,
        api_key: apiKey,
        model_override,
      };
    }

    case "openai": {
      const apiKey = settings.ai.openai.api_key || (await getOpenAiApiKey());
      if (!apiKey) throw new Error("OpenAI API key not configured");
      return {
        provider: "openai",
        workspace,
        model: default_model,
        api_key: apiKey,
        model_override,
      };
    }

    case "openrouter": {
      const { getOpenRouterApiKey } = await import("./session");
      const apiKey = settings.ai.openrouter.api_key || (await getOpenRouterApiKey());
      if (!apiKey) throw new Error("OpenRouter API key not configured");
      const prefs = settings.ai.openrouter.provider_preferences;
      const providerPreferences = prefs ? buildOpenRouterProviderPreferencesJson(prefs) : undefined;
      return {
        provider: "openrouter",
        workspace,
        model: default_model,
        api_key: apiKey,
        ...(providerPreferences && { provider_preferences: providerPreferences }),
        model_override,
      };
    }

    case "ollama": {
      const baseUrl = settings.ai.ollama.base_url;
      return {
        provider: "ollama",
        workspace,
        model: default_model,
        base_url: baseUrl,
        model_override,
      };
    }

    case "gemini": {
      const apiKey = settings.ai.gemini.api_key;
      if (!apiKey) throw new Error("Gemini API key not configured");
      return {
        provider: "gemini",
        workspace,
        model: default_model,
        api_key: apiKey,
        model_override,
      };
    }

    case "groq": {
      const apiKey = settings.ai.groq.api_key;
      if (!apiKey) throw new Error("Groq API key not configured");
      return {
        provider: "groq",
        workspace,
        model: default_model,
        api_key: apiKey,
        model_override,
      };
    }

    case "xai": {
      const apiKey = settings.ai.xai.api_key;
      if (!apiKey) throw new Error("xAI API key not configured");
      return {
        provider: "xai",
        workspace,
        model: default_model,
        api_key: apiKey,
        model_override,
      };
    }

    case "zai_sdk": {
      const apiKey = settings.ai.zai_sdk?.api_key;
      if (!apiKey) throw new Error("Z.AI SDK API key not configured");
      return {
        provider: "zai_sdk",
        workspace,
        model: default_model,
        api_key: apiKey,
        base_url: settings.ai.zai_sdk?.base_url || undefined,
        model_override,
      };
    }

    case "nvidia": {
      const apiKey = settings.ai.nvidia?.api_key;
      if (!apiKey) throw new Error("NVIDIA API key not configured");
      return {
        provider: "nvidia",
        workspace,
        model: default_model,
        api_key: apiKey,
        base_url: settings.ai.nvidia?.base_url || undefined,
        model_override,
      };
    }

    case "deepseek": {
      const apiKey = settings.ai.deepseek?.api_key;
      if (!apiKey) throw new Error("DeepSeek API key not configured");
      return {
        provider: "deepseek",
        workspace,
        model: default_model,
        api_key: apiKey,
        base_url: settings.ai.deepseek?.base_url || undefined,
      };
    }

    case "xiaomi": {
      const apiKey = settings.ai.xiaomi?.api_key;
      if (!apiKey) throw new Error("Xiaomi MiMo API key not configured");
      return {
        provider: "xiaomi",
        workspace,
        model: default_model,
        api_key: apiKey,
        region: settings.ai.xiaomi?.region || undefined,
        default_protocol: settings.ai.xiaomi?.default_protocol || undefined,
        base_url: settings.ai.xiaomi?.openai_base_url || undefined,
        anthropic_base_url: settings.ai.xiaomi?.anthropic_base_url || undefined,
        model_override,
      };
    }

    default:
      throw new Error(`Unknown provider: ${default_provider}`);
  }
}
