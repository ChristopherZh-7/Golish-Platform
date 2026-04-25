//! Executor helper tests.

use super::*;
use super::*;
use golish_llm_providers::ModelCapabilities;
use rig::message::{ReasoningContent, ToolFunction};

fn make_tool_call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        call_id: Some(id.to_string()),
        function: ToolFunction {
            name: name.to_string(),
            arguments: serde_json::json!({}),
        },
        signature: None,
        additional_params: None,
    }
}

#[test]
fn test_build_assistant_content_with_thinking_and_tools() {
    // When thinking is supported and present, it should come FIRST
    let tool_calls = vec![make_tool_call("tc_1", "read_file")];
    let content = build_assistant_content(
        true,                        // supports_thinking_history
        "Let me analyze this...",    // thinking_text
        Some("thinking_123".into()), // thinking_id
        Some("sig_abc".into()),      // thinking_signature
        "I'll read the file.",       // text_content
        &tool_calls,
    );

    assert_eq!(content.len(), 3);

    // First element should be Reasoning
    assert!(
        matches!(&content[0], AssistantContent::Reasoning(_)),
        "First content should be Reasoning, got {:?}",
        content[0]
    );

    // Second element should be Text
    assert!(
        matches!(&content[1], AssistantContent::Text(_)),
        "Second content should be Text"
    );

    // Third element should be ToolCall
    assert!(
        matches!(&content[2], AssistantContent::ToolCall(_)),
        "Third content should be ToolCall"
    );
}

#[test]
fn test_build_assistant_content_thinking_id_only() {
    // OpenAI Responses API may have reasoning ID but empty content
    let tool_calls = vec![make_tool_call("tc_1", "read_file")];
    let content = build_assistant_content(
        true,                  // supports_thinking_history
        "",                    // thinking_text (empty)
        Some("rs_123".into()), // thinking_id (present)
        None,                  // thinking_signature
        "",                    // text_content
        &tool_calls,
    );

    assert_eq!(content.len(), 2);

    // First element should be Reasoning (even with empty content, ID triggers inclusion)
    assert!(
        matches!(&content[0], AssistantContent::Reasoning(_)),
        "First content should be Reasoning when thinking_id is present"
    );

    // Second element should be ToolCall
    assert!(
        matches!(&content[1], AssistantContent::ToolCall(_)),
        "Second content should be ToolCall"
    );
}

#[test]
fn test_build_assistant_content_no_thinking_support() {
    // When model doesn't support thinking history, no Reasoning should be added
    let tool_calls = vec![make_tool_call("tc_1", "read_file")];
    let content = build_assistant_content(
        false,                       // supports_thinking_history = false
        "Some thinking content",     // thinking_text (ignored)
        Some("thinking_123".into()), // thinking_id (ignored)
        Some("sig_abc".into()),      // thinking_signature (ignored)
        "I'll read the file.",       // text_content
        &tool_calls,
    );

    assert_eq!(content.len(), 2);

    // First element should be Text (no Reasoning)
    assert!(
        matches!(&content[0], AssistantContent::Text(_)),
        "First content should be Text when thinking not supported"
    );

    // Second element should be ToolCall
    assert!(
        matches!(&content[1], AssistantContent::ToolCall(_)),
        "Second content should be ToolCall"
    );
}

#[test]
fn test_build_assistant_content_no_thinking_content() {
    // When there's no thinking content and no ID, no Reasoning should be added
    let tool_calls = vec![make_tool_call("tc_1", "read_file")];
    let content = build_assistant_content(
        true, // supports_thinking_history
        "",   // thinking_text (empty)
        None, // thinking_id (none)
        None, // thinking_signature
        "Response text",
        &tool_calls,
    );

    assert_eq!(content.len(), 2);

    // First element should be Text (no Reasoning since both text and id are empty)
    assert!(
        matches!(&content[0], AssistantContent::Text(_)),
        "First content should be Text when no thinking content"
    );
}

