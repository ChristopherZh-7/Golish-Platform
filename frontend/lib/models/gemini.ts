import { GEMINI_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const GEMINI_PROVIDER_GROUP: ProviderGroup = {
  provider: "gemini",
  providerName: "Gemini",
  icon: "💎",
  models: [
    { id: GEMINI_MODELS.GEMINI_3_PRO_PREVIEW, name: "Gemini 3 Pro Preview" },
    { id: GEMINI_MODELS.GEMINI_2_5_PRO, name: "Gemini 2.5 Pro" },
    { id: GEMINI_MODELS.GEMINI_2_5_FLASH, name: "Gemini 2.5 Flash" },
    {
      id: GEMINI_MODELS.GEMINI_2_5_FLASH_LITE,
      name: "Gemini 2.5 Flash Lite",
    },
  ],
};

export const GEMINI_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "gemini",
  providerName: "Gemini",
  icon: "💎",
  models: [
    { id: GEMINI_MODELS.GEMINI_3_PRO_PREVIEW, name: "Gemini 3 Pro Preview" },
    { id: GEMINI_MODELS.GEMINI_2_5_PRO, name: "Gemini 2.5 Pro" },
    { id: GEMINI_MODELS.GEMINI_2_5_FLASH, name: "Gemini 2.5 Flash" },
    {
      id: GEMINI_MODELS.GEMINI_2_5_FLASH_LITE,
      name: "Gemini 2.5 Flash Lite",
    },
  ],
};
