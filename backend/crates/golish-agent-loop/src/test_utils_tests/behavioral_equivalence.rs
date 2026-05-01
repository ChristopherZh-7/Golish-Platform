use super::*;

fn user_message(text: &str) -> Message {
    Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: text.to_string(),
        })),
    }
}

#[tokio::test]
async fn test_behavioral_equivalence_text_response() {
    // Verify that the generic agentic loop produces the same text response
    // as the specialized version would.
    //
    // This test uses MockCompletionModel which returns predefined responses,
    // allowing us to verify that text streaming and accumulation work identically.

    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove) // Auto-approve to simplify testing
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let sub_ctx = test_sub_agent_context();

    // Create model that returns a simple text response
    let expected_text = "This is a test response from the model.";
    let model = MockCompletionModel::with_text(expected_text);

    // Run the generic agentic loop
    let initial_history = vec![user_message("Hello, how are you?")];
    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        initial_history.clone(),
        sub_ctx,
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Agentic loop should succeed");
    let (response_text, _reasoning, final_history, token_usage) = result.unwrap();

    // Verify the response text matches expected
    assert_eq!(
        response_text, expected_text,
        "Response text should match expected"
    );

    // For text-only responses (no tool calls), history contains original messages
    // The final response is returned separately, not appended to history
    assert!(!final_history.is_empty(), "History should contain messages");

    // Verify token usage was tracked
    assert!(token_usage.is_some(), "Token usage should be tracked");
    let usage = token_usage.unwrap();
    assert!(usage.input_tokens > 0, "Input tokens should be non-zero");
    assert!(usage.output_tokens > 0, "Output tokens should be non-zero");

    // Verify correct events were emitted
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();

    // Should have TextDelta events
    let text_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AiEvent::TextDelta { .. }))
        .collect();
    assert!(
        !text_events.is_empty(),
        "Should emit TextDelta events for streaming text"
    );
}

#[tokio::test]
async fn test_behavioral_equivalence_tool_execution() {
    // Verify that tool routing and execution works identically in the
    // generic loop compared to the specialized version.
    //
    // This tests that:
    // 1. Tool calls are correctly parsed from model responses
    // 2. Tools are executed via the tool registry
    // 3. Tool results are correctly added to message history

    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove) // Auto-approve all tools
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let sub_ctx = test_sub_agent_context();

    // Create a file in the workspace to read
    let ws = test_ctx.workspace_path().await;
    let test_file = ws.join("test_file.txt");
    std::fs::write(&test_file, "Hello from test file!").unwrap();

    // Create model that:
    // 1. First returns a read_file tool call
    // 2. Then returns a text response summarizing the file
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_call(
            "read_file",
            serde_json::json!({"path": test_file.to_string_lossy()}),
        ),
        MockResponse::text("I read the file and it says: Hello from test file!"),
    ]);

    // Run the generic agentic loop
    let initial_history = vec![user_message("Read the test file")];
    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        initial_history,
        sub_ctx,
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Agentic loop should succeed");
    let (response_text, _reasoning, final_history, _) = result.unwrap();

    // Verify the final response contains expected text
    assert!(
        response_text.contains("Hello from test file"),
        "Response should reference file contents"
    );

    // Verify history contains tool call and result
    let has_tool_call = final_history.iter().any(|msg| {
        if let Message::Assistant { content, .. } = msg {
            content
                .iter()
                .any(|c| matches!(c, AssistantContent::ToolCall(_)))
        } else {
            false
        }
    });
    assert!(has_tool_call, "History should contain tool call");

    let has_tool_result = final_history.iter().any(|msg| {
        if let Message::User { content } = msg {
            content
                .iter()
                .any(|c| matches!(c, UserContent::ToolResult(_)))
        } else {
            false
        }
    });
    assert!(has_tool_result, "History should contain tool result");

    // Verify tool-related events were emitted
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();

    // Note: ToolRequest is only captured to sidecar, not emitted to frontend
    // Check for ToolAutoApproved event (emitted via emit_event for policy Allow)
    // Since read_file is in ALLOW_TOOLS, it gets auto-approved by policy (since we're in auto-approve mode)
    let has_auto_approved = events.iter().any(|e| {
        matches!(e, AiEvent::ToolAutoApproved { tool_name, .. } if tool_name == "read_file")
    });
    assert!(has_auto_approved, "Should emit ToolAutoApproved event");

    // Should have ToolResult event
    let has_tool_result_event = events.iter().any(|e| {
        matches!(e, AiEvent::ToolResult { tool_name, success, .. } if tool_name == "read_file" && *success)
    });
    assert!(
        has_tool_result_event,
        "Should emit successful ToolResult event"
    );
}

