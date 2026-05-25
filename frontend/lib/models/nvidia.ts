import { NVIDIA_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const NVIDIA_PROVIDER_GROUP: ProviderGroup = {
  provider: "nvidia",
  providerName: "NVIDIA NIM",
  icon: "🟢",
  models: [
    { id: NVIDIA_MODELS.NEMOTRON_3_SUPER_120B, name: "Nemotron 3 Super 120B" },
    { id: NVIDIA_MODELS.NEMOTRON_3_NANO_30B, name: "Nemotron 3 Nano 30B" },
    { id: NVIDIA_MODELS.NEMOTRON_3_NANO_OMNI, name: "Nemotron 3 Nano Omni Reasoning" },
    { id: NVIDIA_MODELS.NEMOTRON_SUPER_49B, name: "Nemotron Super 49B" },
    { id: NVIDIA_MODELS.NEMOTRON_ULTRA_253B, name: "Nemotron Ultra 253B" },
    { id: NVIDIA_MODELS.NEMOTRON_NANO_9B, name: "Nemotron Nano 9B" },
    { id: NVIDIA_MODELS.NEMOTRON_NANO_4B, name: "Nemotron Nano 4B" },
    { id: NVIDIA_MODELS.QWEN3_CODER_480B, name: "Qwen3 Coder 480B" },
    { id: NVIDIA_MODELS.QWEN3_5_397B, name: "Qwen 3.5 397B" },
    { id: NVIDIA_MODELS.QWEN3_5_122B, name: "Qwen 3.5 122B" },
    { id: NVIDIA_MODELS.QWEN3_NEXT_80B, name: "Qwen3 Next 80B" },
    { id: NVIDIA_MODELS.MISTRAL_LARGE_3, name: "Mistral Large 3 675B" },
    { id: NVIDIA_MODELS.MISTRAL_SMALL_4, name: "Mistral Small 4 119B" },
    { id: NVIDIA_MODELS.MISTRAL_MEDIUM_3_5, name: "Mistral Medium 3.5 128B" },
    { id: NVIDIA_MODELS.DEVSTRAL_2_123B, name: "Devstral 2 123B" },
    { id: NVIDIA_MODELS.MISTRAL_NEMOTRON, name: "Mistral Nemotron" },
    { id: NVIDIA_MODELS.MAGISTRAL_SMALL, name: "Magistral Small" },
    { id: NVIDIA_MODELS.DEEPSEEK_V4_FLASH, name: "DeepSeek V4 Flash" },
    { id: NVIDIA_MODELS.DEEPSEEK_V4_PRO, name: "DeepSeek V4 Pro" },
    { id: NVIDIA_MODELS.DEEPSEEK_V3_1_TERMINUS, name: "DeepSeek V3.1 Terminus" },
    { id: NVIDIA_MODELS.KIMI_K2_6, name: "Kimi K2.6" },
    { id: NVIDIA_MODELS.GLM_5_1_NIM, name: "GLM 5.1" },
    { id: NVIDIA_MODELS.GLM_4_7_NIM, name: "GLM 4.7" },
    { id: NVIDIA_MODELS.MINIMAX_M2_7, name: "MiniMax M2.7" },
    { id: NVIDIA_MODELS.GEMMA_4_31B, name: "Gemma 4 31B" },
    { id: NVIDIA_MODELS.LLAMA_4_MAVERICK_17B, name: "Llama 4 Maverick 17B" },
    { id: NVIDIA_MODELS.LLAMA_3_1_405B, name: "Llama 3.1 405B" },
    { id: NVIDIA_MODELS.STEP_3_5_FLASH, name: "Step 3.5 Flash" },
  ],
};

export const NVIDIA_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "nvidia",
  providerName: "NVIDIA NIM",
  icon: "🟢",
  models: [
    {
      name: "NVIDIA Nemotron",
      subModels: [
        { id: NVIDIA_MODELS.NEMOTRON_ULTRA_253B, name: "Ultra 253B" },
        { id: NVIDIA_MODELS.NEMOTRON_3_SUPER_120B, name: "3 Super 120B" },
        { id: NVIDIA_MODELS.NEMOTRON_SUPER_49B, name: "Super 49B" },
        { id: NVIDIA_MODELS.NEMOTRON_3_NANO_30B, name: "3 Nano 30B" },
        { id: NVIDIA_MODELS.NEMOTRON_3_NANO_OMNI, name: "3 Nano Omni Reasoning" },
        { id: NVIDIA_MODELS.NEMOTRON_NANO_9B, name: "Nano 9B" },
        { id: NVIDIA_MODELS.NEMOTRON_NANO_4B, name: "Nano 4B" },
      ],
    },
    {
      name: "Qwen",
      subModels: [
        { id: NVIDIA_MODELS.QWEN3_CODER_480B, name: "Qwen3 Coder 480B" },
        { id: NVIDIA_MODELS.QWEN3_5_397B, name: "Qwen 3.5 397B" },
        { id: NVIDIA_MODELS.QWEN3_5_122B, name: "Qwen 3.5 122B" },
        { id: NVIDIA_MODELS.QWEN3_NEXT_80B, name: "Qwen3 Next 80B" },
      ],
    },
    {
      name: "Mistral",
      subModels: [
        { id: NVIDIA_MODELS.MISTRAL_LARGE_3, name: "Large 3 675B" },
        { id: NVIDIA_MODELS.MISTRAL_SMALL_4, name: "Small 4 119B" },
        { id: NVIDIA_MODELS.MISTRAL_MEDIUM_3_5, name: "Medium 3.5 128B" },
        { id: NVIDIA_MODELS.DEVSTRAL_2_123B, name: "Devstral 2 123B" },
        { id: NVIDIA_MODELS.MISTRAL_NEMOTRON, name: "Nemotron" },
        { id: NVIDIA_MODELS.MAGISTRAL_SMALL, name: "Magistral Small" },
      ],
    },
    {
      name: "DeepSeek",
      subModels: [
        { id: NVIDIA_MODELS.DEEPSEEK_V4_FLASH, name: "V4 Flash" },
        { id: NVIDIA_MODELS.DEEPSEEK_V4_PRO, name: "V4 Pro" },
        { id: NVIDIA_MODELS.DEEPSEEK_V3_1_TERMINUS, name: "V3.1 Terminus" },
      ],
    },
    {
      name: "Moonshot Kimi",
      subModels: [
        { id: NVIDIA_MODELS.KIMI_K2_6, name: "K2.6" },
        { id: NVIDIA_MODELS.KIMI_K2_INSTRUCT_0905, name: "K2 Instruct 0905" },
      ],
    },
    {
      name: "Z.AI GLM",
      subModels: [
        { id: NVIDIA_MODELS.GLM_5_1_NIM, name: "GLM 5.1" },
        { id: NVIDIA_MODELS.GLM_4_7_NIM, name: "GLM 4.7" },
      ],
    },
    {
      name: "Meta Llama",
      subModels: [
        { id: NVIDIA_MODELS.LLAMA_3_1_405B, name: "3.1 405B" },
        { id: NVIDIA_MODELS.LLAMA_4_MAVERICK_17B, name: "4 Maverick 17B" },
      ],
    },
    { id: NVIDIA_MODELS.GEMMA_4_31B, name: "Gemma 4 31B" },
    { id: NVIDIA_MODELS.STEP_3_5_FLASH, name: "Step 3.5 Flash" },
    { id: NVIDIA_MODELS.MINIMAX_M2_7, name: "MiniMax M2.7" },
  ],
};
