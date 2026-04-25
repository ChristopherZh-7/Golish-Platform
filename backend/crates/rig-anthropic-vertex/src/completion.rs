//! `CompletionModel` for Anthropic Claude on Google Cloud Vertex AI.
//!
//! Thin orchestrator: owns the model struct + builder methods and the
//! [`rig::completion::CompletionModel`] trait wiring.  The heavy lifting
//! lives in siblings, each with one responsibility:
//!
//! - [`crate::config`]     — `WebSearchConfig`, `WebFetchConfig`,
//!   `ServerToolsConfig`, beta-feature flags, default-`max_tokens` table.
//! - [`crate::request`]    — build the Anthropic request payload from a
//!   rig `CompletionRequest`.
//! - [`crate::response`]   — decode non-streaming responses + streaming
//!   final-response shape.
//! - [`crate::stream_map`] — per-chunk SSE → `RawStreamingChoice` mapping.

use futures::StreamExt;
use rig::completion::{self, CompletionError, CompletionRequest, CompletionResponse};
use rig::streaming::StreamingCompletionResponse;

use crate::client::Client;
use crate::config::{
    ServerToolsConfig, WebFetchConfig, WebSearchConfig, CONTEXT_1M_BETA, WEB_FETCH_BETA,
};
use crate::request::build_request;
use crate::response::{convert_response, StreamingCompletionResponseData};
use crate::stream_map::map_stream_chunk;
use crate::streaming::StreamingResponse;
use crate::types::{self, CitationsConfig, ServerTool, ThinkingConfig, ToolEntry};

/// Completion model for Anthropic Claude on Vertex AI.
#[derive(Clone)]
pub struct CompletionModel {
    pub(crate) client: Client,
    pub(crate) model: String,
    /// Optional thinking configuration for extended reasoning
    pub(crate) thinking: Option<ThinkingConfig>,
    /// Optional server tools configuration for native web tools
    pub(crate) server_tools: Option<ServerToolsConfig>,
    /// Enable 1M token context window (beta)
    pub(crate) context_1m: bool,
}

impl CompletionModel {
    /// Create a new completion model.
    pub fn new(client: Client, model: String) -> Self {
        Self {
            client,
            model,
            thinking: None,
            server_tools: None,
            context_1m: false,
        }
    }

    /// Enable 1M token context window (beta).
    /// Sends the `context-1m-2025-08-07` beta header with requests.
    pub fn with_context_1m(mut self) -> Self {
        self.context_1m = true;
        self
    }

    /// Enable extended thinking with the specified token budget.
    /// Note: When thinking is enabled, temperature is automatically set to 1.
    pub fn with_thinking(mut self, budget_tokens: u32) -> Self {
        self.thinking = Some(ThinkingConfig::new(budget_tokens));
        self
    }

    /// Enable extended thinking with default budget (10,000 tokens).
    pub fn with_default_thinking(mut self) -> Self {
        self.thinking = Some(ThinkingConfig::default_budget());
        self
    }

    /// Enable Claude's native web search tool with default configuration.
    pub fn with_web_search(mut self) -> Self {
        let config = self
            .server_tools
            .get_or_insert_with(ServerToolsConfig::default);
        config.web_search = Some(WebSearchConfig::default());
        self
    }

    /// Enable Claude's native web search tool with custom configuration.
    pub fn with_web_search_config(mut self, config: WebSearchConfig) -> Self {
        let server_config = self
            .server_tools
            .get_or_insert_with(ServerToolsConfig::default);
        server_config.web_search = Some(config);
        self
    }

    /// Enable Claude's native web fetch tool with default configuration.
    pub fn with_web_fetch(mut self) -> Self {
        let config = self
            .server_tools
            .get_or_insert_with(ServerToolsConfig::default);
        config.web_fetch = Some(WebFetchConfig::default());
        self
    }

    /// Enable Claude's native web fetch tool with custom configuration.
    pub fn with_web_fetch_config(mut self, config: WebFetchConfig) -> Self {
        let server_config = self
            .server_tools
            .get_or_insert_with(ServerToolsConfig::default);
        server_config.web_fetch = Some(config);
        self
    }

    /// Get the model identifier.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Borrow the configured `ThinkingConfig`, if any.
    ///
    /// Used by `request::build_request` to size `max_tokens` and force
    /// `temperature = 1` when extended thinking is enabled.
    pub(crate) fn thinking_config(&self) -> Option<&ThinkingConfig> {
        self.thinking.as_ref()
    }

    /// Check if server tools are enabled (requires beta header).
    fn needs_web_fetch_beta(&self) -> bool {
        self.server_tools
            .as_ref()
            .map(|c| c.web_fetch.is_some())
            .unwrap_or(false)
    }