#[test]
fn test_build_assistant_content_tools_only() {
    // Tool calls only, no text or thinking
    let tool_calls = vec![
        make_tool_call("tc_1", "read_file"),
        make_tool_call("tc_2", "write_file"),
    ];
    let content = build_assistant_content(true, "", None, None, "", &tool_calls);

    assert_eq!(content.len(), 2);
    assert!(matches!(&content[0], AssistantContent::ToolCall(_)));
    assert!(matches!(&content[1], AssistantContent::ToolCall(_)));
}

#[test]
fn test_build_assistant_content_empty() {
    // Edge case: no content at all
    let content = build_assistant_content(true, "", None, None, "", &[]);

    assert!(content.is_empty());
}

#[test]
fn test_build_assistant_content_thinking_with_signature() {
    // Verify signature is included when provided
    let content = build_assistant_content(
        true,
        "Thinking...",
        None,
        Some("signature_xyz".into()),
        "",
        &[],
    );

    assert_eq!(content.len(), 1);
    if let AssistantContent::Reasoning(reasoning) = &content[0] {
        assert!(matches!(
            reasoning.content.first(),
            Some(ReasoningContent::Text { signature: Some(sig), .. }) if sig == "signature_xyz"
        ));
    } else {
        panic!("Expected Reasoning content");
    }
}

#[test]
fn test_anthropic_vertex_model_capabilities() {
    // Verify Anthropic/Vertex models support thinking history
    // Multiple provider name aliases are supported for compatibility
    let caps = ModelCapabilities::detect("anthropic_vertex", "claude-sonnet-4-20250514");
    assert!(
        caps.supports_thinking_history,
        "anthropic_vertex should support thinking history"
    );

    let caps = ModelCapabilities::detect("vertex_ai", "claude-sonnet-4-20250514");
    assert!(
        caps.supports_thinking_history,
        "vertex_ai should support thinking history"
    );

    let caps = ModelCapabilities::detect("vertex_ai_anthropic", "claude-3-5-sonnet");
    assert!(
        caps.supports_thinking_history,
        "vertex_ai_anthropic should support thinking history"
    );

    let caps = ModelCapabilities::detect("anthropic", "claude-3-opus");
    assert!(
        caps.supports_thinking_history,
        "anthropic should support thinking history"
    );
}

#[test]
fn test_non_thinking_model_capabilities() {
    // Verify models that don't support thinking are detected correctly
    let caps = ModelCapabilities::detect("groq", "llama-3.3-70b");
    assert!(
        !caps.supports_thinking_history,
        "Groq should not support thinking history"
    );

    let caps = ModelCapabilities::detect("ollama", "llama3.2");
    assert!(
        !caps.supports_thinking_history,
        "Ollama should not support thinking history"
    );
}

#[test]
fn test_build_assistant_content_text_only() {
    // Text only, no thinking, no tools
    let content = build_assistant_content(true, "", None, None, "Just a text response", &[]);

    assert_eq!(content.len(), 1);
    if let AssistantContent::Text(text) = &content[0] {
        assert_eq!(text.text, "Just a text response");
    } else {
        panic!("Expected Text content");
    }
}

#[test]
fn test_build_assistant_content_verifies_values() {
    // Verify actual content values, not just types
    let tool_calls = vec![make_tool_call("tc_123", "read_file")];
    let content = build_assistant_content(
        true,
        "My thinking process",
        Some("id_456".into()),
        Some("sig_789".into()),
        "My response text",
        &tool_calls,
    );

    assert_eq!(content.len(), 3);

    // Verify Reasoning content
    if let AssistantContent::Reasoning(reasoning) = &content[0] {
        assert_eq!(reasoning.content, vec![ReasoningContent::Text {
            text: "My thinking process".to_string(),
            signature: Some("sig_789".to_string()),
        }]);
        assert_eq!(reasoning.id, Some("id_456".to_string()));
    } else {
        panic!("Expected Reasoning content at index 0");
    }

    // Verify Text content
    if let AssistantContent::Text(text) = &content[1] {
        assert_eq!(text.text, "My response text");
    } else {
        panic!("Expected Text content at index 1");
    }

    // Verify ToolCall content
    if let AssistantContent::ToolCall(tc) = &content[2] {
        assert_eq!(tc.id, "tc_123");
        assert_eq!(tc.function.name, "read_file");
    } else {
        panic!("Expected ToolCall content at index 2");
    }
}

