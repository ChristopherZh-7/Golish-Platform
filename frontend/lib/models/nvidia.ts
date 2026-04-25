import { NVIDIA_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const NVIDIA_PROVIDER_GROUP: ProviderGroup = {
  provider: "nvidia",
  providerName: "NVIDIA NIM",
  icon: "🟢",
  models: [
    { id: NVIDIA_MODELS.NEMOTRON_3_SUPER_120B, name: "Nemotron 3 Super 120B" },
    { id: NVIDIA_MODELS.NEMOTRON_3_NANO_30B, name: "Nemotron 3 Nano 30B" },
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
    { id: NVIDIA_MODELS.MISTRAL_SMALL_3_1, name: "Mistral Small 3.1 24B" },
    { id: NVIDIA_MODELS.MISTRAL_NEMOTRON, name: "Mistral Nemotron" },
    { id: NVIDIA_MODELS.MAGISTRAL_SMALL, name: "Magistral Small" },
    { id: NVIDIA_MODELS.DEEPSEEK_V3_2, name: "DeepSeek V3.2" },
    { id: NVIDIA_MODELS.KIMI_K2_THINKING, name: "Kimi K2 Thinking" },
    { id: NVIDIA_MODELS.GEMMA_4_31B, name: "Gemma 4 31B" },
    { id: NVIDIA_MODELS.PHI_4_MINI_FLASH, name: "Phi-4 Mini Flash" },
    { id: NVIDIA_MODELS.LLAMA_4_MAVERICK_17B, name: "Llama 4 Maverick 17B" },
    { id: NVIDIA_MODELS.LLAMA_3_1_405B, name: "Llama 3.1 405B" },
    { id: NVIDIA_MODELS.STEP_3_5_FLASH, name: "Step 3.5 Flash" },
    { id: NVIDIA_MODELS.MINIMAX_M2_5, name: "MiniMax M2.5" },
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
        { id: NVIDIA_MODELS.NEMOTRON_NANO_12B_VL, name: "Nano 12B VL" },
        { id: NVIDIA_MODELS.NEMOTRON_NANO_9B, name: "Nano 9B" },
        { id: NVIDIA_MODELS.NEMOTRON_NANO_8B, name: "Nano 8B" },
        { id: NVIDIA_MODELS.NEMOTRON_NANO_VL_8B, name: "Nano VL 8B" },
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
        { id: NVIDIA_MODELS.QWEN3_NEXT_80B_THINKING, name: "Qwen3 Next 80B Thinking" },
        { id: NVIDIA_MODELS.QWQ_32B, name: "QwQ 32B" },
        { id: NVIDIA_MODELS.QWEN2_5_CODER_32B, name: "Qwen 2.5 Coder 32B" },
        { id: NVIDIA_MODELS.QWEN2_5_CODER_7B, name: "Qwen 2.5 Coder 7B" },
      ],
    },
    {
      name: "Mistral",
      subModels: [
        { id: NVIDIA_MODELS.MISTRAL_LARGE_3, name: "Large 3 675B" },
        { id: NVIDIA_MODELS.MISTRAL_SMALL_4, name: "Small 4 119B" },
        { id: NVIDIA_MODELS.MISTRAL_MEDIUM_3, name: "Medium 3" },
        { id: NVIDIA_MODELS.MISTRAL_SMALL_3_1, name: "Small 3.1 24B" },
        { id: NVIDIA_MODELS.MISTRAL_SMALL_24B, name: "Small 24B" },
        { id: NVIDIA_MODELS.MISTRAL_NEMOTRON, name: "Nemotron" },
        { id: NVIDIA_MODELS.MAGISTRAL_SMALL, name: "Magistral Small" },
      ],
    },
    {
      name: "DeepSeek",
      subModels: [
        { id: NVIDIA_MODELS.DEEPSEEK_V3_2, name: "V3.2" },
        { id: NVIDIA_MODELS.DEEPSEEK_V3_1, name: "V3.1" },
        { id: NVIDIA_MODELS.DEEPSEEK_R1_DISTILL_QWEN_32B, name: "R1 Distill Qwen 32B" },
        { id: NVIDIA_MODELS.DEEPSEEK_R1_DISTILL_LLAMA_8B, name: "R1 Distill Llama 8B" },
      ],
    },
    {
      name: "Moonshot Kimi",
      subModels: [
        { id: NVIDIA_MODELS.KIMI_K2_THINKING, name: "K2 Thinking" },
        { id: NVIDIA_MODELS.KIMI_K2_INSTRUCT_0905, name: "K2 Instruct 0905" },
        { id: NVIDIA_MODELS.KIMI_K2_INSTRUCT, name: "K2 Instruct" },
      ],
    },
    {
      name: "Google Gemma",
      subModels: [
        { id: NVIDIA_MODELS.GEMMA_4_31B, name: "Gemma 4 31B" },
        { id: NVIDIA_MODELS.GEMMA_3_27B, name: "Gemma 3 27B" },
        { id: NVIDIA_MODELS.GEMMA_3N_E2B, name: "Gemma 3n E2B" },
        { id: NVIDIA_MODELS.GEMMA_3_1B, name: "Gemma 3 1B" },
      ],
    },
    {
      name: "Meta Llama",
      subModels: [
        { id: NVIDIA_MODELS.LLAMA_3_1_405B, name: "3.1 405B" },
        { id: NVIDIA_MODELS.LLAMA_3_3_70B, name: "3.3 70B" },
        { id: NVIDIA_MODELS.LLAMA_4_MAVERICK_17B, name: "4 Maverick 17B" },
      ],
    },
    {
      name: "Microsoft Phi",
      subModels: [
        { id: NVIDIA_MODELS.PHI_4_MULTIMODAL, name: "Phi-4 Multimodal" },
        { id: NVIDIA_MODELS.PHI_4_MINI_FLASH, name: "Phi-4 Mini Flash" },
      ],
    },
    {
      name: "OpenAI OSS",
      subModels: [
        { id: NVIDIA_MODELS.GPT_OSS_120B, name: "GPT-OSS 120B" },
        { id: NVIDIA_MODELS.GPT_OSS_20B, name: "GPT-OSS 20B" },
      ],
    },
    { id: NVIDIA_MODELS.STEP_3_5_FLASH, name: "Step 3.5 Flash" },
    { id: NVIDIA_MODELS.MINIMAX_M2_5, name: "MiniMax M2.5" },
    { id: NVIDIA_MODELS.MARIN_8B, name: "Marin 8B" },
  ],
};
