import { DEEPSEEK_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const DEEPSEEK_PROVIDER_GROUP: ProviderGroup = {
  provider: "deepseek",
  providerName: "DeepSeek",
  icon: "🧠",
  models: [
    { id: DEEPSEEK_MODELS.DEEPSEEK_V4_FLASH, name: "DeepSeek V4 Flash" },
    { id: DEEPSEEK_MODELS.DEEPSEEK_V4_PRO, name: "DeepSeek V4 Pro" },
  ],
};

export const DEEPSEEK_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "deepseek",
  providerName: "DeepSeek",
  icon: "🧠",
  models: [
    {
      name: "DeepSeek V4",
      subModels: [
        { id: DEEPSEEK_MODELS.DEEPSEEK_V4_FLASH, name: "Flash" },
        { id: DEEPSEEK_MODELS.DEEPSEEK_V4_PRO, name: "Pro" },
      ],
    },
  ],
};
