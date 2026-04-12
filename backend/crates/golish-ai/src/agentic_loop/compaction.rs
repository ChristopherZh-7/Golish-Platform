//! Context compaction and directory path utilities.
//!
//! Handles automatic context window compaction when token usage
//! approaches model limits. Also provides path resolution for
//! transcripts, artifacts, and summaries.

use std::path::PathBuf;

use anyhow::Result;
use rig::completion::Message;
use rig::message::{Text, UserContent};
use rig::one_or_many::OneOrMany;

use golish_core::events::AiEvent;

use super::AgenticLoopContext;

/// Result of a context compaction attempt.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    pub success: bool,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub tokens_before: u64,
    pub messages_before: usize,
    pub summarizer_input: Option<String>,
}

fn resolve_golish_base(workspace: &std::path::Path) -> PathBuf {
    let ws_str = workspace.to_string_lossy();
    if ws_str != "." && !ws_str.is_empty() {
        workspace.join(".golish")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".golish")
    }
}

pub fn get_transcript_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".golish")
        .join("transcripts")
}

pub fn get_transcript_dir_for(workspace: &std::path::Path) -> PathBuf {
    resolve_golish_base(workspace).join("transcripts")
}

pub fn get_artifacts_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".golish")
        .join("artifacts")
        .join("compaction")
}

pub fn get_artifacts_dir_for(workspace: &std::path::Path) -> PathBuf {
    resolve_golish_base(workspace)
        .join("artifacts")
        .join("compaction")
}

pub fn get_summaries_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".golish")
        .join("artifacts")
        .join("summaries")
}

pub fn get_summaries_dir_for(workspace: &std::path::Path) -> PathBuf {
    resolve_golish_base(workspace)
        .join("artifacts")
        .join("summaries")
}

/// Check if compaction should be triggered and perform it if needed.
pub async fn maybe_compact(
    ctx: &AgenticLoopContext<'_>,
    session_id: &str,
    chat_history: &mut Vec<Message>,
) -> Result<Option<CompactionResult>> {
    let compaction_state = ctx.compaction_state.read().await;
    let check = ctx
        .context_manager
        .should_compact(&compaction_state, ctx.model_name);
    drop(compaction_state);

    let threshold_tokens = (check.max_tokens as f64 * check.threshold) as u64;
    tracing::info!(
        "[compaction] Check: model={}, current={}, threshold={} ({}% of {}), should_compact={}",
        ctx.model_name,
        check.current_tokens,
        threshold_tokens,
        (check.threshold * 100.0) as u32,
        check.max_tokens,
        check.should_compact
    );

    if !check.should_compact {
        tracing::info!(
            "[compaction] Not triggered: {} (need {} more tokens)",
            check.reason,
            threshold_tokens.saturating_sub(check.current_tokens)
        );
        return Ok(None);
    }

    tracing::info!(
        "[compaction] Triggered: tokens={}/{}, threshold={:.0}%, reason={}",
        check.current_tokens,
        check.max_tokens,
        check.threshold * 100.0,
        check.reason
    );

    let _ = ctx.event_tx.send(AiEvent::CompactionStarted {
        tokens_before: check.current_tokens,
        messages_before: chat_history.len(),
    });

    {
        let mut compaction_state = ctx.compaction_state.write().await;
        compaction_state.mark_attempted();
    }

    let result = perform_compaction(ctx, session_id, chat_history, check.current_tokens).await;

    if result.success {
        let mut compaction_state = ctx.compaction_state.write().await;
        compaction_state.increment_count();
    }

    Ok(Some(result))
}

async fn perform_compaction(
    ctx: &AgenticLoopContext<'_>,
    session_id: &str,
    chat_history: &mut Vec<Message>,
    tokens_before: u64,
) -> CompactionResult {
    let messages_before = chat_history.len();
    let workspace = ctx.workspace.read().await;
    let transcript_dir = get_transcript_dir_for(&workspace);
    let artifacts_dir = get_artifacts_dir_for(&workspace);
    let summaries_dir = get_summaries_dir_for(&workspace);
    drop(workspace);

    let summarizer_input =
        match crate::transcript::build_summarizer_input(&transcript_dir, session_id).await {
            Ok(input) => input,
            Err(e) => {
                tracing::warn!("[compaction] Failed to build summarizer input: {}", e);
                return CompactionResult {
                    success: false,
                    summary: None,
                    error: Some(format!("Failed to build summarizer input: {}", e)),
                    tokens_before,
                    messages_before,
                    summarizer_input: None,
                };
            }
        };

    if let Err(e) =
        crate::transcript::save_summarizer_input(&artifacts_dir, session_id, &summarizer_input)
    {
        tracing::warn!("[compaction] Failed to save summarizer input: {}", e);
    }

    tracing::info!(
        "[compaction] Calling summarizer with {} chars of conversation",
        summarizer_input.len()
    );

    let client = ctx.client.read().await;
    let summary_result = crate::summarizer::generate_summary(&client, &summarizer_input).await;
    drop(client);

    let summary = match summary_result {
        Ok(response) => response.summary,
        Err(e) => {
            tracing::error!("[compaction] Summarizer failed: {}", e);
            let _ = ctx.event_tx.send(AiEvent::Warning {
                message: format!("Context compaction failed: {}", e),
            });
            return CompactionResult {
                success: false,
                summary: None,
                error: Some(format!("Summarizer failed: {}", e)),
                tokens_before,
                messages_before,
                summarizer_input: Some(summarizer_input),
            };
        }
    };

    tracing::info!("[compaction] Summary generated: {} chars", summary.len());

    if let Err(e) = crate::transcript::save_summary(&summaries_dir, session_id, &summary) {
        tracing::warn!("[compaction] Failed to save summary: {}", e);
    }

    let messages_removed = apply_compaction(chat_history, &summary);

    tracing::info!(
        "[compaction] Compaction complete: {} messages removed, {} remaining",
        messages_removed,
        chat_history.len()
    );

    CompactionResult {
        success: true,
        summary: Some(summary),
        error: None,
        tokens_before,
        messages_before,
        summarizer_input: Some(summarizer_input),
    }
}

