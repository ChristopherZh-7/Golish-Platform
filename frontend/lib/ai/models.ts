export const VERTEX_AI_MODELS = {
  CLAUDE_OPUS_4_6: "claude-opus-4-6@default",
  CLAUDE_SONNET_4_6: "claude-sonnet-4-6@default",
  CLAUDE_OPUS_4_5: "claude-opus-4-5@20251101",
  CLAUDE_SONNET_4_5: "claude-sonnet-4-5@20250929",
  CLAUDE_HAIKU_4_5: "claude-haiku-4-5@20251001",
} as const;

export const VERTEX_GEMINI_MODELS = {
  GEMINI_3_PRO_PREVIEW: "gemini-3-pro-preview",
  GEMINI_3_FLASH_PREVIEW: "gemini-3-flash-preview",
  GEMINI_2_5_PRO: "gemini-2.5-pro",
  GEMINI_2_5_FLASH: "gemini-2.5-flash",
  GEMINI_2_5_FLASH_LITE: "gemini-2.5-flash-lite",
  GEMINI_2_0_FLASH: "gemini-2.0-flash",
  GEMINI_2_0_FLASH_LITE: "gemini-2.0-flash-lite",
} as const;

export const OPENAI_MODELS = {
  GPT_5_4: "gpt-5.4",
  GPT_5_2: "gpt-5.2",
  GPT_5_1: "gpt-5.1",
  GPT_5: "gpt-5",
  GPT_5_MINI: "gpt-5-mini",
  GPT_5_NANO: "gpt-5-nano",
  GPT_4_1: "gpt-4.1",
  GPT_4_1_MINI: "gpt-4.1-mini",
  GPT_4_1_NANO: "gpt-4.1-nano",
  GPT_4O: "gpt-4o",
  GPT_4O_MINI: "gpt-4o-mini",
  CHATGPT_4O_LATEST: "chatgpt-4o-latest",
  O4_MINI: "o4-mini",
  O3: "o3",
  O3_MINI: "o3-mini",
  O1: "o1",
  GPT_5_3_CODEX: "gpt-5.3-codex",
  GPT_5_2_CODEX: "gpt-5.2-codex",
  GPT_5_1_CODEX: "gpt-5.1-codex",
  GPT_5_1_CODEX_MAX: "gpt-5.1-codex-max",
  GPT_5_1_CODEX_MINI: "gpt-5.1-codex-mini",
} as const;

export const ANTHROPIC_MODELS = {
  CLAUDE_SONNET_4_6: "claude-sonnet-4-6-20260217",
  CLAUDE_OPUS_4_5: "claude-opus-4-5-20251101",
  CLAUDE_SONNET_4_5: "claude-sonnet-4-5-20250929",
  CLAUDE_HAIKU_4_5: "claude-haiku-4-5-20251001",
} as const;

export const OLLAMA_MODELS = {
  LLAMA_3_2: "llama3.2",
  LLAMA_3_1: "llama3.1",
  MISTRAL: "mistral",
  CODELLAMA: "codellama",
  QWEN_2_5: "qwen2.5",
} as const;

export const GEMINI_MODELS = {
  GEMINI_3_PRO_PREVIEW: "gemini-3-pro-preview",
  GEMINI_2_5_PRO: "gemini-2.5-pro",
  GEMINI_2_5_FLASH: "gemini-2.5-flash",
  GEMINI_2_5_FLASH_LITE: "gemini-2.5-flash-lite",
} as const;

export const GROQ_MODELS = {
  LLAMA_4_SCOUT: "meta-llama/llama-4-scout-17b-16e-instruct",
  LLAMA_4_MAVERICK: "meta-llama/llama-4-maverick-17b-128e-instruct",
  LLAMA_3_3_70B: "llama-3.3-70b-versatile",
  LLAMA_3_1_8B: "llama-3.1-8b-instant",
  GPT_OSS_120B: "openai/gpt-oss-120b",
  GPT_OSS_20B: "openai/gpt-oss-20b",
} as const;

export const XAI_MODELS = {
  GROK_4_1_FAST_REASONING: "grok-4-1-fast-reasoning",
  GROK_4_1_FAST_NON_REASONING: "grok-4-1-fast-non-reasoning",
  GROK_CODE_FAST_1: "grok-code-fast-1",
  GROK_4_FAST_REASONING: "grok-4-fast-reasoning",
  GROK_4_FAST_NON_REASONING: "grok-4-fast-non-reasoning",
} as const;

