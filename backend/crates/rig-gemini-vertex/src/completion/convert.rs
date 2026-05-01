//! Pure conversion helpers between rig types and Gemini Vertex AI types.
//!
//! Lives outside `mod.rs` so the request/response massaging stays separate
//! from the `CompletionModel` orchestration. These are intentionally
//! `pub(super)` free functions so the `CompletionModel` trait impl can call
//! them while the public API surface stays in `mod.rs`.

use rig::completion::{AssistantContent, CompletionRequest, CompletionResponse, Message, ToolDefinition, Usage};
use rig::one_or_many::OneOrMany;

use crate::types::{
    self, Content, FunctionDeclaration, GenerateContentRequest, GenerationConfig, Part, Tool,
    DEFAULT_MAX_TOKENS,
};

use super::CompletionModel;

/// Default max tokens for different Gemini models.
pub(super) fn default_max_tokens_for_model(model: &str) -> u32 {
    if model.contains("2.0") {
        // Gemini 2.0 models have 8K max output.
        8192
    } else {
        // Gemini 2.5+ and 3.x have 64K max output.
        DEFAULT_MAX_TOKENS
    }
}

/// Convert rig's `Message` to Gemini `Content` format.
pub(super) fn convert_message(msg: &Message) -> Content {
    match msg {
        Message::System { content } => Content {
            role: Some("user".to_string()),
            parts: vec![Part::text(content)],
        },
        Message::User { content } => {
            let parts: Vec<Part> = content
                .iter()
                .filter_map(|c| {
                    use rig::message::UserContent;
                    match c {
                        UserContent::Text(text) => Some(Part::text(&text.text)),
                        UserContent::Image(img) => convert_user_image(img),
                        UserContent::ToolResult(result) => {
                            let response = serde_json::json!({ "result": result.content });
                            Some(Part::function_response(&result.id, response))
                        }
                        _ => None,
                    }
                })
                .collect();

            Content {
                role: Some("user".to_string()),
                parts: if parts.is_empty() {
                    vec![Part::text("")]
                } else {
                    parts
                },
            }
        }
        Message::Assistant { content, .. } => {
            let parts: Vec<Part> = content
                .iter()
                .filter_map(|c| match c {
                    AssistantContent::Text(text) => Some(Part::text(&text.text)),
                    AssistantContent::ToolCall(tool_call) => {
                        let mut part = Part::function_call(
                            &tool_call.function.name,
                            tool_call.function.arguments.clone(),
                        );
                        // Include thought signature if present (required for thinking models).
                        part.thought_signature = tool_call.signature.clone();
                        Some(part)
                    }
                    // Thinking content is dropped — we cannot reconstruct it for replay.
                    AssistantContent::Reasoning(_) => None,
                    // Images in assistant content are not supported.
                    AssistantContent::Image(_) => None,
                })
                .collect();

            Content {
                role: Some("model".to_string()),
                parts: if parts.is_empty() {
                    vec![Part::text("")]
                } else {
                    parts
                },
            }
        }
    }
}

/// Convert a user-supplied image to a Gemini inline-data `Part`.
///
/// Returns `None` (with a warning) for unsupported source kinds (e.g. URL refs)
/// rather than failing the whole request.
fn convert_user_image(img: &rig::message::Image) -> Option<Part> {
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

    Some(Part::inline_data(media_type, data))
}

/// Convert rig's `ToolDefinition` to Gemini `FunctionDeclaration`.
pub(super) fn convert_tool(tool: &ToolDefinition) -> FunctionDeclaration {
    // Use parametersJsonSchema which accepts standard JSON Schema format
    // (with lowercase type names like "string", "integer") rather than
    // Google's custom Schema format (with uppercase TYPE names).
    let parameters = if tool.parameters.is_null()
        || tool.parameters == serde_json::json!({})
        || tool
            .parameters
            .as_object()
            .is_some_and(|obj| obj.is_empty())
    {
        // For functions with no parameters, we need to provide a minimal object schema.
        Some(serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }))
    } else {
        Some(tool.parameters.clone())
    };

    FunctionDeclaration {
        name: tool.name.clone(),
        description: tool.description.clone(),
        parameters_json_schema: parameters,
    }
}

/// Build a Gemini request from a rig `CompletionRequest`.
pub(super) fn build_request(
    model: &CompletionModel,
    request: &CompletionRequest,
) -> GenerateContentRequest {
    let contents: Vec<Content> = request
        .chat_history
        .iter()
        .map(convert_message)
        .collect();

    let max_output_tokens = request
        .max_tokens
        .map(|t| t as i32)
        .unwrap_or_else(|| default_max_tokens_for_model(model.model()) as i32);

    let generation_config = Some(GenerationConfig {
        temperature: request.temperature.map(|t| t as f32),
        top_p: None,
        top_k: None,
        candidate_count: None,
        max_output_tokens: Some(max_output_tokens),
        stop_sequences: None,
        response_mime_type: None,
        response_schema: None,
        thinking_config: model.thinking().clone(),
    });

    let tools = if request.tools.is_empty() {
        None
    } else {
        let function_declarations: Vec<FunctionDeclaration> =
            request.tools.iter().map(convert_tool).collect();
        Some(vec![Tool {
            function_declarations: Some(function_declarations),
        }])
    };

    let system_instruction = request
        .preamble
        .as_ref()
        .map(|preamble| Content::system(preamble.clone()));

    GenerateContentRequest {
        contents,
        system_instruction,
        tools,
        tool_config: None,
        safety_settings: None,
        generation_config,
    }
}

/// Convert Gemini response to rig's `CompletionResponse`.
pub(super) fn convert_response(
    response: types::GenerateContentResponse,
) -> CompletionResponse<types::GenerateContentResponse> {
    use rig::message::{Text, ToolCall, ToolFunction};

    let mut content: Vec<AssistantContent> = vec![];

    if let Some(candidate) = response.candidates.first() {
        for part in &candidate.content.parts {
            if let Some(text) = &part.text {
                if !text.is_empty() {
                    if part.thought == Some(true) {
                        content.push(AssistantContent::Reasoning(
                            rig::message::Reasoning::multi(vec![text.clone()]),
                        ));
                    } else {
                        content.push(AssistantContent::Text(Text { text: text.clone() }));
                    }
                }
            }

            if let Some(fc) = &part.function_call {
                content.push(AssistantContent::ToolCall(ToolCall {
                    id: fc.name.clone(), // Gemini doesn't have separate IDs.
                    call_id: None,
                    function: ToolFunction {
                        name: fc.name.clone(),
                        arguments: fc.args.clone(),
                    },
                    signature: None,
                    additional_params: None,
                }));
            }
        }
    }

    let usage = response
        .usage_metadata
        .as_ref()
        .map(|u| Usage {
            input_tokens: u.prompt_token_count as u64,
            output_tokens: u.candidates_token_count as u64,
            total_tokens: u.total_token_count as u64,
            cached_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
        .unwrap_or_default();

    CompletionResponse {
        choice: OneOrMany::many(content).unwrap_or_else(|_| {
            OneOrMany::one(AssistantContent::Text(Text {
                text: String::new(),
            }))
        }),
        usage,
        raw_response: response,
        message_id: None,
    }
}
