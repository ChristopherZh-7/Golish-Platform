use async_openai::types::responses::{
    EasyInputContent, FunctionCallOutput, IncludeEnum, InputContent, InputItem, Item, MessageType,
    ReasoningEffort as OAReasoningEffort, ReasoningSummary, Role, SummaryPart,
};
use rig::completion::AssistantContent;
use rig::message::{Text, ToolCall, ToolFunction, UserContent};
use rig::one_or_many::OneOrMany;

use super::builder::build_request;
use super::conversion::{convert_assistant_content_to_items, convert_user_content};

#[test]
fn test_convert_user_content_text_only() {
    let content = OneOrMany::one(UserContent::Text(Text {
        text: "Hello, world!".to_string(),
    }));
    let result = convert_user_content(&content);
    assert_eq!(result.len(), 1);
    match &result[0] {
        InputItem::EasyMessage(msg) => {
            assert_eq!(msg.role, Role::User);
            match &msg.content {
                EasyInputContent::Text(text) => assert_eq!(text, "Hello, world!"),
                _ => panic!("Expected Text content"),
            }
        }
        _ => panic!("Expected EasyMessage"),
    }
}

#[test]
fn test_convert_user_content_with_image() {
    use rig::message::{DocumentSourceKind, Image, ImageMediaType};

    let content = OneOrMany::many(vec![
        UserContent::Text(Text {
            text: "What's in this image?".to_string(),
        }),
        UserContent::Image(Image {
            data: DocumentSourceKind::Base64("dGVzdA==".to_string()),
            media_type: Some(ImageMediaType::PNG),
            detail: None,
            additional_params: None,
        }),
    ])
    .unwrap();
    let result = convert_user_content(&content);
    assert_eq!(result.len(), 1);
    match &result[0] {
        InputItem::EasyMessage(msg) => {
            assert_eq!(msg.role, Role::User);
            match &msg.content {
                EasyInputContent::ContentList(parts) => {
                    assert_eq!(parts.len(), 2);
                    match &parts[0] {
                        InputContent::InputText(t) => {
                            assert_eq!(t.text, "What's in this image?")
                        }
                        _ => panic!("Expected InputText"),
                    }
                    match &parts[1] {
                        InputContent::InputImage(img) => {
                            assert!(img
                                .image_url
                                .as_ref()
                                .unwrap()
                                .starts_with("data:image/png;base64,"));
                        }
                        _ => panic!("Expected InputImage"),
                    }
                }
                _ => panic!("Expected ContentList"),
            }
        }
        _ => panic!("Expected EasyMessage"),
    }
}

#[test]
fn test_convert_user_content_with_tool_result() {
    use rig::message::{ToolResult, ToolResultContent};

    let content = OneOrMany::one(UserContent::ToolResult(ToolResult {
        id: "result_123".to_string(),
        call_id: Some("call_abc".to_string()),
        content: OneOrMany::one(ToolResultContent::Text(Text {
            text: "Tool execution result".to_string(),
        })),
    }));
    let result = convert_user_content(&content);

    // Should produce a structured FunctionCallOutput, not text.
    assert_eq!(result.len(), 1);
    match &result[0] {
        InputItem::Item(Item::FunctionCallOutput(output)) => {
            assert_eq!(output.call_id, "call_abc");
            assert!(
                output.status.is_none(),
                "status must be None — it is output-only and OpenAI rejects it on input"
            );
            match &output.output {
                FunctionCallOutput::Text(text) => {
                    assert_eq!(text, "Tool execution result");
                }
                _ => panic!("Expected Text output"),
            }
        }
        _ => panic!("Expected Item::FunctionCallOutput"),
    }
}

