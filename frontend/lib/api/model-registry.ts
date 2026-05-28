/**
 * Frontend API for the model registry.
 *
 * Provides typed wrappers for fetching model definitions and capabilities
 * from the backend model registry.
 */

import { invoke } from "@/lib/api/client";

// ---------------------------------------------------------------------------
// Inlined mirrors of Rust types — used to live in `frontend/lib/generated/`
// (ts-rs output). After the M2.5 codegen removal these are hand-maintained;
// keep in sync manually with their Rust sources whenever the structs change.
// ---------------------------------------------------------------------------

/**
 * AI provider selection.
 *
 * Mirror of `backend/crates/golish-core/src/types.rs::AiProvider`.
 */
export type AiProvider =
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
  | "deepseek"
  | "xiaomi";

/**
 * Capabilities that vary across LLM models.
 *
 * Mirror of `backend/crates/golish-models/src/capabilities.rs::ModelCapabilities`.
 * Provides explicit metadata about what a model supports — replaces the
 * runtime string-matching heuristics that used to live in the frontend.
 */
export interface ModelCapabilities {
  /** Whether the model supports the temperature parameter. */
  supports_temperature: boolean;
  /** Whether thinking/reasoning should be tracked in message history. */
  supports_thinking_history: boolean;
  /** Whether the model supports image/vision inputs. */
  supports_vision: boolean;
  /** Whether the model supports native web search tools. */
  supports_web_search: boolean;
  /** Whether this is a reasoning model (uses OpenAI reasoning client). */
  is_reasoning_model: boolean;
  /** Whether this is a coding-optimized model (codex variants). */
  is_codex_model: boolean;
  /** Context window size in tokens. */
  context_window: number;
  /** Maximum output tokens. */
  max_output_tokens: number;
}

/**
 * Owned version of ModelDefinition for serialization to frontend.
 *
 * Mirror of `backend/crates/golish-models/src/registry.rs::OwnedModelDefinition`.
 * This is the primary type exposed via Tauri commands.
 */
export interface OwnedModelDefinition {
  id: string;
  display_name: string;
  provider: AiProvider;
  capabilities: ModelCapabilities;
}

/**
 * Provider metadata for UI display.
 * Note: This is manually defined because the Rust struct uses &'static str
 * which ts-rs doesn't export directly.
 */
export interface ProviderInfo {
  provider: AiProvider;
  name: string;
  icon: string;
  description: string;
}

/**
 * Get all available models, optionally filtered by provider.
 *
 * @param provider - Optional provider to filter by
 * @returns Array of model definitions
 */
export async function getAvailableModels(provider?: AiProvider): Promise<OwnedModelDefinition[]> {
  return invoke("get_available_models", { provider: provider ?? null });
}

/**
 * Get a specific model by its ID.
 *
 * @param modelId - The model ID to look up
 * @returns The model definition, or null if not found
 */
export async function getModelById(modelId: string): Promise<OwnedModelDefinition | null> {
  return invoke("get_model_by_id", { modelId });
}

/**
 * Get capabilities for a specific model.
 *
 * This returns capabilities even for unknown models by using
 * provider-specific defaults.
 *
 * @param provider - The AI provider
 * @param modelId - The model ID
 * @returns The model's capabilities
 */
export async function getModelCapabilities(
  provider: AiProvider,
  modelId: string
): Promise<ModelCapabilities> {
  return invoke("get_model_capabilities_command", { provider, modelId });
}

/**
 * Get information about all available providers.
 *
 * @returns Array of provider information for UI display
 */
export async function getProviders(): Promise<ProviderInfo[]> {
  return invoke("get_providers");
}

/**
 * Get models grouped by provider.
 *
 * @returns Map of provider to their models
 */
export async function getModelsGroupedByProvider(): Promise<
  Map<AiProvider, OwnedModelDefinition[]>
> {
  const models = await getAvailableModels();
  const grouped = new Map<AiProvider, OwnedModelDefinition[]>();

  for (const model of models) {
    const existing = grouped.get(model.provider) ?? [];
    existing.push(model);
    grouped.set(model.provider, existing);
  }

  return grouped;
}

/**
 * Check if a model supports a specific capability.
 *
 * @param capabilities - The model's capabilities
 * @param capability - The capability to check
 * @returns Whether the capability is supported
 */
export function hasCapability(
  capabilities: ModelCapabilities,
  capability: keyof ModelCapabilities
): boolean {
  const value = capabilities[capability];
  if (typeof value === "boolean") {
    return value;
  }
  if (typeof value === "number") {
    return value > 0;
  }
  return false;
}

/**
 * Get provider display name from provider ID.
 *
 * @param providers - Array of provider info (from getProviders())
 * @param provider - The provider ID
 * @returns The display name, or the provider ID if not found
 */
export function getProviderDisplayName(providers: ProviderInfo[], provider: AiProvider): string {
  return providers.find((p) => p.provider === provider)?.name ?? provider;
}

/**
 * Get provider icon from provider ID.
 *
 * @param providers - Array of provider info (from getProviders())
 * @param provider - The provider ID
 * @returns The icon, or empty string if not found
 */
export function getProviderIcon(providers: ProviderInfo[], provider: AiProvider): string {
  return providers.find((p) => p.provider === provider)?.icon ?? "";
}
