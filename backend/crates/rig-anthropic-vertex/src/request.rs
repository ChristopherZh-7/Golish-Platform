//! Request building for the Anthropic Vertex provider.
//!
//! Pure transformations: rig `CompletionRequest` → `types::CompletionRequest`.
//! No HTTP, no streaming.  The flow is:
//!
//! 1. [`build_request`] — top-level entry, called by both `completion()` and
//!    `stream()` with a `stream` bool that flips `Some(true)` on the resulting
//!    request body.
//! 2. [`convert_message`] — rig `Message` → Anthropic `types::Message`,
//!    handling text / image / tool-result for users and text / tool-call /
//!    thinking blocks for assistants.  Critically, when extended thinking is
//!    enabled the assistant's `thinking` blocks must come **first** in the
//!    content list, which is enforced here.
//! 3. [`convert_tool`] — rig `ToolDefinition` → Anthropic `ToolEntry::Function`.

use rig::completion::{AssistantContent, CompletionRequest, Message, ToolDefinition};
use rig::message::ReasoningContent;

use crate::completion::CompletionModel;
use crate::config::default_max_tokens_for_model;
use crate::types::{
    self, CacheControl, ContentBlock, ImageSource, Role, SystemBlock, ToolEntry, ANTHROPIC_VERSION,
};

/// Build an Anthropic request from a rig `CompletionRequest`.
///
/// `stream` toggles the top-level `stream` field on the resulting body.
/// All other fields (messages, tools, thinking, beta opt-ins) are derived
/// from `model` and `request`.
pub(crate) fn build_request(
    model: &CompletionModel,
    request: &CompletionRequest,
    stream: bool,
) -> types::CompletionRequest {
    // Convert chat history to messages
    let mut messages: Vec<types::Message> = request.chat_history.iter().map(convert_message).collect();

    // Add normalized documents as user messages
    for doc in &request.documents {
        messages.push(types::Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: format!("[Document: {}]\n{}", doc.id, doc.text),
                cache_control: None,
            }],
        });
    }

    // Determine max tokens
    let mut max_tokens = request
        .max_tokens
        .map(|t| t as u32)
        .unwrap_or_else(|| default_max_tokens_for_model(model.model()));

    // When thinking is enabled, max_tokens must be greater than budget_tokens
    if let Some(thinking) = model.thinking_config() {
        let min_required = thinking.budget_tokens + 1;
        if max_tokens <= thinking.budget_tokens {
            max_tokens = min_required.max(thinking.budget_tokens + 8192);
        }
    }

    // Convert function tools and add server tools
    let mut tool_entries: Vec<ToolEntry> = request.tools.iter().map(convert_tool).collect();

    // Add cache_control to the last function tool for caching
    if !tool_entries.is_empty() {
        // Find the last function tool and add cache_control
        for entry in tool_entries.iter_mut().rev() {
            if let ToolEntry::Function(ref mut tool_def) = entry {
                tool_def.cache_control = Some(CacheControl::ephemeral());
                break;
            }
        }
    }

    // Add server tools (web_search, web_fetch) if configured
    tool_entries.extend(model.build_server_tools());

    let tools: Option<Vec<ToolEntry>> = if tool_entries.is_empty() {
        None
    } else {
        Some(tool_entries)
    };

    // When thinking is enabled, temperature must be 1
    let temperature = if model.thinking_config().is_some() {
        Some(1.0)
    } else {
        request.temperature.map(|t| t as f32)
    };

    // Log message content block statistics for debugging
    for (i, msg) in messages.iter().enumerate() {
        let image_count = msg
            .content
            .iter()
            .filter(|b| matches!(b, ContentBlock::Image { .. }))
            .count();
        let text_count = msg
            .content
            .iter()
            .filter(|b| matches!(b, ContentBlock::Text { .. }))
            .count();
        if image_count > 0 {
            tracing::info!(
                "build_request: Message {} ({:?}) has {} text blocks, {} image blocks",
                i,
                msg.role,
                text_count,
                image_count
            );
        }
    }

    types::CompletionRequest {
        anthropic_version: ANTHROPIC_VERSION.to_string(),
        messages,
        max_tokens,
        system: request.preamble.as_ref().map(|preamble| {
            // Convert string to array format with cache_control enabled
            vec![SystemBlock::cached(preamble.clone())]
        }),
        temperature,
        top_p: None,
        top_k: None,
        stop_sequences: None,
        tools,
        stream: if stream { Some(true) } else { None },
        thinking: model.thinking_config().cloned(),
    }
}

