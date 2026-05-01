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
