import type { ProviderConfig } from "@/lib/ai";
import type { GolishSettings } from "@/lib/settings";

/**
 * Build a [`ProviderConfig`] from the user's saved settings for a given
 * `(provider, model)` pair.
 *
 * Returns `null` when the provider name does not match a known provider so
 * the caller can decide how to fail (e.g. surface a "no model selected"
 * error to the user).
 */
export function buildProviderConfig(
  provider: string,
  model: string,
  workspace: string,
  settings: GolishSettings
): ProviderConfig | null {
  switch (provider) {
    case "anthropic":
      return {
        provider: "anthropic",
        workspace,
        model,
        api_key: settings.ai.anthropic?.api_key || "",
      };
    case "openai":
      return {
        provider: "openai",
        workspace,
        model,
        api_key: settings.ai.openai?.api_key || "",
      };
    case "openrouter":
      return {
        provider: "openrouter",
        workspace,
        model,
        api_key: settings.ai.openrouter?.api_key || "",
      };
    case "gemini":
      return {
        provider: "gemini",
        workspace,
        model,
        api_key: settings.ai.gemini?.api_key || "",
      };
    case "groq":
      return {
        provider: "groq",
        workspace,
        model,
        api_key: settings.ai.groq?.api_key || "",
      };
    case "xai":
      return {
        provider: "xai",
        workspace,
        model,
        api_key: settings.ai.xai?.api_key || "",
      };
    case "zai_sdk":
      return {
        provider: "zai_sdk",
        workspace,
        model,
        api_key: settings.ai.zai_sdk?.api_key || "",
      };
    case "nvidia":
      return {
        provider: "nvidia",
        workspace,
        model,
        api_key: settings.ai.nvidia?.api_key || "",
      };
    case "vertex_ai":
      return {
        provider: "vertex_ai",
        workspace,
        model,
        credentials_path: settings.ai.vertex_ai?.credentials_path ?? "",
        project_id: settings.ai.vertex_ai?.project_id ?? "",
        location: settings.ai.vertex_ai?.location ?? "us-east5",
      };
    case "vertex_gemini":
      return {
        provider: "vertex_gemini",
        workspace,
        model,
        credentials_path: settings.ai.vertex_gemini?.credentials_path ?? "",
        project_id: settings.ai.vertex_gemini?.project_id ?? "",
        location: settings.ai.vertex_gemini?.location ?? "us-east5",
      };
    case "ollama":
      return { provider: "ollama", workspace, model };
    default:
      return null;
  }
}

/**
 * Derive the set of providers that the user has *configured* (API key set
 * or, for vertex providers, credentials supplied).  Used to filter the
 * model picker to providers the user can actually invoke.
 *
 * `ollama` is always considered configured (it talks to a local daemon
 * that doesn't require credentials in the settings file).
 */
export function getConfiguredProviders(settings: GolishSettings): Set<string> {
  const configured = new Set<string>();
  const ai = settings.ai;
  if (ai.anthropic?.api_key) configured.add("anthropic");
  if (ai.openai?.api_key) configured.add("openai");
  if (ai.openrouter?.api_key) configured.add("openrouter");
  if (ai.gemini?.api_key) configured.add("gemini");
  if (ai.groq?.api_key) configured.add("groq");
  if (ai.xai?.api_key) configured.add("xai");
  if (ai.zai_sdk?.api_key) configured.add("zai_sdk");
  if (ai.nvidia?.api_key) configured.add("nvidia");
  if (ai.vertex_ai?.credentials_path || ai.vertex_ai?.project_id) {
    configured.add("vertex_ai");
  }
  if (ai.vertex_gemini?.credentials_path || ai.vertex_gemini?.project_id) {
    configured.add("vertex_gemini");
  }
  configured.add("ollama");
  return configured;
}
