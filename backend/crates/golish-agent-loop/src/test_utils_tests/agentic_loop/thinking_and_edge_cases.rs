use super::*;

#[tokio::test]
async fn test_agentic_loop_with_thinking() {
    // Test: Model returns thinking/reasoning content, properly handled
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    // Model returns text with thinking
    let model = MockCompletionModel::new(vec![MockResponse::text_with_thinking(
        "The answer is 42.",
        "Let me think about this carefully... The question of life, the universe, and everything...",
    )]);

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("What is the meaning of life?");

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

    // Verify the text response (not the thinking)
    assert_eq!(response, "The answer is 42.");

    // Verify Reasoning events were emitted
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let reasoning_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AiEvent::Reasoning { .. }))
        .collect();
    assert!(
        !reasoning_events.is_empty(),
        "Should have emitted Reasoning events for thinking content"
    );
}

#[tokio::test]
async fn test_agentic_loop_empty_response() {
    // Test: Model returns empty response, loop handles gracefully
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    // Model returns empty text
    let model = MockCompletionModel::with_text("");

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Say nothing");

    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        history,
        sub_ctx,
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Loop should handle empty responses");
    let (response, _reasoning, _final_history, _usage) = result.unwrap();

    // Empty response is valid
    assert_eq!(response, "");
}

#[tokio::test]
async fn test_agentic_loop_cancellation_via_timeout() {
    // Test: Cancellation behavior via external timeout
    // The agentic loop respects external cancellation via tokio timeout/select
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    // Create a model that returns a simple response
    let model = MockCompletionModel::with_text("This should complete quickly.");

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Quick test");

    // Run with a timeout to verify the loop can complete within reasonable time
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        run_agentic_loop_generic(
            &model,
            "You are a helpful assistant.",
            history,
            sub_ctx,
            &ctx,
        ),
    )
    .await;

    // Should complete within timeout
    assert!(result.is_ok(), "Loop should complete within timeout");
    let inner_result = result.unwrap();
    assert!(inner_result.is_ok(), "Loop result should be successful");
}

#[tokio::test]
async fn test_agentic_loop_multiple_tool_calls_in_single_response() {
    // Test: Model returns multiple tool calls in a single response (parallel tool calling)
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    // Create test files
    let ws = test_ctx.workspace_path().await;
    std::fs::write(ws.join("a.txt"), "Content A").unwrap();
    std::fs::write(ws.join("b.txt"), "Content B").unwrap();

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);

    // Model returns multiple tool calls at once
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_calls(vec![
            MockToolCall::new("read_file", serde_json::json!({"path": "a.txt"})),
            MockToolCall::new("read_file", serde_json::json!({"path": "b.txt"})),
        ]),
        MockResponse::text("I read both files: A and B."),
    ]);

    let sub_ctx = test_sub_agent_context();
    let history = initial_history_phase04("Read a.txt and b.txt simultaneously");

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
    assert!(response.contains("both files") || response.contains("A and B"));

    // Verify model was called twice (multi-tool + final)
    assert_eq!(model.call_count(), 2);

    // Verify we got two tool result events
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let tool_results: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AiEvent::ToolResult { .. }))
        .collect();
    assert_eq!(tool_results.len(), 2, "Should have two tool result events");
}

// ========================================================================
// Behavioral Equivalence Tests (Phase 0.6)
// ========================================================================
//
// These tests verify that the generic agentic loop functions produce the
// same behavior as their specialized counterparts. This is critical for
// the consolidation effort, as we will eventually deprecate the specialized
// implementations in favor of the generic ones.
//
// Note: Uses imports from Phase 0.4 section above (run_agentic_loop_generic,
// Message, UserContent)

/// Helper to create a simple user message for testing.
