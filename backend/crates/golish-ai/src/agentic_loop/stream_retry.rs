//! Stream start error classification and retry logic.
//!
//! Handles transient failures when initiating LLM streaming requests,
//! with exponential backoff and jitter.

pub(super) const STREAM_START_MAX_ATTEMPTS: usize = 3;

const STREAM_START_RETRY_BASE_DELAY_MS: u64 = 300;
const STREAM_START_RETRY_MAX_DELAY_MS: u64 = 3_000;

#[derive(Debug, Clone)]
pub(super) struct StreamStartErrorClassification {
    pub error_type: &'static str,
    pub user_message: String,
    pub retriable: bool,
}

pub(super) fn classify_stream_start_error(error_str: &str) -> StreamStartErrorClassification {
    let lower = error_str.to_ascii_lowercase();

    if lower.contains("prompt is too long")
        || lower.contains("too many tokens")
        || lower.contains("context_length_exceeded")
    {
        return StreamStartErrorClassification {
            error_type: "context_overflow",
            user_message:
                "The conversation is too long. Please start a new chat or clear some history."
                    .to_string(),
            retriable: false,
        };
    }

    if lower.contains("authentication")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("401")
        || lower.contains("403")
    {
        return StreamStartErrorClassification {
            error_type: "authentication",
            user_message: "Authentication failed. Please check your API credentials.".to_string(),
            retriable: false,
        };
    }

    if lower.contains("rate_limit") || lower.contains("resource_exhausted") || lower.contains("429")
    {
        return StreamStartErrorClassification {
            error_type: "rate_limit",
            user_message: "Rate limit exceeded. Please wait a moment and try again.".to_string(),
            retriable: true,
        };
    }

    if lower.contains("degraded") {
        return StreamStartErrorClassification {
            error_type: "model_unavailable",
            user_message:
                "The AI model is currently degraded or unavailable. Please try a different model or try again later."
                    .to_string(),
            retriable: false,
        };
    }

    if lower.contains("timeout") || lower.contains("timed out") {
        return StreamStartErrorClassification {
            error_type: "timeout",
            user_message: "Request timed out. Please try again.".to_string(),
            retriable: true,
        };
    }

    let looks_transient = lower.contains("connection")
        || lower.contains("network")
        || lower.contains("temporar")
        || lower.contains("unavailable")
        || lower.contains("internal")
        || lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504");

    StreamStartErrorClassification {
        error_type: "api_error",
        user_message: error_str.to_string(),
        retriable: looks_transient,
    }
}

pub(super) fn stream_start_timeout_classification(
    timeout_secs: u64,
) -> StreamStartErrorClassification {
    StreamStartErrorClassification {
        error_type: "timeout",
        user_message: format!(
            "Request timed out after {} seconds. The AI provider is not responding. This may indicate a connection issue or an API problem.",
            timeout_secs
        ),
        retriable: true,
    }
}

pub(super) fn should_retry_stream_start(
    attempt: usize,
    classification: &StreamStartErrorClassification,
) -> bool {
    classification.retriable && attempt < STREAM_START_MAX_ATTEMPTS
}

