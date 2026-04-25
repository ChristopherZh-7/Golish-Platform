//! Pure rig↔Z.AI conversion logic — no I/O, no async.
//!
//! Adds an `impl CompletionModel { … }` block with these helpers:
//! - [`Self::convert_message`] — rig `Message` → Z.AI `Message`.
//! - [`Self::convert_tool_result`] — bundle a tool-call output into a Z.AI tool message.
//! - [`Self::convert_tool`] — rig `ToolDefinition` → Z.AI `ToolDefinition`.
//! - [`Self::build_request`] — assemble the full `/chat/completions` payload.
//! - [`Self::convert_response`] — Z.AI `Completion` → rig `CompletionResponse`,
//!   including the pseudo-XML tool-call extraction in reasoning + text.

use rig::completion::{
    AssistantContent, CompletionRequest, CompletionResponse, Message, ToolDefinition, Usage,
};
use rig::message::{Reasoning, Text, ToolCall, ToolFunction, ToolResultContent, UserContent};
use rig::one_or_many::OneOrMany;

use crate::text_tool_parser;
use crate::types;

use super::{extract_user_text, CompletionModel, DEFAULT_MAX_TOKENS};

impl CompletionModel {
    /// Convert rig's `Message` to Z.AI message format.
    pub(super) fn convert_message(msg: &Message) -> types::Message {
        match msg {
            Message::User { content } => {
                let text = extract_user_text(content);
                types::Message {
                    role: types::Role::User,
                    content: types::MessageContent::Text(text),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                }
            }
            Message::Assistant { content, .. } => {
                let mut text_parts = Vec::new();
                let mut tool_calls = Vec::new();

                for c in content.iter() {
                    match c {
                        AssistantContent::Text(t) => {
                            text_parts.push(t.text.clone());
                        }
                        AssistantContent::ToolCall(tc) => {
                            tool_calls.push(types::ToolCall {
                                id: tc.id.clone(),
                                call_type: "function".to_string(),
                                function: types::FunctionCall {
                                    name: tc.function.name.clone(),
                                    arguments: serde_json::to_string(&tc.function.arguments)
                                        .unwrap_or_default(),
                                },
                            });
                        }
                        AssistantContent::Reasoning(r) => {
                            let reasoning_text: String = r
                                .content
                                .iter()
                                .filter_map(|c| match c {
                                    rig::message::ReasoningContent::Text { text, .. } => {
                                        Some(text.as_str())
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("");
                            text_parts.push(format!("[Reasoning]: {}", reasoning_text));
                        }
                        _ => {}
                    }
                }

                types::Message {
                    role: types::Role::Assistant,
                    content: types::MessageContent::Text(text_parts.join("\n")),
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                    name: None,
                }
            }
        }
    }

    /// Convert a tool result from user content to a Z.AI tool message.
    pub(super) fn convert_tool_result(
        tool_call_id: &str,
        content: &OneOrMany<ToolResultContent>,
    ) -> types::Message {
        let text: String = content
            .iter()
            .filter_map(|c| match c {
                ToolResultContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        types::Message {
            role: types::Role::Tool,
            content: types::MessageContent::Text(text),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.to_string()),
            name: None,
        }
    }

    /// Convert rig's `ToolDefinition` to Z.AI format.
    pub(super) fn convert_tool(tool: &ToolDefinition) -> types::ToolDefinition {
        types::ToolDefinition {
            tool_type: "function".to_string(),
            function: types::FunctionDefinition {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.parameters.clone(),
            },
        }
    }

    /// Build a Z.AI request from a rig `CompletionRequest`.
    pub(super) fn build_request(
        &self,
        request: &CompletionRequest,
        stream: bool,
    ) -> types::CompletionRequest {
        let mut messages = Vec::new();

        if let Some(ref preamble) = request.preamble {
            messages.push(types::Message::system(preamble.clone()));
        }

        for msg in request.chat_history.iter() {
            // Tool results are user-content variants in rig but separate
            // tool-role messages in Z.AI; pull them out and emit one
            // tool message per result.
            if let Message::User { content } = msg {
                for c in content.iter() {
                    if let UserContent::ToolResult(result) = c {
                        messages.push(Self::convert_tool_result(&result.id, &result.content));
                    }
                }
                let text = extract_user_text(content);
                if !text.is_empty()
                    && !content
                        .iter()
                        .all(|c| matches!(c, UserContent::ToolResult(_)))
                {
                    messages.push(types::Message::user(text));
                }
            } else {
                messages.push(Self::convert_message(msg));
            }
        }

        for doc in &request.documents {
            messages.push(types::Message::user(format!(
                "[Document: {}]\n{}",
                doc.id, doc.text
            )));
        }

        let tools: Option<Vec<types::ToolDefinition>> = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.iter().map(Self::convert_tool).collect())
        };

        // Clamp temperature to (0.0, 1.0) open interval as per Z.AI's API
        // requirements.
        let temperature = request.temperature.map(|t| {
            let t = t as f32;
            if t <= 0.0 {
                0.01
            } else if t >= 1.0 {
                0.99
            } else {
                t
            }
        });

        types::CompletionRequest {
            model: self.model.clone(),
            messages,
            stream: if stream { Some(true) } else { None },
            temperature,
            top_p: None,
            max_tokens: Some(
                request
                    .max_tokens
                    .map(|t| t as u32)
                    .unwrap_or(DEFAULT_MAX_TOKENS),
            ),
            stop: None,
            seed: None,
            tools,
            tool_choice: None,
            thinking: Some(types::ThinkingConfig::enabled()),
            tool_stream: if stream { Some(true) } else { None },
        }
    }

    /// Convert Z.AI response to rig's `CompletionResponse`.
    pub(super) fn convert_response(
        response: types::Completion,
    ) -> CompletionResponse<types::Completion> {
        let mut content: Vec<AssistantContent> = Vec::new();
        let mut pseudo_tool_call_counter = 0u32;

        if let Some(choice) = response.choices.first() {
            // Add reasoning content first if present, checking for
            // pseudo-XML tool calls.
            if let Some(ref reasoning) = choice.message.reasoning_content {
                if !reasoning.is_empty() {
                    if text_tool_parser::contains_pseudo_xml_tool_calls(reasoning) {
                        let (parsed_calls, remaining_reasoning) =
                            text_tool_parser::parse_tool_calls_from_text(reasoning);

                        if !remaining_reasoning.is_empty() {
                            content.push(AssistantContent::Reasoning(Reasoning::new(
                                &remaining_reasoning,
                            )));
                        }

                        for parsed in parsed_calls {
                            pseudo_tool_call_counter += 1;
                            let id = format!("pseudo_call_{}", pseudo_tool_call_counter);
                            tracing::info!(
                                "Extracted pseudo-XML tool call from reasoning content: {}",
                                parsed.name
                            );
                            content.push(AssistantContent::ToolCall(ToolCall {
                                id,
                                call_id: None,
                                function: ToolFunction {
                                    name: parsed.name,
                                    arguments: parsed.arguments,
                                },
                                signature: None,
                                additional_params: None,
                            }));
                        }
                    } else {
                        content.push(AssistantContent::Reasoning(Reasoning::new(reasoning)));
                    }
                }
            }

            // Add text content, checking for pseudo-XML tool calls.
            if let Some(ref text) = choice.message.content {
                if !text.is_empty() {
                    if text_tool_parser::contains_pseudo_xml_tool_calls(text) {
                        let (parsed_calls, remaining_text) =
                            text_tool_parser::parse_tool_calls_from_text(text);

                        if !remaining_text.is_empty() {
                            content.push(AssistantContent::Text(Text {
                                text: remaining_text,
                            }));
                        }

                        for parsed in parsed_calls {
                            pseudo_tool_call_counter += 1;
                            let id = format!("pseudo_call_{}", pseudo_tool_call_counter);
                            tracing::info!(
                                "Extracted pseudo-XML tool call from non-streaming response: {}",
                                parsed.name
                            );
                            content.push(AssistantContent::ToolCall(ToolCall {
                                id,
                                call_id: None,
                                function: ToolFunction {
                                    name: parsed.name,
                                    arguments: parsed.arguments,
                                },
                                signature: None,
                                additional_params: None,
                            }));
                        }
                    } else {
                        content.push(AssistantContent::Text(Text { text: text.clone() }));
                    }
                }
            }

            // Add tool calls from the structured API.
            if let Some(ref tool_calls) = choice.message.tool_calls {
                for tc in tool_calls {
                    let arguments = golish_json_repair::parse_tool_args(&tc.function.arguments);
                    content.push(AssistantContent::ToolCall(ToolCall {
                        id: tc.id.clone(),
                        call_id: None,
                        function: ToolFunction {
                            name: tc.function.name.clone(),
                            arguments,
                        },
                        signature: None,
                        additional_params: None,
                    }));
                }
            }
        }

        CompletionResponse {
            choice: OneOrMany::many(content).unwrap_or_else(|_| {
                OneOrMany::one(AssistantContent::Text(Text {
                    text: String::new(),
                }))
            }),
            usage: Usage {
                input_tokens: response.usage.prompt_tokens as u64,
                output_tokens: response.usage.completion_tokens as u64,
                total_tokens: response.usage.total_tokens as u64,
                cached_input_tokens: 0,
            },
            raw_response: response,
            message_id: None,
        }
    }
}
