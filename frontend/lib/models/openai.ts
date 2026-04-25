import { OPENAI_MODELS } from "../ai";
import type { ProviderGroup, ProviderGroupNested } from "./types";

export const OPENAI_PROVIDER_GROUP: ProviderGroup = {
  provider: "openai",
  providerName: "OpenAI",
  icon: "⚪",
  models: [
    // GPT-5 series (with reasoning effort variants)
    {
      id: OPENAI_MODELS.GPT_5_4,
      name: "GPT 5.4 (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.GPT_5_4,
      name: "GPT 5.4 (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5_4,
      name: "GPT 5.4 (High)",
      reasoningEffort: "high",
    },
    {
      id: OPENAI_MODELS.GPT_5_4,
      name: "GPT 5.4 (Extra High)",
      reasoningEffort: "extra_high",
    },
    {
      id: OPENAI_MODELS.GPT_5_2,
      name: "GPT 5.2 (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.GPT_5_2,
      name: "GPT 5.2 (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5_2,
      name: "GPT 5.2 (High)",
      reasoningEffort: "high",
    },
    {
      id: OPENAI_MODELS.GPT_5_2,
      name: "GPT 5.2 (Extra High)",
      reasoningEffort: "extra_high",
    },
    {
      id: OPENAI_MODELS.GPT_5_1,
      name: "GPT 5.1 (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.GPT_5_1,
      name: "GPT 5.1 (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5_1,
      name: "GPT 5.1 (High)",
      reasoningEffort: "high",
    },
    { id: OPENAI_MODELS.GPT_5, name: "GPT 5 (Low)", reasoningEffort: "low" },
    {
      id: OPENAI_MODELS.GPT_5,
      name: "GPT 5 (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5,
      name: "GPT 5 (High)",
      reasoningEffort: "high",
    },
    { id: OPENAI_MODELS.GPT_5_MINI, name: "GPT 5 Mini" },
    { id: OPENAI_MODELS.GPT_5_NANO, name: "GPT 5 Nano" },
    // GPT-4.1 series
    { id: OPENAI_MODELS.GPT_4_1, name: "GPT 4.1" },
    { id: OPENAI_MODELS.GPT_4_1_MINI, name: "GPT 4.1 Mini" },
    { id: OPENAI_MODELS.GPT_4_1_NANO, name: "GPT 4.1 Nano" },
    // GPT-4o series
    { id: OPENAI_MODELS.GPT_4O, name: "GPT 4o" },
    { id: OPENAI_MODELS.GPT_4O_MINI, name: "GPT 4o Mini" },
    { id: OPENAI_MODELS.CHATGPT_4O_LATEST, name: "ChatGPT 4o Latest" },
    // o-series reasoning models (with reasoning effort variants)
    {
      id: OPENAI_MODELS.O4_MINI,
      name: "o4 Mini (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.O4_MINI,
      name: "o4 Mini (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.O4_MINI,
      name: "o4 Mini (High)",
      reasoningEffort: "high",
    },
    { id: OPENAI_MODELS.O3, name: "o3 (Low)", reasoningEffort: "low" },
    { id: OPENAI_MODELS.O3, name: "o3 (Medium)", reasoningEffort: "medium" },
    { id: OPENAI_MODELS.O3, name: "o3 (High)", reasoningEffort: "high" },
    {
      id: OPENAI_MODELS.O3_MINI,
      name: "o3 Mini (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.O3_MINI,
      name: "o3 Mini (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.O3_MINI,
      name: "o3 Mini (High)",
      reasoningEffort: "high",
    },
    { id: OPENAI_MODELS.O1, name: "o1 (Low)", reasoningEffort: "low" },
    { id: OPENAI_MODELS.O1, name: "o1 (Medium)", reasoningEffort: "medium" },
    { id: OPENAI_MODELS.O1, name: "o1 (High)", reasoningEffort: "high" },
    // Codex models (coding-optimized)
    {
      id: OPENAI_MODELS.GPT_5_3_CODEX,
      name: "GPT 5.3 Codex (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.GPT_5_3_CODEX,
      name: "GPT 5.3 Codex (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5_3_CODEX,
      name: "GPT 5.3 Codex (High)",
      reasoningEffort: "high",
    },
    {
      id: OPENAI_MODELS.GPT_5_3_CODEX,
      name: "GPT 5.3 Codex (Extra High)",
      reasoningEffort: "extra_high",
    },
    {
      id: OPENAI_MODELS.GPT_5_2_CODEX,
      name: "GPT 5.2 Codex (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.GPT_5_2_CODEX,
      name: "GPT 5.2 Codex (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5_2_CODEX,
      name: "GPT 5.2 Codex (High)",
      reasoningEffort: "high",
    },
    {
      id: OPENAI_MODELS.GPT_5_2_CODEX,
      name: "GPT 5.2 Codex (Extra High)",
      reasoningEffort: "extra_high",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX,
      name: "GPT 5.1 Codex (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX,
      name: "GPT 5.1 Codex (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX,
      name: "GPT 5.1 Codex (High)",
      reasoningEffort: "high",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX,
      name: "GPT 5.1 Codex (Extra High)",
      reasoningEffort: "extra_high",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX_MAX,
      name: "GPT 5.1 Codex Max (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX_MAX,
      name: "GPT 5.1 Codex Max (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX_MAX,
      name: "GPT 5.1 Codex Max (High)",
      reasoningEffort: "high",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX_MAX,
      name: "GPT 5.1 Codex Max (Extra High)",
      reasoningEffort: "extra_high",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX_MINI,
      name: "GPT 5.1 Codex Mini (Low)",
      reasoningEffort: "low",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX_MINI,
      name: "GPT 5.1 Codex Mini (Medium)",
      reasoningEffort: "medium",
    },
    {
      id: OPENAI_MODELS.GPT_5_1_CODEX_MINI,
      name: "GPT 5.1 Codex Mini (High)",
      reasoningEffort: "high",
    },
  ],
};

