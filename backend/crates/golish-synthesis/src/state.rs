use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use gcp_auth::{CustomServiceAccount, TokenProvider};

use golish_settings::schema::{
    SynthesisGrokSettings, SynthesisOpenAiSettings, SynthesisVertexSettings,
};

use crate::config::{SynthesisBackend, SynthesisConfig};

const VERTEX_AI_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
use crate::prompts::*;

// =============================================================================

/// Input for state.md synthesis
#[derive(Debug, Clone)]
pub struct StateSynthesisInput {
    /// Current state.md body content (empty string if new session)
    pub current_state: String,
    /// Type of the latest event (e.g., "tool_call", "ai_response", "user_prompt")
    pub event_type: String,
    /// Content/details about the event
    pub event_details: String,
    /// Files involved in the event
    pub files: Vec<String>,
}

impl StateSynthesisInput {
    /// Create a new state synthesis input
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

    /// Build the user prompt for LLM
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

/// Result of state synthesis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSynthesisResult {
    /// The updated state body (markdown without frontmatter)
    pub state_body: String,
    /// Which backend was used
    pub backend: String,
}


// =============================================================================

/// Trait for state.md synthesis
#[async_trait::async_trait]
pub trait StateSynthesizer: Send + Sync {
    /// Generate an updated state body from input
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult>;

    /// Get the backend name (used in tests)
    #[allow(dead_code)]
    fn backend_name(&self) -> &'static str;
}

/// Template-based state synthesizer (rule-based updates, no LLM)
pub struct TemplateStateSynthesizer;

impl TemplateStateSynthesizer {
    pub fn new() -> Self {
        Self
    }

    /// Update the Changes section with new files
    fn update_changes_section(state: &str, files: &[String]) -> String {
        if files.is_empty() {
            return state.to_string();
        }

        // Format new file entries
        let new_entries: Vec<String> = files
            .iter()
            .map(|f| {
                // Extract just the filename if it contains diff info
                let path = f.lines().next().unwrap_or(f);
                format!("- `{}`", path)
            })
            .collect();

        // Check if we have a Changes section
        if let Some(changes_idx) = state.find("## Changes") {
            let (before, after_changes) = state.split_at(changes_idx);

            // Find the end of the Changes section (next ## or end of string)
            let changes_content_start = after_changes.find('\n').unwrap_or(after_changes.len());
            let rest = &after_changes[changes_content_start..];

            // Find the next section or end
            let next_section = rest.find("\n## ").map(|i| i + 1);
            let (changes_body, remainder) = match next_section {
                Some(idx) => rest.split_at(idx),
                None => (rest, ""),
            };

            // Check if changes body has "(none yet)"
            let updated_changes = if changes_body.contains("(none yet)") {
                // Replace "(none yet)" with actual changes
                new_entries.join("\n")
            } else {
                // Append to existing changes (deduplicate)
                let existing: std::collections::HashSet<_> = changes_body
                    .lines()
                    .filter(|l| l.starts_with("- "))
                    .collect();

                let mut all_changes: Vec<String> = changes_body
                    .lines()
                    .filter(|l| l.starts_with("- "))
                    .map(|s| s.to_string())
                    .collect();

                for entry in &new_entries {
                    if !existing.contains(entry.as_str()) {
                        all_changes.push(entry.clone());
                    }
                }

                all_changes.join("\n")
            };

            format!(
                "{}## Changes\n{}\n{}",
                before,
                updated_changes,
                remainder.trim_start()
            )
        } else {
            // No Changes section found, append one
            format!("{}\n## Changes\n{}\n", state, new_entries.join("\n"))
        }
    }
}

impl Default for TemplateStateSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl StateSynthesizer for TemplateStateSynthesizer {
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult> {
        // Update state with file changes if any
        let state_body = Self::update_changes_section(&input.current_state, &input.files);

        Ok(StateSynthesisResult {
            state_body,
            backend: "template".to_string(),
        })
    }

    fn backend_name(&self) -> &'static str {
        "template"
    }
}

/// OpenAI-based state synthesizer
pub struct OpenAiStateSynthesizer {
    api_key: String,
    model: String,
    base_url: Option<String>,
}

impl OpenAiStateSynthesizer {
    pub fn new(config: &SynthesisOpenAiSettings) -> Result<Self> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .context("OpenAI API key not configured")?;

        Ok(Self {
            api_key,
            model: config.model.clone(),
            base_url: config.base_url.clone(),
        })
    }
}

#[async_trait::async_trait]
impl StateSynthesizer for OpenAiStateSynthesizer {
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult> {
        let base_url = self
            .base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1");

        let client = reqwest::Client::new();

        let request_body = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": STATE_UPDATE_SYSTEM_PROMPT
                },
                {
                    "role": "user",
                    "content": input.build_prompt()
                }
            ],
            "max_tokens": 1500,
            "temperature": 0.3
        });

        let response = client
            .post(format!("{}/chat/completions", base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send state synthesis request to OpenAI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("OpenAI API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let state_body = response_body["choices"][0]["message"]["content"]
            .as_str()
            .context("Invalid response format from OpenAI")?
            .trim()
            .to_string();

        Ok(StateSynthesisResult {
            state_body,
            backend: "openai".to_string(),
        })
    }

    fn backend_name(&self) -> &'static str {
        "openai"
    }
}

