import { ANTHROPIC_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const ANTHROPIC_PROVIDER_GROUP: ProviderGroup = {
  provider: "anthropic",
  providerName: "Anthropic",
  icon: "🔶",
  models: [
    { id: ANTHROPIC_MODELS.CLAUDE_SONNET_4_6, name: "Claude Sonnet 4.6" },
    { id: ANTHROPIC_MODELS.CLAUDE_OPUS_4_5, name: "Claude Opus 4.5" },
    { id: ANTHROPIC_MODELS.CLAUDE_SONNET_4_5, name: "Claude Sonnet 4.5" },
    { id: ANTHROPIC_MODELS.CLAUDE_HAIKU_4_5, name: "Claude Haiku 4.5" },
  ],
};

export const ANTHROPIC_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "anthropic",
  providerName: "Anthropic",
  icon: "🔶",
  models: [
    { id: ANTHROPIC_MODELS.CLAUDE_SONNET_4_6, name: "Claude Sonnet 4.6" },
    { id: ANTHROPIC_MODELS.CLAUDE_OPUS_4_5, name: "Claude Opus 4.5" },
    { id: ANTHROPIC_MODELS.CLAUDE_SONNET_4_5, name: "Claude Sonnet 4.5" },
    { id: ANTHROPIC_MODELS.CLAUDE_HAIKU_4_5, name: "Claude Haiku 4.5" },
  ],
};
