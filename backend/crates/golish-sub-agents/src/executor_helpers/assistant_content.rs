//! Construct the assistant content vector for chat history (reasoning,
//! text, tool calls in the order required by Anthropic + OpenAI Responses).

use rig::completion::{AssistantContent, Message};
use rig::message::{Reasoning, Text, ToolCall, UserContent};

use super::chain::deserialize_chat_history;



/// Build assistant content for chat history with proper ordering.
///
/// When thinking is enabled, thinking blocks MUST come first (required by Anthropic API).
/// This function ensures the correct order: Reasoning -> Text -> ToolCalls
///
/// # Arguments
/// * `supports_thinking_history` - Whether the model supports thinking history
/// * `thinking_text` - The accumulated thinking/reasoning text
/// * `thinking_id` - Optional reasoning ID (used by OpenAI Responses API)
/// * `thinking_signature` - Optional thinking signature (used by Anthropic)
/// * `text_content` - The text response content
/// * `tool_calls` - List of tool calls to include
///
/// # Returns
/// A vector of AssistantContent in the correct order for the API
pub fn build_assistant_content(
    supports_thinking_history: bool,
    thinking_text: &str,
    thinking_id: Option<String>,
    thinking_signature: Option<String>,
    text_content: &str,
    tool_calls: &[ToolCall],
) -> Vec<AssistantContent> {
    let mut content: Vec<AssistantContent> = vec![];

    // Add thinking content FIRST (required by Anthropic API when thinking is enabled)
    let has_reasoning = !thinking_text.is_empty() || thinking_id.is_some();
    if supports_thinking_history && has_reasoning {
        content.push(AssistantContent::Reasoning(
            Reasoning::new_with_signature(thinking_text, thinking_signature)
                .optional_id(thinking_id),
        ));
    }

    // Add text content
    if !text_content.is_empty() {
        content.push(AssistantContent::Text(Text {
            text: text_content.to_string(),
        }));
    }

    // Add tool calls
    for tc in tool_calls {
        content.push(AssistantContent::ToolCall(tc.clone()));
    }

    content
}

/// Map agent_id to the DB AgentType enum for message_chains storage.
pub(crate) fn agent_id_to_db_type(agent_id: &str) -> &'static str {
    match agent_id {
        "pentester" => "pentester",
        "coder" => "coder",
        "explorer" | "searcher" | "researcher" => "searcher",
        "memorist" => "memorist",
        "reporter" => "reporter",
        "adviser" | "analyzer" => "adviser",
        _ => "primary",
    }
}

/// Restore an existing conversation chain from DB, or create a new one.
/// Returns (chain_id, restored_messages) where restored_messages is empty for new chains.
pub(crate) async fn restore_or_create_chain(
    pool: &sqlx::PgPool,
    session_id: uuid::Uuid,
    task_id: Option<uuid::Uuid>,
    agent_id: &str,
) -> anyhow::Result<(uuid::Uuid, Vec<Message>)> {
    let agent_type = agent_id_to_db_type(agent_id);

    // Look for existing chain for this agent + session + task
    let existing: Option<(uuid::Uuid, Option<serde_json::Value>)> = sqlx::query_as(
        r#"SELECT id, chain FROM message_chains
           WHERE session_id = $1 AND agent::text = $2
             AND ($3::uuid IS NULL AND task_id IS NULL OR task_id = $3)
           ORDER BY updated_at DESC LIMIT 1"#,
    )
    .bind(session_id)
    .bind(agent_type)
    .bind(task_id)
    .fetch_optional(pool)
    .await?;

    if let Some((chain_id, chain_data)) = existing {
        let messages = chain_data
            .and_then(|v| deserialize_chat_history(&v))
            .unwrap_or_default();
        return Ok((chain_id, messages));
    }

    // Create new chain
    let (chain_id,): (uuid::Uuid,) = sqlx::query_as(
        r#"INSERT INTO message_chains (session_id, task_id, agent)
           VALUES ($1, $2, $3::agent_type)
           RETURNING id"#,
    )
    .bind(session_id)
    .bind(task_id)
    .bind(agent_type)
    .fetch_one(pool)
    .await?;

    Ok((chain_id, Vec::new()))
}

/// Serialize rig Message history to JSON for DB storage.
pub(crate) fn serialize_chat_history(messages: &[Message]) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = messages
        .iter()
        .filter_map(|msg| {
            match msg {
                Message::User { content } => {
                    let texts: Vec<String> = content
                        .iter()
                        .filter_map(|c| match c {
                            UserContent::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(serde_json::json!({
                            "role": "user",
                            "content": texts.join("\n"),
                        }))
                    }
                }
                Message::Assistant { content, .. } => {
                    let texts: Vec<String> = content
                        .iter()
                        .filter_map(|c| match c {
                            AssistantContent::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(serde_json::json!({
                            "role": "assistant",
                            "content": texts.join("\n"),
                        }))
                    }
                }
            }
        })
        .collect();
    serde_json::json!(entries)
}
