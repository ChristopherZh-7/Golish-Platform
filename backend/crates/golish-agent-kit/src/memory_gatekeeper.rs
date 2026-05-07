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
                    while end > 0 && !user_message.is_char_boundary(end) {
                        end -= 1;
                    }
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
    if matches!(client, LlmClient::Mock) {
        return Ok(false);
    }

    let response = client
        .one_shot_completion(GATEKEEPER_PROMPT, user_message, Some(0.0f64), Some(8))
        .await?;
    let trimmed = response.trim().to_uppercase();
    Ok(trimmed.starts_with("YES"))
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
