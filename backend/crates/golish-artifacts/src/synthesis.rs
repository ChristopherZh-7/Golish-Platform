use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use golish_settings::schema::{
    SynthesisGrokSettings, SynthesisOpenAiSettings, SynthesisVertexSettings,
};

use crate::prompts::*;
use crate::generators::{generate_readme_update, generate_claude_md_update};

/// Backend for artifact synthesis (similar to commit message synthesis)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSynthesisBackend {
    /// Rule-based template generation (no API calls)
    #[default]
    Template,
    /// Anthropic Claude via Vertex AI
    VertexAnthropic,
    /// OpenAI API (or compatible)
    OpenAi,
    /// Grok API (xAI)
    Grok,
}

impl std::str::FromStr for ArtifactSynthesisBackend {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "template" => Ok(ArtifactSynthesisBackend::Template),
            "vertex_anthropic" | "vertex" => Ok(ArtifactSynthesisBackend::VertexAnthropic),
            "openai" => Ok(ArtifactSynthesisBackend::OpenAi),
            "grok" => Ok(ArtifactSynthesisBackend::Grok),
            _ => bail!("Unknown artifact synthesis backend: {}", s),
        }
    }
}

impl std::fmt::Display for ArtifactSynthesisBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactSynthesisBackend::Template => write!(f, "template"),
            ArtifactSynthesisBackend::VertexAnthropic => write!(f, "vertex_anthropic"),
            ArtifactSynthesisBackend::OpenAi => write!(f, "openai"),
            ArtifactSynthesisBackend::Grok => write!(f, "grok"),
        }
    }
}

/// Configuration for artifact synthesis
#[derive(Debug, Clone)]
pub struct ArtifactSynthesisConfig {
    /// Which backend to use
    pub backend: ArtifactSynthesisBackend,
    /// Vertex AI settings (when backend = vertex_anthropic)
    pub vertex: SynthesisVertexSettings,
    /// OpenAI settings (when backend = openai)
    pub openai: SynthesisOpenAiSettings,
    /// Grok settings (when backend = grok)
    pub grok: SynthesisGrokSettings,
}

impl Default for ArtifactSynthesisConfig {
    fn default() -> Self {
        Self {
            backend: ArtifactSynthesisBackend::Template,
            vertex: SynthesisVertexSettings::default(),
            openai: SynthesisOpenAiSettings::default(),
            grok: SynthesisGrokSettings::default(),
        }
    }
}

impl ArtifactSynthesisConfig {
    /// Create config from sidecar settings (reuses synthesis settings)
    pub fn from_sidecar_settings(settings: &golish_settings::schema::SidecarSettings) -> Self {
        // Artifact synthesis reuses the same backend config as commit message synthesis
        let backend = settings
            .synthesis_backend
            .parse()
            .unwrap_or(ArtifactSynthesisBackend::Template);

        Self {
            backend,
            vertex: settings.synthesis_vertex.clone(),
            openai: settings.synthesis_openai.clone(),
            grok: settings.synthesis_grok.clone(),
        }
    }

    /// Check if using LLM backend (not template)
    pub fn uses_llm(&self) -> bool {
        self.backend != ArtifactSynthesisBackend::Template
    }
}

// =============================================================================
// LLM Artifact Synthesizers
// =============================================================================

/// Input for artifact synthesis
#[derive(Debug, Clone)]
pub struct ArtifactSynthesisInput {
    /// Existing content of the target file
    pub existing_content: String,
    /// Summary of patches (commit subjects)
    pub patches_summary: Vec<String>,
    /// Session context (goals, progress)
    pub session_context: String,
}

impl ArtifactSynthesisInput {
    /// Create new synthesis input
    pub fn new(
        existing_content: String,
        patches_summary: Vec<String>,
        session_context: String,
    ) -> Self {
        Self {
            existing_content,
            patches_summary,
            session_context,
        }
    }

    /// Build the user prompt for README.md
    pub fn build_readme_prompt(&self) -> String {
        let patches = if self.patches_summary.is_empty() {
            "No patches available.".to_string()
        } else {
            self.patches_summary
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{}. {}", i + 1, s))
                .collect::<Vec<_>>()
                .join("\n")
        };

        README_USER_PROMPT
            .replace("{existing_content}", &self.existing_content)
            .replace("{patches_summary}", &patches)
            .replace("{session_context}", &self.session_context)
    }

    /// Build the user prompt for CLAUDE.md
    pub fn build_claude_md_prompt(&self) -> String {
        let patches = if self.patches_summary.is_empty() {
            "No patches available.".to_string()
        } else {
            self.patches_summary
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{}. {}", i + 1, s))
                .collect::<Vec<_>>()
                .join("\n")
        };

        CLAUDE_MD_USER_PROMPT
            .replace("{existing_content}", &self.existing_content)
            .replace("{patches_summary}", &patches)
            .replace("{session_context}", &self.session_context)
    }
}

/// Result of artifact synthesis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSynthesisResult {
    /// The generated content
    pub content: String,
    /// Which backend was used
    pub backend: String,
}