/// Grok-based state synthesizer
pub struct GrokStateSynthesizer {
    api_key: String,
    model: String,
}

impl GrokStateSynthesizer {
    pub fn new(config: &SynthesisGrokSettings) -> Result<Self> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| std::env::var("GROK_API_KEY").ok())
            .or_else(|| std::env::var("XAI_API_KEY").ok())
            .context("Grok API key not configured")?;

        Ok(Self {
            api_key,
            model: config.model.clone(),
        })
    }
}

#[async_trait::async_trait]
impl StateSynthesizer for GrokStateSynthesizer {
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult> {
        let client = reqwest::Client::new();

        let request_body = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": STATE_UPDATE_SYSTEM_PROMPT
                },
                {
                    "role": "user",
                    "content": input.build_prompt()
                }
            ],
            "max_tokens": 1500,
            "temperature": 0.3
        });

        let response = client
            .post("https://api.x.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send state synthesis request to Grok")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Grok API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let state_body = response_body["choices"][0]["message"]["content"]
            .as_str()
            .context("Invalid response format from Grok")?
            .trim()
            .to_string();

        Ok(StateSynthesisResult {
            state_body,
            backend: "grok".to_string(),
        })
    }

    fn backend_name(&self) -> &'static str {
        "grok"
    }
}

/// Vertex AI Anthropic state synthesizer
pub struct VertexAnthropicStateSynthesizer {
    project_id: String,
    location: String,
    model: String,
    credentials_path: Option<String>,
}

impl VertexAnthropicStateSynthesizer {
    pub fn new(config: &SynthesisVertexSettings) -> Result<Self> {
        let project_id = config
            .project_id
            .clone()
            .or_else(|| std::env::var("VERTEX_AI_PROJECT_ID").ok())
            .context("Vertex AI project ID not configured")?;

        let location = config
            .location
            .clone()
            .or_else(|| std::env::var("VERTEX_AI_LOCATION").ok())
            .unwrap_or_else(|| "us-east5".to_string());

        Ok(Self {
            project_id,
            location,
            model: config.model.clone(),
            credentials_path: config.credentials_path.clone(),
        })
    }

    /// Get an access token using service account credentials
    async fn get_access_token(&self) -> Result<String> {
        // Try service account credentials first
        if let Some(creds_path) = &self.credentials_path {
            return self.get_token_from_service_account(creds_path).await;
        }

        // Fall back to GOOGLE_APPLICATION_CREDENTIALS
        if let Ok(creds_path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            return self.get_token_from_service_account(&creds_path).await;
        }

        // Fall back to application default credentials
        self.get_token_from_default().await
    }

    async fn get_token_from_service_account(&self, creds_path: &str) -> Result<String> {
        let service_account = CustomServiceAccount::from_file(creds_path)
            .context("Failed to load service account credentials")?;

        let token = service_account
            .token(&[VERTEX_AI_SCOPE])
            .await
            .context("Failed to get access token from service account")?;

        Ok(token.as_str().to_string())
    }

    async fn get_token_from_default(&self) -> Result<String> {
        let provider = gcp_auth::provider()
            .await
            .context("Failed to get default credentials provider")?;

        let token = provider
            .token(&[VERTEX_AI_SCOPE])
            .await
            .context("Failed to get access token from default credentials")?;

        Ok(token.as_str().to_string())
    }
}

#[async_trait::async_trait]
impl StateSynthesizer for VertexAnthropicStateSynthesizer {
    async fn synthesize_state(&self, input: &StateSynthesisInput) -> Result<StateSynthesisResult> {
        let access_token = self.get_access_token().await?;

        let client = reqwest::Client::new();

        let url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:rawPredict",
            self.location, self.project_id, self.location, self.model
        );

        let user_prompt = input.build_prompt();

        // Log the prompts being used
        tracing::info!(
            "[synthesis] State synthesis request:\n  event_type={}\n  files={:?}\n  current_state_len={}",
            input.event_type,
            input.files,
            input.current_state.len()
        );
        tracing::debug!(
            "[synthesis] System prompt:\n{}\n\n[synthesis] User prompt:\n{}",
            STATE_UPDATE_SYSTEM_PROMPT,
            user_prompt
        );

        let request_body = serde_json::json!({
            "anthropic_version": "vertex-2023-10-16",
            "max_tokens": 1500,
            "system": STATE_UPDATE_SYSTEM_PROMPT,
            "messages": [
                {
                    "role": "user",
                    "content": user_prompt
                }
            ]
        });

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send state synthesis request to Vertex AI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Vertex AI API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let state_body = response_body["content"][0]["text"]
            .as_str()
            .context("Invalid response format from Vertex AI")?
            .trim()
            .to_string();

        // Log the LLM response
        tracing::info!(
            "[synthesis] State synthesis response (len={}):\n{}",
            state_body.len(),
            state_body
        );

        Ok(StateSynthesisResult {
            state_body,
            backend: "vertex_anthropic".to_string(),
        })
    }

    fn backend_name(&self) -> &'static str {
        "vertex_anthropic"
    }
}

/// Create a state synthesizer based on configuration
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
