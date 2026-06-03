//! E1/E2 provider-resilience recovery: when a stream degenerates into
//! repetition (E1) or a retriable error truncates it mid-flight (E2), the turn
//! loop injects a bounded corrective re-prompt and retries instead of accepting
//! the broken output — and stops once the per-run budget is exhausted.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use tokio::sync::RwLock;

use rig::completion::{
    self, AssistantContent, CompletionError, CompletionResponse, Message, Usage,
};
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;
use rig::streaming::{RawStreamingChoice, StreamingCompletionResponse};

use golish_llm_providers::LlmClient;
use golish_sub_agents::SubAgentContext;

use crate::agentic_loop::run_agentic_loop_generic;
use crate::test_utils::{MockStreamingResponseData, TestContextBuilder};

/// One scripted chunk emitted by a stream() call.
#[derive(Debug, Clone)]
enum ScriptChunk {
    Text(String),
    Err(String),
    Final,
}

/// A model whose Nth `stream()` call replays `scripts[N]`. When
/// `repeat_last` is set, calls past the end replay the final script (used to
/// simulate a model that *keeps* degenerating, to exercise the budget cap).
#[derive(Debug, Clone)]
struct ScriptedStreamingModel {
    scripts: Arc<Vec<Vec<ScriptChunk>>>,
    repeat_last: bool,
    calls: Arc<AtomicUsize>,
}

impl ScriptedStreamingModel {
    fn new(scripts: Vec<Vec<ScriptChunk>>, repeat_last: bool) -> Self {
        Self {
            scripts: Arc::new(scripts),
            repeat_last,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn stream_call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl completion::CompletionModel for ScriptedStreamingModel {
    type Response = MockStreamingResponseData;
    type StreamingResponse = MockStreamingResponseData;
    type Client = ();

    fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
        Self::new(vec![vec![ScriptChunk::Final]], false)
    }

    async fn completion(
        &self,
        _request: rig::completion::CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        Ok(CompletionResponse {
            choice: OneOrMany::one(AssistantContent::Text(Text {
                text: String::new(),
            })),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                cached_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
            raw_response: MockStreamingResponseData {
                text: String::new(),
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
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        let script = self
            .scripts
            .get(index)
            .or_else(|| {
                if self.repeat_last {
                    self.scripts.last()
                } else {
                    None
                }
            })
            .cloned()
            .unwrap_or_else(|| vec![ScriptChunk::Final]);

        let items: Vec<Result<RawStreamingChoice<MockStreamingResponseData>, CompletionError>> =
            script
                .into_iter()
                .map(|chunk| match chunk {
                    ScriptChunk::Text(text) => Ok(RawStreamingChoice::Message(text)),
                    ScriptChunk::Err(msg) => Err(CompletionError::ProviderError(msg)),
                    ScriptChunk::Final => Ok(RawStreamingChoice::FinalResponse(
                        MockStreamingResponseData {
                            text: String::new(),
                            input_tokens: 10,
                            output_tokens: 5,
                        },
                    )),
                })
                .collect();

        let stream: BoxStream<
            'static,
            Result<RawStreamingChoice<MockStreamingResponseData>, CompletionError>,
        > = stream::iter(items).boxed();

        Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
    }
}

fn user_history() -> Vec<Message> {
    vec![Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: "analyze the target".to_string(),
        })),
    }]
}

/// Known-degenerate output: the same fingerprinted sentence repeated 3×, which
/// `detect_repetitive_text` flags (see `repetitive_text_tests`).
fn repetitive_text() -> String {
    "该网站运行的是一个基于Vue3构建的前端应用，名为管理系统，以下是关键发现。\
     我已经完成了对该网站的JavaScript代码分析。如果你有其他需要测试或分析的域名或目标，请告诉我。\
     我已经完成了对该网站的JavaScript代码分析。如果你有其他需要，请直接告诉我。\
     我已经完成了对该网站的JavaScript代码分析。请告诉我你接下来需要什么帮助。"
        .to_string()
}

#[tokio::test]
async fn repetitive_stream_triggers_recovery_reprompt_then_succeeds() {
    let test_ctx = TestContextBuilder::new().build().await;
    let client = Arc::new(RwLock::new(LlmClient::Mock));
    let mut ctx = test_ctx.as_agentic_context_with_client(&client);
    ctx.llm.provider_name = "openai";
    ctx.llm.model_name = "gpt-4o-mini";

    // Call 0: degenerate repetition → loop should inject a recovery re-prompt.
    // Call 1: a clean, concise answer → loop should accept it and finish.
    let model = ScriptedStreamingModel::new(
        vec![
            vec![ScriptChunk::Text(repetitive_text()), ScriptChunk::Final],
            vec![
                ScriptChunk::Text("Recovered: the target runs Vue 3.".to_string()),
                ScriptChunk::Final,
            ],
        ],
        false,
    );

    let (response, _reasoning, _history, _usage) = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant",
        user_history(),
        SubAgentContext::default(),
        &ctx,
    )
    .await
    .expect("recovery path must not error");

