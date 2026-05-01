use super::*;

#[tokio::test]
async fn test_sub_agent_events_emitted() {
    // Verify SubAgentStarted, SubAgentCompleted events are emitted
    let test_ctx = TestContextBuilder::new().build().await;
    let workspace = test_ctx.workspace_path().await;

    let parent_context = test_sub_agent_context();
    let model = MockCompletionModel::with_text("Events test complete.");

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let tool_registry = Arc::new(RwLock::new(ToolRegistry::new(workspace.clone()).await));

    let sub_ctx = SubAgentExecutorContext {
        event_tx: &event_tx,
        tool_registry: &tool_registry,
        workspace: &Arc::new(RwLock::new(workspace)),
        provider_name: "mock",
        model_name: "mock-model",
        session_id: None,
        transcript_base_dir: None,
        api_request_stats: None,
        briefing: None,
        temperature_override: None,
        max_tokens_override: None,
        top_p_override: None,
        chain_persistence: None,
        sub_agent_registry: None,
        post_shell_hook: None,
    };

    let agent_def = test_sub_agent_definition_for_executor("event_tester");
    let tool_provider = MockToolProvider::new();

    let _result = execute_sub_agent(
        &agent_def,
        &serde_json::json!({ "task": "Test event emission" }),
        &parent_context,
        &model,
        sub_ctx,
        &tool_provider,
        "test-parent-request-id",
    )
    .await
    .unwrap();

    // Collect all emitted events
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }

    // Verify SubAgentStarted event was emitted
    let started_event = events.iter().find(|e| {
        matches!(e, AiEvent::SubAgentStarted { agent_id, .. } if agent_id == "event_tester")
    });
    assert!(started_event.is_some(), "Should emit SubAgentStarted event");

    // Verify SubAgentStarted has correct fields
    if let Some(AiEvent::SubAgentStarted {
        agent_id,
        agent_name,
        task,
        depth,
        ..
    }) = started_event
    {
        assert_eq!(agent_id, "event_tester");
        assert!(agent_name.contains("Test Agent"));
        assert_eq!(task, "Test event emission");
        assert_eq!(*depth, 1); // Parent depth was 0
    }

    // Verify SubAgentCompleted event was emitted
    let completed_event = events.iter().find(|e| {
        matches!(e, AiEvent::SubAgentCompleted { agent_id, .. } if agent_id == "event_tester")
    });
    assert!(
        completed_event.is_some(),
        "Should emit SubAgentCompleted event"
    );

    // Verify SubAgentCompleted has correct fields
    if let Some(AiEvent::SubAgentCompleted {
        agent_id,
        response,
        duration_ms: _,
        parent_request_id: _,
    }) = completed_event
    {
        assert_eq!(agent_id, "event_tester");
        assert!(response.contains("Events test complete"));
        // duration_ms may be 0 on very fast mock execution
    }
}