/// Synthesize README.md content using the configured backend
pub async fn synthesize_readme(
    config: &ArtifactSynthesisConfig,
    input: &ArtifactSynthesisInput,
) -> Result<ArtifactSynthesisResult> {
    match config.backend {
        ArtifactSynthesisBackend::Template => {
            // Use rule-based generation
            let content = generate_readme_update(
                &input.existing_content,
                &input.session_context,
                &input.patches_summary,
            );
            Ok(ArtifactSynthesisResult {
                content,
                backend: "template".to_string(),
            })
        }
        ArtifactSynthesisBackend::OpenAi => {
            synthesize_with_openai(
                &config.openai,
                README_SYSTEM_PROMPT,
                &input.build_readme_prompt(),
            )
            .await
        }
        ArtifactSynthesisBackend::Grok => {
            synthesize_with_grok(
                &config.grok,
                README_SYSTEM_PROMPT,
                &input.build_readme_prompt(),
            )
            .await
        }
        ArtifactSynthesisBackend::VertexAnthropic => {
            synthesize_with_vertex(
                &config.vertex,
                README_SYSTEM_PROMPT,
                &input.build_readme_prompt(),
            )
            .await
        }
    }
}

/// Synthesize CLAUDE.md content using the configured backend
pub async fn synthesize_claude_md(
    config: &ArtifactSynthesisConfig,
    input: &ArtifactSynthesisInput,
) -> Result<ArtifactSynthesisResult> {
    match config.backend {
        ArtifactSynthesisBackend::Template => {
            // Use rule-based generation
            let content = generate_claude_md_update(
                &input.existing_content,
                &input.session_context,
                &input.patches_summary,
            );
            Ok(ArtifactSynthesisResult {
                content,
                backend: "template".to_string(),
            })
        }
        ArtifactSynthesisBackend::OpenAi => {
            synthesize_with_openai(
                &config.openai,
                CLAUDE_MD_SYSTEM_PROMPT,
                &input.build_claude_md_prompt(),
            )
            .await
        }
        ArtifactSynthesisBackend::Grok => {
            synthesize_with_grok(
                &config.grok,
                CLAUDE_MD_SYSTEM_PROMPT,
                &input.build_claude_md_prompt(),
            )
            .await
        }
        ArtifactSynthesisBackend::VertexAnthropic => {
            synthesize_with_vertex(
                &config.vertex,
                CLAUDE_MD_SYSTEM_PROMPT,
                &input.build_claude_md_prompt(),
            )
            .await
        }
    }
}

/// Synthesize using OpenAI API
async fn synthesize_with_openai(
    config: &SynthesisOpenAiSettings,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<ArtifactSynthesisResult> {
    let api_key = config
        .api_key
        .clone()
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .context("OpenAI API key not configured")?;

    let base_url = config
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com/v1");

    let client = reqwest::Client::new();

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": [
            {
                "role": "system",
                "content": system_prompt
            },
            {
                "role": "user",
                "content": user_prompt
            }
        ],
        "max_tokens": 4000,
        "temperature": 0.3
    });

    let response = client
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {}", api_key))
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
    let content = response_body["choices"][0]["message"]["content"]
        .as_str()
        .context("Invalid response format from OpenAI")?
        .trim()
        .to_string();

    Ok(ArtifactSynthesisResult {
        content,
        backend: "openai".to_string(),
    })
}

/// Synthesize using Grok API (xAI)
async fn synthesize_with_grok(
    config: &SynthesisGrokSettings,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<ArtifactSynthesisResult> {
    let api_key = config
        .api_key
        .clone()
        .or_else(|| std::env::var("GROK_API_KEY").ok())
        .or_else(|| std::env::var("XAI_API_KEY").ok())
        .context("Grok API key not configured")?;

    let client = reqwest::Client::new();

    let request_body = serde_json::json!({
        "model": config.model,
        "messages": [
            {
                "role": "system",
                "content": system_prompt
            },
            {
                "role": "user",
                "content": user_prompt
            }
        ],
        "max_tokens": 4000,
        "temperature": 0.3
    });

    let response = client
        .post("https://api.x.ai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
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
    let content = response_body["choices"][0]["message"]["content"]
        .as_str()
        .context("Invalid response format from Grok")?
        .trim()
        .to_string();

    Ok(ArtifactSynthesisResult {
        content,
        backend: "grok".to_string(),
    })
}

/// Synthesize using Vertex AI (Anthropic)
async fn synthesize_with_vertex(
    config: &SynthesisVertexSettings,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<ArtifactSynthesisResult> {
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

    // Get access token from gcloud
    let access_token = get_gcloud_access_token().await?;

    let client = reqwest::Client::new();

    let url = format!(
        "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/anthropic/models/{}:rawPredict",
        location, project_id, location, config.model
    );

    let request_body = serde_json::json!({
        "anthropic_version": "vertex-2023-10-16",
        "max_tokens": 4000,
        "system": system_prompt,
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
        .context("Failed to send request to Vertex AI")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Vertex AI API error ({}): {}", status, body);
    }

    let response_body: serde_json::Value = response.json().await?;
    let content = response_body["content"][0]["text"]
        .as_str()
        .context("Invalid response format from Vertex AI")?
        .trim()
        .to_string();

    Ok(ArtifactSynthesisResult {
        content,
        backend: "vertex_anthropic".to_string(),
    })
}

/// Get access token from gcloud CLI
async fn get_gcloud_access_token() -> Result<String> {
    let output = tokio::process::Command::new("gcloud")
        .args(["auth", "print-access-token"])
        .output()
        .await
        .context("Failed to run gcloud auth print-access-token")?;

    if !output.status.success() {
        bail!(
            "gcloud auth failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