/// Convert rig's `Message` to Anthropic message format.
///
/// For assistants with extended-thinking content, the resulting block list has
/// `Thinking` blocks first followed by everything else — Anthropic's API
/// requires that ordering.
pub(crate) fn convert_message(msg: &Message) -> types::Message {
    match msg {
        Message::User { content } => {
            let blocks: Vec<ContentBlock> = content
                .iter()
                .filter_map(|c| {
                    use rig::message::UserContent;
                    match c {
                        UserContent::Text(text) => Some(ContentBlock::Text {
                            text: text.text.clone(),
                            cache_control: None,
                        }),
                        UserContent::Image(img) => {
                            // Extract base64 data from rig's Image type
                            use base64::Engine;
                            let data = match &img.data {
                                rig::message::DocumentSourceKind::Base64(b64) => b64.clone(),
                                rig::message::DocumentSourceKind::Url(_url) => {
                                    tracing::warn!("Image URLs not yet supported, skipping");
                                    return None;
                                }
                                rig::message::DocumentSourceKind::Raw(bytes) => {
                                    base64::engine::general_purpose::STANDARD.encode(bytes)
                                }
                                // Handle any future variants added to this non-exhaustive enum
                                _ => {
                                    tracing::warn!("Unsupported image source kind, skipping");
                                    return None;
                                }
                            };

                            let media_type = img
                                .media_type
                                .as_ref()
                                .map(|mt| {
                                    use rig::message::ImageMediaType;
                                    match mt {
                                        ImageMediaType::PNG => "image/png",
                                        ImageMediaType::JPEG => "image/jpeg",
                                        ImageMediaType::GIF => "image/gif",
                                        ImageMediaType::WEBP => "image/webp",
                                        ImageMediaType::HEIC => "image/heic",
                                        ImageMediaType::HEIF => "image/heif",
                                        ImageMediaType::SVG => "image/svg+xml",
                                    }
                                    .to_string()
                                })
                                .unwrap_or_else(|| "image/png".to_string());

                            Some(ContentBlock::Image {
                                source: ImageSource {
                                    source_type: "base64".to_string(),
                                    media_type,
                                    data,
                                },
                                cache_control: None,
                            })
                        }
                        UserContent::ToolResult(result) => Some(ContentBlock::ToolResult {
                            tool_use_id: result.id.clone(),
                            content: serde_json::to_string(&result.content).unwrap_or_default(),
                            is_error: None,
                            cache_control: None,
                        }),
                        // Skip other content types (Audio, Video, Document) not supported yet
                        _ => None,
                    }
                })
                .collect();

            types::Message {
                role: Role::User,
                content: if blocks.is_empty() {
                    vec![ContentBlock::Text {
                        text: String::new(),
                        cache_control: None,
                    }]
                } else {
                    blocks
                },
            }
        }
        Message::Assistant { content, .. } => {
            // When thinking is enabled, assistant messages must start with thinking blocks
            // Collect thinking blocks first, then other content
            let mut thinking_blocks: Vec<ContentBlock> = Vec::new();
            let mut other_blocks: Vec<ContentBlock> = Vec::new();

            for c in content.iter() {
                match c {
                    AssistantContent::Text(text) => {
                        other_blocks.push(ContentBlock::Text {
                            text: text.text.clone(),
                            cache_control: None,
                        });
                    }
                    AssistantContent::ToolCall(tool_call) => {
                        // Ensure input is always a valid object (Anthropic API requirement)
                        let input = match &tool_call.function.arguments {
                            serde_json::Value::Object(_) => tool_call.function.arguments.clone(),
                            serde_json::Value::Null => serde_json::json!({}),
                            other => serde_json::json!({ "value": other }),
                        };
                        other_blocks.push(ContentBlock::ToolUse {
                            id: tool_call.id.clone(),
                            name: tool_call.function.name.clone(),
                            input,
                        });
                    }
                    AssistantContent::Reasoning(reasoning) => {
                        // Include thinking blocks for extended thinking mode
                        // Extract text and signature from content vector
                        let mut thinking_text = String::new();
                        let mut sig = String::new();
                        for rc in &reasoning.content {
                            if let ReasoningContent::Text { text, signature } = rc {
                                thinking_text.push_str(text);
                                if let Some(s) = signature {
                                    sig = s.clone();
                                }
                            }
                        }
                        if !thinking_text.is_empty() {
                            thinking_blocks.push(ContentBlock::Thinking {
                                thinking: thinking_text,
                                // Signature is required but we may not have it from history
                                // Use empty string as placeholder (API may reject this)
                                signature: sig,
                            });
                        }
                    }
                    AssistantContent::Image(_) => {
                        // Images in assistant content are not supported by Anthropic API
                        // Skip them silently
                    }
                }
            }

            // Combine: thinking blocks first (required by API), then other content
            let mut blocks = thinking_blocks;
            blocks.append(&mut other_blocks);

            types::Message {
                role: Role::Assistant,
                content: if blocks.is_empty() {
                    vec![ContentBlock::Text {
                        text: String::new(),
                        cache_control: None,
                    }]
                } else {
                    blocks
                },
            }
        }
    }
}