#[test]
fn test_convert_assistant_content_with_tool_call() {
    let content = OneOrMany::one(AssistantContent::ToolCall(ToolCall {
        id: "tool_123".to_string(),
        call_id: Some("call_xyz".to_string()),
        function: ToolFunction {
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "test.txt"}),
        },
        signature: None,
        additional_params: None,
    }));
    let result = convert_assistant_content_to_items(&content);

    // Should produce a structured FunctionCall, not text.
    assert_eq!(result.len(), 1);
    match &result[0] {
        InputItem::Item(Item::FunctionCall(fc)) => {
            assert_eq!(fc.name, "read_file");
            assert_eq!(fc.call_id, "call_xyz");
            assert!(
                fc.status.is_none(),
                "status must be None — it is output-only and OpenAI rejects it on input"
            );
            assert!(fc.arguments.contains("test.txt"));
        }
        _ => panic!("Expected Item::FunctionCall"),
    }
}

#[test]
fn test_convert_assistant_content_with_reasoning() {
    let reasoning = rig::message::Reasoning::multi(vec![
        "First, I need to consider...".to_string(),
        "Then, I should analyze...".to_string(),
    ])
    .with_id("rs_test123".to_string());
    let content = OneOrMany::one(AssistantContent::Reasoning(reasoning));
    let result = convert_assistant_content_to_items(&content);

    assert_eq!(result.len(), 1);
    match &result[0] {
        InputItem::Item(Item::Reasoning(reasoning)) => {
            assert_eq!(reasoning.id, "rs_test123");
            assert_eq!(reasoning.summary.len(), 2);
            assert!(
                reasoning.status.is_none(),
                "status must be None — it is output-only and OpenAI rejects it on input"
            );
            match &reasoning.summary[0] {
                SummaryPart::SummaryText(s) => {
                    assert_eq!(s.text, "First, I need to consider...");
                }
            }
            match &reasoning.summary[1] {
                SummaryPart::SummaryText(s) => {
                    assert_eq!(s.text, "Then, I should analyze...");
                }
            }
        }
        _ => panic!("Expected Item::Reasoning"),
    }
}

/// Test that `encrypted_content` is passed through from `signature`
/// field to `ReasoningItem`. Critical for stateless multi-turn
/// conversations with reasoning models.
#[test]
fn test_reasoning_encrypted_content_roundtrip() {
    // rig-core ≥0.36 dropped `Reasoning::with_signature`; use the
    // `new_with_signature` constructor instead (still attaches the same
    // encrypted blob to the underlying `ReasoningContent`).
    let reasoning = rig::message::Reasoning::new_with_signature(
        "I'm thinking...",
        Some("encrypted_data_blob_xyz".to_string()),
    )
    .with_id("rs_abc123".to_string());

    let content = OneOrMany::one(AssistantContent::Reasoning(reasoning));
    let result = convert_assistant_content_to_items(&content);

    assert_eq!(result.len(), 1);
    match &result[0] {
        InputItem::Item(Item::Reasoning(reasoning_item)) => {
            assert_eq!(reasoning_item.id, "rs_abc123");
            assert_eq!(
                reasoning_item.encrypted_content,
                Some("encrypted_data_blob_xyz".to_string()),
                "encrypted_content must be passed through for stateless operation"
            );
        }
        _ => panic!("Expected Item::Reasoning"),
    }
}

#[test]
fn test_reasoning_without_encrypted_content() {
    let reasoning = rig::message::Reasoning::multi(vec!["Just thinking...".to_string()])
        .with_id("rs_no_encryption".to_string());

    let content = OneOrMany::one(AssistantContent::Reasoning(reasoning));
    let result = convert_assistant_content_to_items(&content);

    assert_eq!(result.len(), 1);
    match &result[0] {
        InputItem::Item(Item::Reasoning(reasoning_item)) => {
            assert_eq!(reasoning_item.id, "rs_no_encryption");
            assert!(
                reasoning_item.encrypted_content.is_none(),
                "encrypted_content should be None when no signature was set"
            );
        }
        _ => panic!("Expected Item::Reasoning"),
    }
}

// ============================================================================
// build_request tests
//
// These test the request-building logic directly using the pub(crate)
// method, without making any HTTP calls. All tests are pure unit tests.
// ============================================================================

mod build_request_tests;

#[cfg(test)]
fn _msg_unused_imports() {
    // Suppress unused MessageType warning if it's pulled in by a test
    // module that doesn't end up using it after pruning.
    let _ = MessageType::Message;
}
