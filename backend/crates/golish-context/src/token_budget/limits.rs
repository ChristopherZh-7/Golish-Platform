//! Per-model context limits + Default.

use serde::{Deserialize, Serialize};


/// Model-specific context window sizes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelContextLimits {
    // Claude models
    pub claude_3_5_sonnet: usize,
    pub claude_3_opus: usize,
    pub claude_3_haiku: usize,
    pub claude_4_sonnet: usize,
    pub claude_4_opus: usize,
    pub claude_4_6_opus: usize,
    pub claude_4_6_sonnet: usize,
    pub claude_4_5_opus: usize,
    pub claude_4_5_sonnet: usize,
    pub claude_4_5_haiku: usize,
    // OpenAI models
    pub gpt_4o: usize,
    pub gpt_4_turbo: usize,
    pub gpt_4_1: usize,
    pub gpt_5_1: usize,
    pub gpt_5_2: usize,
    pub codex: usize,
    pub o1: usize,
    pub o3: usize,
    // Google models
    pub gemini_pro: usize,
    pub gemini_flash: usize,
}

impl Default for ModelContextLimits {
    fn default() -> Self {
        Self {
            // Claude models: 200k context
            claude_3_5_sonnet: 200_000,
            claude_3_opus: 200_000,
            claude_3_haiku: 200_000,
            claude_4_sonnet: 200_000,
            claude_4_opus: 200_000,
            claude_4_6_opus: 1_000_000,
            claude_4_6_sonnet: 1_000_000,
            claude_4_5_opus: 200_000,
            claude_4_5_sonnet: 200_000,
            claude_4_5_haiku: 200_000,
            // OpenAI models
            gpt_4o: 128_000,
            gpt_4_turbo: 128_000,
            gpt_4_1: 1_047_576, // GPT-4.1 has ~1M context
            gpt_5_1: 400_000,   // GPT-5.x has 400k context
            gpt_5_2: 400_000,   // GPT-5.x has 400k context
            codex: 192_000,     // Codex has 192k context
            o1: 200_000,
            o3: 200_000,
            // Google models: 1M context
            gemini_pro: 1_000_000,
            gemini_flash: 1_000_000,
        }
    }
}
