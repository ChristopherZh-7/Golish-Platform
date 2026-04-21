use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use gcp_auth::{CustomServiceAccount, TokenProvider};

use golish_settings::schema::{
    SynthesisGrokSettings, SynthesisOpenAiSettings, SynthesisVertexSettings,
};

use crate::config::{SynthesisBackend, SynthesisConfig};

const VERTEX_AI_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
use crate::prompts::*;


/// Input for session title synthesis
#[derive(Debug, Clone)]
pub struct SessionTitleInput {
    /// The user's initial request
    pub initial_request: String,
    /// Current state body (goals, changes)
    pub state_body: Option<String>,
}

impl SessionTitleInput {
    pub fn new(initial_request: String) -> Self {
        Self {
            initial_request,
            state_body: None,
        }
    }

    pub fn with_state(mut self, state_body: String) -> Self {
        self.state_body = Some(state_body);
        self
    }

    fn build_prompt(&self) -> String {
        let context = self
            .state_body
            .as_ref()
            .map(|s| format!("\n\nSession progress:\n{}", s))
            .unwrap_or_default();

        format!(
            "User's initial request:\n{}\n{}",
            self.initial_request, context
        )
    }
}

/// Result of session title synthesis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTitleResult {
    pub title: String,
    pub backend: String,
}

/// Trait for session title synthesis
#[async_trait::async_trait]
pub trait SessionTitleSynthesizer: Send + Sync {
    async fn synthesize_title(&self, input: &SessionTitleInput) -> Result<SessionTitleResult>;
}

/// Template-based title synthesizer (fallback)
pub struct TemplateTitleSynthesizer;

impl TemplateTitleSynthesizer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TemplateTitleSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SessionTitleSynthesizer for TemplateTitleSynthesizer {
    async fn synthesize_title(&self, input: &SessionTitleInput) -> Result<SessionTitleResult> {
        // Fallback: use truncated first prompt
        let title = truncate_title(&input.initial_request, 50);
        Ok(SessionTitleResult {
            title,
            backend: "template".to_string(),
        })
    }
}

/// OpenAI-based title synthesizer
pub struct OpenAiTitleSynthesizer {
    api_key: String,
    model: String,
    base_url: Option<String>,
}

impl OpenAiTitleSynthesizer {
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
impl SessionTitleSynthesizer for OpenAiTitleSynthesizer {
    async fn synthesize_title(&self, input: &SessionTitleInput) -> Result<SessionTitleResult> {
        let base_url = self
            .base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1");

        let client = reqwest::Client::new();

        let request_body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SESSION_TITLE_SYSTEM_PROMPT },
                { "role": "user", "content": input.build_prompt() }
            ],
            "max_tokens": 50,
            "temperature": 0.3
        });

        let response = client
            .post(format!("{}/chat/completions", base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to OpenAI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("OpenAI API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let raw_response = response_body["choices"][0]["message"]["content"]
            .as_str()
            .context("Invalid response format from OpenAI")?;

        // Validate the title response - if invalid, use fallback
        let title = match validate_title_response(raw_response) {
            Some(valid_title) => valid_title,
            None => {
                tracing::warn!(
                    "[synthesis] LLM title response failed validation, using fallback. Raw: {}",
                    raw_response.chars().take(100).collect::<String>()
                );
                truncate_title(&input.initial_request, 50)
            }
        };

        Ok(SessionTitleResult {
            title,
            backend: "openai".to_string(),
        })
    }
}

/// Grok-based title synthesizer
pub struct GrokTitleSynthesizer {
    api_key: String,
    model: String,
}

impl GrokTitleSynthesizer {
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
impl SessionTitleSynthesizer for GrokTitleSynthesizer {
    async fn synthesize_title(&self, input: &SessionTitleInput) -> Result<SessionTitleResult> {
        let client = reqwest::Client::new();

        let request_body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SESSION_TITLE_SYSTEM_PROMPT },
                { "role": "user", "content": input.build_prompt() }
            ],
            "max_tokens": 50,
            "temperature": 0.3
        });

        let response = client
            .post("https://api.x.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to Grok")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Grok API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let raw_response = response_body["choices"][0]["message"]["content"]
            .as_str()
            .context("Invalid response format from Grok")?;

        // Validate the title response - if invalid, use fallback
        let title = match validate_title_response(raw_response) {
            Some(valid_title) => valid_title,
            None => {
                tracing::warn!(
                    "[synthesis] LLM title response failed validation, using fallback. Raw: {}",
                    raw_response.chars().take(100).collect::<String>()
                );
                truncate_title(&input.initial_request, 50)
            }
        };

        Ok(SessionTitleResult {
            title,
            backend: "grok".to_string(),
        })
    }
}

/// Vertex AI Anthropic title synthesizer
pub struct VertexAnthropicTitleSynthesizer {
    project_id: String,
    location: String,
    model: String,
    credentials_path: Option<String>,
}

