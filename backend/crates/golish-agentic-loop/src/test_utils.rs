//! Test utilities for the AI agent system.
//!
//! This module provides mock implementations and helpers for testing the
//! agentic loop, HITL approval flows, and tool routing logic.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use rig::completion::{
    self, AssistantContent, CompletionError, CompletionRequest, CompletionResponse, GetTokenUsage,
    Usage,
};
use rig::message::{Reasoning, ReasoningContent, Text, ToolCall, ToolFunction};
use rig::one_or_many::OneOrMany;
use rig::streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse};
use serde::{Deserialize, Serialize};

#[cfg(test)]
use tokio::sync::{mpsc, RwLock};
#[cfg(test)]
use golish_core::events::AiEvent;
#[cfg(test)]
use golish_llm_providers::LlmClient;
#[cfg(test)]
use golish_tools::ToolRegistry;
#[cfg(test)]
use golish_agent_loop::agent_mode::AgentMode;

/// A mock response that the MockCompletionModel will return.
#[derive(Debug, Clone)]
pub struct MockResponse {
    /// Text content to return (if any)
    pub text: Option<String>,
    /// Tool calls to return (if any)
    pub tool_calls: Vec<MockToolCall>,
    /// Thinking/reasoning content to return (if any)
    pub thinking: Option<String>,
}

impl Default for MockResponse {
    fn default() -> Self {
        Self {
            text: Some("Mock response".to_string()),
            tool_calls: vec![],
            thinking: None,
        }
    }
}

impl MockResponse {
    /// Create a text-only response.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            text: Some(content.into()),
            tool_calls: vec![],
            thinking: None,
        }
    }

    /// Create a response with a tool call.
    pub fn tool_call(name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            text: None,
            tool_calls: vec![MockToolCall {
                name: name.into(),
                args,
            }],
            thinking: None,
        }
    }

    /// Create a response with multiple tool calls.
    pub fn tool_calls(calls: Vec<MockToolCall>) -> Self {
        Self {
            text: None,
            tool_calls: calls,
            thinking: None,
        }
    }

    /// Create a response with thinking content.
    pub fn with_thinking(mut self, thinking: impl Into<String>) -> Self {
        self.thinking = Some(thinking.into());
        self
    }

    /// Create a response with text and thinking.
    pub fn text_with_thinking(text: impl Into<String>, thinking: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            tool_calls: vec![],
            thinking: Some(thinking.into()),
        }
    }
}

/// A mock tool call.
#[derive(Debug, Clone)]
pub struct MockToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

impl MockToolCall {
    pub fn new(name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}

/// Streaming response data for the mock model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockStreamingResponseData {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Default for MockStreamingResponseData {
    fn default() -> Self {
        Self {
            text: String::new(),
            input_tokens: 100,
            output_tokens: 50,
        }
    }
}