export const OPENAI_PROVIDER_GROUP_NESTED: ProviderGroupNested = {
  provider: "openai",
  providerName: "OpenAI",
  icon: "⚪",
  models: [
    // GPT-5 series grouped with 3-level nesting for reasoning effort
    {
      name: "GPT-5 Series",
      subModels: [
        {
          name: "GPT 5.4",
          subModels: [
            { id: OPENAI_MODELS.GPT_5_4, name: "Low", reasoningEffort: "low" },
            { id: OPENAI_MODELS.GPT_5_4, name: "Medium", reasoningEffort: "medium" },
            { id: OPENAI_MODELS.GPT_5_4, name: "High", reasoningEffort: "high" },
            { id: OPENAI_MODELS.GPT_5_4, name: "Extra High", reasoningEffort: "extra_high" },
          ],
        },
        {
          name: "GPT 5.2",
          subModels: [
            {
              id: OPENAI_MODELS.GPT_5_2,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.GPT_5_2,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.GPT_5_2,
              name: "High",
              reasoningEffort: "high",
            },
            {
              id: OPENAI_MODELS.GPT_5_2,
              name: "Extra High",
              reasoningEffort: "extra_high",
            },
          ],
        },
        {
          name: "GPT 5.1",
          subModels: [
            {
              id: OPENAI_MODELS.GPT_5_1,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.GPT_5_1,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.GPT_5_1,
              name: "High",
              reasoningEffort: "high",
            },
          ],
        },
        {
          name: "GPT 5",
          subModels: [
            { id: OPENAI_MODELS.GPT_5, name: "Low", reasoningEffort: "low" },
            {
              id: OPENAI_MODELS.GPT_5,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.GPT_5,
              name: "High",
              reasoningEffort: "high",
            },
          ],
        },
        { id: OPENAI_MODELS.GPT_5_MINI, name: "GPT 5 Mini" },
        { id: OPENAI_MODELS.GPT_5_NANO, name: "GPT 5 Nano" },
      ],
    },
    // GPT-4 series grouped (no reasoning effort needed)
    {
      name: "GPT-4 Series",
      subModels: [
        { id: OPENAI_MODELS.GPT_4_1, name: "GPT 4.1" },
        { id: OPENAI_MODELS.GPT_4_1_MINI, name: "GPT 4.1 Mini" },
        { id: OPENAI_MODELS.GPT_4_1_NANO, name: "GPT 4.1 Nano" },
        { id: OPENAI_MODELS.GPT_4O, name: "GPT 4o" },
        { id: OPENAI_MODELS.GPT_4O_MINI, name: "GPT 4o Mini" },
        { id: OPENAI_MODELS.CHATGPT_4O_LATEST, name: "ChatGPT 4o Latest" },
      ],
    },
    // o-series reasoning models with 3-level nesting
    {
      name: "o-Series",
      subModels: [
        {
          name: "o4 Mini",
          subModels: [
            {
              id: OPENAI_MODELS.O4_MINI,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.O4_MINI,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.O4_MINI,
              name: "High",
              reasoningEffort: "high",
            },
          ],
        },
        {
          name: "o3",
          subModels: [
            { id: OPENAI_MODELS.O3, name: "Low", reasoningEffort: "low" },
            {
              id: OPENAI_MODELS.O3,
              name: "Medium",
              reasoningEffort: "medium",
            },
            { id: OPENAI_MODELS.O3, name: "High", reasoningEffort: "high" },
          ],
        },
        {
          name: "o3 Mini",
          subModels: [
            {
              id: OPENAI_MODELS.O3_MINI,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.O3_MINI,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.O3_MINI,
              name: "High",
              reasoningEffort: "high",
            },
          ],
        },
        {
          name: "o1",
          subModels: [
            { id: OPENAI_MODELS.O1, name: "Low", reasoningEffort: "low" },
            {
              id: OPENAI_MODELS.O1,
              name: "Medium",
              reasoningEffort: "medium",
            },
            { id: OPENAI_MODELS.O1, name: "High", reasoningEffort: "high" },
          ],
        },
      ],
    },
    // Codex models grouped
    {
      name: "Codex",
      subModels: [
        {
          name: "GPT 5.3 Codex",
          subModels: [
            {
              id: OPENAI_MODELS.GPT_5_3_CODEX,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.GPT_5_3_CODEX,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.GPT_5_3_CODEX,
              name: "High",
              reasoningEffort: "high",
            },
            {
              id: OPENAI_MODELS.GPT_5_3_CODEX,
              name: "Extra High",
              reasoningEffort: "extra_high",
            },
          ],
        },
        {
          name: "GPT 5.2 Codex",
          subModels: [
            {
              id: OPENAI_MODELS.GPT_5_2_CODEX,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.GPT_5_2_CODEX,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.GPT_5_2_CODEX,
              name: "High",
              reasoningEffort: "high",
            },
            {
              id: OPENAI_MODELS.GPT_5_2_CODEX,
              name: "Extra High",
              reasoningEffort: "extra_high",
            },
          ],
        },
        {
          name: "GPT 5.1 Codex",
          subModels: [
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX,
              name: "High",
              reasoningEffort: "high",
            },
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX,
              name: "Extra High",
              reasoningEffort: "extra_high",
            },
          ],
        },
        {
          name: "GPT 5.1 Codex Max",
          subModels: [
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX_MAX,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX_MAX,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX_MAX,
              name: "High",
              reasoningEffort: "high",
            },
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX_MAX,
              name: "Extra High",
              reasoningEffort: "extra_high",
            },
          ],
        },
        {
          name: "GPT 5.1 Codex Mini",
          subModels: [
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX_MINI,
              name: "Low",
              reasoningEffort: "low",
            },
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX_MINI,
              name: "Medium",
              reasoningEffort: "medium",
            },
            {
              id: OPENAI_MODELS.GPT_5_1_CODEX_MINI,
              name: "High",
              reasoningEffort: "high",
            },
          ],
        },
      ],
    },
  ],
};
