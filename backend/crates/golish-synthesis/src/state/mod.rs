//! State synthesis: generate or update the running `state.md` for a session.
//!
//! `mod.rs` defines the input/output types, the [`StateSynthesizer`] trait,
//! and the [`create_state_synthesizer`] factory. Concrete backends live in
//! sibling modules:
//!
//! - [`template`] — rule-based default (no LLM required)
//! - [`openai`] — OpenAI-compatible chat completions
//! - [`grok`] — xAI Grok chat completions
//! - [`vertex`] — Anthropic Claude on Google Cloud Vertex AI

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::{SynthesisBackend, SynthesisConfig};
use crate::prompts::STATE_UPDATE_USER_PROMPT;

mod grok;
mod openai;
mod template;
mod vertex;

pub use grok::GrokStateSynthesizer;
pub use openai::OpenAiStateSynthesizer;
pub use template::TemplateStateSynthesizer;
pub use vertex::VertexAnthropicStateSynthesizer;

/// Input for state.md synthesis.
#[derive(Debug, Clone)]
pub struct StateSynthesisInput {
    /// Current state.md body content (empty string if new session).
    pub current_state: String,
    /// Type of the latest event (e.g., `"tool_call"`, `"ai_response"`, `"user_prompt"`).
    pub event_type: String,
    /// Content/details about the event.
    pub event_details: String,
    /// Files involved in the event.
    pub files: Vec<String>,
}

impl StateSynthesisInput {
    /// Create a new state synthesis input.
    pub fn new(
        current_state: String,
        event_type: String,
        event_details: String,
        files: Vec<String>,
    ) -> Self {
        Self {
            current_state,
            event_type,
            event_details,
            files,
        }
    }

    /// Build the user prompt for the LLM.
    pub fn build_prompt(&self) -> String {
        let files_str = if self.files.is_empty() {
            "(none)".to_string()
        } else {
            self.files.join(", ")
        };

        STATE_UPDATE_USER_PROMPT
            .replace("{current_state}", &self.current_state)
            .replace("{event_type}", &self.event_type)
            .replace("{event_details}", &self.event_details)
            .replace("{files}", &files_str)
    }
}

/// Result of state synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSynthesisResult {
    /// The updated state body (markdown without frontmatter).
    pub state_body: String,
    /// Which backend was used.
    pub backend: String,
}

/// Trait for state.md synthesis.
#[async_trait::async_trait]
pub trait StateSynthesizer: Send + Sync {
    /// Generate an updated state body from input.
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult>;

    /// Get the backend name (used in tests).
    #[allow(dead_code)]
    fn backend_name(&self) -> &'static str;
}

/// Create a state synthesizer based on configuration.
pub fn create_state_synthesizer(config: &SynthesisConfig) -> Result<Box<dyn StateSynthesizer>> {
    match config.backend {
        SynthesisBackend::Template => Ok(Box::new(TemplateStateSynthesizer::new())),
        SynthesisBackend::OpenAi => Ok(Box::new(OpenAiStateSynthesizer::new(&config.openai)?)),
        SynthesisBackend::Grok => Ok(Box::new(GrokStateSynthesizer::new(&config.grok)?)),
        SynthesisBackend::VertexAnthropic => Ok(Box::new(VertexAnthropicStateSynthesizer::new(
            &config.vertex,
        )?)),
    }
}
