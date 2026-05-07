//! Isolated commit message writer agent.
//!
//! This module provides a dedicated AI agent for generating git commit messages.
//! It is completely isolated from the main agent and sub-agent system - it cannot
//! be called by any other agent and has no tools. It simply takes a diff and
//! generates a commit message.

use crate::error::GolishError;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

use super::ai_session_not_initialized_error;

/// Response from the commit message generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitMessageResponse {
    /// The generated commit summary (first line, max 72 chars)
    pub summary: String,
    /// The generated commit description (optional, can be empty)
    pub description: String,
}

/// System prompt for the commit message writer agent.
const COMMIT_WRITER_SYSTEM_PROMPT: &str = r#"You are a git commit message generator. Your sole purpose is to analyze git diffs and generate clear, concise commit messages following conventional commit format.

<format>
Generate a commit message with:
1. A summary line (max 72 characters) in the format: <type>(<scope>): <description>
2. Optionally, a longer description if the changes are complex

Types:
- feat: A new feature
- fix: A bug fix
- docs: Documentation changes
- style: Code style changes (formatting, whitespace)
- refactor: Code refactoring without behavior changes
- perf: Performance improvements
- test: Adding or modifying tests
- build: Build system or dependency changes
- ci: CI/CD changes
- chore: Maintenance tasks

Scope: The area of the codebase affected (e.g., auth, api, ui, git-panel)
</format>

<output>
Return ONLY valid JSON in this exact format:
{"summary": "<type>(<scope>): <short description>", "description": "<optional longer description or empty string>"}

Do NOT include any text before or after the JSON. Do NOT use markdown code blocks.
</output>

<rules>
- Keep the summary under 72 characters
- Use imperative mood ("Add feature" not "Added feature")
- Be specific but concise
- Focus on WHAT changed and WHY, not HOW
- If there are multiple logical changes, focus on the primary one
- The description should explain motivation/context if the summary isn't sufficient
</rules>"#;

/// Generate a commit message from a git diff.
///
/// This is a completely isolated agent that cannot be called by the main agent
/// or any sub-agents. It only generates commit messages based on the provided diff.
///
/// # Arguments
/// * `session_id` - The session ID to use for the LLM client
/// * `diff` - The git diff to analyze
/// * `file_summary` - Optional summary of files changed (e.g., "3 files: src/foo.rs, src/bar.rs, ...")
///
/// # Returns
/// A CommitMessageResponse with the generated summary and description
///
/// IMPORTANT: Uses get_session_bridge() to clone the Arc and release the map
/// lock immediately. This allows other sessions to initialize/shutdown while
/// this session is making LLM calls.
#[tauri::command]
pub async fn generate_commit_message(
    state: State<'_, AppState>,
    session_id: String,
    diff: String,
    file_summary: Option<String>,
) -> Result<CommitMessageResponse, GolishError> {
    // Get Arc clone and release map lock immediately
    let bridge = state
        .ai_state
        .get_session_bridge(&session_id)
        .await
        .ok_or_else(|| ai_session_not_initialized_error(&session_id))?;

    let client = bridge.client().clone();

    let user_prompt = if let Some(summary) = file_summary {
        format!(
            "Generate a commit message for the following changes:\n\nFiles changed: {}\n\nDiff:\n```\n{}\n```",
            summary, diff
        )
    } else {
        format!(
            "Generate a commit message for the following changes:\n\nDiff:\n```\n{}\n```",
            diff
        )
    };

    let client_guard = client.read().await;
    let response_text = client_guard
        .one_shot_completion(
            COMMIT_WRITER_SYSTEM_PROMPT,
            &user_prompt,
            Some(0.3),
            Some(1024),
        )
        .await
        .map_err(|e| format!("LLM completion failed: {}", e))?;

    parse_commit_response(&response_text)
}

/// Parse the LLM response into a CommitMessageResponse.
fn parse_commit_response(response: &str) -> Result<CommitMessageResponse, GolishError> {
    // Try to parse as JSON first
    let trimmed = response.trim();

    // Handle markdown code blocks if present
    let json_str = if trimmed.starts_with("```") {
        // Extract content between code blocks
        let without_start = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .unwrap_or(trimmed);
        without_start
            .strip_suffix("```")
            .unwrap_or(without_start)
            .trim()
    } else {
        trimmed
    };

    // Try to parse as JSON
    match serde_json::from_str::<CommitMessageResponse>(json_str) {
        Ok(resp) => Ok(resp),
        Err(json_err) => {
            // Fallback: treat the entire response as the summary
            tracing::warn!(
                "Failed to parse commit message as JSON: {}. Response: {}",
                json_err,
                response
            );

            // Try to extract something useful
            let lines: Vec<&str> = trimmed.lines().collect();
            if lines.is_empty() {
                return Err(GolishError::Internal("Empty response from LLM".into()));
            }

            // Use first non-empty line as summary, rest as description
            let summary = lines[0].trim().to_string();
            let description = if lines.len() > 1 {
                lines[1..].join("\n").trim().to_string()
            } else {
                String::new()
            };

            Ok(CommitMessageResponse {
                summary,
                description,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_commit_response_json() {
        let response = r#"{"summary": "feat(git): add commit message generator", "description": "Adds an isolated AI agent for generating commit messages"}"#;
        let result = parse_commit_response(response).unwrap();
        assert_eq!(result.summary, "feat(git): add commit message generator");
        assert_eq!(
            result.description,
            "Adds an isolated AI agent for generating commit messages"
        );
    }

    #[test]
    fn test_parse_commit_response_json_in_code_block() {
        let response = r#"```json
{"summary": "fix(ui): correct button styling", "description": ""}
```"#;
        let result = parse_commit_response(response).unwrap();
        assert_eq!(result.summary, "fix(ui): correct button styling");
        assert_eq!(result.description, "");
    }

    #[test]
    fn test_parse_commit_response_fallback() {
        let response = "feat(git): add commit writer\n\nThis adds a new feature";
        let result = parse_commit_response(response).unwrap();
        assert_eq!(result.summary, "feat(git): add commit writer");
        assert!(result.description.contains("This adds"));
    }
}