    /// Build the combined beta header value from all enabled beta features.
    /// Returns `None` if no beta features are enabled.
    fn beta_header_value(&self) -> Option<String> {
        let mut betas = Vec::new();
        if self.needs_web_fetch_beta() {
            betas.push(WEB_FETCH_BETA);
        }
        if self.context_1m {
            betas.push(CONTEXT_1M_BETA);
        }
        if betas.is_empty() {
            None
        } else {
            Some(betas.join(","))
        }
    }

    /// Build server tool entries (web search / fetch) for the API request.
    pub(crate) fn build_server_tools(&self) -> Vec<ToolEntry> {
        let mut tools = Vec::new();

        if let Some(ref config) = self.server_tools {
            if let Some(ref search) = config.web_search {
                tools.push(ToolEntry::Server(ServerTool::WebSearch {
                    name: "web_search".to_string(),
                    max_uses: search.max_uses,
                    allowed_domains: search.allowed_domains.clone(),
                    blocked_domains: search.blocked_domains.clone(),
                }));
            }

            if let Some(ref fetch) = config.web_fetch {
                tools.push(ToolEntry::Server(ServerTool::WebFetch {
                    name: "web_fetch".to_string(),
                    max_uses: fetch.max_uses,
                    citations: if fetch.citations_enabled {
                        Some(CitationsConfig { enabled: true })
                    } else {
                        None
                    },
                    max_content_tokens: fetch.max_content_tokens,
                }));
            }
        }

        tools
    }
}

impl std::fmt::Debug for CompletionModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionModel")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl completion::CompletionModel for CompletionModel {
    type Response = types::CompletionResponse;
    type StreamingResponse = StreamingCompletionResponseData;
    type Client = Client;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self::new(client.clone(), model.into())
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let anthropic_request = build_request(self, &request, false);

        // Build URL for rawPredict (non-streaming)
        let url = self.client.endpoint_url(&self.model, "rawPredict");

        // Get headers with auth (include beta headers for enabled features)
        let beta = self.beta_header_value();
        let headers = self
            .client
            .build_headers_with_beta(beta.as_deref())
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        // Make the request
        let response = self
            .client
            .http_client()
            .post(&url)
            .headers(headers)
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| CompletionError::RequestError(Box::new(e)))?;

        // Check for errors
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(CompletionError::ProviderError(format!(
                "API error ({}): {}",
                status, body
            )));
        }

        // Parse response
        let body = response
            .text()
            .await
            .map_err(|e| CompletionError::RequestError(Box::new(e)))?;

        let anthropic_response: types::CompletionResponse = serde_json::from_str(&body)?;

        Ok(convert_response(anthropic_response))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let anthropic_request = build_request(self, &request, true);

        // Log request details
        tracing::debug!(
            "stream(): thinking={:?}, max_tokens={}, messages={}",
            anthropic_request.thinking.as_ref().map(|t| t.budget_tokens),
            anthropic_request.max_tokens,
            anthropic_request.messages.len()
        );

        // Build URL for streamRawPredict
        let url = self.client.endpoint_url(&self.model, "streamRawPredict");
        tracing::info!("stream(): POST {}", url);

        // Get headers with auth (include beta headers for enabled features)
        let beta = self.beta_header_value();
        let headers = self
            .client
            .build_headers_with_beta(beta.as_deref())
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        // Make the request
        let response = self
            .client
            .http_client()
            .post(&url)
            .headers(headers)
            .json(&anthropic_request)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("stream(): Request failed: {}", e);
                CompletionError::RequestError(Box::new(e))
            })?;

        let status = response.status();
        tracing::info!("stream(): Response status: {}", status);

        // Check for errors
        if !status.is_success() {
            let status_code = status.as_u16();
            let body = response.text().await.unwrap_or_default();
            tracing::error!("stream(): API error ({}): {}", status_code, body);
            return Err(CompletionError::ProviderError(format!(
                "API error ({}): {}",
                status_code, body
            )));
        }

        // Create streaming response
        tracing::info!(
            "stream(): Creating streaming response wrapper, status={}",
            status
        );
        let stream = StreamingResponse::new(response);

        // Convert each Anthropic chunk into a rig RawStreamingChoice via stream_map
        let mapped_stream = stream.map(|chunk_result| {
            chunk_result.map(map_stream_chunk).map_err(|e| {
                tracing::error!("map_to_raw: chunk error: {}", e);
                CompletionError::ProviderError(e.to_string())
            })
        });

        tracing::info!("Returning StreamingCompletionResponse");
        Ok(StreamingCompletionResponse::stream(Box::pin(mapped_stream)))
    }
}