/// Convert rig's `ToolDefinition` to Anthropic format as a `ToolEntry`.
pub(crate) fn convert_tool(tool: &ToolDefinition) -> ToolEntry {
    ToolEntry::Function(types::ToolDefinition {
        name: tool.name.clone(),
        description: tool.description.clone(),
        input_schema: tool.parameters.clone(),
        cache_control: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::message::{DocumentSourceKind, Image, ImageMediaType, Text, UserContent};
    use rig::one_or_many::OneOrMany;

    #[test]
    fn test_convert_message_with_image() {
        // Create a rig Message with text and image content
        let image = Image {
            data: DocumentSourceKind::Base64("iVBORw0KGgoAAAANSUhEUg==".to_string()),
            media_type: Some(ImageMediaType::PNG),
            detail: None,
            additional_params: None,
        };

        let content = vec![
            UserContent::Text(Text {
                text: "What is in this image?".to_string(),
            }),
            UserContent::Image(image),
        ];

        let msg = Message::User {
            content: OneOrMany::many(content).unwrap(),
        };

        let converted = convert_message(&msg);

        assert_eq!(converted.content.len(), 2, "Should have 2 content blocks");

        match &converted.content[0] {
            ContentBlock::Text { text, .. } => {
                assert_eq!(text, "What is in this image?");
            }
            _ => panic!("Expected Text block at index 0"),
        }

        match &converted.content[1] {
            ContentBlock::Image { source, .. } => {
                assert_eq!(source.source_type, "base64");
                assert_eq!(source.media_type, "image/png");
                assert_eq!(source.data, "iVBORw0KGgoAAAANSUhEUg==");
            }
            _ => panic!("Expected Image block at index 1"),
        }

        // Verify JSON serialization
        let json = serde_json::to_string_pretty(&converted).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["role"], "user");
        assert_eq!(parsed["content"][0]["type"], "text");
        assert_eq!(parsed["content"][1]["type"], "image");
        assert_eq!(parsed["content"][1]["source"]["type"], "base64");
        assert_eq!(parsed["content"][1]["source"]["media_type"], "image/png");
    }

    #[test]
    fn test_convert_message_image_only() {
        let image = Image {
            data: DocumentSourceKind::Base64("YWJjZGVm".to_string()),
            media_type: Some(ImageMediaType::JPEG),
            detail: None,
            additional_params: None,
        };

        let content = vec![UserContent::Image(image)];

        let msg = Message::User {
            content: OneOrMany::many(content).unwrap(),
        };

        let converted = convert_message(&msg);

        assert_eq!(converted.content.len(), 1, "Should have 1 content block");

        match &converted.content[0] {
            ContentBlock::Image { source, .. } => {
                assert_eq!(source.source_type, "base64");
                assert_eq!(source.media_type, "image/jpeg");
                assert_eq!(source.data, "YWJjZGVm");
            }
            _ => panic!("Expected Image block"),
        }
    }
}
