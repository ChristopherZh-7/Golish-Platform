use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use gcp_auth::{CustomServiceAccount, TokenProvider};

use golish_settings::schema::{
    SynthesisGrokSettings, SynthesisOpenAiSettings, SynthesisVertexSettings,
};

use crate::config::{SynthesisBackend, SynthesisConfig};

const VERTEX_AI_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
use crate::prompts::*;
use crate::template::generate_template_message;


/// Input for commit message synthesis
#[derive(Debug, Clone)]
pub struct SynthesisInput {
    /// Git diff content
    pub diff: String,
    /// Files changed (for context)
    pub files: Vec<PathBuf>,
    /// Session context (from state.md)
    pub session_context: Option<String>,
}

impl SynthesisInput {
    /// Create a new synthesis input
    pub fn new(diff: String, files: Vec<PathBuf>) -> Self {
        Self {
            diff,
            files,
            session_context: None,
        }
    }

    /// Add session context
    pub fn with_context(mut self, context: String) -> Self {
        self.session_context = Some(context);
        self
    }

    /// Format files list for prompt
    fn format_files(&self) -> String {
        self.files
            .iter()
            .map(|p| format!("- {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Build the user prompt for LLM
    pub fn build_prompt(&self) -> String {
        let context = self
            .session_context
            .as_deref()
            .unwrap_or("No session context available.");

        COMMIT_MESSAGE_USER_PROMPT
            .replace("{context}", context)
            .replace("{diff}", &self.diff)
            .replace("{files}", &self.format_files())
    }
}

/// Result of commit message synthesis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisResult {
    /// The generated commit message
    pub message: String,
    /// Which backend was used
    pub backend: String,
    /// Whether this was regenerated
    pub regenerated: bool,
}


// =============================================================================
// Synthesizer Trait and Implementations
// =============================================================================

/// Trait for commit message synthesis
#[async_trait::async_trait]
pub trait CommitMessageSynthesizer: Send + Sync {
    /// Generate a commit message from input
    async fn synthesize(&self, input: &SynthesisInput) -> Result<SynthesisResult>;

    /// Get the backend name (used in tests)
    #[allow(dead_code)]
    fn backend_name(&self) -> &'static str;
}

/// Template-based (rule-based) synthesizer
pub struct TemplateSynthesizer;

impl TemplateSynthesizer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TemplateSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CommitMessageSynthesizer for TemplateSynthesizer {
    async fn synthesize(&self, input: &SynthesisInput) -> Result<SynthesisResult> {
        let message = generate_template_message(&input.files, &input.diff);
        Ok(SynthesisResult {
            message,
            backend: "template".to_string(),
            regenerated: false,
        })
    }

    fn backend_name(&self) -> &'static str {
        "template"
    }
}

/// OpenAI-based synthesizer (also works with compatible APIs)
pub struct OpenAiSynthesizer {
    api_key: String,
    model: String,
    base_url: Option<String>,
}

impl OpenAiSynthesizer {
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
impl CommitMessageSynthesizer for OpenAiSynthesizer {
    async fn synthesize(&self, input: &SynthesisInput) -> Result<SynthesisResult> {
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
                    "content": COMMIT_MESSAGE_SYSTEM_PROMPT
                },
                {
                    "role": "user",
                    "content": input.build_prompt()
                }
            ],
            "max_tokens": 500,
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
        let message = response_body["choices"][0]["message"]["content"]
            .as_str()
            .context("Invalid response format from OpenAI")?
            .trim()
            .to_string();

        Ok(SynthesisResult {
            message,
            backend: "openai".to_string(),
            regenerated: false,
        })
    }

    fn backend_name(&self) -> &'static str {
        "openai"
    }
}

/// Grok-based synthesizer (xAI)
pub struct GrokSynthesizer {
    api_key: String,
    model: String,
}

impl GrokSynthesizer {
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
impl CommitMessageSynthesizer for GrokSynthesizer {
    async fn synthesize(&self, input: &SynthesisInput) -> Result<SynthesisResult> {
        let client = reqwest::Client::new();

        // Grok uses OpenAI-compatible API
        let request_body = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": COMMIT_MESSAGE_SYSTEM_PROMPT
                },
                {
                    "role": "user",
                    "content": input.build_prompt()
                }
            ],
            "max_tokens": 500,
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
        let message = response_body["choices"][0]["message"]["content"]
            .as_str()
            .context("Invalid response format from Grok")?
            .trim()
            .to_string();

        Ok(SynthesisResult {
            message,
            backend: "grok".to_string(),
            regenerated: false,
        })
    }

    fn backend_name(&self) -> &'static str {
        "grok"
    }
}

/// Vertex AI Anthropic synthesizer
pub struct VertexAnthropicSynthesizer {
    project_id: String,
    location: String,
    model: String,
    credentials_path: Option<String>,
}

impl VertexAnthropicSynthesizer {
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
impl CommitMessageSynthesizer for VertexAnthropicSynthesizer {
    async fn synthesize(&self, input: &SynthesisInput) -> Result<SynthesisResult> {
        let access_token = self.get_access_token().await?;

        let client = reqwest::Client::new();

        // Vertex AI Anthropic uses a specific endpoint format
        let url = format!(
            "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:rawPredict",
            self.location, self.project_id, self.location, self.model
        );

        let request_body = serde_json::json!({
            "anthropic_version": "vertex-2023-10-16",
            "max_tokens": 500,
            "system": COMMIT_MESSAGE_SYSTEM_PROMPT,
            "messages": [
                {
                    "role": "user",
                    "content": input.build_prompt()
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
            .context("Failed to send request to Vertex AI")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Vertex AI API error ({}): {}", status, body);
        }

        let response_body: serde_json::Value = response.json().await?;
        let message = response_body["content"][0]["text"]
            .as_str()
            .context("Invalid response format from Vertex AI")?
            .trim()
            .to_string();

        Ok(SynthesisResult {
            message,
            backend: "vertex_anthropic".to_string(),
            regenerated: false,
        })
    }

    fn backend_name(&self) -> &'static str {
        "vertex_anthropic"
    }
}

// =============================================================================
// Factory Function
// =============================================================================

/// Create a synthesizer based on configuration
pub fn create_synthesizer(config: &SynthesisConfig) -> Result<Box<dyn CommitMessageSynthesizer>> {
    match config.backend {
        SynthesisBackend::Template => Ok(Box::new(TemplateSynthesizer::new())),
        SynthesisBackend::OpenAi => Ok(Box::new(OpenAiSynthesizer::new(&config.openai)?)),
        SynthesisBackend::Grok => Ok(Box::new(GrokSynthesizer::new(&config.grok)?)),
        SynthesisBackend::VertexAnthropic => {
            Ok(Box::new(VertexAnthropicSynthesizer::new(&config.vertex)?))
        }
    }
}