export const ZAI_SDK_MODELS = {
  GLM_5: "glm-5",
  GLM_4_7: "glm-4.7",
  GLM_4_6V: "glm-4.6v",
  GLM_4_5_AIR: "glm-4.5-air",
  GLM_4_FLASH: "glm-4-flash",
} as const;

export const NVIDIA_MODELS = {
  NEMOTRON_ULTRA_253B: "nvidia/llama-3.1-nemotron-ultra-253b-v1",
  NEMOTRON_3_SUPER_120B: "nvidia/nemotron-3-super-120b-a12b",
  NEMOTRON_SUPER_49B: "nvidia/llama-3.3-nemotron-super-49b-v1.5",
  NEMOTRON_3_NANO_30B: "nvidia/nemotron-3-nano-30b-a3b",
  NEMOTRON_NANO_12B_VL: "nvidia/nemotron-nano-12b-v2-vl",
  NEMOTRON_NANO_9B: "nvidia/nvidia-nemotron-nano-9b-v2",
  NEMOTRON_NANO_8B: "nvidia/llama-3.1-nemotron-nano-8b-v1",
  NEMOTRON_NANO_VL_8B: "nvidia/llama-3.1-nemotron-nano-vl-8b-v1",
  NEMOTRON_NANO_4B: "nvidia/llama-3.1-nemotron-nano-4b-v1.1",
  QWEN3_CODER_480B: "qwen/qwen3-coder-480b-a35b-instruct",
  QWEN3_5_397B: "qwen/qwen3.5-397b-a17b",
  QWEN3_5_122B: "qwen/qwen3.5-122b-a10b",
  QWEN3_NEXT_80B: "qwen/qwen3-next-80b-a3b-instruct",
  QWEN3_NEXT_80B_THINKING: "qwen/qwen3-next-80b-a3b-thinking",
  QWQ_32B: "qwen/qwq-32b",
  QWEN2_5_CODER_32B: "qwen/qwen2.5-coder-32b-instruct",
  QWEN2_5_CODER_7B: "qwen/qwen2.5-coder-7b-instruct",
  MISTRAL_LARGE_3: "mistralai/mistral-large-3-675b-instruct-2512",
  MISTRAL_SMALL_4: "mistralai/mistral-small-4-119b-2603",
  MISTRAL_MEDIUM_3: "mistralai/mistral-medium-3-instruct",
  MISTRAL_SMALL_3_1: "mistralai/mistral-small-3.1-24b-instruct-2503",
  MISTRAL_SMALL_24B: "mistralai/mistral-small-24b-instruct",
  MISTRAL_NEMOTRON: "mistralai/mistral-nemotron",
  MAGISTRAL_SMALL: "mistralai/magistral-small-2506",
  DEEPSEEK_V3_2: "deepseek-ai/deepseek-v3.2",
  DEEPSEEK_V3_1: "deepseek-ai/deepseek-v3.1",
  DEEPSEEK_R1_DISTILL_QWEN_32B: "deepseek-ai/deepseek-r1-distill-qwen-32b",
  DEEPSEEK_R1_DISTILL_LLAMA_8B: "deepseek-ai/deepseek-r1-distill-llama-8b",
  KIMI_K2_THINKING: "moonshotai/kimi-k2-thinking",
  KIMI_K2_INSTRUCT_0905: "moonshotai/kimi-k2-instruct-0905",
  KIMI_K2_INSTRUCT: "moonshotai/kimi-k2-instruct",
  GEMMA_4_31B: "google/gemma-4-31b-it",
  GEMMA_3_27B: "google/gemma-3-27b-it",
  GEMMA_3N_E2B: "google/gemma-3n-e2b-it",
  GEMMA_3_1B: "google/gemma-3-1b-it",
  PHI_4_MINI_FLASH: "microsoft/phi-4-mini-flash-reasoning",
  PHI_4_MULTIMODAL: "microsoft/phi-4-multimodal-instruct",
  LLAMA_4_MAVERICK_17B: "meta/llama-4-maverick-17b-128e-instruct",
  LLAMA_3_3_70B: "meta/llama-3.3-70b-instruct",
  LLAMA_3_1_405B: "meta/llama-3.1-405b-instruct",
  GPT_OSS_120B: "openai/gpt-oss-120b",
  GPT_OSS_20B: "openai/gpt-oss-20b",
  STEP_3_5_FLASH: "stepfun-ai/step-3.5-flash",
  MINIMAX_M2_5: "minimaxai/minimax-m2.5",
  MARIN_8B: "marin/marin-8b-instruct",
} as const;
