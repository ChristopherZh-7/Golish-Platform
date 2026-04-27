use super::*;

mod concurrent_dispatch_tests {
    use super::*;
    use crate::agentic_loop::sub_agent_dispatch::is_sub_agent_tool;

    fn make_tool_call(name: &str, id: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            call_id: Some(id.to_string()),
            function: rig::message::ToolFunction {
                name: name.to_string(),
                arguments: json!({}),
            },
            signature: None,
            additional_params: None,
        }
    }

    #[test]
    fn test_is_sub_agent_tool() {
        assert!(is_sub_agent_tool("sub_agent_coder"));
        assert!(is_sub_agent_tool("sub_agent_explorer"));
        assert!(!is_sub_agent_tool("read_file"));
        assert!(!is_sub_agent_tool("run_pty_cmd"));
    }

    #[test]
    fn test_partition_tool_calls_mixed() {
        let calls = vec![
            make_tool_call("read_file", "tc1"),
            make_tool_call("sub_agent_coder", "tc2"),
            make_tool_call("write_file", "tc3"),
            make_tool_call("sub_agent_explorer", "tc4"),
        ];
        let (sub_agents, others) = partition_tool_calls(calls);
        assert_eq!(sub_agents.len(), 2);
        assert_eq!(others.len(), 2);
        assert_eq!(sub_agents[0].0, 1);
        assert_eq!(sub_agents[1].0, 3);
        assert_eq!(others[0].0, 0);
        assert_eq!(others[1].0, 2);
    }

    #[test]
    fn test_partition_tool_calls_empty() {
        let (sub_agents, others) = partition_tool_calls(vec![]);
        assert_eq!(sub_agents.len(), 0);
        assert_eq!(others.len(), 0);
    }
}

mod loop_capture_context_tests {
    use super::*;

