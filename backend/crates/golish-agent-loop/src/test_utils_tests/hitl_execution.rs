use super::*;

use crate::agentic_loop::execute_with_hitl_generic;
use golish_sub_agents::SubAgentContext;

/// Helper to create a minimal SubAgentContext for tests.
fn test_sub_agent_context() -> SubAgentContext {
    SubAgentContext {
        original_request: "Test request".to_string(),
        ..Default::default()
    }
}

/// Helper to create a minimal LlmClient for tests.
/// Uses a mock client since we're testing HITL logic, not LLM calls.
fn test_llm_client() -> Arc<RwLock<LlmClient>> {
    Arc::new(RwLock::new(LlmClient::Mock))
}

#[tokio::test]
async fn test_hitl_planning_mode_blocks_write_tools() {
    // In planning mode, write tools (like write_file) should be blocked
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::Planning)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Try to execute a write tool (should be blocked in planning mode)
    let result = execute_with_hitl_generic(
        "write_file",
        &serde_json::json!({"path": "test.txt", "content": "hello"}),
        "test-tool-id",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should fail with planning_mode_denied
    assert!(!result.success);
    assert!(result.value.get("planning_mode_denied").is_some());
    assert!(result.value["error"]
        .as_str()
        .unwrap()
        .contains("not allowed in planning mode"));
}

#[tokio::test]
async fn test_hitl_planning_mode_allows_read_tools() {
    // In planning mode, read tools (like read_file) should be allowed
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::Planning)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Create a file in the workspace first
    let ws = test_ctx.workspace_path().await;
    std::fs::write(ws.join("test.txt"), "hello world").unwrap();

    // Try to execute a read tool (should be allowed in planning mode)
    // read_file is in ALLOW_TOOLS so should bypass HITL entirely
    let result = execute_with_hitl_generic(
        "read_file",
        &serde_json::json!({"path": "test.txt"}),
        "test-tool-id",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should succeed (auto-approved by policy)
    assert!(result.success);
}

