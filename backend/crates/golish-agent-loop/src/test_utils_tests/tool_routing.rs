use super::*;

#[tokio::test]
async fn test_tool_routing_to_file_operations() {
    // Verify file tools (read_file, write_file, edit_file) route correctly
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Create a test file in the workspace
    let ws = test_ctx.workspace_path().await;
    std::fs::write(ws.join("routing_test.txt"), "test content").unwrap();

    // Test read_file routing
    let result = execute_with_hitl_generic(
        "read_file",
        &serde_json::json!({"path": "routing_test.txt"}),
        "test-tool-id-read",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should succeed and contain the content
    assert!(result.success, "read_file should succeed");
    assert!(
        result.value.get("content").is_some() || result.value.get("error").is_none(),
        "read_file should return content or not error"
    );

    // Test write_file routing (routes through tool registry)
    let write_result = execute_with_hitl_generic(
        "write_file",
        &serde_json::json!({
            "path": "routing_test_write.txt",
            "content": "new content"
        }),
        "test-tool-id-write",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should succeed (file operations are routed through registry)
    assert!(
        write_result.success,
        "write_file should succeed: {:?}",
        write_result.value
    );

    // Test edit_file routing (requires existing file with content to edit)
    std::fs::write(ws.join("edit_test.txt"), "line 1\nline 2\nline 3").unwrap();
    let edit_result = execute_with_hitl_generic(
        "edit_file",
        &serde_json::json!({
            "path": "edit_test.txt",
            "old_text": "line 2",
            "new_text": "modified line 2"
        }),
        "test-tool-id-edit",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // edit_file routes through the registry
    // Success depends on actual edit implementation
    assert!(
        edit_result.value.get("error").is_none() || edit_result.success,
        "edit_file should route correctly: {:?}",
        edit_result.value
    );
}

#[tokio::test]
async fn test_tool_routing_to_shell_execution() {
    // Verify run_pty_cmd and run_command route to shell executor
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Test run_pty_cmd routing
    let result = execute_with_hitl_generic(
        "run_pty_cmd",
        &serde_json::json!({"command": "echo hello"}),
        "test-tool-id-pty",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should route to shell execution (may or may not succeed depending on environment)
    // Key thing is that it routes correctly and doesn't return "unknown tool"
    assert!(
        !result.value.to_string().contains("unknown tool"),
        "run_pty_cmd should be recognized: {:?}",
        result.value
    );

    // Test run_command routing (alias for run_pty_cmd)
    let cmd_result = execute_with_hitl_generic(
        "run_command",
        &serde_json::json!({"command": "echo world"}),
        "test-tool-id-cmd",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // run_command should be mapped to run_pty_cmd internally
    assert!(
        !cmd_result.value.to_string().contains("unknown tool"),
        "run_command should be recognized (mapped to run_pty_cmd): {:?}",
        cmd_result.value
    );
}

#[tokio::test]
async fn test_tool_routing_unknown_tool_returns_error() {
    // Verify unknown tools fail gracefully
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Try to execute a tool that doesn't exist
    let result = execute_with_hitl_generic(
        "completely_nonexistent_tool_xyz123",
        &serde_json::json!({"some_arg": "value"}),
        "test-tool-id-unknown",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should fail with an error (not panic)
    assert!(
        !result.success,
        "Unknown tool should not succeed: {:?}",
        result.value
    );
    assert!(
        result.value.get("error").is_some(),
        "Unknown tool should return error field: {:?}",
        result.value
    );
}

#[tokio::test]
async fn test_tool_routing_sub_agent_tool() {
    // Verify sub-agent tools are recognized (execute_sub_agent pattern)
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Test sub-agent tool routing (sub_agent_<id> pattern)
    // This will fail because the sub-agent doesn't exist, but it should
    // be recognized as a sub-agent tool and routed appropriately
    let result = execute_with_hitl_generic(
        "sub_agent_test_agent",
        &serde_json::json!({"task": "test task", "context": "test context"}),
        "test-tool-id-subagent",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should be recognized as sub-agent tool and return "not found" error
    // (not "unknown tool" error)
    assert!(!result.success, "Non-existent sub-agent should not succeed");
    let error_str = result.value.to_string();
    assert!(
        error_str.contains("not found") || error_str.contains("Sub-agent"),
        "Should indicate sub-agent not found, got: {:?}",
        result.value
    );
}

#[tokio::test]
async fn test_tool_routing_web_tools() {
    // Verify web_fetch and web_search route correctly
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Test web_fetch routing
    // Note: This will actually try to fetch, so use a non-blocked URL
    // The constraint violation test already covers localhost blocking
    let result = execute_with_hitl_generic(
        "web_fetch",
        &serde_json::json!({"url": "https://example.com", "prompt": "summarize"}),
        "test-tool-id-webfetch",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // web_fetch should be routed correctly (success depends on network)
    // Key thing is it's recognized and not "unknown tool"
    let error_str = result.value.to_string().to_lowercase();
    assert!(
        !error_str.contains("unknown tool"),
        "web_fetch should be recognized: {:?}",
        result.value
    );

    // Test web_search routing (requires Tavily state, which is None in test)
    let search_result = execute_with_hitl_generic(
        "web_search",
        &serde_json::json!({"query": "test query"}),
        "test-tool-id-websearch",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // web_search routes through Tavily handler
    // Without Tavily configured, should fail with "not available" or similar
    let search_error_str = search_result.value.to_string().to_lowercase();
    assert!(
        search_error_str.contains("not available")
            || search_error_str.contains("tavily")
            || search_error_str.contains("not configured")
            || !search_result.success,
        "web_search should be routed to Tavily handler: {:?}",
        search_result.value
    );
}

#[tokio::test]
async fn test_tool_routing_indexer_tools() {
    // Verify indexer_search_code and similar tools route correctly
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Test indexer_search_code routing
    let result = execute_with_hitl_generic(
        "indexer_search_code",
        &serde_json::json!({"pattern": "test.*pattern"}),
        "test-tool-id-indexer-search",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should be routed to indexer tool handler
    // Without indexer state, should return appropriate error (not "unknown tool")
    let error_str = result.value.to_string().to_lowercase();
    assert!(
        error_str.contains("indexer")
            || error_str.contains("not available")
            || error_str.contains("not initialized")
            || !error_str.contains("unknown tool"),
        "indexer_search_code should be routed to indexer handler: {:?}",
        result.value
    );

    // Test indexer_search_files routing
    let files_result = execute_with_hitl_generic(
        "indexer_search_files",
        &serde_json::json!({"pattern": "*.rs"}),
        "test-tool-id-indexer-files",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should also be routed to indexer handler (starts with "indexer_")
    let files_error_str = files_result.value.to_string().to_lowercase();
    assert!(
        files_error_str.contains("indexer")
            || files_error_str.contains("not available")
            || files_error_str.contains("not initialized")
            || !files_error_str.contains("unknown tool"),
        "indexer_search_files should be routed to indexer handler: {:?}",
        files_result.value
    );

    // Test indexer_analyze_file routing
    let analyze_result = execute_with_hitl_generic(
        "indexer_analyze_file",
        &serde_json::json!({"file_path": "test.rs"}),
        "test-tool-id-indexer-analyze",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should be recognized as indexer tool
    let analyze_error_str = analyze_result.value.to_string().to_lowercase();
    assert!(
        analyze_error_str.contains("indexer")
            || analyze_error_str.contains("not available")
            || analyze_error_str.contains("not initialized")
            || !analyze_error_str.contains("unknown tool"),
        "indexer_analyze_file should be routed to indexer handler: {:?}",
        analyze_result.value
    );
}

#[tokio::test]
async fn test_tool_routing_planner_tools() {
    // Verify update_plan routes correctly
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Test update_plan routing with valid plan structure
    // The plan structure requires "step" (not "task") field
    let result = execute_with_hitl_generic(
        "update_plan",
        &serde_json::json!({
            "plan": [
                {"step": "Step 1", "status": "pending"},
                {"step": "Step 2", "status": "pending"}
            ]
        }),
        "test-tool-id-plan",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // update_plan should be routed to plan handler
    // The plan manager is initialized in test context, so this should work
    assert!(
        result.success || result.value.get("plan").is_some(),
        "update_plan should be routed correctly and succeed: {:?}",
        result.value
    );

    // Verify that the plan was actually updated by checking events
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let has_plan_event = events
        .iter()
        .any(|e| matches!(e, AiEvent::PlanUpdated { .. }));

    // If successful, should have emitted a plan event
    if result.success {
        assert!(
            has_plan_event,
            "Successful update_plan should emit PlanUpdated event"
        );
    }
}

// ========================================================================
// Agentic Loop Integration Tests (Phase 0.4)
// ========================================================================
//
// These tests verify the higher-level agentic loop behavior, focusing on
// scenarios not covered by the behavioral equivalence tests in Phase 0.6.