impl VertexAnthropicTitleSynthesizer {
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

    async fn get_access_token(&self) -> Result<String> {
        let creds_path = self
            .credentials_path
            .clone()
            .or_else(|| std::env::var("GOOGLE_APPLICATION_CREDENTIALS").ok())
            .context("GCP credentials not configured")?;

        let sa = CustomServiceAccount::from_file(&creds_path)?;
        let token = sa
            .token(&[VERTEX_AI_SCOPE])
            .await
            .context("Failed to get access token")?;

        Ok(token.as_str().to_string())
    }
}

#[async_trait::async_trait]
impl SessionTitleSynthesizer for VertexAnthropicTitleSynthesizer {
    async fn synthesize_title(&self, input: &SessionTitleInput) -> Result<SessionTitleResult> {
        let access_token = self.get_access_token().await?;

        let url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:rawPredict",
            self.location, self.project_id, self.location, self.model
        );

        let request_body = serde_json::json!({
            "anthropic_version": "vertex-2023-10-16",
            "messages": [
                { "role": "user", "content": input.build_prompt() }
            ],
            "system": SESSION_TITLE_SYSTEM_PROMPT,
            "max_tokens": 50,
            "temperature": 0.3
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to Vertex AI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Vertex AI API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let raw_response = response_body["content"][0]["text"]
            .as_str()
            .context("Invalid response format from Vertex AI")?;

        // Validate the title response - if invalid, use fallback
        let title = match validate_title_response(raw_response) {
            Some(valid_title) => valid_title,
            None => {
                tracing::warn!(
                    "[synthesis] LLM title response failed validation, using fallback. Raw: {}",
                    raw_response.chars().take(100).collect::<String>()
                );
                // Use template-based fallback
                truncate_title(&input.initial_request, 50)
            }
        };

        Ok(SessionTitleResult {
            title,
            backend: "vertex_anthropic".to_string(),
        })
    }
}

/// Create a title synthesizer based on configuration
pub fn create_title_synthesizer(
    config: &SynthesisConfig,
) -> Result<Box<dyn SessionTitleSynthesizer>> {
    match config.backend {
        SynthesisBackend::Template => Ok(Box::new(TemplateTitleSynthesizer::new())),
        SynthesisBackend::OpenAi => Ok(Box::new(OpenAiTitleSynthesizer::new(&config.openai)?)),
        SynthesisBackend::Grok => Ok(Box::new(GrokTitleSynthesizer::new(&config.grok)?)),
        SynthesisBackend::VertexAnthropic => Ok(Box::new(VertexAnthropicTitleSynthesizer::new(
            &config.vertex,
        )?)),
    }
}

/// Truncate text to create a title (for template fallback)
fn truncate_title(text: &str, max_len: usize) -> String {
    // Strip XML context tags first
    let mut clean = text.to_string();
    for tag in ["context", "cwd", "session_id"] {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        while let Some(start) = clean.find(&open) {
            if let Some(end_offset) = clean[start..].find(&close) {
                let end = start + end_offset + close.len();
                clean = format!("{}{}", &clean[..start], &clean[end..]);
            } else {
                break;
            }
        }
    }

    let clean = clean.trim();
    if clean.len() <= max_len {
        clean.to_string()
    } else {
        let truncated = &clean[..max_len];
        let last_space = truncated.rfind(' ');
        if let Some(pos) = last_space {
            if pos > max_len / 2 {
                return format!("{}...", &truncated[..pos]);
            }
        }
        format!("{}...", truncated)
    }
}

/// Validate and sanitize a title response from an LLM
/// Returns None if the response doesn't look like a valid title
fn validate_title_response(response: &str) -> Option<String> {
    let title = response.trim().trim_matches('"');

    // Reject if empty
    if title.is_empty() {
        return None;
    }

    // Reject if it contains a question mark (LLM is asking for clarification)
    if title.contains('?') {
        tracing::debug!("[synthesis] Rejecting title with question mark: {}", title);
        return None;
    }

    // Reject if it's too long (more than 80 chars is not a title)
    if title.len() > 80 {
        tracing::debug!(
            "[synthesis] Rejecting title that's too long ({} chars): {}...",
            title.len(),
            &title[..50]
        );
        return None;
    }

    // Reject if it has multiple lines (not a title)
    if title.lines().count() > 1 {
        tracing::debug!("[synthesis] Rejecting multi-line title");
        return None;
    }

    // Reject if it starts with common conversational patterns
    let lower = title.to_lowercase();
    let conversational_starts = [
        "i need",
        "i'm not",
        "i don't",
        "could you",
        "can you",
        "please",
        "sorry",
        "unfortunately",
        "i cannot",
        "i can't",
    ];
    for pattern in conversational_starts {
        if lower.starts_with(pattern) {
            tracing::debug!(
                "[synthesis] Rejecting conversational title starting with '{}': {}",
                pattern,
                title
            );
            return None;
        }
    }

    Some(title.to_string())
}

