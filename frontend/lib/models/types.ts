import type { ReasoningEffort } from "../ai";
import type {
  ModelCapabilities,
  OwnedModelDefinition,
  ProviderInfo,
} from "../model-registry";
import type { AiProvider } from "../settings";

export interface ModelInfo {
  id: string;
  name: string;
  reasoningEffort?: ReasoningEffort;
}

/**
 * A model entry that can either be a simple model or a group with sub-options.
 * Supports recursive nesting (e.g., "GPT-5 Series" → "GPT 5.2" → Low/Medium/High).
 */
export interface ModelEntry {
  /** Display name for the model or group */
  name: string;
  /** Model ID (for leaf models) */
  id?: string;
  /** Reasoning effort (for leaf models with reasoning) */
  reasoningEffort?: ReasoningEffort;
  /** Sub-options (supports recursive nesting) */
  subModels?: ModelEntry[];
}

export interface ProviderGroup {
  provider: AiProvider;
  providerName: string;
  icon: string;
  models: ModelInfo[];
}

/**
 * Provider group with nested model entries for sub-menus.
 */
export interface ProviderGroupNested {
  provider: AiProvider;
  providerName: string;
  icon: string;
  models: ModelEntry[];
}

export type { ModelCapabilities, OwnedModelDefinition, ProviderInfo };