#[tokio::test]
async fn test_sub_agent_error_handling() {
    // Verify errors in sub-agent are handled gracefully
    let test_ctx = TestContextBuilder::new().build().await;
    let workspace = test_ctx.workspace_path().await;

    let parent_context = test_sub_agent_context();

    // Create a model that simulates an error by returning empty responses repeatedly
    // until max_iterations is hit (which triggers a final toolless summary call)
    let model = MockCompletionModel::new(vec![
        // Return tool call that will fail
        MockResponse::tool_call("nonexistent_tool", serde_json::json!({ "arg": "value" })),
        // Continue returning tool calls to hit max_iterations
        MockResponse::tool_call("another_nonexistent_tool", serde_json::json!({})),
        MockResponse::tool_call("yet_another_tool", serde_json::json!({})),
        // After max_iterations (3), a final toolless call is made for summary
        MockResponse::text("Summary of work done before hitting iteration limit."),
    ]);

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let tool_registry = Arc::new(RwLock::new(ToolRegistry::new(workspace.clone()).await));

    let sub_ctx = SubAgentExecutorContext {
        event_tx: &event_tx,
        tool_registry: &tool_registry,
        workspace: &Arc::new(RwLock::new(workspace)),
        provider_name: "mock",
        model_name: "mock-model",
        session_id: None,
        transcript_base_dir: None,
        api_request_stats: None,
        briefing: None,
        temperature_override: None,
        max_tokens_override: None,
        top_p_override: None,
        chain_persistence: None,
        sub_agent_registry: None,
        post_shell_hook: None,
    };

    // Create agent with very low max_iterations to trigger the error path
    let agent_def = SubAgentDefinition::new(
        "error_tester",
        "Error Test Agent",
        "Tests error handling",
        "You are a test agent.",
    )
    .with_tools(vec![]) // No tools allowed
    .with_max_iterations(3);

    let tool_provider = MockToolProvider::new();

    let result = execute_sub_agent(
        &agent_def,
        &serde_json::json!({ "task": "Trigger error condition" }),
        &parent_context,
        &model,
        sub_ctx,
        &tool_provider,
        "test-parent-request-id",
    )
    .await
    .unwrap();

    // The sub-agent should complete (not panic) even with max iterations hit
    // Result is returned with the summary from the final toolless call
    assert!(
        result.agent_id == "error_tester",
        "Should return a result with correct agent_id even when max iterations hit"
    );
    assert!(
        result.success,
        "Should return success when max iterations hit (final summary call succeeds)"
    );
    assert!(
        result.response.contains("Summary of work done"),
        "Response should contain the summary from the final toolless call"
    );

    // Collect events and verify SubAgentCompleted was emitted (not SubAgentError)
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }

    let completed_event = events.iter().find(
        |e| matches!(e, AiEvent::SubAgentCompleted { agent_id, .. } if agent_id == "error_tester"),
    );

    assert!(
        completed_event.is_some(),
        "Should emit SubAgentCompleted event when max iterations reached"
    );
}

#[tokio::test]
async fn test_sub_agent_tool_restrictions() {
    // Verify sub-agents respect tool policies (allowed_tools)
    let test_ctx = TestContextBuilder::new().build().await;
    let workspace = test_ctx.workspace_path().await;

    let parent_context = test_sub_agent_context();
    let model = MockCompletionModel::with_text("Tool restriction test complete.");

    let (event_tx, _event_rx) = mpsc::unbounded_channel();
    let tool_registry = Arc::new(RwLock::new(ToolRegistry::new(workspace.clone()).await));

    let sub_ctx = SubAgentExecutorContext {
        event_tx: &event_tx,
        tool_registry: &tool_registry,
        workspace: &Arc::new(RwLock::new(workspace)),
        provider_name: "mock",
        model_name: "mock-model",
        session_id: None,
        transcript_base_dir: None,
        api_request_stats: None,
        briefing: None,
        temperature_override: None,
        max_tokens_override: None,
        top_p_override: None,
        chain_persistence: None,
        sub_agent_registry: None,
        post_shell_hook: None,
    };

    // Create agent with restricted tools (only read_file allowed)
    let agent_def = SubAgentDefinition::new(
        "restricted_agent",
        "Restricted Agent",
        "Agent with limited tools",
        "You are a restricted agent with only read access.",
    )
    .with_tools(vec!["read_file".to_string()]) // Only read_file allowed
    .with_max_iterations(5);

    // Create tool provider with more tools than allowed
    let tool_provider = MockToolProvider::with_allowed_tools(vec![
        "read_file".to_string(),
        "write_file".to_string(),
        "delete_file".to_string(),
        "glob".to_string(),
    ]);

    // Get filtered tools
    let all_tools = tool_provider.get_all_tool_definitions();
    let filtered_tools =
        tool_provider.filter_tools_by_allowed(all_tools, &agent_def.allowed_tools);

    // Verify tool filtering works correctly
    assert_eq!(
        filtered_tools.len(),
        1,
        "Should only have 1 tool after filtering"
    );
    assert_eq!(
        filtered_tools[0].name, "read_file",
        "Filtered tool should be read_file"
    );

    // Execute sub-agent to verify it works with restricted tools
    let result = execute_sub_agent(
        &agent_def,
        &serde_json::json!({ "task": "Read a file" }),
        &parent_context,
        &model,
        sub_ctx,
        &tool_provider,
        "test-parent-request-id",
    )
    .await
    .unwrap();

    assert!(
        result.success,
        "Sub-agent should succeed with restricted tools"
    );
}

