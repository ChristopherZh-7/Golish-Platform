import { ZAI_SDK_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const ZAI_SDK_PROVIDER_GROUP: ProviderGroup = {
  provider: "zai_sdk",
  providerName: "Z.AI SDK",
  icon: "🤖",
  models: [
    { id: ZAI_SDK_MODELS.GLM_5, name: "GLM 5" },
    { id: ZAI_SDK_MODELS.GLM_4_7, name: "GLM 4.7" },
    { id: ZAI_SDK_MODELS.GLM_4_6V, name: "GLM 4.6v" },
    { id: ZAI_SDK_MODELS.GLM_4_5_AIR, name: "GLM 4.5 Air" },
    { id: ZAI_SDK_MODELS.GLM_4_FLASH, name: "GLM 4 Flash" },
  ],
};

export const ZAI_SDK_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "zai_sdk",
  providerName: "Z.AI SDK",
  icon: "🤖",
  models: [
    { id: ZAI_SDK_MODELS.GLM_5, name: "GLM 5" },
    { id: ZAI_SDK_MODELS.GLM_4_7, name: "GLM 4.7" },
    { id: ZAI_SDK_MODELS.GLM_4_6V, name: "GLM 4.6v" },
    { id: ZAI_SDK_MODELS.GLM_4_5_AIR, name: "GLM 4.5 Air" },
    { id: ZAI_SDK_MODELS.GLM_4_FLASH, name: "GLM 4 Flash" },
  ],
};
