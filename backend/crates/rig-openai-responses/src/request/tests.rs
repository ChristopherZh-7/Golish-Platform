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
    let reasoning = rig::message::Reasoning::multi(vec!["I'm thinking...".to_string()])
        .with_id("rs_abc123".to_string())
        .with_signature(Some("encrypted_data_blob_xyz".to_string()));

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

mod build_request_tests {
    use super::*;
    use crate::client::{Client, ReasoningEffort};
    use rig::completion::{CompletionRequest, Message};
    use rig::message::UserContent;

    /// Construct a minimal valid `CompletionRequest` with a single
    /// user text message.
    fn minimal_request() -> CompletionRequest {
        CompletionRequest {
            preamble: None,
            chat_history: OneOrMany::one(Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
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
        }
    }

    fn make_model(model: &str, effort: Option<ReasoningEffort>) -> crate::completion::CompletionModel {
        let client = Client::new("test-key");
        let mut m = client.completion_model(model);
        if let Some(e) = effort {
            m = m.with_reasoning_effort(e);
        }
        m
    }

    // -------------------------------------------------------------------------
    // Fix 1: reasoning models always get Detailed summary
    // -------------------------------------------------------------------------

    #[test]
    fn test_reasoning_model_without_effort_has_detailed_summary() {
        let model = make_model("gpt-5.2", None);
        let req = build_request(&model, &minimal_request()).unwrap();

        let reasoning = req
            .reasoning
            .expect("gpt-5.2 must have reasoning config even without explicit effort setting");
        assert_eq!(
            reasoning.summary,
            Some(ReasoningSummary::Detailed),
            "summary must be Detailed so chain-of-thought is always streamed"
        );
        assert!(
            reasoning.effort.is_none(),
            "no effort was configured, so effort should be None"
        );
    }

    #[test]
    fn test_reasoning_model_with_effort_uses_detailed_not_auto() {
        let model = make_model("gpt-5.2", Some(ReasoningEffort::High));
        let req = build_request(&model, &minimal_request()).unwrap();

        let reasoning = req.reasoning.expect("must have reasoning config");
        assert_eq!(
            reasoning.summary,
            Some(ReasoningSummary::Detailed),
            "explicit effort must still produce Detailed summary, not Auto"
        );
        assert_eq!(
            reasoning.effort,
            Some(OAReasoningEffort::High),
            "effort level must be preserved"
        );
    }

    #[test]
    fn test_codex_model_without_effort_has_detailed_summary() {
        let model = make_model("gpt-5.2-codex", None);
        let req = build_request(&model, &minimal_request()).unwrap();

        let reasoning = req
            .reasoning
            .expect("gpt-5.2-codex is a reasoning model and must have reasoning config");
        assert_eq!(reasoning.summary, Some(ReasoningSummary::Detailed));
    }

    #[test]
    fn test_all_reasoning_model_prefixes_get_config_without_effort() {
        let reasoning_models = [
            "o1",
            "o1-preview",
            "o3",
            "o3-mini",
            "o4-mini",
            "gpt-5",
            "gpt-5.1",
            "gpt-5.2-codex",
        ];
        for model_id in &reasoning_models {
            let model = make_model(model_id, None);
            let req = build_request(&model, &minimal_request()).unwrap();
            let reasoning = req.reasoning.unwrap_or_else(|| {
                panic!(
                    "{} must have reasoning config even without explicit effort",
                    model_id
                )
            });
            assert_eq!(
                reasoning.summary,
                Some(ReasoningSummary::Detailed),
                "{} must use Detailed summary",
                model_id
            );
        }
    }

    #[test]
    fn test_non_reasoning_model_has_no_reasoning_config() {
        for model_id in &["gpt-4.1", "gpt-4o", "gpt-4o-mini", "chatgpt-4o-latest"] {
            let model = make_model(model_id, None);
            let req = build_request(&model, &minimal_request()).unwrap();
            assert!(
                req.reasoning.is_none(),
                "{} must not have reasoning config",
                model_id
            );
            assert!(
                req.include.is_none(),
                "{} must not request encrypted_content include",
                model_id
            );
        }
    }

    #[test]
    fn test_reasoning_model_requests_encrypted_content_include() {
        let model = make_model("gpt-5.2", None);
        let req = build_request(&model, &minimal_request()).unwrap();

        let include = req
            .include
            .expect("reasoning models must have include parameter");
        assert!(
            include.contains(&IncludeEnum::ReasoningEncryptedContent),
            "must include reasoning.encrypted_content for stateless operation"
        );
    }

    #[test]
    fn test_all_reasoning_models_request_encrypted_content() {
        let reasoning_models = ["o1", "o3-mini", "o4-mini", "gpt-5", "gpt-5.2-codex"];
        for model_id in &reasoning_models {
            let model = make_model(model_id, None);
            let req = build_request(&model, &minimal_request()).unwrap();
            let include = req
                .include
                .unwrap_or_else(|| panic!("{} must have include parameter", model_id));
            assert!(
                include.contains(&IncludeEnum::ReasoningEncryptedContent),
                "{} must request encrypted_content",
                model_id
            );
        }
    }

    #[test]
    fn test_reasoning_effort_levels_are_preserved() {
        let cases = [
            (ReasoningEffort::Low, OAReasoningEffort::Low),
            (ReasoningEffort::Medium, OAReasoningEffort::Medium),
            (ReasoningEffort::High, OAReasoningEffort::High),
            (ReasoningEffort::ExtraHigh, OAReasoningEffort::Xhigh),
        ];
        for (input, expected) in cases {
            let model = make_model("gpt-5.2", Some(input));
            let req = build_request(&model, &minimal_request()).unwrap();
            let reasoning = req.reasoning.expect("must have reasoning");
            assert_eq!(
                reasoning.effort,
                Some(expected),
                "effort level must round-trip correctly"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Fix 2: additional_params reasoning overrides
    // -------------------------------------------------------------------------

    #[test]
    fn test_additional_params_reasoning_effort_is_applied() {
        let model = make_model("gpt-5.2", None);
        let mut req = minimal_request();
        req.additional_params = Some(serde_json::json!({
            "reasoning": { "effort": "low" }
        }));
        let built = build_request(&model, &req).unwrap();

        let reasoning = built.reasoning.expect("must have reasoning config");
        assert_eq!(
            reasoning.effort,
            Some(OAReasoningEffort::Low),
            "effort from additional_params must override the struct default (None)"
        );
        assert_eq!(reasoning.summary, Some(ReasoningSummary::Detailed));
    }

    #[test]
    fn test_additional_params_summary_overrides_default() {
        let model = make_model("gpt-5.2", Some(ReasoningEffort::Medium));
        let mut req = minimal_request();
        req.additional_params = Some(serde_json::json!({
            "reasoning": { "summary": "concise" }
        }));
        let built = build_request(&model, &req).unwrap();

        let reasoning = built.reasoning.expect("must have reasoning config");
        assert_eq!(
            reasoning.summary,
            Some(ReasoningSummary::Concise),
            "summary from additional_params must override the Detailed default"
        );
        assert_eq!(reasoning.effort, Some(OAReasoningEffort::Medium));
    }

    #[test]
    fn test_additional_params_effort_and_summary_both_applied() {
        let model = make_model("gpt-5.2", None);
        let mut req = minimal_request();
        req.additional_params = Some(serde_json::json!({
            "reasoning": { "effort": "high", "summary": "concise" }
        }));
        let built = build_request(&model, &req).unwrap();

        let reasoning = built.reasoning.expect("must have reasoning config");
        assert_eq!(reasoning.effort, Some(OAReasoningEffort::High));
        assert_eq!(reasoning.summary, Some(ReasoningSummary::Concise));
    }

    #[test]
    fn test_additional_params_unknown_keys_are_ignored() {
        let model = make_model("gpt-5.2", None);
        let mut req = minimal_request();
        req.additional_params = Some(serde_json::json!({
            "some_future_field": "value",
            "tools": [{ "type": "web_search_preview" }]
        }));
        let built = build_request(&model, &req).unwrap();
        assert!(
            built.reasoning.is_some(),
            "reasoning config must still be present when additional_params has no reasoning key"
        );
    }

    #[test]
    fn test_additional_params_without_reasoning_key_is_noop() {
        let model = make_model("gpt-5.2", Some(ReasoningEffort::High));
        let mut req_with = minimal_request();
        req_with.additional_params = Some(serde_json::json!({ "unrelated": true }));
        let mut req_without = minimal_request();
        req_without.additional_params = None;

        let built_with = build_request(&model, &req_with).unwrap();
        let built_without = build_request(&model, &req_without).unwrap();

        assert_eq!(built_with.reasoning, built_without.reasoning);
    }

    #[test]
    fn test_additional_params_invalid_effort_string_is_ignored() {
        let model = make_model("gpt-5.2", Some(ReasoningEffort::Medium));
        let mut req = minimal_request();
        req.additional_params = Some(serde_json::json!({
            "reasoning": { "effort": "ultra-high" }
        }));
        let built = build_request(&model, &req).unwrap();

        let reasoning = built.reasoning.expect("must have reasoning config");
        assert_eq!(
            reasoning.effort,
            Some(OAReasoningEffort::Medium),
            "invalid effort string must be ignored, preserving the model struct value"
        );
    }
}

#[cfg(test)]
fn _msg_unused_imports() {
    // Suppress unused MessageType warning if it's pulled in by a test
    // module that doesn't end up using it after pruning.
    let _ = MessageType::Message;
}
