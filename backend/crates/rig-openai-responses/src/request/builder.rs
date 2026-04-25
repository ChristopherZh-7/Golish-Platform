//! [`build_request`] — top-level orchestrator that turns a rig
//! `CompletionRequest` into a Responses API `CreateResponse`.

use async_openai::types::responses::{
    CreateResponse, EasyInputContent, EasyInputMessage, IncludeEnum, InputItem, InputParam,
    MessageType, Reasoning, ReasoningSummary, Role, Tool,
};
use rig::completion::{CompletionRequest, Message};

use crate::completion::CompletionModel;
use crate::error::OpenAiResponsesError;

use super::conversion::{
    convert_assistant_content_to_items, convert_tool_definition, convert_user_content,
};
use super::reasoning::apply_additional_params_reasoning;

/// Build an OpenAI Responses API request from a rig `CompletionRequest`.
///
/// This is the single entry point used both by `CompletionModel::completion`
/// and `CompletionModel::stream`; only the `stream` flag on the eventual
/// HTTP call differs between the two paths.
pub(crate) fn build_request(
    model: &CompletionModel,
    request: &CompletionRequest,
) -> Result<CreateResponse, OpenAiResponsesError> {
    // Convert chat history to input items using EasyInputMessage.
    let mut input_items: Vec<InputItem> = Vec::new();

    for msg in request.chat_history.iter() {
        match msg {
            Message::User { content } => {
                let user_items = convert_user_content(content);
                input_items.extend(user_items);
            }
            Message::Assistant { content, .. } => {
                let assistant_items = convert_assistant_content_to_items(content);
                input_items.extend(assistant_items);
            }
        }
    }

    // Add the current prompt/preamble as a system/developer message.
    if let Some(preamble) = &request.preamble {
        input_items.insert(
            0,
            InputItem::EasyMessage(EasyInputMessage {
                r#type: MessageType::Message,
                role: Role::Developer,
                content: EasyInputContent::Text(preamble.clone()),
            }),
        );
    }

    // Build the input.
    let input = if input_items.is_empty() {
        InputParam::Text(String::new())
    } else if input_items.len() == 1 {
        // For a single user text message, use simple text input.
        if let InputItem::EasyMessage(msg) = &input_items[0] {
            if matches!(msg.role, Role::User) {
                if let EasyInputContent::Text(text) = &msg.content {
                    InputParam::Text(text.clone())
                } else {
                    InputParam::Items(input_items)
                }
            } else {
                InputParam::Items(input_items)
            }
        } else {
            InputParam::Items(input_items)
        }
    } else {
        InputParam::Items(input_items)
    };

    // Convert tools.
    let tools: Option<Vec<Tool>> = if request.tools.is_empty() {
        None
    } else {
        Some(request.tools.iter().map(convert_tool_definition).collect())
    };

    // Build reasoning config.
    //
    // All reasoning models always get Detailed summary so the full
    // chain-of-thought is streamed for traces and the UI ThinkingBlock,
    // even when no explicit effort is configured by the user. Detailed
    // guarantees the reasoning text is emitted as
    // ResponseReasoningSummaryTextDelta events; Auto leaves it to
    // OpenAI's discretion.
    let model_name = model.model();
    let reasoning = if crate::is_reasoning_model(model_name) {
        Some(Reasoning {
            effort: model.reasoning_effort_oa(),
            summary: Some(ReasoningSummary::Detailed),
        })
    } else {
        None
    };

    // Apply overrides from additional_params if a "reasoning" key is
    // present. This allows the agentic loop to override effort/summary
    // without conflicting with the model struct settings. Unknown keys
    // are silently ignored.
    let reasoning =
        apply_additional_params_reasoning(reasoning, request.additional_params.as_ref());

    // Build the request.
    //
    // Note: reasoning models (o1, o3, o4, gpt-5.x) don't support temperature.
    let temperature = if crate::is_reasoning_model(model_name) {
        if request.temperature.is_some() {
            tracing::debug!(
                "Ignoring temperature parameter for reasoning model {}",
                model_name
            );
        }
        None
    } else {
        request.temperature.map(|t| t as f32)
    };

    // For reasoning models, request encrypted_content to enable
    // stateless multi-turn conversations. Without this, OpenAI rejects
    // reasoning items in subsequent turns with:
    // "Item 'rs_...' of type 'reasoning' was provided without its
    //  required following item"
    let include = if crate::is_reasoning_model(model_name) {
        Some(vec![IncludeEnum::ReasoningEncryptedContent])
    } else {
        None
    };

    // For stateless operation with reasoning models, we must set
    // `store: false`. This tells OpenAI we're managing conversation
    // history ourselves and will include `encrypted_content` in
    // reasoning items for multi-turn conversations.
    //
    // See: <https://community.openai.com/t/one-potential-cause-of-item-rs-xx-of-type-reasoning-was-provided-without-its-required-following-item-error-stateless-using-agents-sdk/1370540>
    let store = if crate::is_reasoning_model(model_name) {
        Some(false)
    } else {
        None
    };

    Ok(CreateResponse {
        model: Some(model_name.to_string()),
        input,
        tools,
        reasoning,
        temperature,
        max_output_tokens: request.max_tokens.map(|t| t as u32),
        include,
        store,
        ..Default::default()
    })
}
