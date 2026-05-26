import { NVIDIA_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const NVIDIA_PROVIDER_GROUP: ProviderGroup = {
  provider: "nvidia",
  providerName: "NVIDIA NIM",
  icon: "🟢",
  models: [
    { id: NVIDIA_MODELS.NEMOTRON_ULTRA_253B, name: "Nemotron Ultra 253B" },
    { id: NVIDIA_MODELS.QWEN3_CODER_480B, name: "Qwen3 Coder 480B" },
    { id: NVIDIA_MODELS.QWEN3_5_397B, name: "Qwen 3.5 397B" },
    { id: NVIDIA_MODELS.QWEN3_5_122B, name: "Qwen 3.5 122B" },
    { id: NVIDIA_MODELS.QWEN3_NEXT_80B, name: "Qwen3 Next 80B" },
    { id: NVIDIA_MODELS.MISTRAL_LARGE_3, name: "Mistral Large 3 675B" },
    { id: NVIDIA_MODELS.MISTRAL_MEDIUM_3_5, name: "Mistral Medium 3.5 128B" },
    { id: NVIDIA_MODELS.MISTRAL_NEMOTRON, name: "Mistral Nemotron" },
    { id: NVIDIA_MODELS.DEEPSEEK_V4_FLASH, name: "DeepSeek V4 Flash" },
    { id: NVIDIA_MODELS.DEEPSEEK_V4_PRO, name: "DeepSeek V4 Pro" },
    { id: NVIDIA_MODELS.KIMI_K2_6, name: "Kimi K2.6" },
    { id: NVIDIA_MODELS.GLM_5_1_NIM, name: "GLM 5.1" },
    { id: NVIDIA_MODELS.MINIMAX_M2_7, name: "MiniMax M2.7" },
    { id: NVIDIA_MODELS.STEP_3_5_FLASH, name: "Step 3.5 Flash" },
  ],
};

export const NVIDIA_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "nvidia",
  providerName: "NVIDIA NIM",
  icon: "🟢",
  models: [
    { id: NVIDIA_MODELS.NEMOTRON_ULTRA_253B, name: "Nemotron Ultra 253B" },
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
        { id: NVIDIA_MODELS.MISTRAL_MEDIUM_3_5, name: "Medium 3.5 128B" },
        { id: NVIDIA_MODELS.MISTRAL_NEMOTRON, name: "Nemotron" },
      ],
    },
    {
      name: "DeepSeek",
      subModels: [
        { id: NVIDIA_MODELS.DEEPSEEK_V4_FLASH, name: "V4 Flash" },
        { id: NVIDIA_MODELS.DEEPSEEK_V4_PRO, name: "V4 Pro" },
      ],
    },
    { id: NVIDIA_MODELS.KIMI_K2_6, name: "Kimi K2.6" },
    { id: NVIDIA_MODELS.GLM_5_1_NIM, name: "GLM 5.1" },
    { id: NVIDIA_MODELS.MINIMAX_M2_7, name: "MiniMax M2.7" },
    { id: NVIDIA_MODELS.STEP_3_5_FLASH, name: "Step 3.5 Flash" },
  ],
};
