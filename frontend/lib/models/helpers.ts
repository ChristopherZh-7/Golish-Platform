import type { ReasoningEffort } from "../ai";
import {
  type AiProvider as BackendAiProvider,
  getAvailableModels,
  getModelCapabilities,
  getProviders,
} from "../model-registry";
import type { AiProvider } from "../settings";
import { PROVIDER_GROUPS, PROVIDER_GROUPS_NESTED } from "./groups";
import type {
  ModelCapabilities,
  ModelInfo,
  OwnedModelDefinition,
  ProviderGroup,
  ProviderGroupNested,
  ProviderInfo,
} from "./types";

/**
 * Get a provider group by provider ID
 */
export function getProviderGroup(provider: AiProvider): ProviderGroup | undefined {
  return PROVIDER_GROUPS.find((g) => g.provider === provider);
}

/**
 * Get a nested provider group by provider ID
 */
export function getProviderGroupNested(provider: AiProvider): ProviderGroupNested | undefined {
  return PROVIDER_GROUPS_NESTED.find((g) => g.provider === provider);
}

/**
 * Get all models as a flat list
 */
export function getAllModels(): (ModelInfo & { provider: AiProvider })[] {
  return PROVIDER_GROUPS.flatMap((group) =>
    group.models.map((model) => ({ ...model, provider: group.provider }))
  );
}

/**
 * Find a model by ID across all providers
 */
export function findModelById(
  modelId: string,
  reasoningEffort?: ReasoningEffort
): (ModelInfo & { provider: AiProvider; providerName: string }) | undefined {
  for (const group of PROVIDER_GROUPS) {
    const model = group.models.find(
      (m) =>
        m.id === modelId && (reasoningEffort === undefined || m.reasoningEffort === reasoningEffort)
    );
    if (model) {
      return {
        ...model,
        provider: group.provider,
        providerName: group.providerName,
      };
    }
  }
  return undefined;
}

/**
 * Format a model ID to a display name
 */
export function formatModelName(modelId: string, reasoningEffort?: ReasoningEffort): string {
  if (!modelId) return "No Model";

  const model = findModelById(modelId, reasoningEffort);
  if (model) return model.name;

  // Fallback: try to find by ID only (for cases where reasoning effort doesn't match)
  const anyModel = findModelById(modelId);
  if (anyModel) {
    // For OpenAI, append reasoning effort if provided
    if (anyModel.provider === "openai" && reasoningEffort) {
      return `GPT 5.2 (${reasoningEffort.charAt(0).toUpperCase() + reasoningEffort.slice(1)})`;
    }
    return anyModel.name;
  }

  return modelId;
}

// =============================================================================
// Backend Model Registry Integration
// =============================================================================

/**
 * Fetch all models from the backend registry.
 * Returns models grouped by provider in the ProviderGroup format.
 */
export async function fetchProviderGroups(): Promise<ProviderGroup[]> {
  const [backendModels, backendProviders] = await Promise.all([
    getAvailableModels(),
    getProviders(),
  ]);

  // Create a map of provider info for quick lookup
  const providerInfoMap = new Map<string, ProviderInfo>();
  for (const p of backendProviders) {
    providerInfoMap.set(p.provider, p);
  }

  // Group models by provider
  const grouped = new Map<string, OwnedModelDefinition[]>();
  for (const model of backendModels) {
    const existing = grouped.get(model.provider) ?? [];
    existing.push(model);
    grouped.set(model.provider, existing);
  }

  // Convert to ProviderGroup format
  const groups: ProviderGroup[] = [];
  for (const [provider, models] of grouped) {
    const info = providerInfoMap.get(provider);
    if (!info) continue;

    groups.push({
      provider: provider as AiProvider,
      providerName: info.name,
      icon: info.icon,
      models: models.map((m) => ({
        id: m.id,
        name: m.display_name,
      })),
    });
  }

  // Sort alphabetically by provider name
  groups.sort((a, b) => a.providerName.localeCompare(b.providerName));

  return groups;
}

/**
 * Fetch models for a specific provider from the backend.
 */
export async function fetchModelsForProvider(provider: AiProvider): Promise<ModelInfo[]> {
  const models = await getAvailableModels(provider as BackendAiProvider);
  return models.map((m) => ({
    id: m.id,
    name: m.display_name,
  }));
}

/**
 * Get model capabilities from the backend.
 */
export async function fetchModelCapabilities(
  provider: AiProvider,
  modelId: string
): Promise<ModelCapabilities> {
  return getModelCapabilities(provider as BackendAiProvider, modelId);
}

/**
 * Check if a model supports a specific capability.
 */
export function modelSupports(
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
 * Convert backend OwnedModelDefinition to frontend ModelInfo.
 */
export function toModelInfo(model: OwnedModelDefinition): ModelInfo & { provider: AiProvider } {
  return {
    id: model.id,
    name: model.display_name,
    provider: model.provider as AiProvider,
  };
}

/**
 * Fetch provider info from the backend.
 * Useful for getting display names and icons.
 */
export async function fetchProviderInfo(): Promise<ProviderInfo[]> {
  return getProviders();
}