#[tokio::test]
async fn test_behavioral_equivalence_event_sequence() {
    // Verify that the sequence of events emitted by the generic loop
    // matches the expected behavior pattern.
    //
    // Event sequence for a tool call should be:
    // 1. ToolRequest - when tool call is detected
    // 2. ToolAutoApproved/ToolApprovalRequest - approval decision
    // 3. ToolResult - after execution
    // 4. TextDelta events - for final response

    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let sub_ctx = test_sub_agent_context();

    // Create a file to read
    let ws = test_ctx.workspace_path().await;
    let test_file = ws.join("sequence_test.txt");
    std::fs::write(&test_file, "Sequence test content").unwrap();

    // Model returns tool call then text
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_call(
            "read_file",
            serde_json::json!({"path": test_file.to_string_lossy()}),
        ),
        MockResponse::text("Done reading the file."),
    ]);

    let initial_history = vec![user_message("Read the sequence test file")];
    let _ = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        initial_history,
        sub_ctx,
        &ctx,
    )
    .await;

    // Collect and analyze event sequence
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();

    // Find indices of key events
    // Note: ToolRequest is only captured to sidecar, not emitted to frontend channel
    let auto_approved_idx = events
        .iter()
        .position(|e| matches!(e, AiEvent::ToolAutoApproved { .. }));
    let tool_result_idx = events
        .iter()
        .position(|e| matches!(e, AiEvent::ToolResult { .. }));
    let text_delta_idx = events
        .iter()
        .position(|e| matches!(e, AiEvent::TextDelta { .. }));

    // Verify expected frontend events are present
    // read_file is in ALLOW_TOOLS so gets auto-approved by policy
    assert!(
        auto_approved_idx.is_some(),
        "Should have ToolAutoApproved event"
    );
    assert!(tool_result_idx.is_some(), "Should have ToolResult event");
    assert!(text_delta_idx.is_some(), "Should have TextDelta event");

    // Verify event ordering: Approved -> Result
    // (TextDelta can come before or after depending on streaming)
    let approved_idx = auto_approved_idx.unwrap();
    let result_idx = tool_result_idx.unwrap();

    assert!(
        approved_idx < result_idx,
        "ToolAutoApproved should come before ToolResult"
    );
}

