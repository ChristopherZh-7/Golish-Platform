//! Vertex AI Anthropic state synthesizer.
//!
//! Calls Anthropic Claude on Google Cloud Vertex AI's `:rawPredict` endpoint.
//! Authentication falls back through three sources (in order):
//! 1. `credentials_path` from the synthesis settings,
//! 2. `GOOGLE_APPLICATION_CREDENTIALS` env var,
//! 3. application-default credentials provided by `gcp_auth::provider`.

use anyhow::{bail, Context, Result};
use gcp_auth::{CustomServiceAccount, TokenProvider};
use golish_settings::schema::SynthesisVertexSettings;

use super::{StateSynthesisInput, StateSynthesisResult, StateSynthesizer};
use crate::prompts::STATE_UPDATE_SYSTEM_PROMPT;

const VERTEX_AI_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// Vertex AI Anthropic state synthesizer.
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

    /// Get an access token using service account credentials.
    async fn get_access_token(&self) -> Result<String> {
        if let Some(creds_path) = &self.credentials_path {
            return self.get_token_from_service_account(creds_path).await;
        }

        if let Ok(creds_path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
            return self.get_token_from_service_account(&creds_path).await;
        }

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
