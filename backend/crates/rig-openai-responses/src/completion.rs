//! `CompletionModel` for the OpenAI Responses API.
//!
//! This module is intentionally thin — it owns just the model struct and the
//! [`rig::completion::CompletionModel`] trait wiring.  The actual work lives
//! in three siblings, each with one responsibility:
//!
//! - [`crate::request`]   — build the `CreateResponse` payload.
//! - [`crate::response`]  — decode the non-streaming `Response` body.
//! - [`crate::stream_map`] — translate streaming events to `RawStreamingChoice`.
//!
//! Splitting them out keeps each file focused and lets tests target a single
//! transformation without dragging in the rest of the I/O surface.

use async_openai::types::responses::{ReasoningEffort as OAReasoningEffort, Response};
use futures::StreamExt;
use rig::completion::{self, CompletionError, CompletionRequest, CompletionResponse};
use rig::streaming::{RawStreamingChoice, StreamingCompletionResponse};

use crate::client::{Client, ReasoningEffort};
use crate::request::build_request;
use crate::response::{convert_response, StreamingResponseData};
use crate::stream_map::map_stream_event;

/// Completion model for OpenAI Responses API with explicit reasoning support.
#[derive(Clone)]
pub struct CompletionModel {
    pub(crate) client: Client,
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<ReasoningEffort>,
}

impl CompletionModel {
    /// Create a new completion model.
    pub fn new(client: Client, model: String) -> Self {
        Self {
            client,
            model,
            reasoning_effort: None,
        }
    }

    /// Set the reasoning effort level for reasoning models.
    pub fn with_reasoning_effort(mut self, effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(effort);
        self
    }

    /// Get the model name.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Reasoning effort as the async-openai enum, if configured.
    ///
    /// Used by `request::build_request` to bridge our local `ReasoningEffort`
    /// to async-openai without leaking that type into the public API.
    pub(crate) fn reasoning_effort_oa(&self) -> Option<OAReasoningEffort> {
        self.reasoning_effort.map(Into::into)
    }
}

impl std::fmt::Debug for CompletionModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionModel")
            .field("model", &self.model)
            .field("reasoning_effort", &self.reasoning_effort)
            .finish_non_exhaustive()
    }
}

impl completion::CompletionModel for CompletionModel {
    type Response = Response;
    type StreamingResponse = StreamingResponseData;
    type Client = Client;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self::new(client.clone(), model.into())
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let openai_request = build_request(self, &request)?;

        let response = self
            .client
            .inner
            .responses()
            .create(openai_request)
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        Ok(convert_response(response))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let openai_request = build_request(self, &request)?;

        tracing::debug!("Starting OpenAI Responses stream for model: {}", self.model);

        let stream = self
            .client
            .inner
            .responses()
            .create_stream(openai_request)
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        // Map async-openai events to rig-core RawStreamingChoice
        let mapped = stream.filter_map(|result| async move {
            match result {
                Ok(event) => map_stream_event(event).map(Ok),
                Err(e) => {
                    // The OpenAI Responses API sends keepalive events that
                    // async-openai doesn't recognise yet. Skip them silently
                    // instead of surfacing a noisy deserialization error.
                    let msg = e.to_string();
                    if msg.contains("keepalive") || msg.contains("unknown variant") {
                        tracing::debug!("Skipping unrecognised OpenAI stream event: {}", msg);
                        None
                    } else {
                        tracing::error!("OpenAI stream error: {}", e);
                        Some(Ok(RawStreamingChoice::Message(format!("[Error: {}]", e))))
                    }
                }
            }
        });

        Ok(StreamingCompletionResponse::stream(Box::pin(mapped)))
    }
}