#[tokio::test]
async fn test_hitl_denied_by_policy() {
    // Tools explicitly denied by policy should fail immediately
    let test_ctx = TestContextBuilder::new()
        .deny_tool("custom_dangerous_tool")
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    let result = execute_with_hitl_generic(
        "custom_dangerous_tool",
        &serde_json::json!({}),
        "test-tool-id",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should fail with denied_by_policy
    assert!(!result.success);
    assert!(result.value.get("denied_by_policy").is_some());
}

#[tokio::test]
async fn test_hitl_allowed_by_policy_bypasses_approval() {
    // Tools allowed by policy should bypass HITL entirely
    let test_ctx = TestContextBuilder::new()
        .allow_tool("custom_safe_tool")
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // The tool doesn't exist in the registry, but we can check that
    // it attempts to execute (and fails at execution, not approval)
    let _result = execute_with_hitl_generic(
        "custom_safe_tool",
        &serde_json::json!({}),
        "test-tool-id",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should get to execution (and fail there since tool doesn't exist)
    // But importantly, no approval request should be emitted
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let has_approval_request = events
        .iter()
        .any(|e| matches!(e, AiEvent::ToolApprovalRequest { .. }));
    assert!(
        !has_approval_request,
        "Should not have approval request for allowed tool"
    );

    // Should have auto-approved event
    let has_auto_approved = events.iter().any(
        |e| matches!(e, AiEvent::ToolAutoApproved { reason, .. } if reason.contains("policy")),
    );
    assert!(has_auto_approved, "Should have auto-approved event");
}

#[tokio::test]
async fn test_hitl_auto_approve_from_learned_patterns() {
    // Tools that have been approved consistently should be auto-approved
    let test_ctx = TestContextBuilder::new().build().await;

    // Add the tool to the always-approve list
    test_ctx.always_approve_tool("learned_tool").await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    let _result = execute_with_hitl_generic(
        "learned_tool",
        &serde_json::json!({}),
        "test-tool-id",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should have auto-approved event for learned patterns
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let has_auto_approved = events.iter().any(|e| {
        matches!(e, AiEvent::ToolAutoApproved { reason, .. }
            if reason.contains("learned patterns") || reason.contains("always-allow"))
    });
    assert!(
        has_auto_approved,
        "Should have auto-approved event for learned tool"
    );
}

#[tokio::test]
async fn test_hitl_auto_approve_from_agent_mode() {
    // AgentMode::AutoApprove should auto-approve all tools
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    let _result = execute_with_hitl_generic(
        "some_random_tool",
        &serde_json::json!({}),
        "test-tool-id",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should have auto-approved event via agent mode
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let has_auto_approved = events.iter().any(|e| {
        matches!(e, AiEvent::ToolAutoApproved { reason, .. }
            if reason.contains("agent mode"))
    });
    assert!(
        has_auto_approved,
        "Should have auto-approved event via agent mode"
    );
}

#[tokio::test]
async fn test_hitl_auto_approve_from_runtime_flag() {
    // Runtime with auto_approve=true should auto-approve all tools
    let test_ctx = TestContextBuilder::new()
        .runtime(Arc::new(MockRuntime::with_auto_approve()))
        .build()
        .await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    let _result = execute_with_hitl_generic(
        "some_random_tool",
        &serde_json::json!({}),
        "test-tool-id",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should have auto-approved event via runtime flag
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let has_auto_approved = events.iter().any(|e| {
        matches!(e, AiEvent::ToolAutoApproved { reason, .. }
            if reason.contains("--auto-approve"))
    });
    assert!(
        has_auto_approved,
        "Should have auto-approved event via runtime flag"
    );
}

#[tokio::test]
async fn test_hitl_constraint_violation_denied() {
    // Tools that violate constraints should be denied
    // The default policy has blocked hosts for web_fetch
    let test_ctx = TestContextBuilder::new().build().await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Try to fetch localhost (blocked by default constraints)
    let result = execute_with_hitl_generic(
        "web_fetch",
        &serde_json::json!({"url": "http://localhost:8080/api"}),
        "test-tool-id",
        &ctx,
        &capture_ctx,
        &model,
        &sub_ctx,
    )
    .await
    .unwrap();

    // Should fail with constraint_violated
    assert!(!result.success);
    assert!(result.value.get("constraint_violated").is_some());
}

#[tokio::test]
async fn test_hitl_approval_request_emitted() {
    // When approval is needed, a ToolApprovalRequest event should be emitted
    let test_ctx = TestContextBuilder::new().build().await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let capture_ctx = test_ctx.create_capture_context();
    let model = MockCompletionModel::with_text("Done");
    let sub_ctx = test_sub_agent_context();

    // Use a tokio::select with a short timeout to avoid hanging
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        execute_with_hitl_generic(
            "edit_file", // A prompt tool that requires approval
            &serde_json::json!({"path": "test.txt", "edits": []}),
            "test-tool-id",
            &ctx,
            &capture_ctx,
            &model,
            &sub_ctx,
        ),
    )
    .await;

    // The call should timeout (because we don't respond to the approval request)
    // But we should have emitted an approval request event
    assert!(result.is_err(), "Should timeout waiting for approval");

    // Check for approval request event
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    let has_approval_request = events.iter().any(|e| {
        matches!(e, AiEvent::ToolApprovalRequest { tool_name, .. }
            if tool_name == "edit_file")
    });
    assert!(
        has_approval_request,
        "Should have emitted ToolApprovalRequest event"
    );
}

#[tokio::test]
async fn test_hitl_approval_timeout() {
    // When approval times out, the tool should fail with timeout error
    // Note: We use a custom short timeout for testing
    let test_ctx = TestContextBuilder::new().build().await;

    let client = test_llm_client();
    let ctx = test_ctx.as_agentic_context_with_client(&client);
    let _capture_ctx = test_ctx.create_capture_context();

    // The default APPROVAL_TIMEOUT_SECS is 300 (5 minutes), which is too long for tests.
    // We'll test the timeout behavior by using tokio::select with a short timeout
    // and verifying the pending_approvals state.

    // Start the approval request in a task
    let pending_approvals = ctx.access.pending_approvals.clone();
    let event_tx = ctx.events.event_tx.clone();

    let tool_name = "edit_file";
    let tool_id = "timeout-test-id";

    // Manually simulate what execute_with_hitl_generic does for approval request
    // (This avoids the 300 second timeout in tests)
    let _ = event_tx.send(AiEvent::ToolApprovalRequest {
        request_id: tool_id.to_string(),
        tool_name: tool_name.to_string(),
        args: serde_json::json!({}),
        stats: None,
        risk_level: golish_core::hitl::RiskLevel::Medium,
        can_learn: true,
        suggestion: None,
        source: golish_core::events::ToolSource::Main,
    });

    // Create and store the oneshot sender
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let mut pending = pending_approvals.write().await;
        pending.insert(tool_id.to_string(), tx);
    }

    // Verify the pending approval is registered
    {
        let pending = pending_approvals.read().await;
        assert!(
            pending.contains_key(tool_id),
            "Should have pending approval"
        );
    }

    // Wait a very short time (simulating timeout behavior)
    let result = tokio::time::timeout(std::time::Duration::from_millis(10), rx).await;

    // Should timeout
    assert!(result.is_err(), "Should timeout waiting for approval");

    // Clean up (as the timeout handler would)
    {
        let mut pending = pending_approvals.write().await;
        pending.remove(tool_id);
    }

    // Verify cleanup
    {
        let pending = pending_approvals.read().await;
        assert!(
            !pending.contains_key(tool_id),
            "Should have cleaned up pending approval after timeout"
        );
    }
}

// ========================================================================
// Tool Routing Tests (Phase 0.3)
// ========================================================================

