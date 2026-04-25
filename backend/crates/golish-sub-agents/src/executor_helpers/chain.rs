//! Chat-history deserialization helper.

use rig::completion::{AssistantContent, Message};
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;



/// Deserialize stored JSON back into rig Messages (simplified text-only restoration).
pub(super) fn deserialize_chat_history(value: &serde_json::Value) -> Option<Vec<Message>> {
    let arr = value.as_array()?;
    let mut messages = Vec::new();
    for entry in arr {
        let role = entry.get("role")?.as_str()?;
        let content = entry.get("content")?.as_str()?.to_string();
        if content.is_empty() {
            continue;
        }
        match role {
            "user" => messages.push(Message::User {
                content: OneOrMany::one(UserContent::Text(Text { text: content })),
            }),
            "assistant" => messages.push(Message::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::Text(Text { text: content })),
            }),
            _ => {}
        }
    }
    Some(messages)
}