#[tokio::test]
async fn test_sub_agent_timeout_behavior() {
    // Verify sub-agents stop appropriately when hitting max_iterations
    // and make a final toolless call for summary
    let test_ctx = TestContextBuilder::new().build().await;
    let workspace = test_ctx.workspace_path().await;

    let parent_context = test_sub_agent_context();

    // Create model that continuously returns tool calls to simulate long-running operation
    // The last response is the summary from the final toolless call
    let model = MockCompletionModel::new(vec![
        MockResponse::tool_call("read_file", serde_json::json!({ "path": "file1.txt" })),
        MockResponse::tool_call("read_file", serde_json::json!({ "path": "file2.txt" })),
        MockResponse::tool_call("read_file", serde_json::json!({ "path": "file3.txt" })),
        MockResponse::tool_call("read_file", serde_json::json!({ "path": "file4.txt" })),
        MockResponse::tool_call("read_file", serde_json::json!({ "path": "file5.txt" })),
        // More calls than max_iterations; final toolless call returns summary
    ]);

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let tool_registry = Arc::new(RwLock::new(ToolRegistry::new(workspace.clone()).await));

    let sub_ctx = SubAgentExecutorContext {
        event_tx: &event_tx,
        tool_registry: &tool_registry,
        workspace: &Arc::new(RwLock::new(workspace)),
        provider_name: "mock",
        model_name: "mock-model",
        session_id: None,
        transcript_base_dir: None,
        api_request_stats: None,
        briefing: None,
        temperature_override: None,
        max_tokens_override: None,
        top_p_override: None,
        chain_persistence: None,
        sub_agent_registry: None,
        post_shell_hook: None,
    };

    // Create agent with very low max_iterations to simulate timeout
    let agent_def = SubAgentDefinition::new(
        "timeout_tester",
        "Timeout Test Agent",
        "Tests timeout via max_iterations",
        "You are a test agent.",
    )
    .with_tools(vec!["read_file".to_string()])
    .with_max_iterations(2); // Very low to trigger "timeout"

    let tool_provider = MockToolProvider::new();

    let start = std::time::Instant::now();
    let result = execute_sub_agent(
        &agent_def,
        &serde_json::json!({ "task": "Read many files" }),
        &parent_context,
        &model,
        sub_ctx,
        &tool_provider,
        "test-parent-request-id",
    )
    .await
    .unwrap();
    let elapsed = start.elapsed();

    // Collect events
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }

    // Verify SubAgentCompleted was emitted (final toolless call produces a response)
    let completed_event = events.iter().find(|e| {
        matches!(e, AiEvent::SubAgentCompleted { agent_id, .. } if agent_id == "timeout_tester")
    });
    assert!(
        completed_event.is_some(),
        "Should emit SubAgentCompleted when max_iterations exceeded"
    );

    // Verify it didn't take too long (should be fast since it's mocked)
    assert!(
        elapsed.as_secs() < 5,
        "Sub-agent should complete quickly after hitting max_iterations"
    );

    // Verify the result is returned even when "timed out"
    // Agent ID should match to confirm we got a valid result
    assert_eq!(
        result.agent_id, "timeout_tester",
        "Should return result with correct agent_id when max_iterations hit"
    );
    assert!(
        result.success,
        "Should return success when max_iterations hit (final summary call made)"
    );
}