/// Apply a summary to replace the message history with a compacted version.
pub fn apply_compaction(chat_history: &mut Vec<Message>, summary: &str) -> usize {
    let original_len = chat_history.len();

    let last_user_message = chat_history.iter().rev().find_map(|msg| {
        if let Message::User { content } = msg {
            let text = content
                .iter()
                .filter_map(|c| {
                    if let UserContent::Text(t) = c {
                        Some(t.text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                Some(text)
            } else {
                None
            }
        } else {
            None
        }
    });

    chat_history.clear();

    let message_text = match last_user_message {
        Some(last_msg) => format!(
            "[Context Summary - Previous conversation has been compacted]\n\n{}\n\n[End of Summary]\n\nThe user's most recent request was:\n\n{}",
            summary,
            last_msg
        ),
        None => format!(
            "[Context Summary - Previous conversation has been compacted]\n\n{}\n\n[End of Summary]",
            summary
        ),
    };

    let summary_message = Message::User {
        content: OneOrMany::one(UserContent::Text(Text { text: message_text })),
    };
    chat_history.push(summary_message);

    original_len.saturating_sub(chat_history.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::AssistantContent;

    #[test]
    fn test_get_transcript_dir() {
        let path = get_transcript_dir();
        assert!(path.to_string_lossy().contains(".golish"));
        assert!(path.to_string_lossy().contains("transcripts"));
    }

    #[test]
    fn test_get_artifacts_dir() {
        let path = get_artifacts_dir();
        assert!(path.to_string_lossy().contains(".golish"));
        assert!(path.to_string_lossy().contains("artifacts"));
        assert!(path.to_string_lossy().contains("compaction"));
    }

    #[test]
    fn test_get_summaries_dir() {
        let path = get_summaries_dir();
        assert!(path.to_string_lossy().contains(".golish"));
        assert!(path.to_string_lossy().contains("artifacts"));
        assert!(path.to_string_lossy().contains("summaries"));
    }

    #[test]
    fn test_compaction_result_default_fields() {
        let result = CompactionResult {
            success: false,
            summary: None,
            error: Some("test error".to_string()),
            tokens_before: 100_000,
            messages_before: 50,
            summarizer_input: None,
        };

        assert!(!result.success);
        assert!(result.summary.is_none());
        assert_eq!(result.error, Some("test error".to_string()));
        assert_eq!(result.tokens_before, 100_000);
        assert_eq!(result.messages_before, 50);
    }

    #[test]
    fn test_apply_compaction_empty_history() {
        let mut history: Vec<Message> = vec![];
        let removed = apply_compaction(&mut history, "Test summary");

        assert_eq!(history.len(), 1);
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_apply_compaction_replaces_all_messages() {
        let mut history = vec![
            Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: "First message".to_string(),
                })),
            },
            Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: "Last message".to_string(),
                })),
            },
        ];

        let removed = apply_compaction(&mut history, "Test summary");

        assert_eq!(history.len(), 1);
        assert_eq!(removed, 1);

        if let Message::User { ref content } = history[0] {
            let text = content.iter().next().unwrap();
            if let UserContent::Text(t) = text {
                assert!(t.text.contains("[Context Summary"));
                assert!(t.text.contains("Test summary"));
            } else {
                panic!("Expected text content");
            }
        } else {
            panic!("Expected user message");
        }
    }

    #[test]
    fn test_apply_compaction_removes_many_messages() {
        let mut history: Vec<Message> = (0..10)
            .map(|i| Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: format!("Message {}", i),
                })),
            })
            .collect();

        let removed = apply_compaction(&mut history, "Comprehensive summary");

        assert_eq!(history.len(), 1);
        assert_eq!(removed, 9);
    }

    #[test]
    fn test_apply_compaction_summary_format() {
        let mut history = vec![Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: "Original message".to_string(),
            })),
        }];

        apply_compaction(&mut history, "This is the summary content");

        if let Message::User { ref content } = history[0] {
            let text = content.iter().next().unwrap();
            if let UserContent::Text(t) = text {
                assert!(t
                    .text
                    .contains("[Context Summary - Previous conversation has been compacted]"));
                assert!(t.text.contains("This is the summary content"));
                assert!(t.text.contains("[End of Summary]"));
                assert!(t.text.contains("The user's most recent request was:"));
                assert!(t.text.contains("Original message"));
            }
        }
    }

    #[test]
    fn test_apply_compaction_includes_last_user_message() {
        let mut history = vec![
            Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: "First user message".to_string(),
                })),
            },
            Message::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::Text(Text {
                    text: "Assistant response".to_string(),
                })),
            },
            Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: "This is my latest request".to_string(),
                })),
            },
        ];

        apply_compaction(&mut history, "Summary of conversation");

        if let Message::User { ref content } = history[0] {
            let text = content.iter().next().unwrap();
            if let UserContent::Text(t) = text {
                assert!(t.text.contains("Summary of conversation"));
                assert!(t.text.contains("This is my latest request"));
                assert!(t.text.contains("The user's most recent request was:"));
            } else {
                panic!("Expected text content");
            }
        } else {
            panic!("Expected user message");
        }
    }
}
