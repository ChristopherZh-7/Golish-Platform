use super::*;

#[test]
fn test_mock_response_text() {
    let response = MockResponse::text("Hello");
    assert_eq!(response.text, Some("Hello".to_string()));
    assert!(response.tool_calls.is_empty());
    assert!(response.thinking.is_none());
}

#[test]
fn test_mock_response_tool_call() {
    let response = MockResponse::tool_call("read_file", serde_json::json!({"path": "/test"}));
    assert!(response.text.is_none());
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "read_file");
}

#[test]
fn test_mock_response_with_thinking() {
    let response = MockResponse::text("Response").with_thinking("Thinking about this...");
    assert_eq!(response.text, Some("Response".to_string()));
    assert_eq!(
        response.thinking,
        Some("Thinking about this...".to_string())
    );
}

#[test]
fn test_mock_model_response_sequence() {
    let model = MockCompletionModel::new(vec![
        MockResponse::text("First"),
        MockResponse::text("Second"),
        MockResponse::text("Third"),
    ]);

    assert_eq!(model.call_count(), 0);

    let r1 = model.next_response();
    assert_eq!(r1.text, Some("First".to_string()));
    assert_eq!(model.call_count(), 1);

    let r2 = model.next_response();
    assert_eq!(r2.text, Some("Second".to_string()));
    assert_eq!(model.call_count(), 2);

    let r3 = model.next_response();
    assert_eq!(r3.text, Some("Third".to_string()));
    assert_eq!(model.call_count(), 3);

    // Exhausted - returns empty string
    let r4 = model.next_response();
    assert_eq!(r4.text, Some("".to_string()));
}

#[test]
fn test_mock_model_reset() {
    let model = MockCompletionModel::new(vec![
        MockResponse::text("First"),
        MockResponse::text("Second"),
    ]);

    let _ = model.next_response();
    let _ = model.next_response();
    assert_eq!(model.call_count(), 2);

    model.reset();
    assert_eq!(model.call_count(), 0);

    let r1 = model.next_response();
    assert_eq!(r1.text, Some("First".to_string()));
}

#[tokio::test]
async fn test_mock_model_completion() {
    let model = MockCompletionModel::with_text("Test response");
    let request = CompletionRequest {
        preamble: None,
        chat_history: OneOrMany::one(rig::completion::Message::User {
            content: OneOrMany::one(rig::message::UserContent::Text(Text {
                text: "Hello".to_string(),
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: None,
        max_tokens: None,
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
    };

    let response = model.completion(request).await.unwrap();
    assert!(matches!(
        response.choice.iter().next().unwrap(),
        AssistantContent::Text(Text { text }) if text == "Test response"
    ));
}

#[tokio::test]
async fn test_mock_model_stream() {
    let model = MockCompletionModel::with_text("Streamed response");
    let request = CompletionRequest {
        preamble: None,
        chat_history: OneOrMany::one(rig::completion::Message::User {
            content: OneOrMany::one(rig::message::UserContent::Text(Text {
                text: "Hello".to_string(),
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: None,
        max_tokens: None,
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
    };

    let mut stream = model.stream(request).await.unwrap();
    let mut found_text = false;
    let mut found_final = false;

    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            StreamedAssistantContent::Text(t) => {
                assert_eq!(t.text, "Streamed response");
                found_text = true;
            }
            StreamedAssistantContent::Final(_) => {
                found_final = true;
            }
            _ => {}
        }
    }

    assert!(found_text);
    assert!(found_final);
}

#[tokio::test]
async fn test_mock_model_tool_call_stream() {
    let model = MockCompletionModel::new(vec![MockResponse::tool_call(
        "read_file",
        serde_json::json!({"path": "/test.txt"}),
    )]);

    let request = CompletionRequest {
        preamble: None,
        chat_history: OneOrMany::one(rig::completion::Message::User {
            content: OneOrMany::one(rig::message::UserContent::Text(Text {
                text: "Read the file".to_string(),
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: None,
        max_tokens: None,
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
    };

    let mut stream = model.stream(request).await.unwrap();
    let mut found_tool_call = false;

    while let Some(chunk) = stream.next().await {
        if let StreamedAssistantContent::ToolCall { tool_call: tc, .. } = chunk.unwrap() {
            assert_eq!(tc.function.name, "read_file");
            found_tool_call = true;
        }
    }

    assert!(found_tool_call);
}

#[tokio::test]
async fn test_mock_model_with_thinking() {
    let model = MockCompletionModel::new(vec![MockResponse::text_with_thinking(
        "Final answer",
        "Let me think about this...",
    )]);

    let request = CompletionRequest {
        preamble: None,
        chat_history: OneOrMany::one(rig::completion::Message::User {
            content: OneOrMany::one(rig::message::UserContent::Text(Text {
                text: "What is 2+2?".to_string(),
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: None,
        max_tokens: None,
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
    };

    let mut stream = model.stream(request).await.unwrap();
    let mut found_reasoning = false;
    let mut found_text = false;

    while let Some(chunk) = stream.next().await {
        match chunk.unwrap() {
            StreamedAssistantContent::Reasoning(r) => {
                assert_eq!(r.content, vec![ReasoningContent::Text {
                    text: "Let me think about this...".to_string(),
                    signature: Some("mock-signature".to_string()),
                }]);
                found_reasoning = true;
            }
            StreamedAssistantContent::Text(t) => {
                assert_eq!(t.text, "Final answer");
                found_text = true;
            }
            _ => {}
        }
    }

    assert!(found_reasoning);
    assert!(found_text);
}

// ========================================================================
// Test Context Builder Tests
// ========================================================================

#[tokio::test]
async fn test_context_builder_creates_valid_context() {
    let test_ctx = TestContextBuilder::new().build().await;

    // Verify all components are initialized
    assert!(test_ctx
        .event_tx
        .send(AiEvent::Started {
            turn_id: "test".to_string()
        })
        .is_ok());

    // Collect the event
    let mut test_ctx = test_ctx;
    let events = test_ctx.collect_events();
    assert_eq!(events.len(), 1);

    // Verify it's the event we sent
    match &events[0] {
        AiEvent::Started { turn_id } => assert_eq!(turn_id, "test"),
        _ => panic!("Unexpected event type"),
    }
}

#[tokio::test]
async fn test_context_builder_with_planning_mode() {
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::Planning)
        .build()
        .await;

    // Verify agent mode is set correctly
    let mode = test_ctx.agent_mode.read().await;
    assert!(mode.is_planning());
}

#[tokio::test]
async fn test_context_builder_with_auto_approve_mode() {
    let test_ctx = TestContextBuilder::new()
        .agent_mode(AgentMode::AutoApprove)
        .build()
        .await;

    // Verify agent mode is set correctly
    let mode = test_ctx.agent_mode.read().await;
    assert!(mode.is_auto_approve());
}

// ========================================================================
// HITL Approval Flow Tests (Phase 0.2)
// ========================================================================

