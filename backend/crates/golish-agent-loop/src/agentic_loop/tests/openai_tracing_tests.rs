use super::*;
use crate::test_utils::{MockCompletionModel, MockResponse, TestContextBuilder};
use golish_llm_providers::LlmClient;
use golish_sub_agents::SubAgentContext;
use std::sync::Arc;
use tokio::sync::RwLock;

fn openai_reasoning_sub_context() -> SubAgentContext {
    SubAgentContext {
        original_request: "Test OpenAI tracing".to_string(),
        ..Default::default()
    }
}

/// Verify that Reasoning events are emitted when the model returns thinking content.
/// This is critical for GPT-5.2/Codex: thinking shown in the UI must also appear in traces.
#[tokio::test]
async fn test_openai_reasoning_emits_reasoning_event() {
    let test_ctx = TestContextBuilder::new()
        .agent_mode(crate::agent_mode::AgentMode::AutoApprove)
        .build()
        .await;

    // Model returns thinking + text (simulates gpt-5.2 with reasoning summary)
    let model = MockCompletionModel::new(vec![MockResponse::text_with_thinking(
        "I will read the file now.",
        "Let me think: I should use read_file to inspect the codebase.",
    )]);

    let client = Arc::new(RwLock::new(LlmClient::Mock));
    let mut ctx = test_ctx.as_agentic_context_with_client(&client);
    // Use openai_reasoning provider to test the correct code path
    ctx.llm.provider_name = "openai_reasoning";
    ctx.llm.model_name = "gpt-5.2";

    let initial_history = vec![rig::completion::Message::User {
        content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
            rig::message::Text {
                text: "Read the main.rs file".to_string(),
            },
        )),
    }];

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        initial_history,
        openai_reasoning_sub_context(),
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Loop should succeed: {:?}", result.err());
    let (response, reasoning, _history, _usage) = result.unwrap();

    // The reasoning content must be returned (for Langfuse span recording)
    assert!(
        reasoning.is_some(),
        "Reasoning content must be returned when model provides thinking"
    );
    assert!(
        reasoning.as_ref().unwrap().contains("read_file"),
        "Reasoning should contain thinking content, got: {:?}",
        reasoning
    );

    // The response text must also be present
    assert!(
        response.contains("I will read"),
        "Response should contain model text, got: {:?}",
        response
    );

    // Verify AiEvent::Reasoning was emitted (so UI ThinkingBlock works)
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let reasoning_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AiEvent::Reasoning { .. }))
        .collect();
    assert!(
        !reasoning_events.is_empty(),
        "AiEvent::Reasoning must be emitted for UI ThinkingBlock, but no Reasoning events found"
    );
}

/// Verify that a tool-call-only response (no text) still produces a Completed event
/// with token usage, and that the loop correctly handles the no-text case.
/// GPT-5.2/Codex commonly return tool calls without any accompanying text.
#[tokio::test]
async fn test_openai_tool_call_only_response_completes() {
    let test_ctx = TestContextBuilder::new()
        .agent_mode(crate::agent_mode::AgentMode::AutoApprove)
        .build()
        .await;

    // Create a file the tool can actually read
    let ws = test_ctx.workspace_path().await;
    std::fs::write(ws.join("test.txt"), "hello world").unwrap();

    // First response: tool call only (no text) — simulates gpt-5.2 behaviour
    // Second response: text summary
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_call("read_file", serde_json::json!({"path": "test.txt"})),
        MockResponse::text("I read the file and it contains 'hello world'."),
    ]);

    let client = Arc::new(RwLock::new(LlmClient::Mock));
    let mut ctx = test_ctx.as_agentic_context_with_client(&client);
    ctx.llm.provider_name = "openai_reasoning";
    ctx.llm.model_name = "gpt-5.2";

    let initial_history = vec![rig::completion::Message::User {
        content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
            rig::message::Text {
                text: "Read test.txt".to_string(),
            },
        )),
    }];

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        initial_history,
        openai_reasoning_sub_context(),
        &ctx,
    )
    .await;

    assert!(
        result.is_ok(),
        "Loop should succeed even with tool-call-only first response: {:?}",
        result.err()
    );
    let (response, _reasoning, _history, _usage) = result.unwrap();
    assert!(
        response.contains("hello world"),
        "Final response should include file content reference, got: {:?}",
        response
    );

    // Verify the loop produced a final text response (loop emits TextDelta events)
    // Note: AiEvent::Completed is emitted by agent_bridge.rs, not run_agentic_loop_generic directly.
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let text_deltas: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AiEvent::TextDelta { .. }))
        .collect();
    assert!(
        !text_deltas.is_empty(),
        "TextDelta events must be emitted for the text response after the tool call"
    );
    // Also verify a tool was auto-approved (auto-approve mode was set)
    let auto_approved = events
        .iter()
        .any(|e| matches!(e, AiEvent::ToolAutoApproved { .. }));
    assert!(
        auto_approved,
        "Tool should have been auto-approved in AutoApprove mode"
    );
}

