import type { ProviderGroup, ProviderGroupNested } from "./types";

export const OPENROUTER_PROVIDER_GROUP: ProviderGroup = {
  provider: "openrouter",
  providerName: "OpenRouter",
  icon: "🔀",
  models: [
    { id: "mistralai/devstral-2512", name: "Devstral 2512" },
    { id: "deepseek/deepseek-v3.2", name: "Deepseek v3.2" },
    { id: "z-ai/glm-4.6", name: "GLM 4.6" },
    { id: "x-ai/grok-code-fast-1", name: "Grok Code Fast 1" },
    { id: "openai/gpt-oss-20b", name: "GPT OSS 20B" },
    { id: "openai/gpt-oss-120b", name: "GPT OSS 120B" },
    { id: "openai/gpt-5.2", name: "GPT 5.2" },
  ],
};

export const OPENROUTER_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "openrouter",
  providerName: "OpenRouter",
  icon: "🔀",
  models: [
    { id: "mistralai/devstral-2512", name: "Devstral 2512" },
    { id: "deepseek/deepseek-v3.2", name: "Deepseek v3.2" },
    { id: "z-ai/glm-4.6", name: "GLM 4.6" },
    { id: "x-ai/grok-code-fast-1", name: "Grok Code Fast 1" },
    { id: "openai/gpt-oss-20b", name: "GPT OSS 20B" },
    { id: "openai/gpt-oss-120b", name: "GPT OSS 120B" },
    { id: "openai/gpt-5.2", name: "GPT 5.2" },
  ],
};
