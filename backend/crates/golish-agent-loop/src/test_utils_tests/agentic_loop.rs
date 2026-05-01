use super::*;

use crate::agentic_loop::run_agentic_loop_generic;
use rig::completion::Message;
use rig::message::UserContent;
use rig::one_or_many::OneOrMany;

/// Helper to create initial chat history with a user message.
fn initial_history_phase04(user_message_text: &str) -> Vec<Message> {
    vec![Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: user_message_text.to_string(),
        })),
    }]
}

#[tokio::test]
async fn test_agentic_loop_simple_text_response() {
    // Test: Model returns text only, loop completes with that response
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let model = MockCompletionModel::with_text("Hello! This is a simple text response.");
    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Say hello");

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        history,
        sub_ctx,
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Agentic loop should complete successfully");
    let (response, _reasoning, final_history, usage) = result.unwrap();

    // Verify the response text
    assert_eq!(response, "Hello! This is a simple text response.");

    // Verify token usage was tracked
    assert!(usage.is_some());
    let usage = usage.unwrap();
    assert!(usage.input_tokens > 0 || usage.output_tokens > 0);

    // Verify history contains the original user message
    assert!(!final_history.is_empty());

    // Verify model was called exactly once
    assert_eq!(model.call_count(), 1);
}

#[tokio::test]
async fn test_agentic_loop_single_tool_call() {
    // Test: Model returns one tool call, executes it, then returns text
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    // Create a test file in the workspace
    let ws = test_ctx.workspace_path().await;
    std::fs::write(ws.join("test.txt"), "Hello from test file").unwrap();

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    // Model: first returns tool call, then returns text response
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_call("read_file", serde_json::json!({"path": "test.txt"})),
        MockResponse::text("I read the file. It contains: Hello from test file"),
    ]);

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Read the test.txt file");

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        history,
        sub_ctx,
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Agentic loop should complete successfully");
    let (response, _reasoning, final_history, _usage) = result.unwrap();

    // Verify the final response
    assert!(response.contains("Hello from test file") || response.contains("I read the file"));

    // Verify model was called twice (tool call + final response)
    assert_eq!(model.call_count(), 2);

    // Verify history grew (user + assistant with tool + user with result + assistant final)
    assert!(final_history.len() >= 3);
}

#[tokio::test]
async fn test_agentic_loop_multiple_tool_calls() {
    // Test: Model returns multiple tool calls in sequence (one per iteration)
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    // Create test files
    let ws = test_ctx.workspace_path().await;
    std::fs::write(ws.join("file1.txt"), "Content of file 1").unwrap();
    std::fs::write(ws.join("file2.txt"), "Content of file 2").unwrap();

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    // Model: calls two tools in sequence, then returns final response
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_call("read_file", serde_json::json!({"path": "file1.txt"})),
        MockResponse::tool_call("read_file", serde_json::json!({"path": "file2.txt"})),
        MockResponse::text("I read both files successfully."),
    ]);

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Read both file1.txt and file2.txt");

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        history,
        sub_ctx,
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Agentic loop should complete successfully");
    let (response, _reasoning, _final_history, _usage) = result.unwrap();

    // Verify the final response
    assert!(response.contains("both files") || response.contains("successfully"));

    // Verify model was called three times
    assert_eq!(model.call_count(), 3);
}

#[tokio::test]
async fn test_agentic_loop_tool_then_text() {
    // Test: Model calls tool, receives result, then returns text incorporating the result
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    // Create a file with specific content
    let ws = test_ctx.workspace_path().await;
    std::fs::write(ws.join("data.json"), r#"{"key": "value123"}"#).unwrap();

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    let model = MockCompletionModel::with_tool_call_then_text(
        "read_file",
        serde_json::json!({"path": "data.json"}),
        "The file contains JSON with key='value123'.",
    );

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("What's in data.json?");

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        history,
        sub_ctx,
        &ctx,
    )
    .await;

    assert!(result.is_ok());
    let (response, _reasoning, _final_history, _usage) = result.unwrap();

    // Verify the response incorporates the expected content
    assert!(response.contains("value123") || response.contains("JSON"));
}

#[tokio::test]
async fn test_agentic_loop_max_iterations_reached() {
    // Test: Loop stops when max iterations are hit (MAX_TOOL_ITERATIONS = 100)
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    // Create file for read_file tool
    let ws = test_ctx.workspace_path().await;
    std::fs::write(ws.join("endless.txt"), "keep reading me").unwrap();

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    // Create a model that always returns tool calls (would loop forever without limit)
    let mut responses: Vec<MockResponse> = Vec::new();
    for _ in 0..150 {
        responses.push(MockResponse::tool_call(
            "read_file",
            serde_json::json!({"path": "endless.txt"}),
        ));
    }
    let model = MockCompletionModel::new(responses);

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Keep reading the file");

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        history,
        sub_ctx,
        &ctx,
    )
    .await;

    // The loop should complete (not hang)
    assert!(
        result.is_ok(),
        "Loop should complete even when hitting max iterations"
    );

    // Collect events to verify max iterations event was emitted
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let has_max_iterations_error = events.iter().any(
        |e| matches!(e, AiEvent::Error { error_type, .. } if error_type == "max_iterations"),
    );
    assert!(
        has_max_iterations_error,
        "Should emit max_iterations error event"
    );
}

#[tokio::test]
async fn test_agentic_loop_context_events_emitted() {
    // Test: Verify TextDelta events are emitted during streaming
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let model = MockCompletionModel::with_text("Event test response");
    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Test events");

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        history,
        sub_ctx,
        &ctx,
    )
    .await;
    assert!(result.is_ok());

    // Collect all events
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();

    // Verify TextDelta events were emitted
    let text_delta_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AiEvent::TextDelta { .. }))
        .collect();
    assert!(
        !text_delta_events.is_empty(),
        "Should have emitted TextDelta events"
    );

    // Verify the accumulated text matches
    let final_text_delta = text_delta_events.last();
    if let Some(AiEvent::TextDelta { accumulated, .. }) = final_text_delta {
        assert_eq!(accumulated, "Event test response");
    }
}

#[tokio::test]
async fn test_agentic_loop_tool_error_handling() {
    // Test: Tool returns error, loop handles it gracefully and continues
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    // Don't create the file - read_file will fail
    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    // Model tries to read non-existent file, then responds to the error
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_call("read_file", serde_json::json!({"path": "nonexistent.txt"})),
        MockResponse::text("The file doesn't exist, I received an error."),
    ]);

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Read nonexistent.txt");

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        history,
        sub_ctx,
        &ctx,
    )
    .await;

    // Should complete successfully (error is passed back to LLM)
    assert!(result.is_ok(), "Loop should handle tool errors gracefully");
    let (response, _reasoning, _final_history, _usage) = result.unwrap();

    // The model should have received the error and responded
    assert!(response.contains("error") || response.contains("doesn't exist"));

    // Verify ToolResult event shows failure
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let tool_results: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AiEvent::ToolResult { success: false, .. }))
        .collect();
    assert!(
        !tool_results.is_empty(),
        "Should have a failed tool result event"
    );
}

mod thinking_and_edge_cases;
