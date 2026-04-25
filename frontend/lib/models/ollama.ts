import { OLLAMA_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const OLLAMA_PROVIDER_GROUP: ProviderGroup = {
  provider: "ollama",
  providerName: "Ollama",
  icon: "🦙",
  models: [
    { id: OLLAMA_MODELS.LLAMA_3_2, name: "Llama 3.2" },
    { id: OLLAMA_MODELS.LLAMA_3_1, name: "Llama 3.1" },
    { id: OLLAMA_MODELS.QWEN_2_5, name: "Qwen 2.5" },
    { id: OLLAMA_MODELS.MISTRAL, name: "Mistral" },
    { id: OLLAMA_MODELS.CODELLAMA, name: "CodeLlama" },
  ],
};

export const OLLAMA_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "ollama",
  providerName: "Ollama",
  icon: "🦙",
  models: [
    { id: OLLAMA_MODELS.LLAMA_3_2, name: "Llama 3.2" },
    { id: OLLAMA_MODELS.LLAMA_3_1, name: "Llama 3.1" },
    { id: OLLAMA_MODELS.QWEN_2_5, name: "Qwen 2.5" },
    { id: OLLAMA_MODELS.MISTRAL, name: "Mistral" },
    { id: OLLAMA_MODELS.CODELLAMA, name: "CodeLlama" },
  ],
};
