/**
 * AUTO-GENERATED FROM `resources/llm-models/<provider>.json` AND
 * `frontend/scripts/model-const-keys.json`. DO NOT EDIT BY HAND.
 *
 * Regenerate with:
 *   node frontend/scripts/generate-model-constants.mjs
 *
 * See `docs/design/2026-05-25-llm-models-json-driven.md` for the rationale.
 */

export const ANTHROPIC_MODELS = {
  CLAUDE_SONNET_4_6: "claude-sonnet-4-6-20260217",
  CLAUDE_OPUS_4_5: "claude-opus-4-5-20251101",
  CLAUDE_SONNET_4_5: "claude-sonnet-4-5-20250929",
  CLAUDE_HAIKU_4_5: "claude-haiku-4-5-20251001",
} as const;

export const DEEPSEEK_MODELS = {
  DEEPSEEK_V4_FLASH: "deepseek-v4-flash",
  DEEPSEEK_V4_PRO: "deepseek-v4-pro",
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

export const NVIDIA_MODELS = {
  NEMOTRON_ULTRA_253B: "nvidia/llama-3.1-nemotron-ultra-253b-v1",
  QWEN3_CODER_480B: "qwen/qwen3-coder-480b-a35b-instruct",
  QWEN3_5_397B: "qwen/qwen3.5-397b-a17b",
  QWEN3_5_122B: "qwen/qwen3.5-122b-a10b",
  QWEN3_NEXT_80B: "qwen/qwen3-next-80b-a3b-instruct",
  MISTRAL_LARGE_3: "mistralai/mistral-large-3-675b-instruct-2512",
  MISTRAL_MEDIUM_3_5: "mistralai/mistral-medium-3.5-128b",
  MISTRAL_NEMOTRON: "mistralai/mistral-nemotron",
  DEEPSEEK_V4_FLASH: "deepseek-ai/deepseek-v4-flash",
  DEEPSEEK_V4_PRO: "deepseek-ai/deepseek-v4-pro",
  KIMI_K2_6: "moonshotai/kimi-k2.6",
  GLM_5_1_NIM: "z-ai/glm-5.1",
  MINIMAX_M2_7: "minimaxai/minimax-m2.7",
  STEP_3_5_FLASH: "stepfun-ai/step-3.5-flash",
} as const;

export const OLLAMA_MODELS = {
  LLAMA_3_2: "llama3.2",
  LLAMA_3_1: "llama3.1",
  MISTRAL: "mistral",
  CODELLAMA: "codellama",
  QWEN_2_5: "qwen2.5",
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

export const XAI_MODELS = {
  GROK_4_1_FAST_REASONING: "grok-4-1-fast-reasoning",
  GROK_4_1_FAST_NON_REASONING: "grok-4-1-fast-non-reasoning",
  GROK_CODE_FAST_1: "grok-code-fast-1",
  GROK_4_FAST_REASONING: "grok-4-fast-reasoning",
  GROK_4_FAST_NON_REASONING: "grok-4-fast-non-reasoning",
} as const;

export const XIAOMI_MODELS = {
  MIMO_V2_5_PRO: "mimo-v2.5-pro",
  MIMO_V2_5: "mimo-v2.5",
  MIMO_V2_PRO: "mimo-v2-pro",
  MIMO_V2_OMNI: "mimo-v2-omni",
} as const;

export const ZAI_SDK_MODELS = {
  GLM_5: "glm-5",
  GLM_4_7: "glm-4.7",
  GLM_4_6V: "glm-4.6v",
  GLM_4_5_AIR: "glm-4.5-air",
  GLM_4_FLASH: "glm-4-flash",
} as const;