impl GetTokenUsage for MockStreamingResponseData {
    fn token_usage(&self) -> Option<Usage> {
        Some(Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.input_tokens + self.output_tokens,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }
}

/// A mock CompletionModel for testing agentic loop behavior.
///
/// This model returns predefined responses in sequence, allowing
/// multi-turn testing of the agentic loop.
#[derive(Debug, Clone)]
pub struct MockCompletionModel {
    responses: Arc<Vec<MockResponse>>,
    current_index: Arc<AtomicUsize>,
}

impl MockCompletionModel {
    /// Create a new mock model with a sequence of responses.
    pub fn new(responses: Vec<MockResponse>) -> Self {
        Self {
            responses: Arc::new(responses),
            current_index: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Create a mock model that returns a single text response.
    pub fn with_text(text: impl Into<String>) -> Self {
        Self::new(vec![MockResponse::text(text)])
    }

    /// Create a mock model that returns a single tool call then text.
    pub fn with_tool_call_then_text(
        tool_name: impl Into<String>,
        tool_args: serde_json::Value,
        final_text: impl Into<String>,
    ) -> Self {
        Self::new(vec![
            MockResponse::tool_call(tool_name, tool_args),
            MockResponse::text(final_text),
        ])
    }

    /// Get the next response in the sequence.
    fn next_response(&self) -> MockResponse {
        let index = self.current_index.fetch_add(1, Ordering::SeqCst);
        if index < self.responses.len() {
            self.responses[index].clone()
        } else {
            // Return empty text response if we've exhausted all responses
            MockResponse::text("")
        }
    }

    /// Reset the response index to start from the beginning.
    pub fn reset(&self) {
        self.current_index.store(0, Ordering::SeqCst);
    }

    /// Get the number of times a response has been requested.
    pub fn call_count(&self) -> usize {
        self.current_index.load(Ordering::SeqCst)
    }

    /// Build a CompletionResponse from a MockResponse.
    fn build_completion_response(
        &self,
        mock_response: &MockResponse,
        call_count: usize,
    ) -> CompletionResponse<MockStreamingResponseData> {
        let mut content: Vec<AssistantContent> = vec![];

        // Add thinking content first (if any)
        if let Some(thinking) = &mock_response.thinking {
            content.push(AssistantContent::Reasoning(
                Reasoning::new(thinking).optional_id(Some(format!("mock-thinking-{}", call_count))),
            ));
        }

        // Add text content (if any)
        if let Some(text) = &mock_response.text {
            content.push(AssistantContent::Text(Text { text: text.clone() }));
        }

        // Add tool calls (if any)
        for (i, tool_call) in mock_response.tool_calls.iter().enumerate() {
            let id = format!("mock-tool-{}-{}", call_count, i);
            content.push(AssistantContent::ToolCall(ToolCall {
                id: id.clone(),
                call_id: Some(id),
                function: ToolFunction {
                    name: tool_call.name.clone(),
                    arguments: tool_call.args.clone(),
                },
                signature: None,
                additional_params: None,
            }));
        }

        let choice = if content.len() == 1 {
            OneOrMany::one(content.pop().unwrap())
        } else if content.is_empty() {
            OneOrMany::one(AssistantContent::Text(Text {
                text: String::new(),
            }))
        } else {
            OneOrMany::many(content).unwrap()
        };

        CompletionResponse {
            choice,
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                total_tokens: 150,
                cached_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
            raw_response: MockStreamingResponseData::default(),
            message_id: None,
        }
    }

    /// Build streaming chunks from a MockResponse.
    fn build_stream_chunks(
        mock_response: &MockResponse,
        call_count: usize,
    ) -> Vec<RawStreamingChoice<MockStreamingResponseData>> {
        let mut chunks: Vec<RawStreamingChoice<MockStreamingResponseData>> = vec![];

        // Add thinking content first (if any)
        if let Some(thinking) = &mock_response.thinking {
            chunks.push(RawStreamingChoice::Reasoning {
                id: Some(format!("mock-thinking-{}", call_count)),
                content: ReasoningContent::Text {
                    text: thinking.clone(),
                    signature: Some("mock-signature".to_string()),
                },
            });
        }

        // Add text content (if any)
        if let Some(text) = &mock_response.text {
            chunks.push(RawStreamingChoice::Message(text.clone()));
        }

        // Add tool calls (if any)
        for (i, tool_call) in mock_response.tool_calls.iter().enumerate() {
            let id = format!("mock-tool-{}-{}", call_count, i);
            chunks.push(RawStreamingChoice::ToolCall(RawStreamingToolCall {
                id: id.clone(),
                internal_call_id: id.clone(),
                call_id: Some(id),
                name: tool_call.name.clone(),
                arguments: tool_call.args.clone(),
                signature: None,
                additional_params: None,
            }));
        }

        // Add final response
        chunks.push(RawStreamingChoice::FinalResponse(
            MockStreamingResponseData {
                text: mock_response.text.clone().unwrap_or_default(),
                input_tokens: 100,
                output_tokens: 50,
            },
        ));

        chunks
    }
}

impl completion::CompletionModel for MockCompletionModel {
    type Response = MockStreamingResponseData;
    type StreamingResponse = MockStreamingResponseData;
    type Client = ();

    fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
        Self::new(vec![MockResponse::default()])
    }

    async fn completion(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let mock_response = self.next_response();
        let call_count = self.call_count();
        Ok(self.build_completion_response(&mock_response, call_count))
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let mock_response = self.next_response();
        let call_count = self.call_count();
        let chunks = Self::build_stream_chunks(&mock_response, call_count);

        // Convert to stream of RawStreamingChoice
        let stream: BoxStream<
            'static,
            Result<RawStreamingChoice<MockStreamingResponseData>, CompletionError>,
        > = stream::iter(chunks.into_iter().map(Ok)).boxed();

        Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
    }
}

// ============================================================================
// Test Context Infrastructure
// ============================================================================

mod context;
pub use context::{MockRuntime, TestContext, TestContextBuilder};

#[cfg(test)]
#[path = "test_utils_tests.rs"]
mod test_utils_tests;
