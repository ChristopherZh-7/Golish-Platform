import { XIAOMI_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

/**
 * Xiaomi MiMo models.
 *
 * The wire protocol (OpenAI Chat Completions vs Anthropic Messages) is not a
 * per-model choice — it's a provider-wide setting (`Settings → Xiaomi MiMo →
 * Default protocol`). Model id `@anthropic` / `@openai` suffixes are still
 * honored by the backend `resolve_protocol` helper for power users who want
 * to override per request via `settings.default_model`, but they are not
 * surfaced in the picker to avoid confusing end users.
 */
export const XIAOMI_PROVIDER_GROUP: ProviderGroup = {
  provider: "xiaomi",
  providerName: "Xiaomi MiMo",
  icon: "🟠",
  models: [
    { id: XIAOMI_MODELS.MIMO_V2_5_PRO, name: "MiMo V2.5 Pro" },
    { id: XIAOMI_MODELS.MIMO_V2_5, name: "MiMo V2.5 (multimodal)" },
    { id: XIAOMI_MODELS.MIMO_V2_PRO, name: "MiMo V2 Pro" },
    { id: XIAOMI_MODELS.MIMO_V2_OMNI, name: "MiMo V2 Omni (multimodal)" },
  ],
};

export const XIAOMI_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "xiaomi",
  providerName: "Xiaomi MiMo",
  icon: "🟠",
  models: [
    { id: XIAOMI_MODELS.MIMO_V2_5_PRO, name: "MiMo V2.5 Pro" },
    { id: XIAOMI_MODELS.MIMO_V2_5, name: "MiMo V2.5 (multimodal)" },
    { id: XIAOMI_MODELS.MIMO_V2_PRO, name: "MiMo V2 Pro" },
    { id: XIAOMI_MODELS.MIMO_V2_OMNI, name: "MiMo V2 Omni (multimodal)" },
  ],
};