    #[test]
    fn test_loop_capture_context_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LoopCaptureContext>();
    }

    #[test]
    fn test_loop_capture_context_shared_ref_process() {
        let ctx = LoopCaptureContext::new(None);
        let event = AiEvent::ToolRequest {
            request_id: "test".to_string(),
            tool_name: "read_file".to_string(),
            args: json!({}),
            source: golish_core::events::ToolSource::Main,
        };
        ctx.process(&event);
        ctx.process(&event);
    }

    #[tokio::test]
    async fn test_loop_capture_context_concurrent_access() {
        let ctx = Arc::new(LoopCaptureContext::new(None));
        let mut handles = vec![];
        for i in 0..5 {
            let ctx = Arc::clone(&ctx);
            handles.push(tokio::spawn(async move {
                let event = AiEvent::ToolRequest {
                    request_id: format!("req-{}", i),
                    tool_name: "read_file".to_string(),
                    args: json!({}),
                    source: golish_core::events::ToolSource::Main,
                };
                ctx.process(&event);
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
    }
}

mod unified_loop_tests {
    use super::*;
    use golish_llm_providers::ModelCapabilities;

    #[test]
    fn test_agentic_loop_config_main_agent_anthropic() {
        let config = AgenticLoopConfig::main_agent_anthropic();
        assert!(
            config.capabilities.supports_thinking_history,
            "Anthropic config should support thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "Anthropic config should support temperature"
        );
        assert!(config.require_hitl, "Main agent should require HITL");
        assert!(!config.is_sub_agent, "Main agent should not be sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_main_agent_generic() {
        let config = AgenticLoopConfig::main_agent_generic();
        assert!(
            !config.capabilities.supports_thinking_history,
            "Generic config should not support thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "Generic config should support temperature"
        );
        assert!(config.require_hitl, "Main agent should require HITL");
        assert!(!config.is_sub_agent, "Main agent should not be sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_sub_agent() {
        let config = AgenticLoopConfig::sub_agent(ModelCapabilities::conservative_defaults());
        assert!(
            !config.capabilities.supports_thinking_history,
            "Conservative defaults should not support thinking history"
        );
        assert!(!config.require_hitl, "Sub-agent should not require HITL");
        assert!(config.is_sub_agent, "Should be marked as sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_sub_agent_with_anthropic_capabilities() {
        let config = AgenticLoopConfig::sub_agent(ModelCapabilities::anthropic_defaults());
        assert!(
            config.capabilities.supports_thinking_history,
            "Anthropic sub-agent should support thinking history"
        );
        assert!(!config.require_hitl, "Sub-agent should not require HITL");
        assert!(config.is_sub_agent, "Should be marked as sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_with_detection_anthropic() {
        let config = AgenticLoopConfig::with_detection("anthropic", "claude-3-opus", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "Anthropic detection should enable thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "Anthropic detection should enable temperature"
        );
        assert!(config.require_hitl, "Non-sub-agent should require HITL");
        assert!(!config.is_sub_agent);
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_reasoning() {
        let config = AgenticLoopConfig::with_detection("openai", "o3-mini", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "OpenAI reasoning model should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "OpenAI reasoning model should not support temperature"
        );
        assert!(config.require_hitl);
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_regular() {
        let config = AgenticLoopConfig::with_detection("openai", "gpt-4o", false);
        assert!(
            !config.capabilities.supports_thinking_history,
            "Regular OpenAI model should not support thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "Regular OpenAI model should support temperature"
        );
    }

    #[test]
    fn test_agentic_loop_config_with_detection_sub_agent() {
        let config = AgenticLoopConfig::with_detection("openai", "gpt-4o", true);
        assert!(!config.require_hitl, "Sub-agent should not require HITL");
        assert!(config.is_sub_agent, "Should be marked as sub-agent");
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_gpt5_series() {
        // GPT-5 base model
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "GPT-5 should support thinking history (reasoning model)"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5 should not support temperature (reasoning model)"
        );

        // GPT-5.1
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5.1", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "GPT-5.1 should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5.1 should not support temperature"
        );

        // GPT-5.2
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5.2", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "GPT-5.2 should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5.2 should not support temperature"
        );

        // GPT-5-mini
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5-mini", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "GPT-5-mini should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5-mini should not support temperature"
        );
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_responses_gpt5() {
        // OpenAI Responses API with GPT-5.2
        let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-5.2", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "OpenAI Responses API should support thinking history"
        );
        assert!(
            !config.capabilities.supports_temperature,
            "GPT-5.2 via Responses API should not support temperature"
        );

        // Contrast with GPT-4.1 which DOES support temperature
        let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-4.1", false);
        assert!(
            config.capabilities.supports_thinking_history,
            "OpenAI Responses API should support thinking history"
        );
        assert!(
            config.capabilities.supports_temperature,
            "GPT-4.1 via Responses API should support temperature"
        );
    }

    #[test]
    fn test_agentic_loop_config_with_detection_openai_codex() {
        // Codex models don't support temperature
        let config = AgenticLoopConfig::with_detection("openai", "gpt-5.1-codex-max", false);
        assert!(
            !config.capabilities.supports_temperature,
            "Codex models should not support temperature"
        );

        let config = AgenticLoopConfig::with_detection("openai_responses", "gpt-5.2-codex", false);
        assert!(
            !config.capabilities.supports_temperature,
            "Codex models via Responses API should not support temperature"
        );
    }
}

mod repetitive_text_tests {
    use super::*;

    #[test]
    fn test_short_text_not_repetitive() {
        assert!(!detect_repetitive_text("你好"));
        assert!(!detect_repetitive_text(""));
        assert!(!detect_repetitive_text("这是一个正常的回答。"));
    }

    #[test]
    fn test_normal_text_not_repetitive() {
        let text = "example.com 是一个官方保留的测试域名。\
                    它解析到 104.20.23.154 和 172.66.147.243。\
                    这些地址由 Cloudflare 托管。";
        assert!(!detect_repetitive_text(text));
    }

    #[test]
    fn test_repeated_sentences_detected() {
        // Simulate real degenerate output: repeated "I've completed your request" sentences
        let text = "该网站运行的是一个基于Vue3构建的前端应用，名为管理系统，以下是关键发现。\
                    我已经完成了对该网站的JavaScript代码分析。如果你有其他需要测试或分析的域名或目标，请告诉我。\
                    我已经完成了对该网站的JavaScript代码分析。如果你有其他需要，请直接告诉我。\
                    我已经完成了对该网站的JavaScript代码分析。请告诉我你接下来需要什么帮助。";
        assert!(detect_repetitive_text(text));
    }

    #[test]
    fn test_repeated_english_detected() {
        let text = "The scan has completed successfully and found the following services running on the target.\n\
                    I have completed your request. Let me know if you need anything else or any other targets to scan.\n\
                    I have completed your request. If you have other targets or need further analysis, let me know.\n\
                    I have completed your request. Please tell me what you need next or if there are other targets.\n";
        assert!(detect_repetitive_text(text));
    }

    #[test]
    fn test_two_similar_not_detected() {
        // Only 2 repeats — threshold is 3
        let text = "该网站运行的是一个基于Vue3构建的前端应用，名为管理系统，以下是关键发现。\
                    我已经完成了对该网站的JavaScript代码分析。如果你有其他需要测试或分析的域名或目标，请告诉我。\
                    我已经完成了对该网站的JavaScript代码分析。请告诉我你接下来需要什么帮助。";
        assert!(!detect_repetitive_text(text));
    }
}

mod utf8_truncation_tests {
    #[test]
    fn test_utf8_safe_truncation_ascii() {
        let text = "Hello, World!";
        let mut end = 5;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(&text[..end], "Hello");
    }

    #[test]
    fn test_utf8_safe_truncation_multibyte() {
        // "─" is 3 bytes (E2 94 80), testing truncation at various positions
        let text = "abc─def"; // a=0, b=1, c=2, ─=3-5, d=6, e=7, f=8

        // Truncate at position 4 (middle of ─)
        let mut end = 4;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 3); // Should back up to position 3 (start of ─)
        assert_eq!(&text[..end], "abc");

        // Truncate at position 5 (still in ─)
        let mut end = 5;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 3);
        assert_eq!(&text[..end], "abc");

        // Truncate at position 6 (after ─)
        let mut end = 6;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 6);
        assert_eq!(&text[..end], "abc─");
    }

    #[test]
    fn test_utf8_safe_truncation_emoji() {
        // Emoji like 🎉 is 4 bytes
        let text = "Hi🎉!"; // H=0, i=1, 🎉=2-5, !=6

        // Truncate at position 3 (middle of emoji)
        let mut end = 3;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 2);
        assert_eq!(&text[..end], "Hi");

        // Truncate at position 6 (after emoji)
        let mut end = 6;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(end, 6);
        assert_eq!(&text[..end], "Hi🎉");
    }

    #[test]
    fn test_utf8_safe_truncation_mixed_box_drawing() {
        // Box drawing characters like those that caused the original panic
        let text = "Summary:\n─────────";
        let target = 12; // Might land in middle of a box char

        let mut end = target.min(text.len());
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }

        // Should not panic and result should be valid UTF-8
        let truncated = &text[..end];
        assert!(truncated.len() <= target);
        // Verify it's valid UTF-8 by checking we can iterate chars
        assert!(truncated.chars().count() > 0);
    }
}


mod token_estimation_tests {
    use super::*;

    fn user_text_msg(text: &str) -> Message {
        Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: text.to_string(),
            })),
        }
    }

    fn assistant_text_msg(text: &str) -> Message {
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::Text(Text {
                text: text.to_string(),
            })),
        }
    }

    fn tool_result_msg(id: &str, result_text: &str) -> Message {
        Message::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: id.to_string(),
                call_id: Some(id.to_string()),
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: result_text.to_string(),
                })),
            })),
        }
    }

    fn tool_call_msg(name: &str, args: serde_json::Value) -> Message {
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                id: "call_1".to_string(),
                call_id: Some("call_1".to_string()),
                function: rig::message::ToolFunction {
                    name: name.to_string(),
                    arguments: args,
                },
                signature: None,
                additional_params: None,
            })),
        }
    }

    #[test]
    fn test_estimate_user_text_message() {
        let msg = user_text_msg("Hello, how are you doing today?");
        let tokens = estimate_message_tokens(&msg);
        // ~7 words, should be roughly 7-8 tokens
        assert!(
            (5..=12).contains(&tokens),
            "Simple text should estimate 5-12 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_estimate_empty_message() {
        let msg = user_text_msg("");
        let tokens = estimate_message_tokens(&msg);
        assert_eq!(tokens, 0, "Empty message should be 0 tokens");
    }

    #[test]
    fn test_estimate_large_tool_result() {
        // Simulate reading a file — this is the key scenario for proactive counting
        let file_content = "use std::collections::HashMap;\n".repeat(200);
        let msg = tool_result_msg("read_file_1", &file_content);
        let tokens = estimate_message_tokens(&msg);

        // ~6000 chars of code, should be well over 1000 tokens
        assert!(
            tokens > 1000,
            "Large tool result should estimate >1000 tokens, got {}",
            tokens
        );
        assert!(
            tokens < 3000,
            "Large tool result should not wildly overcount, got {}",
            tokens
        );
    }

    #[test]
    fn test_estimate_tool_call_message() {
        let args = json!({
            "path": "src/main.rs",
            "line_start": 1,
            "line_end": 50
        });
        let msg = tool_call_msg("read_file", args);
        let tokens = estimate_message_tokens(&msg);
        assert!(
            tokens > 5,
            "Tool call should estimate some tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_estimate_assistant_text() {
        let msg = assistant_text_msg(
            "I'll help you with that. Let me read the file first to understand the codebase.",
        );
        let tokens = estimate_message_tokens(&msg);
        assert!(
            (10..=25).contains(&tokens),
            "Assistant text should estimate 10-25 tokens, got {}",
            tokens
        );
    }

    #[test]
    fn test_estimate_multiple_messages_accumulate() {
        // Simulate a realistic tool-heavy conversation fragment
        let messages = [
            user_text_msg("Read the main.rs file and fix the bug"),
            tool_call_msg("read_file", json!({"path": "src/main.rs"})),
            tool_result_msg("r1", &"fn main() { todo!() }\n".repeat(100)),
            tool_call_msg(
                "edit_file",
                json!({"path": "src/main.rs", "old_text": "todo!()", "new_text": "println!(\"fixed\")"}),
            ),
            tool_result_msg("r2", r#"{"success": true, "path": "src/main.rs"}"#),
        ];

        let total: usize = messages.iter().map(estimate_message_tokens).sum();

        // Should be dominated by the large tool result (~2200 chars of code)
        assert!(
            total > 400,
            "Multi-message conversation should estimate >400 tokens, got {}",
            total
        );
    }

    #[test]
    fn test_estimate_extracts_tool_result_content() {
        // Tests that estimate_message_tokens correctly extracts text from ToolResult
        // (our extraction logic, not tokenx-rs accuracy)
        let small_result = tool_result_msg("r1", "ok");
        let large_result = tool_result_msg("r1", &"x".repeat(10_000));

        let small_tokens = estimate_message_tokens(&small_result);
        let large_tokens = estimate_message_tokens(&large_result);

        assert!(small_tokens > 0, "Non-empty tool result should have tokens");
        assert!(
            large_tokens > small_tokens * 10,
            "10x larger content should produce substantially more tokens (small={}, large={})",
            small_tokens,
            large_tokens
        );
    }

    #[test]
    fn test_estimate_extracts_tool_call_args() {
        // Tests that estimate_message_tokens serializes and counts tool call arguments
        let small_call = tool_call_msg("read_file", json!({"path": "a.rs"}));
        let large_call = tool_call_msg(
            "edit_file",
            json!({
                "path": "src/very/long/path/to/some/module.rs",
                "old_text": "fn old() { todo!() }".repeat(50),
                "new_text": "fn new() { println!(\"done\") }".repeat(50),
            }),
        );

        let small_tokens = estimate_message_tokens(&small_call);
        let large_tokens = estimate_message_tokens(&large_call);

        assert!(small_tokens > 0, "Tool call should produce tokens");
        assert!(
            large_tokens > small_tokens,
            "Larger args should produce more tokens (small={}, large={})",
            small_tokens,
            large_tokens
        );
    }

    #[test]
    fn test_estimate_messages_scale_linearly() {
        // Adding more messages should increase the total proportionally
        let one_msg: usize = std::iter::once(user_text_msg("Hello world"))
            .map(|m| estimate_message_tokens(&m))
            .sum();

        let five_msgs: usize = (0..5)
            .map(|_| user_text_msg("Hello world"))
            .map(|m| estimate_message_tokens(&m))
            .sum();

        assert_eq!(
            five_msgs,
            one_msg * 5,
            "Token count should scale linearly with identical messages"
        );
    }

    #[test]
    fn test_tool_heavy_session_compaction_pipeline() {
        // End-to-end: builds realistic messages → estimate_message_tokens → compaction state → should_compact
        // Tests the full pipeline without testing tokenx-rs accuracy
        use golish_context::context_manager::{CompactionState, ContextManagerConfig};
        use golish_context::ContextManager;

        let manager = ContextManager::with_config(
            "claude-3-5-sonnet",
            ContextManagerConfig {
                enabled: true,
                compaction_threshold: 0.80,
                ..Default::default()
            },
        );

        // Build messages with tool results of known relative sizes
        let small_session: Vec<Message> = vec![user_text_msg("hello"), tool_result_msg("r1", "ok")];

        let large_session: Vec<Message> = (0..50)
            .flat_map(|i| {
                vec![
                    tool_call_msg("read_file", json!({"path": format!("file_{}.rs", i)})),
                    tool_result_msg(&format!("r{}", i), &"use std::io::Result;\n".repeat(200)),
                ]
            })
            .collect();

        let small_tokens: u64 = small_session
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>() as u64;
        let large_tokens: u64 = large_session
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>() as u64;

        // Small session should not trigger compaction
        let mut state = CompactionState::new();
        state.update_tokens_estimated(small_tokens);
        assert!(
            !manager
                .should_compact(&state, "claude-3-5-sonnet")
                .should_compact,
            "Small session ({} tokens) should not trigger compaction",
            small_tokens
        );

        // Large session (50 file reads) should produce enough tokens to matter
        // The exact threshold depends on tokenx-rs output, but 50 files x 200 lines
        // should be substantial
        assert!(
            large_tokens > small_tokens * 100,
            "Large session should be much bigger than small (small={}, large={})",
            small_tokens,
            large_tokens
        );
    }
}

mod openai_tracing_tests {
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
}
