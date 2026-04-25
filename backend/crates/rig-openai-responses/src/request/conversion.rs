//! Per-message → Responses API conversion.
//!
//! Three pure functions:
//! - [`convert_user_content`] — `User` messages → `InputItem`s, with
//!   text/image/tool-result handling (and base64 data-URL packing for
//!   inline images).
//! - [`convert_assistant_content_to_items`] — `Assistant` messages →
//!   `InputItem`s, with reasoning-item round-tripping (so
//!   `encrypted_content` survives stateless multi-turn).
//! - [`convert_tool_definition`] — rig `ToolDefinition` → async-openai
//!   `FunctionTool`.

use async_openai::types::responses::{
    EasyInputContent, EasyInputMessage, FunctionCallOutput, FunctionCallOutputItemParam,
    FunctionTool, FunctionToolCall, ImageDetail, InputContent, InputImageContent, InputItem,
    InputTextContent, Item, MessageType, ReasoningItem, Role, Summary, SummaryPart, Tool,
};
use rig::completion::{AssistantContent, ToolDefinition};
use rig::message::UserContent;
use rig::one_or_many::OneOrMany;

/// Convert user content to OpenAI `InputItem`s, handling text, images,
/// and tool results.
///
/// For text and images, returns an `EasyInputMessage`. For tool results,
/// returns structured `Item::FunctionCallOutput`.
pub(crate) fn convert_user_content(content: &OneOrMany<UserContent>) -> Vec<InputItem> {
    use base64::Engine;

    let mut has_images = false;
    let mut input_parts: Vec<InputContent> = Vec::new();
    let mut result_items: Vec<InputItem> = Vec::new();

    /// Helper to flush pending text/image content into an `EasyInputMessage`.
    fn flush_pending(
        parts: &mut Vec<InputContent>,
        has_img: bool,
        result_items: &mut Vec<InputItem>,
    ) {
        if parts.is_empty() {
            return;
        }

        let content = if has_img {
            EasyInputContent::ContentList(parts.clone())
        } else {
            // For text-only, join all text parts.
            let text = parts
                .iter()
                .filter_map(|p| {
                    if let InputContent::InputText(t) = p {
                        Some(t.text.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            EasyInputContent::Text(text)
        };

        result_items.push(InputItem::EasyMessage(EasyInputMessage {
            r#type: MessageType::Message,
            role: Role::User,
            content,
        }));

        parts.clear();
    }

    for c in content.iter() {
        match c {
            UserContent::Text(text) => {
                if !text.text.is_empty() {
                    input_parts.push(InputContent::InputText(InputTextContent {
                        text: text.text.clone(),
                    }));
                }
            }
            UserContent::Image(img) => {
                // Convert rig Image to OpenAI InputImageContent.
                let image_url = match &img.data {
                    rig::message::DocumentSourceKind::Base64(b64) => {
                        // Already base64, construct data URL.
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
                            })
                            .unwrap_or("image/png");
                        format!("data:{};base64,{}", media_type, b64)
                    }
                    rig::message::DocumentSourceKind::Url(url) => {
                        // Direct URL.
                        url.clone()
                    }
                    rig::message::DocumentSourceKind::Raw(bytes) => {
                        // Raw bytes, encode to base64.
                        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
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
                            })
                            .unwrap_or("image/png");
                        format!("data:{};base64,{}", media_type, b64)
                    }
                    // Handle any future variants added to this
                    // non-exhaustive enum.
                    _ => {
                        tracing::warn!("Unsupported image source kind, skipping");
                        continue;
                    }
                };

                // Convert rig ImageDetail to async-openai ImageDetail.
                let detail = img
                    .detail
                    .as_ref()
                    .map(|d| {
                        use rig::message::ImageDetail as RigImageDetail;
                        match d {
                            RigImageDetail::Auto => ImageDetail::Auto,
                            RigImageDetail::High => ImageDetail::High,
                            RigImageDetail::Low => ImageDetail::Low,
                        }
                    })
                    .unwrap_or(ImageDetail::Auto);

                input_parts.push(InputContent::InputImage(InputImageContent {
                    detail,
                    file_id: None,
                    image_url: Some(image_url),
                }));
                has_images = true;
            }
            UserContent::ToolResult(result) => {
                // Flush any pending text/image content before adding tool
                // result.
                flush_pending(&mut input_parts, has_images, &mut result_items);
                has_images = false;

                // Extract text from tool result content.
                let result_text = result
                    .content
                    .iter()
                    .filter_map(|item| {
                        if let rig::message::ToolResultContent::Text(t) = item {
                            Some(t.text.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                // Use structured FunctionCallOutput with proper call_id linkage.
                //
                // Note: `id` and `status` are generated by the API for output
                // items and must NOT be set on input items — OpenAI rejects
                // unknown input parameters.
                let call_id = result.call_id.clone().unwrap_or_else(|| result.id.clone());
                result_items.push(InputItem::Item(Item::FunctionCallOutput(
                    FunctionCallOutputItemParam {
                        call_id,
                        output: FunctionCallOutput::Text(result_text),
                        id: None,
                        status: None,
                    },
                )));
            }
            // Skip other content types (Audio, Video, Document) not
            // supported yet.
            _ => {
                tracing::debug!("Skipping unsupported user content type");
            }
        }
    }

    // Flush any remaining text/image content.
    flush_pending(&mut input_parts, has_images, &mut result_items);

    result_items
}

/// Convert assistant content to OpenAI `InputItem`s, handling text,
/// tool calls, and reasoning.
///
/// Returns structured items for tool calls (`Item::FunctionCall`),
/// reasoning (`Item::Reasoning`), and text (`EasyInputMessage`).
///
/// IMPORTANT: For reasoning models (GPT-5, o-series), reasoning items
/// must be passed back with tool call outputs.
/// See: <https://platform.openai.com/docs/guides/function-calling>
pub(crate) fn convert_assistant_content_to_items(
    content: &OneOrMany<AssistantContent>,
) -> Vec<InputItem> {
    let mut items: Vec<InputItem> = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();

    /// Helper to flush pending text content into an `EasyInputMessage`.
    fn flush_text(text_parts: &mut Vec<String>, items: &mut Vec<InputItem>) {
        if !text_parts.is_empty() {
            let combined_text = text_parts.join("\n");
            items.push(InputItem::EasyMessage(EasyInputMessage {
                r#type: MessageType::Message,
                role: Role::Assistant,
                content: EasyInputContent::Text(combined_text),
            }));
            text_parts.clear();
        }
    }

    for c in content.iter() {
        match c {
            AssistantContent::Text(text) => {
                text_parts.push(text.text.clone());
            }
            AssistantContent::ToolCall(tc) => {
                // Flush any pending text before adding tool call.
                flush_text(&mut text_parts, &mut items);

                // Emit structured tool call.
                //
                // Note: `id` and `status` are output-only fields. Only
                // `call_id` is needed on input to link to the
                // corresponding `function_call_output`.
                let arguments = serde_json::to_string(&tc.function.arguments)
                    .unwrap_or_else(|_| "{}".to_string());
                let call_id = tc.call_id.clone().unwrap_or_else(|| tc.id.clone());
                items.push(InputItem::Item(Item::FunctionCall(FunctionToolCall {
                    arguments,
                    call_id,
                    name: tc.function.name.clone(),
                    id: None,
                    status: None,
                })));
            }
            AssistantContent::Reasoning(reasoning) => {
                // Flush any pending text before adding reasoning.
                flush_text(&mut text_parts, &mut items);

                // For reasoning models, we MUST include reasoning items
                // in the conversation. Convert rig Reasoning to OpenAI
                // ReasoningItem.
                let id = reasoning.id.clone().unwrap_or_else(|| {
                    // Generate a unique ID if not provided.
                    format!(
                        "rs_{:x}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    )
                });

                // Convert reasoning content to summary parts, and
                // extract signature.
                let mut summary: Vec<SummaryPart> = Vec::new();
                let mut encrypted_content: Option<String> = None;
                for rc in &reasoning.content {
                    match rc {
                        rig::message::ReasoningContent::Text { text, signature } => {
                            summary.push(SummaryPart::SummaryText(Summary { text: text.clone() }));
                            if encrypted_content.is_none() {
                                if let Some(sig) = signature {
                                    encrypted_content = Some(sig.clone());
                                }
                            }
                        }
                        rig::message::ReasoningContent::Summary(text) => {
                            summary.push(SummaryPart::SummaryText(Summary { text: text.clone() }));
                        }
                        _ => {}
                    }
                }

                // Pass through encrypted_content (stored in `signature`
                // on content blocks) for stateless operation. This is
                // required for multi-turn conversations with reasoning
                // models when not using `previous_response_id`.
                // See: <https://platform.openai.com/docs/guides/reasoning>

                // Log whether encrypted_content is present (critical for
                // multi-turn).
                if encrypted_content.is_some() {
                    tracing::debug!(
                        "[OpenAI] Converting reasoning {} WITH encrypted_content ({} bytes)",
                        id,
                        encrypted_content.as_ref().map(|s| s.len()).unwrap_or(0)
                    );
                } else {
                    tracing::warn!(
                        "[OpenAI] Converting reasoning {} WITHOUT encrypted_content! \
                         This will cause 'provided without its required following item' error on next turn.",
                        id
                    );
                }

                items.push(InputItem::Item(Item::Reasoning(ReasoningItem {
                    id,
                    summary,
                    content: None,
                    encrypted_content,
                    status: None,
                })));
            }
            _ => {
                // Skip other content types.
            }
        }
    }

    // Flush any remaining text.
    flush_text(&mut text_parts, &mut items);

    items
}

/// Convert a rig `ToolDefinition` to an async-openai `Tool`.
pub(crate) fn convert_tool_definition(tool: &ToolDefinition) -> Tool {
    Tool::Function(FunctionTool {
        name: tool.name.clone(),
        description: Some(tool.description.clone()),
        parameters: Some(tool.parameters.clone()),
        strict: Some(true),
    })
}
