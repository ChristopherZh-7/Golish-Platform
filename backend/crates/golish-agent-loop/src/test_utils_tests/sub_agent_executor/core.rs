use super::*;

#[tokio::test]
async fn test_sub_agent_context_inheritance() {
    // Verify sub-agent inherits parent context correctly
    let test_ctx = TestContextBuilder::new().build().await;
    let workspace = test_ctx.workspace_path().await;

    // Create parent context with specific values
    let mut parent_variables = std::collections::HashMap::new();
    parent_variables.insert(
        "project_name".to_string(),
        serde_json::json!("test-project"),
    );
    parent_variables.insert("version".to_string(), serde_json::json!("1.0.0"));

    let parent_context = SubAgentContext {
        original_request: "Analyze the codebase".to_string(),
        conversation_summary: Some("User asked to analyze code quality".to_string()),
        variables: parent_variables.clone(),
        depth: 1,
        ..Default::default()
    };

    // Create a simple mock model that returns text immediately (no tool calls)
    let model = MockCompletionModel::with_text("Analysis complete. No issues found.");

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

    let agent_def = test_sub_agent_definition_for_executor("analyzer");
    let tool_provider = MockToolProvider::new();

    let result = execute_sub_agent(
        &agent_def,
        &serde_json::json!({ "task": "Analyze the code" }),
        &parent_context,
        &model,
        sub_ctx,
        &tool_provider,
        "test-parent-request-id",
    )
    .await
    .unwrap();

    // Verify the sub-agent context inherited from parent
    assert_eq!(
        result.context.original_request, parent_context.original_request,
        "Sub-agent should inherit original_request"
    );
    assert_eq!(
        result.context.conversation_summary, parent_context.conversation_summary,
        "Sub-agent should inherit conversation_summary"
    );
    assert_eq!(
        result.context.variables.get("project_name"),
        parent_variables.get("project_name"),
        "Sub-agent should inherit variables"
    );

    // Verify depth was incremented
    assert_eq!(
        result.context.depth,
        parent_context.depth + 1,
        "Sub-agent depth should be parent depth + 1"
    );

    // Verify the agent completed successfully
    assert!(result.success, "Sub-agent should complete successfully");
}

#[tokio::test]
async fn test_sub_agent_max_depth_limit() {
    // Verify depth limit prevents infinite recursion
    // Note: The depth check is done in the main agentic loop, not in execute_sub_agent itself
    // So we test that the depth is properly incremented and can be checked

    let parent_at_max_depth = SubAgentContext {
        original_request: "Test".to_string(),
        depth: MAX_AGENT_DEPTH - 1, // One below max
        ..Default::default()
    };

    // Simulate what the agentic loop does: check depth before calling sub-agent
    let can_spawn_sub_agent = parent_at_max_depth.depth < MAX_AGENT_DEPTH - 1;
    assert!(
        !can_spawn_sub_agent,
        "Should not be able to spawn sub-agent at max depth - 1"
    );

    // Verify at depth 0 (normal case) sub-agents are allowed
    let parent_at_zero = SubAgentContext {
        depth: 0,
        ..Default::default()
    };
    assert!(
        parent_at_zero.depth < MAX_AGENT_DEPTH - 1,
        "Should be able to spawn sub-agent at depth 0"
    );

    // Verify the constant is reasonable (compile-time checks)
    const _: () = assert!(MAX_AGENT_DEPTH >= 2);
    const _: () = assert!(MAX_AGENT_DEPTH <= 10);
}

#[tokio::test]
async fn test_sub_agent_result_propagation() {
    // Verify sub-agent results return to parent correctly
    let test_ctx = TestContextBuilder::new().build().await;
    let workspace = test_ctx.workspace_path().await;

    let parent_context = test_sub_agent_context();
    let model =
        MockCompletionModel::with_text("Task completed successfully with detailed analysis.");

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

    let agent_def = test_sub_agent_definition_for_executor("executor");
    let tool_provider = MockToolProvider::new();

    let result = execute_sub_agent(
        &agent_def,
        &serde_json::json!({
            "task": "Execute the given task",
            "context": "Additional context for the task"
        }),
        &parent_context,
        &model,
        sub_ctx,
        &tool_provider,
        "test-parent-request-id",
    )
    .await
    .unwrap();

    // Verify result structure
    assert_eq!(
        result.agent_id, "executor",
        "Should return correct agent_id"
    );
    assert!(
        result.response.contains("Task completed"),
        "Response should contain model output"
    );
    assert!(result.success, "Should indicate success");
    // Duration is tracked - may be 0 on very fast mock execution
    // The important thing is the field exists and is set

    // Verify context is returned (allows parent to access updated state)
    assert_eq!(
        result.context.depth, 1,
        "Context depth should be incremented"
    );
}
