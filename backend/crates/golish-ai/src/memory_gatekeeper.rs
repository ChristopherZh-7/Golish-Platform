//! Memory gatekeeper: classifies whether memory search is warranted.
//!
//! Before the main agent starts processing, the gatekeeper classifies the user's
//! message to determine whether calling `search_memories` would be useful.
//! This avoids injecting unnecessary tool calls for simple greetings or
//! meta-questions, while ensuring continuity for security-related work.
//!
//! TODO: Use a cheaper/smaller model for classification instead of the main agent's
//! model. Currently this wastes tokens on an expensive model (Claude Sonnet/Opus)
//! for a simple YES/NO classification. Consider using a dedicated fast model
//! (e.g., Claude Haiku, GPT-4o-mini) or a local classifier.

use anyhow::Result;
use golish_llm_providers::LlmClient;
use rig::completion::{AssistantContent, CompletionModel as _, CompletionRequest, Message};
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;

const GATEKEEPER_PROMPT: &str = r#"You are a binary classifier. Given a user message from a penetration testing assistant, decide whether the assistant should search its long-term memory database before responding.

Reply with ONLY "YES" or "NO".

Search memory when the message:
- References targets, hosts, IPs, domains, or URLs
- Asks about prior scan results, findings, or vulnerabilities
- Mentions credentials, configurations, or techniques
- Continues previous work or references past sessions
- Requests reconnaissance, scanning, or exploitation

Do NOT search memory when the message:
- Is a simple greeting (hi, hello, 你好)
- Is a general question about concepts or tools
- Is an acknowledgment (ok, thanks, got it)
- Is asking the assistant to explain itself
- Is clearly unrelated to prior work"#;

/// Classify whether memory search is warranted for the given user message.
///
/// Returns `true` if the model recommends searching memories, `false` otherwise.
/// On any error (timeout, model failure, etc.), returns `false` to avoid blocking.
pub async fn should_search_memory(client: &LlmClient, user_message: &str) -> bool {
    match classify(client, user_message).await {
        Ok(should) => {
            tracing::info!(
                "[memory-gatekeeper] Decision: {} for message: {:?}",
                if should { "SEARCH" } else { "SKIP" },
                &user_message[..{
                    let max = user_message.len().min(80);
                    let mut end = max;
                    while end > 0 && !user_message.is_char_boundary(end) { end -= 1; }
                    end
                }]
            );
            should
        }
        Err(e) => {
            tracing::warn!("[memory-gatekeeper] Classification failed, skipping: {}", e);
            false
        }
    }
}

async fn classify(client: &LlmClient, user_message: &str) -> Result<bool> {
    let user_msg = Message::User {
        content: OneOrMany::one(UserContent::Text(Text {
            text: user_message.to_string(),
        })),
    };

    let response = call_gatekeeper_model(client, user_msg).await?;
    let trimmed = response.trim().to_uppercase();

    Ok(trimmed.starts_with("YES"))
}

fn extract_text(choice: &OneOrMany<AssistantContent>) -> String {
    let mut text = String::new();
    for content in choice.iter() {
        if let AssistantContent::Text(t) = content {
            text.push_str(&t.text);
        }
    }
    text
}

async fn call_gatekeeper_model(client: &LlmClient, user_message: Message) -> Result<String> {
    let request = CompletionRequest {
        preamble: Some(GATEKEEPER_PROMPT.to_string()),
        chat_history: OneOrMany::one(user_message.clone()),
        documents: vec![],
        tools: vec![],
        temperature: Some(0.0),
        max_tokens: Some(8),
        tool_choice: None,
        additional_params: None,
        model: None,
        output_schema: None,
    };

    macro_rules! complete_with_model {
        ($model:expr) => {{
            let response = $model.completion(request).await?;
            Ok(extract_text(&response.choice))
        }};
    }

    match client {
        LlmClient::VertexAnthropic(model) => complete_with_model!(model),
        LlmClient::RigOpenRouter(model) => complete_with_model!(model),
        LlmClient::RigOpenAi(model) => complete_with_model!(model),
        LlmClient::RigOpenAiResponses(model) => complete_with_model!(model),
        LlmClient::OpenAiReasoning(model) => complete_with_model!(model),
        LlmClient::RigAnthropic(model) => complete_with_model!(model),
        LlmClient::RigOllama(model) => complete_with_model!(model),
        LlmClient::RigGemini(model) => complete_with_model!(model),
        LlmClient::RigGroq(model) => complete_with_model!(model),
        LlmClient::RigXai(model) => complete_with_model!(model),
        LlmClient::RigZaiSdk(model) => complete_with_model!(model),
        LlmClient::RigNvidia(model) => {
            let nvidia_history = vec![
                Message::User {
                    content: OneOrMany::one(UserContent::text(GATEKEEPER_PROMPT)),
                },
                user_message.clone(),
            ];
            let nvidia_request = CompletionRequest {
                preamble: None,
                chat_history: OneOrMany::many(nvidia_history)
                    .expect("nvidia_history always has 2 elements"),
                documents: vec![],
                tools: vec![],
                temperature: Some(0.0),
                max_tokens: Some(8),
                tool_choice: None,
                additional_params: None,
                model: None,
                output_schema: None,
            };
            let response = model.completion(nvidia_request).await?;
            Ok(extract_text(&response.choice))
        }
        LlmClient::VertexGemini(model) => complete_with_model!(model),
        LlmClient::Mock => Ok("NO".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gatekeeper_prompt_exists() {
        assert!(GATEKEEPER_PROMPT.contains("YES"));
        assert!(GATEKEEPER_PROMPT.contains("NO"));
    }
}
