import { GROQ_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const GROQ_PROVIDER_GROUP: ProviderGroup = {
  provider: "groq",
  providerName: "Groq",
  icon: "⚡",
  models: [
    { id: GROQ_MODELS.LLAMA_4_SCOUT, name: "Llama 4 Scout 17B" },
    { id: GROQ_MODELS.LLAMA_4_MAVERICK, name: "Llama 4 Maverick 17B" },
    { id: GROQ_MODELS.LLAMA_3_3_70B, name: "Llama 3.3 70B" },
    { id: GROQ_MODELS.LLAMA_3_1_8B, name: "Llama 3.1 8B Instant" },
    { id: GROQ_MODELS.GPT_OSS_120B, name: "GPT OSS 120B" },
    { id: GROQ_MODELS.GPT_OSS_20B, name: "GPT OSS 20B" },
  ],
};

export const GROQ_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "groq",
  providerName: "Groq",
  icon: "⚡",
  models: [
    { id: GROQ_MODELS.LLAMA_4_SCOUT, name: "Llama 4 Scout 17B" },
    { id: GROQ_MODELS.LLAMA_4_MAVERICK, name: "Llama 4 Maverick 17B" },
    { id: GROQ_MODELS.LLAMA_3_3_70B, name: "Llama 3.3 70B" },
    { id: GROQ_MODELS.LLAMA_3_1_8B, name: "Llama 3.1 8B Instant" },
    { id: GROQ_MODELS.GPT_OSS_120B, name: "GPT OSS 120B" },
    { id: GROQ_MODELS.GPT_OSS_20B, name: "GPT OSS 20B" },
  ],
};
