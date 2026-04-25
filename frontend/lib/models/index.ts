/**
 * Shared model configuration - single source of truth for all model selectors.
 *
 * This barrel re-exports types, per-provider definitions, the assembled provider
 * groups, and helper/fetch functions, preserving the original public surface of
 * `frontend/lib/models.ts`.
 */

export * from "./anthropic";
export * from "./gemini";
export * from "./groq";
export * from "./groups";
export * from "./helpers";
export * from "./nvidia";
export * from "./ollama";
export * from "./openai";
export * from "./openrouter";
export * from "./types";
export * from "./vertex";
export * from "./xai";
export * from "./zai";
