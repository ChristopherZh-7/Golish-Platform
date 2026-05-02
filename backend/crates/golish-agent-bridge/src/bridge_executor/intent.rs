//! User intent classification (task vs. conversation).

use rig::completion::CompletionRequest;
use rig::message::{Message, Text, UserContent};
use rig::one_or_many::OneOrMany;

use golish_agent_kit::task_orchestrator::prompts;

use super::{complete_with_client, truncate_to_char_boundary};
use crate::agent_bridge::AgentBridge;

/// Whether the user's message is an actionable task or casual conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserIntent {
    Task,
    Conversation,
}

/// Quick LLM classification: is this message an actionable task or just conversation?
///
/// Uses a minimal one-shot call with low max_tokens so it completes fast.
/// Falls back to `Task` if anything goes wrong (conservative — don't silently ignore tasks).
///
/// The prompt is truncated to 500 bytes for classification — the full content
/// is not needed to determine intent, and large prompts can cause timeouts
/// on some providers (e.g., Nvidia NIM).
pub async fn classify_user_intent(bridge: &AgentBridge, prompt: &str) -> UserIntent {
    let user_message = prompt
        .find("[User Message]\n")
        .map(|idx| &prompt[idx + "[User Message]\n".len()..])
        .unwrap_or(prompt);

    let truncated_prompt = truncate_to_char_boundary(user_message, 500);

    tracing::info!(
        prompt_len = prompt.len(),
        truncated_len = truncated_prompt.len(),
        "[IntentClassifier] Starting classification"
    );

    let request = CompletionRequest {
        model: None,
        preamble: Some(prompts::intent_classifier_prompt().to_string()),
        chat_history: OneOrMany::one(Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: truncated_prompt.to_string(),
            })),
        }),
        documents: vec![],
        tools: vec![],
        temperature: Some(0.0),
        max_tokens: Some(8),
        tool_choice: None,
        additional_params: None,
        output_schema: None,
    };

    let client = { let g = bridge.llm.client.read().await; (*g).clone() };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        complete_with_client(&client, request),
    )
    .await;

    match result {
        Ok(Ok(response)) => {
            let word = response.trim().to_uppercase();
            tracing::info!(
                classification = %word,
                prompt_preview = %truncate_to_char_boundary(prompt, 80),
                "[IntentClassifier] Result"
            );
            if word.contains("CHAT") {
                UserIntent::Conversation
            } else {
                UserIntent::Task
            }
        }
        Ok(Err(e)) => {
            tracing::warn!(
                error = %e,
                "[IntentClassifier] Classification failed, defaulting to Task"
            );
            UserIntent::Task
        }
        Err(_) => {
            tracing::warn!(
                "[IntentClassifier] Classification timed out (15s), defaulting to Task"
            );
            UserIntent::Task
        }
    }
}
