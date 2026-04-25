import { XAI_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const XAI_PROVIDER_GROUP: ProviderGroup = {
  provider: "xai",
  providerName: "xAI",
  icon: "𝕏",
  models: [
    {
      id: XAI_MODELS.GROK_4_1_FAST_REASONING,
      name: "Grok 4.1 Fast (Reasoning)",
    },
    { id: XAI_MODELS.GROK_4_1_FAST_NON_REASONING, name: "Grok 4.1 Fast" },
    { id: XAI_MODELS.GROK_4_FAST_REASONING, name: "Grok 4 (Reasoning)" },
    { id: XAI_MODELS.GROK_4_FAST_NON_REASONING, name: "Grok 4" },
    { id: XAI_MODELS.GROK_CODE_FAST_1, name: "Grok Code" },
  ],
};

export const XAI_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "xai",
  providerName: "xAI",
  icon: "𝕏",
  models: [
    {
      name: "Grok 4 Series",
      subModels: [
        { id: XAI_MODELS.GROK_4_1_FAST_REASONING, name: "Grok 4.1 (Reasoning)" },
        { id: XAI_MODELS.GROK_4_1_FAST_NON_REASONING, name: "Grok 4.1" },
        { id: XAI_MODELS.GROK_4_FAST_REASONING, name: "Grok 4 (Reasoning)" },
        { id: XAI_MODELS.GROK_4_FAST_NON_REASONING, name: "Grok 4" },
      ],
    },
    { id: XAI_MODELS.GROK_CODE_FAST_1, name: "Grok Code" },
  ],
};