pub(super) fn compute_retry_backoff_delay(attempt: usize) -> std::time::Duration {
    let exponent = (attempt.saturating_sub(1)).min(6) as u32;
    let factor = 1_u64 << exponent;
    let uncapped = STREAM_START_RETRY_BASE_DELAY_MS.saturating_mul(factor);
    let capped = uncapped.min(STREAM_START_RETRY_MAX_DELAY_MS);

    // Add small jitter (0-20%) to reduce synchronized retries.
    let jitter_bound = (capped / 5).max(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let jitter = nanos % jitter_bound;

    std::time::Duration::from_millis(capped + jitter)
}

pub(super) async fn sleep_for_retry_delay(delay: std::time::Duration) {
    #[cfg(test)]
    {
        let _ = delay;
        tokio::task::yield_now().await;
    }

    #[cfg(not(test))]
    {
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agentic_loop::{run_agentic_loop_generic, TerminalErrorEmitted};
    use crate::test_utils::{MockStreamingResponseData, TestContextBuilder};
    use futures::stream::{self, BoxStream};
    use futures::StreamExt;
    use golish_core::events::AiEvent;
    use golish_llm_providers::LlmClient;
    use golish_sub_agents::SubAgentContext;
    use rig::completion::{self, AssistantContent, CompletionError, CompletionResponse};
    use rig::message::{Text, UserContent};
    use rig::one_or_many::OneOrMany;
    use rig::streaming::{RawStreamingChoice, StreamingCompletionResponse};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use rig::completion::Message;

    #[derive(Debug, Clone)]
    enum StreamStartAttempt {
        Error(String),
        SuccessText(String),
    }

    #[derive(Debug, Clone)]
    struct ScriptedStreamStartModel {
        attempts: Arc<Vec<StreamStartAttempt>>,
        stream_calls: Arc<AtomicUsize>,
    }

    impl ScriptedStreamStartModel {
        fn new(attempts: Vec<StreamStartAttempt>) -> Self {
            Self {
                attempts: Arc::new(attempts),
                stream_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn stream_call_count(&self) -> usize {
            self.stream_calls.load(Ordering::SeqCst)
        }
    }

    impl completion::CompletionModel for ScriptedStreamStartModel {
        type Response = MockStreamingResponseData;
        type StreamingResponse = MockStreamingResponseData;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            Self::new(vec![StreamStartAttempt::SuccessText("default".to_string())])
        }

        async fn completion(
            &self,
            _request: rig::completion::CompletionRequest,
        ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
            let text = self
                .attempts
                .iter()
                .find_map(|attempt| match attempt {
                    StreamStartAttempt::SuccessText(text) => Some(text.clone()),
                    StreamStartAttempt::Error(_) => None,
                })
                .unwrap_or_default();

            Ok(CompletionResponse {
                choice: OneOrMany::one(AssistantContent::Text(Text { text: text.clone() })),
                usage: rig::completion::Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                    cached_input_tokens: 0,
                },
                raw_response: MockStreamingResponseData {
                    text,
                    input_tokens: 10,
                    output_tokens: 5,
                },
                message_id: None,
            })
        }

        async fn stream(
            &self,
            _request: rig::completion::CompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            let index = self.stream_calls.fetch_add(1, Ordering::SeqCst);
            let attempt = self.attempts.get(index).cloned().unwrap_or_else(|| {
                StreamStartAttempt::Error("No scripted attempt remaining".to_string())
            });

            match attempt {
                StreamStartAttempt::Error(message) => Err(CompletionError::ProviderError(message)),
                StreamStartAttempt::SuccessText(text) => {
                    let chunks = vec![
                        RawStreamingChoice::Message(text.clone()),
                        RawStreamingChoice::FinalResponse(MockStreamingResponseData {
                            text,
                            input_tokens: 10,
                            output_tokens: 5,
                        }),
                    ];

                    let stream: BoxStream<
                        'static,
                        Result<RawStreamingChoice<MockStreamingResponseData>, CompletionError>,
                    > = stream::iter(chunks.into_iter().map(Ok)).boxed();

                    Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
                }
            }
        }
    }

    fn simple_user_history() -> Vec<Message> {
        vec![Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: "test stream-start behavior".to_string(),
            })),
        }]
    }

    #[tokio::test]
    async fn retries_transient_stream_start_failure_then_succeeds() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.provider_name = "openai";
        ctx.model_name = "gpt-4o-mini";

        let model = ScriptedStreamStartModel::new(vec![
            StreamStartAttempt::Error("API error (429): RESOURCE_EXHAUSTED".to_string()),
            StreamStartAttempt::SuccessText("Recovered after retry".to_string()),
        ]);

        let result = run_agentic_loop_generic(
            &model,
            "You are a helpful assistant",
            simple_user_history(),
            SubAgentContext::default(),
            &ctx,
        )
        .await;

        assert!(
            result.is_ok(),
            "expected retry to recover: {:?}",
            result.err()
        );
        let (response, _reasoning, _history, _usage) = result.unwrap();
        assert!(response.contains("Recovered after retry"));
        assert_eq!(model.stream_call_count(), 2);

        let mut test_ctx = test_ctx;
        let events = test_ctx.collect_events();

        let retry_warnings: Vec<&String> = events
            .iter()
            .filter_map(|event| match event {
                AiEvent::Warning { message } if message.contains("Retrying") => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(retry_warnings.len(), 1);
        assert!(retry_warnings[0].contains("attempt 2/3"));

        let terminal_errors = events
            .iter()
            .filter(|event| matches!(event, AiEvent::Error { .. }))
            .count();
        assert_eq!(terminal_errors, 0);
    }

    #[tokio::test]
    async fn retries_up_to_max_attempts_then_emits_single_error() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.provider_name = "openai";
        ctx.model_name = "gpt-4o-mini";

        let attempts = (0..STREAM_START_MAX_ATTEMPTS)
            .map(|_| StreamStartAttempt::Error("429 RESOURCE_EXHAUSTED".to_string()))
            .collect();
        let model = ScriptedStreamStartModel::new(attempts);

        let result = run_agentic_loop_generic(
            &model,
            "You are a helpful assistant",
            simple_user_history(),
            SubAgentContext::default(),
            &ctx,
        )
        .await;

        let err = result.expect_err("expected max-attempt failure");
        let terminal_error = err
            .downcast_ref::<TerminalErrorEmitted>()
            .expect("expected TerminalErrorEmitted marker");
        assert!(terminal_error.partial_response().is_none());
        assert!(terminal_error.final_history().is_some());
        assert_eq!(model.stream_call_count(), STREAM_START_MAX_ATTEMPTS);

        let mut test_ctx = test_ctx;
        let events = test_ctx.collect_events();

        let retry_warnings = events
            .iter()
            .filter(|event| {
                matches!(event, AiEvent::Warning { message } if message.contains("Retrying"))
            })
            .count();
        assert_eq!(retry_warnings, STREAM_START_MAX_ATTEMPTS - 1);

        let error_events: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AiEvent::Error {
                    message,
                    error_type,
                } => Some((message, error_type)),
                _ => None,
            })
            .collect();

        assert_eq!(error_events.len(), 1);
        assert_eq!(error_events[0].1, "rate_limit");
    }

    #[tokio::test]
    async fn non_retriable_stream_start_error_fails_fast_without_retry_warning() {
        let test_ctx = TestContextBuilder::new().build().await;
        let client = Arc::new(RwLock::new(LlmClient::Mock));
        let mut ctx = test_ctx.as_agentic_context_with_client(&client);
        ctx.provider_name = "openai";
        ctx.model_name = "gpt-4o-mini";

        let model = ScriptedStreamStartModel::new(vec![StreamStartAttempt::Error(
            "401 Unauthorized".to_string(),
        )]);

        let result = run_agentic_loop_generic(
            &model,
            "You are a helpful assistant",
            simple_user_history(),
            SubAgentContext::default(),
            &ctx,
        )
        .await;

        let err = result.expect_err("expected immediate non-retriable failure");
        let terminal_error = err
            .downcast_ref::<TerminalErrorEmitted>()
            .expect("expected TerminalErrorEmitted marker");
        assert!(terminal_error.partial_response().is_none());
        assert!(terminal_error.final_history().is_some());
        assert_eq!(model.stream_call_count(), 1);

        let mut test_ctx = test_ctx;
        let events = test_ctx.collect_events();

        let retry_warnings = events
            .iter()
            .filter(|event| {
                matches!(event, AiEvent::Warning { message } if message.contains("Retrying"))
            })
            .count();
        assert_eq!(retry_warnings, 0);

        let error_events: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AiEvent::Error {
                    message,
                    error_type,
                } => Some((message, error_type)),
                _ => None,
            })
            .collect();

        assert_eq!(error_events.len(), 1);
        assert_eq!(error_events[0].1, "authentication");
        assert!(error_events[0].0.contains("Authentication failed"));
    }
}