#[test]
fn test_build_assistant_content_multiple_tools_preserve_order() {
    // Multiple tool calls should preserve their order
    let tool_calls = vec![
        make_tool_call("tc_1", "read_file"),
        make_tool_call("tc_2", "write_file"),
        make_tool_call("tc_3", "list_dir"),
    ];
    let content = build_assistant_content(
        false, // no thinking
        "",
        None,
        None,
        "",
        &tool_calls,
    );

    assert_eq!(content.len(), 3);

    // Verify order is preserved
    let names: Vec<&str> = content
        .iter()
        .filter_map(|c| {
            if let AssistantContent::ToolCall(tc) = c {
                Some(tc.function.name.as_str())
            } else {
                None
            }
        })
        .collect();

    assert_eq!(names, vec!["read_file", "write_file", "list_dir"]);
}

#[test]
fn test_build_assistant_content_thinking_only() {
    // Thinking only, no text, no tools
    let content = build_assistant_content(true, "Just thinking aloud...", None, None, "", &[]);

    assert_eq!(content.len(), 1);
    assert!(matches!(&content[0], AssistantContent::Reasoning(_)));
}

#[test]
fn test_openai_responses_api_model_capabilities() {
    // OpenAI Responses API always needs reasoning history preserved
    let caps = ModelCapabilities::detect("openai_responses", "gpt-4o");
    assert!(
        caps.supports_thinking_history,
        "OpenAI Responses API should support thinking history"
    );

    let caps = ModelCapabilities::detect("openai_responses", "o3-mini");
    assert!(
        caps.supports_thinking_history,
        "OpenAI Responses API with o3 should support thinking history"
    );
}

#[test]
fn test_zai_model_capabilities() {
    // Z.AI GLM-4.7 supports thinking, GLM-4.5 does not
    let caps = ModelCapabilities::detect("zai", "GLM-4.7");
    assert!(
        caps.supports_thinking_history,
        "Z.AI GLM-4.7 should support thinking history"
    );

    let caps = ModelCapabilities::detect("zai", "glm-4.7-flash");
    assert!(
        caps.supports_thinking_history,
        "Z.AI GLM-4.7-flash should support thinking history"
    );

    let caps = ModelCapabilities::detect("zai", "GLM-4.5-air");
    assert!(
        !caps.supports_thinking_history,
        "Z.AI GLM-4.5 should not support thinking history"
    );
}

#[test]
fn test_build_assistant_content_with_id_and_signature() {
    // Both ID and signature present (Anthropic case with streaming)
    let content = build_assistant_content(
        true,
        "Extended thinking...",
        Some("thinking_id_abc".into()),
        Some("signature_xyz".into()),
        "",
        &[make_tool_call("tc_1", "bash")],
    );

    assert_eq!(content.len(), 2);

    if let AssistantContent::Reasoning(reasoning) = &content[0] {
        assert_eq!(reasoning.id, Some("thinking_id_abc".to_string()));
        assert!(matches!(
            reasoning.content.first(),
            Some(ReasoningContent::Text { signature: Some(sig), .. }) if sig == "signature_xyz"
        ));
        assert!(!reasoning.content.is_empty());
    } else {
        panic!("Expected Reasoning content");
    }
}

#[test]
fn test_openrouter_does_not_support_thinking() {
    // OpenRouter proxies requests but doesn't have native thinking support
    let caps = ModelCapabilities::detect("openrouter", "anthropic/claude-3-opus");
    assert!(
        !caps.supports_thinking_history,
        "OpenRouter should not support thinking history (proxy)"
    );
}