/// Verify that reasoning/thinking content from the model is returned in the
/// (response, reasoning, history, usage) tuple so the caller can record it on spans.
#[tokio::test]
async fn test_openai_thinking_returned_in_result() {
    let test_ctx = TestContextBuilder::new()
        .agent_mode(crate::agent_mode::AgentMode::AutoApprove)
        .build()
        .await;

    let thinking = "Step 1: understand the request. Step 2: formulate response.";
    let model = MockCompletionModel::new(vec![
        MockResponse::text("Here is my answer.").with_thinking(thinking)
    ]);

    let client = Arc::new(RwLock::new(LlmClient::Mock));
    let mut ctx = test_ctx.as_agentic_context_with_client(&client);
    ctx.llm.provider_name = "openai_reasoning";
    ctx.llm.model_name = "gpt-5.2-codex";

    let initial_history = vec![rig::completion::Message::User {
        content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
            rig::message::Text {
                text: "What is 2+2?".to_string(),
            },
        )),
    }];

    let (_, reasoning, _, _) = run_agentic_loop_generic(
        &model,
        "You are a math tutor.",
        initial_history,
        openai_reasoning_sub_context(),
        &ctx,
    )
    .await
    .unwrap();

    assert!(
        reasoning.is_some(),
        "Reasoning must be returned when model provides thinking content"
    );
    let r = reasoning.unwrap();
    assert!(
        r.contains("Step 1"),
        "Returned reasoning should match model thinking, got: {:?}",
        r
    );
}

/// Verify that the "openai_reasoning" provider correctly detects model capabilities
/// so the loop uses the right temperature/thinking settings.
#[test]
fn test_openai_reasoning_loop_config_detection() {
    // gpt-5.2 via openai_reasoning: reasoning model, no temperature, thinking history
    let config = AgenticLoopConfig::with_detection("openai_reasoning", "gpt-5.2", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "gpt-5.2 via openai_reasoning must support thinking history for span recording"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "gpt-5.2 via openai_reasoning must not use temperature"
    );

    // gpt-5.2-codex via openai_reasoning
    let config = AgenticLoopConfig::with_detection("openai_reasoning", "gpt-5.2-codex", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "gpt-5.2-codex via openai_reasoning must support thinking history"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "gpt-5.2-codex must not use temperature"
    );

    // o4-mini via openai_reasoning
    let config = AgenticLoopConfig::with_detection("openai_reasoning", "o4-mini", false);
    assert!(
        config.capabilities.supports_thinking_history,
        "o4-mini via openai_reasoning must support thinking history"
    );
    assert!(
        !config.capabilities.supports_temperature,
        "o4-mini must not use temperature"
    );
}

/// Verify that "openai_reasoning" ALWAYS includes reasoning in conversation history,
/// even for text-only responses (no tool calls). The OpenAI Responses API tracks rs_...
/// IDs server-side and requires them to be echoed back in every subsequent turn.
///
/// Contrast with "openai_responses" where reasoning must only be included when paired
/// with a tool call.
#[tokio::test]
async fn test_openai_reasoning_includes_reasoning_in_history_for_text_only_turns() {
    let test_ctx = TestContextBuilder::new()
        .agent_mode(crate::agent_mode::AgentMode::AutoApprove)
        .build()
        .await;

    // Model returns thinking + text (no tool calls). For openai_reasoning, the reasoning
    // MUST be included in history so OpenAI can find the rs_... item on the next turn.
    let model = MockCompletionModel::new(vec![MockResponse::text_with_thinking(
        "The answer is 4.",
        "Simple arithmetic: 2+2=4",
    )]);

    let client = Arc::new(RwLock::new(LlmClient::Mock));
    let mut ctx = test_ctx.as_agentic_context_with_client(&client);
    ctx.llm.provider_name = "openai_reasoning";
    ctx.llm.model_name = "gpt-5.2";

    let initial_history = vec![rig::completion::Message::User {
        content: rig::one_or_many::OneOrMany::one(rig::message::UserContent::Text(
            rig::message::Text {
                text: "What is 2+2?".to_string(),
            },
        )),
    }];

    let result = run_agentic_loop_generic(
        &model,
        "You are a math tutor.",
        initial_history,
        openai_reasoning_sub_context(),
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Loop should succeed: {:?}", result.err());
    let (response, _reasoning, history, _usage) = result.unwrap();
    assert!(response.contains("4"), "Response should contain the answer");

    // For openai_reasoning, the Reasoning block MUST be present in the assistant history
    // even for text-only turns. OpenAI's server tracks rs_... IDs and requires them on
    // subsequent turns (failing with "Item 'rs_...' was provided without its required
    // following item" if a previously-seen rs_ ID is absent from the next request).
    let has_reasoning_in_history = history.iter().any(|msg| {
        if let rig::completion::Message::Assistant { content, .. } = msg {
            content
                .iter()
                .any(|c| matches!(c, rig::completion::AssistantContent::Reasoning(_)))
        } else {
            false
        }
    });
    assert!(
        has_reasoning_in_history,
        "openai_reasoning MUST include reasoning in history for text-only turns \
         so OpenAI can find the rs_... item on subsequent turns"
    );
}
