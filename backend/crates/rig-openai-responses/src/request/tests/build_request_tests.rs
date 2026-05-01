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
