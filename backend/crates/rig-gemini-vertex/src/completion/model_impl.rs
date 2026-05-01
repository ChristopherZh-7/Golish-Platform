//! `rig::completion::CompletionModel` trait impl for [`super::CompletionModel`].
//!
//! Holds the two HTTP entry points (`completion`, `stream`). Pure data
//! shaping lives in [`super::convert`]; this module only does network I/O,
//! error mapping, and SSE chunk → rig stream-event translation.

use rig::completion::{self, CompletionError, CompletionRequest, CompletionResponse};
use rig::streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse};

use crate::client::Client;
use crate::types;

use super::convert::{build_request, convert_response};
use super::{CompletionModel, StreamingCompletionResponseData};

impl completion::CompletionModel for CompletionModel {
    type Response = types::GenerateContentResponse;
    type StreamingResponse = StreamingCompletionResponseData;
    type Client = Client;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self::new(client.clone(), model.into())
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let gemini_request = build_request(self, &request);

        // Build URL for generateContent (non-streaming).
        let url = self.client().endpoint_url(self.model(), "generateContent");

        let headers = self
            .client()
            .build_headers()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        let response = self
            .client()
            .http_client()
            .post(&url)
            .headers(headers)
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| CompletionError::RequestError(Box::new(e)))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(CompletionError::ProviderError(format!(
                "API error ({}): {}",
                status, body
            )));
        }

        let body = response
            .text()
            .await
            .map_err(|e| CompletionError::RequestError(Box::new(e)))?;

        let gemini_response: types::GenerateContentResponse = serde_json::from_str(&body)?;

        Ok(convert_response(gemini_response))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let gemini_request = build_request(self, &request);

        // Build URL for streamGenerateContent with SSE.
        let url = format!(
            "{}?alt=sse",
            self.client().endpoint_url(self.model(), "streamGenerateContent")
        );
        tracing::debug!("stream(): POST {}", url);

        let headers = self
            .client()
            .build_headers()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        let response = self
            .client()
            .http_client()
            .post(&url)
            .headers(headers)
            .json(&gemini_request)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("stream(): Request failed: {}", e);
                CompletionError::RequestError(Box::new(e))
            })?;

        let status = response.status();
        tracing::debug!("stream(): Response status: {}", status);

        if !status.is_success() {
            let status_code = status.as_u16();
            let body = response.text().await.unwrap_or_default();
            tracing::error!("stream(): API error ({}): {}", status_code, body);
            return Err(CompletionError::ProviderError(format!(
                "API error ({}): {}",
                status_code, body
            )));
        }

        use crate::streaming::{create_stream, StreamChunk};
        use futures::StreamExt;

        let stream = create_stream(response);

        // Map Gemini SSE chunks to rig's streaming format.
        let mapped_stream = stream.map(|chunk_result| {
            chunk_result
                .map(|chunk| match chunk {
                    StreamChunk::TextDelta { text, .. } => RawStreamingChoice::Message(text),
                    StreamChunk::FunctionCall {
                        name,
                        args,
                        signature,
                    } => RawStreamingChoice::ToolCall(RawStreamingToolCall {
                        id: name.clone(),
                        call_id: Some(name.clone()),
                        name,
                        arguments: args,
                        signature,
                        additional_params: None,
                        internal_call_id: nanoid::nanoid!(),
                    }),
                    StreamChunk::ThinkingDelta { thinking } => RawStreamingChoice::Reasoning {
                        id: None,
                        content: rig::message::ReasoningContent::Text {
                            text: thinking,
                            signature: None,
                        },
                    },
                    StreamChunk::Done { usage, .. } => {
                        RawStreamingChoice::FinalResponse(StreamingCompletionResponseData {
                            text: String::new(),
                            usage,
                        })
                    }
                })
                .map_err(|e| {
                    tracing::error!("stream map error: {}", e);
                    CompletionError::ProviderError(e.to_string())
                })
        });

        Ok(StreamingCompletionResponse::stream(Box::pin(mapped_stream)))
    }
}
