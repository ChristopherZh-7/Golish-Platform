import { VERTEX_AI_MODELS, VERTEX_GEMINI_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const VERTEX_AI_PROVIDER_GROUP: ProviderGroup = {
  provider: "vertex_ai",
  providerName: "Vertex AI",
  icon: "🔷",
  models: [
    { id: VERTEX_AI_MODELS.CLAUDE_OPUS_4_6, name: "Claude Opus 4.6" },
    { id: VERTEX_AI_MODELS.CLAUDE_SONNET_4_6, name: "Claude Sonnet 4.6" },
    { id: VERTEX_AI_MODELS.CLAUDE_OPUS_4_5, name: "Claude Opus 4.5" },
    { id: VERTEX_AI_MODELS.CLAUDE_SONNET_4_5, name: "Claude Sonnet 4.5" },
    { id: VERTEX_AI_MODELS.CLAUDE_HAIKU_4_5, name: "Claude Haiku 4.5" },
  ],
};

export const VERTEX_GEMINI_PROVIDER_GROUP: ProviderGroup = {
  provider: "vertex_gemini",
  providerName: "Vertex AI Gemini",
  icon: "💎",
  models: [
    { id: VERTEX_GEMINI_MODELS.GEMINI_3_PRO_PREVIEW, name: "Gemini 3 Pro (Preview)" },
    { id: VERTEX_GEMINI_MODELS.GEMINI_3_FLASH_PREVIEW, name: "Gemini 3 Flash (Preview)" },
    { id: VERTEX_GEMINI_MODELS.GEMINI_2_5_PRO, name: "Gemini 2.5 Pro" },
    { id: VERTEX_GEMINI_MODELS.GEMINI_2_5_FLASH, name: "Gemini 2.5 Flash" },
    { id: VERTEX_GEMINI_MODELS.GEMINI_2_5_FLASH_LITE, name: "Gemini 2.5 Flash Lite" },
    { id: VERTEX_GEMINI_MODELS.GEMINI_2_0_FLASH, name: "Gemini 2.0 Flash" },
    { id: VERTEX_GEMINI_MODELS.GEMINI_2_0_FLASH_LITE, name: "Gemini 2.0 Flash Lite" },
  ],
};

export const VERTEX_AI_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "vertex_ai",
  providerName: "Vertex AI",
  icon: "🔷",
  models: [
    { id: VERTEX_AI_MODELS.CLAUDE_OPUS_4_6, name: "Claude Opus 4.6" },
    { id: VERTEX_AI_MODELS.CLAUDE_SONNET_4_6, name: "Claude Sonnet 4.6" },
    { id: VERTEX_AI_MODELS.CLAUDE_OPUS_4_5, name: "Claude Opus 4.5" },
    { id: VERTEX_AI_MODELS.CLAUDE_SONNET_4_5, name: "Claude Sonnet 4.5" },
    { id: VERTEX_AI_MODELS.CLAUDE_HAIKU_4_5, name: "Claude Haiku 4.5" },
  ],
};

export const VERTEX_GEMINI_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "vertex_gemini",
  providerName: "Vertex AI Gemini",
  icon: "💎",
  models: [
    {
      name: "Gemini 3 (Preview)",
      subModels: [
        { id: VERTEX_GEMINI_MODELS.GEMINI_3_PRO_PREVIEW, name: "Pro" },
        { id: VERTEX_GEMINI_MODELS.GEMINI_3_FLASH_PREVIEW, name: "Flash" },
      ],
    },
    {
      name: "Gemini 2.5",
      subModels: [
        { id: VERTEX_GEMINI_MODELS.GEMINI_2_5_PRO, name: "Pro" },
        { id: VERTEX_GEMINI_MODELS.GEMINI_2_5_FLASH, name: "Flash" },
        { id: VERTEX_GEMINI_MODELS.GEMINI_2_5_FLASH_LITE, name: "Flash Lite" },
      ],
    },
    {
      name: "Gemini 2.0",
      subModels: [
        { id: VERTEX_GEMINI_MODELS.GEMINI_2_0_FLASH, name: "Flash" },
        { id: VERTEX_GEMINI_MODELS.GEMINI_2_0_FLASH_LITE, name: "Flash Lite" },
      ],
    },
  ],
};