#[tokio::test]
async fn test_behavioral_equivalence_error_handling() {
    // Verify that error handling in the generic loop matches expected behavior.
    //
    // Tests:
    // 1. Tool policy denials produce correct error results
    // 2. Planning mode restrictions work correctly
    // 3. Constraint violations are handled properly

    // Test 1: Policy denial (in Default mode, denied tools should be rejected)
    {
        let test_ctx = TestContextBuilder::new()
            .deny_tool("forbidden_tool")
            .agent_mode(AgentMode::Default)
            .build()
            .await;

        let client = test_llm_client();
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let sub_ctx = test_sub_agent_context();

        // Model tries to call a denied tool
        let model = MockCompletionModel::new(vec![
            MockResponse::tool_call("forbidden_tool", serde_json::json!({})),
            MockResponse::text("Understood, the tool was denied."),
        ]);

        let result = run_agentic_loop_generic(
            &model,
            "You are a helpful assistant.",
            vec![user_message("Use the forbidden tool")],
            sub_ctx,
            &ctx,
        )
        .await;

        assert!(result.is_ok(), "Loop should complete even with denied tool");

        // Verify denial event was emitted
        let mut test_ctx = test_ctx;
        let events = test_ctx.collect_events();
        let has_denied = events.iter().any(|e| {
            matches!(e, AiEvent::ToolDenied { tool_name, .. } if tool_name == "forbidden_tool")
        });
        assert!(has_denied, "Should emit ToolDenied event for policy denial");
    }

    // Test 2: Planning mode restriction
    {
        let test_ctx = TestContextBuilder::new()
            .agent_mode(AgentMode::Planning)
            .build()
            .await;

        let client = test_llm_client();
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let sub_ctx = test_sub_agent_context();

        // Model tries to call a write tool in planning mode
        let model = MockCompletionModel::new(vec![
            MockResponse::tool_call(
                "write_file",
                serde_json::json!({"path": "test.txt", "content": "test"}),
            ),
            MockResponse::text("Cannot write in planning mode."),
        ]);

        let result = run_agentic_loop_generic(
            &model,
            "You are a helpful assistant.",
            vec![user_message("Write a file")],
            sub_ctx,
            &ctx,
        )
        .await;

        assert!(
            result.is_ok(),
            "Loop should complete even with planning mode denial"
        );

        // Verify denial event was emitted
        let mut test_ctx = test_ctx;
        let events = test_ctx.collect_events();
        let has_planning_denied = events.iter().any(|e| {
            matches!(e, AiEvent::ToolDenied { reason, .. } if reason.to_lowercase().contains("planning mode"))
        });
        assert!(
            has_planning_denied,
            "Should emit ToolDenied event for planning mode restriction"
        );
    }

    // Test 3: Constraint violation (e.g., blocked URL in web_fetch)
    {
        let test_ctx = TestContextBuilder::new()
            .agent_mode(AgentMode::AutoApprove)
            .build()
            .await;

        let client = test_llm_client();
        let ctx = test_ctx.as_agentic_context_with_client(&client);
        let sub_ctx = test_sub_agent_context();

        // Model tries to fetch localhost (blocked by default constraints)
        let model = MockCompletionModel::new(vec![
            MockResponse::tool_call(
                "web_fetch",
                serde_json::json!({"url": "http://localhost:8080/api"}),
            ),
            MockResponse::text("The URL was blocked."),
        ]);

        let result = run_agentic_loop_generic(
            &model,
            "You are a helpful assistant.",
            vec![user_message("Fetch localhost")],
            sub_ctx,
            &ctx,
        )
        .await;

        assert!(
            result.is_ok(),
            "Loop should complete even with constraint violation"
        );

        // Verify denial event was emitted
        let mut test_ctx = test_ctx;
        let events = test_ctx.collect_events();
        let has_constraint_denied = events
            .iter()
            .any(|e| matches!(e, AiEvent::ToolDenied { .. }));
        assert!(
            has_constraint_denied,
            "Should emit ToolDenied event for constraint violation"
        );
    }
}

#[tokio::test]
async fn test_behavioral_equivalence_context_management() {
    // Verify that context window management works correctly in the generic loop.
    //
    // Tests:
    // 1. Token usage is tracked and accumulated across iterations
    // 2. Context manager is updated with message history
    // 3. Large tool responses are truncated appropriately

    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let sub_ctx = test_sub_agent_context();

    // Create a file with substantial content
    let ws = test_ctx.workspace_path().await;
    let test_file = ws.join("context_test.txt");
    let large_content = "This is line 1.\n".repeat(100);
    std::fs::write(&test_file, &large_content).unwrap();

    // Model performs multiple tool calls to accumulate tokens
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_call(
            "read_file",
            serde_json::json!({"path": test_file.to_string_lossy()}),
        ),
        MockResponse::text("I read the file with 100 lines."),
    ]);

    let initial_history = vec![user_message("Read the context test file")];
    let result = run_agentic_loop_generic(
        &model,
        "You are a helpful assistant.",
        initial_history,
        sub_ctx,
        &ctx,
    )
    .await;

    assert!(result.is_ok(), "Agentic loop should succeed");
    let (_, _reasoning, final_history, token_usage) = result.unwrap();

    // Verify token usage accumulation
    let usage = token_usage.expect("Should have token usage");
    assert!(
        usage.total() > 0,
        "Total tokens should be non-zero after tool execution"
    );

    // Verify message history was properly maintained
    // Should have: initial user message, assistant with tool call, user with tool result, final assistant response
    assert!(
        final_history.len() >= 3,
        "History should contain multiple messages after tool execution"
    );

    // Verify context manager state was updated
    let ctx_stats = ctx.context_manager.stats().await;
    assert!(
        ctx_stats.total_tokens > 0,
        "Context manager should track estimated tokens"
    );

    // Collect events and verify context-related events if any warnings/truncations occurred
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();

    // Check for truncation event (may or may not occur depending on response size)
    let truncation_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AiEvent::ToolResponseTruncated { .. }))
        .collect();

    // If there was a truncation event, verify it has valid data
    for event in truncation_events {
        if let AiEvent::ToolResponseTruncated {
            original_tokens,
            truncated_tokens,
            ..
        } = event
        {
            assert!(
                *truncated_tokens <= *original_tokens,
                "Truncated tokens should be <= original"
            );
        }
    }
}
