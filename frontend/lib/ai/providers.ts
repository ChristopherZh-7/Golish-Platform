import { invoke } from "@tauri-apps/api/core";
import type { GolishSettings, OpenRouterProviderPreferences } from "../settings";
import type {
  AiProvider,
  OpenAiConfig,
  ProviderConfig,
  VertexAiConfig,
  VertexAiEnvConfig,
} from "./types";
import { ANTHROPIC_MODELS, GEMINI_MODELS, GROQ_MODELS, OLLAMA_MODELS, VERTEX_AI_MODELS, XAI_MODELS } from "./models";

export async function getVertexAiConfig(): Promise<VertexAiEnvConfig> {
  return invoke("get_vertex_ai_config");
}

export async function initClaudeOpus(workspace: string, apiKey: string): Promise<void> {
  return invoke("init_ai_agent", {
    workspace,
    provider: "openrouter",
    model: "anthropic/claude-opus-4.5",
    apiKey,
  });
}

export async function initVertexAiAgent(config: VertexAiConfig): Promise<void> {
  return invoke("init_ai_agent_vertex", {
    workspace: config.workspace,
    credentialsPath: config.credentialsPath,
    projectId: config.projectId,
    location: config.location,
    model: config.model,
  });
}

export async function initVertexClaudeOpus(
  workspace: string,
  credentialsPath: string,
  projectId: string,
  location: string = "us-east5"
): Promise<void> {
  return initVertexAiAgent({
    workspace,
    credentialsPath,
    projectId,
    location,
    model: VERTEX_AI_MODELS.CLAUDE_OPUS_4_5,
  });
}

export async function getOpenAiApiKey(): Promise<string | null> {
  return invoke("get_openai_api_key");
}

export async function initOpenAiAgent(config: OpenAiConfig): Promise<void> {
  return invoke("init_ai_agent_openai", {
    workspace: config.workspace,
    model: config.model,
    apiKey: config.apiKey,
    baseUrl: config.baseUrl,
    reasoningEffort: config.reasoningEffort,
  });
}

export async function initAiAgentUnified(config: ProviderConfig): Promise<void> {
  return invoke("init_ai_agent_unified", { config });
}

export async function initWithAnthropic(
  workspace: string,
  apiKey: string,
  model: string = ANTHROPIC_MODELS.CLAUDE_SONNET_4_5
): Promise<void> {
  return initAiAgentUnified({ provider: "anthropic", workspace, model, api_key: apiKey });
}

export async function initWithOllama(
  workspace: string,
  model: string = OLLAMA_MODELS.LLAMA_3_2,
  baseUrl?: string
): Promise<void> {
  return initAiAgentUnified({ provider: "ollama", workspace, model, base_url: baseUrl });
}

export async function initWithGemini(
  workspace: string,
  apiKey: string,
  model: string = GEMINI_MODELS.GEMINI_2_5_FLASH
): Promise<void> {
  return initAiAgentUnified({ provider: "gemini", workspace, model, api_key: apiKey });
}

export async function initWithGroq(
  workspace: string,
  apiKey: string,
  model: string = GROQ_MODELS.LLAMA_4_SCOUT
): Promise<void> {
  return initAiAgentUnified({ provider: "groq", workspace, model, api_key: apiKey });
}

export async function initWithXai(
  workspace: string,
  apiKey: string,
  model: string = XAI_MODELS.GROK_4_1_FAST_REASONING
): Promise<void> {
  return initAiAgentUnified({ provider: "xai", workspace, model, api_key: apiKey });
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

  switch (default_provider) {
    case "vertex_ai": {
      const { vertex_ai } = settings.ai;
      if (!vertex_ai.project_id) {
        throw new Error("Vertex AI project_id is required");
      }
      return {
        provider: "vertex_ai",
        workspace,
        credentials_path: vertex_ai.credentials_path || null,
        project_id: vertex_ai.project_id,
        location: vertex_ai.location || "us-east5",
        model: default_model,
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
        credentials_path: vertex_gemini.credentials_path || null,
        project_id: vertex_gemini.project_id,
        location: vertex_gemini.location || "us-central1",
        model: default_model,
      };
    }

    case "anthropic": {
      const apiKey = settings.ai.anthropic.api_key || (await getAnthropicApiKey());
      if (!apiKey) throw new Error("Anthropic API key not configured");
      return { provider: "anthropic", workspace, model: default_model, api_key: apiKey };
    }

    case "openai": {
      const apiKey = settings.ai.openai.api_key || (await getOpenAiApiKey());
      if (!apiKey) throw new Error("OpenAI API key not configured");
      return { provider: "openai", workspace, model: default_model, api_key: apiKey };
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
      };
    }

    case "ollama": {
      const baseUrl = settings.ai.ollama.base_url;
      return { provider: "ollama", workspace, model: default_model, base_url: baseUrl };
    }

    case "gemini": {
      const apiKey = settings.ai.gemini.api_key;
      if (!apiKey) throw new Error("Gemini API key not configured");
      return { provider: "gemini", workspace, model: default_model, api_key: apiKey };
    }

    case "groq": {
      const apiKey = settings.ai.groq.api_key;
      if (!apiKey) throw new Error("Groq API key not configured");
      return { provider: "groq", workspace, model: default_model, api_key: apiKey };
    }

    case "xai": {
      const apiKey = settings.ai.xai.api_key;
      if (!apiKey) throw new Error("xAI API key not configured");
      return { provider: "xai", workspace, model: default_model, api_key: apiKey };
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
      };
    }

    default:
      throw new Error(`Unknown provider: ${default_provider}`);
  }
}
