//! Async I/O — `completion()` + `stream()` against `/chat/completions`,
//! plus the `make()` constructor for the `rig::completion::CompletionModel`
//! trait.

use futures::StreamExt;
use rig::completion::{self, CompletionError, CompletionRequest, CompletionResponse};
use rig::streaming::{
    RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse, ToolCallDeltaContent,
};

use crate::client::Client;
use crate::streaming::{StreamChunk, StreamingResponse};
use crate::types;

use super::{CompletionModel, StreamingResponseData, StreamingUsage};

impl completion::CompletionModel for CompletionModel {
    type Response = types::Completion;
    type StreamingResponse = StreamingResponseData;
    type Client = Client;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self::new(client.clone(), model.into())
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let zai_request = self.build_request(&request, false);

        let url = self.client.endpoint_url("/chat/completions");
        let headers = self
            .client
            .build_headers()
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        tracing::debug!("Z.AI completion request to: {}", url);

        let response = self
            .client
            .http_client()
            .post(&url)
            .headers(headers)
            .json(&zai_request)
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

        let zai_response: types::Completion = serde_json::from_str(&body)?;

        Ok(Self::convert_response(zai_response))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let zai_request = self.build_request(&request, true);

        let url = self.client.endpoint_url("/chat/completions");
        let headers = self
            .client
            .build_headers()
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        tracing::debug!("Z.AI streaming request to: {}", url);

        let response = self
            .client
            .http_client()
            .post(&url)
            .headers(headers)
            .json(&zai_request)
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

        // Create streaming response.
        let stream = StreamingResponse::new(response);

        // Map to rig's streaming format.
        let mapped_stream = stream.map(|chunk_result| {
            chunk_result
                .map(|chunk| match chunk {
                    StreamChunk::TextDelta { text } => RawStreamingChoice::Message(text),
                    StreamChunk::ReasoningDelta { reasoning } => RawStreamingChoice::Reasoning {
                        id: None,
                        content: rig::message::ReasoningContent::Text {
                            text: reasoning,
                            signature: None,
                        },
                    },
                    StreamChunk::ToolCallStart { id, name, .. } => {
                        tracing::info!("Tool call started: {} ({})", name, id);
                        RawStreamingChoice::ToolCall(RawStreamingToolCall {
                            id: id.clone(),
                            call_id: Some(id),
                            name,
                            arguments: serde_json::json!({}),
                            signature: None,
                            additional_params: None,
                            internal_call_id: nanoid::nanoid!(),
                        })
                    }
                    StreamChunk::ToolCallDelta { arguments, .. } => {
                        RawStreamingChoice::ToolCallDelta {
                            id: String::new(),
                            content: ToolCallDeltaContent::Delta(arguments),
                            internal_call_id: nanoid::nanoid!(),
                        }
                    }
                    StreamChunk::ToolCallsComplete { tool_calls } => {
                        // Emit the first tool call as complete (rig
                        // handles one at a time).
                        if let Some(tc) = tool_calls.first() {
                            let arguments = golish_json_repair::parse_tool_args(&tc.arguments);
                            RawStreamingChoice::ToolCall(RawStreamingToolCall {
                                id: tc.id.clone(),
                                call_id: Some(tc.id.clone()),
                                name: tc.name.clone(),
                                arguments,
                                signature: None,
                                additional_params: None,
                                internal_call_id: nanoid::nanoid!(),
                            })
                        } else {
                            RawStreamingChoice::Message(String::new())
                        }
                    }
                    StreamChunk::Done { usage } => {
                        RawStreamingChoice::FinalResponse(StreamingResponseData {
                            usage: usage.map(|u| StreamingUsage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                total_tokens: u.total_tokens,
                            }),
                        })
                    }
                    StreamChunk::Error { message } => {
                        RawStreamingChoice::Message(format!("[Error: {}]", message))
                    }
                    StreamChunk::Empty => RawStreamingChoice::Message(String::new()),
                })
                .map_err(|e| {
                    tracing::error!("Stream chunk error: {}", e);
                    CompletionError::ProviderError(e.to_string())
                })
        });

        Ok(StreamingCompletionResponse::stream(Box::pin(mapped_stream)))
    }
}