    assert_eq!(
        model.stream_call_count(),
        2,
        "repetition must trigger exactly one recovery retry"
    );
    assert!(
        response.contains("Recovered: the target runs Vue 3."),
        "final response must contain the recovered answer, got: {response:?}"
    );
}

#[tokio::test]
async fn persistent_repetition_stops_after_recovery_budget() {
    let test_ctx = TestContextBuilder::new().build().await;
    let client = Arc::new(RwLock::new(LlmClient::Mock));
    let mut ctx = test_ctx.as_agentic_context_with_client(&client);
    ctx.llm.provider_name = "openai";
    ctx.llm.model_name = "gpt-4o-mini";

    // Every call degenerates: initial attempt + MAX_REPETITION_RECOVERIES (2)
    // retries = 3 stream calls, then the loop gives up (no infinite spin).
    let model = ScriptedStreamingModel::new(
        vec![vec![
            ScriptChunk::Text(repetitive_text()),
            ScriptChunk::Final,
        ]],
        true,
    );

    let _ = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant",
        user_history(),
        SubAgentContext::default(),
        &ctx,
    )
    .await
    .expect("budget-exhausted path must terminate, not error");

    assert_eq!(
        model.stream_call_count(),
        3,
        "initial attempt + 2 bounded recoveries, then stop"
    );
}

#[tokio::test]
async fn retriable_mid_stream_error_triggers_continuation_then_succeeds() {
    let test_ctx = TestContextBuilder::new().build().await;
    let client = Arc::new(RwLock::new(LlmClient::Mock));
    let mut ctx = test_ctx.as_agentic_context_with_client(&client);
    ctx.llm.provider_name = "openai";
    ctx.llm.model_name = "gpt-4o-mini";

    // Call 0: partial text, then a retriable (503) chunk error truncates it.
    // Call 1: a clean completion → loop should accept it and finish.
    let model = ScriptedStreamingModel::new(
        vec![
            vec![
                ScriptChunk::Text("Starting analysis of the target: ".to_string()),
                ScriptChunk::Err("503 Service Unavailable".to_string()),
            ],
            vec![
                ScriptChunk::Text("Completed analysis: 2 open ports.".to_string()),
                ScriptChunk::Final,
            ],
        ],
        false,
    );

    let (response, _reasoning, _history, _usage) = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant",
        user_history(),
        SubAgentContext::default(),
        &ctx,
    )
    .await
    .expect("mid-stream retry path must not error");

    assert_eq!(
        model.stream_call_count(),
        2,
        "a retriable mid-stream error must trigger exactly one continuation retry"
    );
    assert!(
        response.contains("Completed analysis: 2 open ports."),
        "final response must contain the recovered completion, got: {response:?}"
    );
}
