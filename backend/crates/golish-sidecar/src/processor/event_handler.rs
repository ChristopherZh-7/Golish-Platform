//! Event-level processing: routes a single `SessionEvent` to file tracking,
//! state.md/log.md updates, synthesis, and patch generation.

use anyhow::Result;

use golish_core::utils::truncate_str;

use crate::commits::BoundaryReason;
use crate::events::{EventType, SessionEvent};
use crate::session::Session;

use super::patches::generate_patch;
use super::session_state::SessionProcessorState;
use super::synthesis::synthesize_state_update;
use super::ProcessorConfig;

/// Handle a single event
pub(super) async fn handle_event(
    config: &ProcessorConfig,
    session_id: &str,
    event: &SessionEvent,
    session_state: &mut SessionProcessorState,
) -> Result<()> {
    session_state.event_count += 1;

    if config.generate_patches {
        track_file_changes(event, session_state);

        // Check for commit boundary - generate patch using git to detect files
        if let Some(boundary_info) = session_state.boundary_detector.check_boundary(event) {
            let reason = parse_boundary_reason(&boundary_info.reason);
            generate_patch(config, session_id, session_state, reason).await?;
        }
    }

    update_session_files(config, session_id, event, session_state).await?;

    tracing::debug!(
        "Processed event for session {}: {:?}",
        session_id,
        event.event_type.name()
    );
    Ok(())
}

/// Update session state.md and log.md based on event
async fn update_session_files(
    config: &ProcessorConfig,
    session_id: &str,
    event: &SessionEvent,
    session_state: &mut SessionProcessorState,
) -> Result<()> {
    tracing::trace!(
        "[processor] update_session_files called: event_type={}, session_id={}",
        event.event_type.name(),
        session_id
    );

    let mut session = match Session::load(&config.sessions_dir, session_id).await {
        Ok(s) => {
            tracing::debug!("[processor] Session {} loaded successfully", session_id);
            s
        }
        Err(e) => {
            tracing::warn!(
                "[processor] Could not load session {} for update: {}",
                session_id,
                e
            );
            return Ok(());
        }
    };

    match &event.event_type {
        EventType::FileEdit {
            path, operation, ..
        } => {
            tracing::info!(
                "[processor] FileEdit event: path={}, all_modified_files before={}",
                path.display(),
                session_state.all_modified_files.len()
            );
            session_state.record_modified_file(path.clone());
            tracing::info!(
                "[processor] FileEdit recorded: all_modified_files after={}",
                session_state.all_modified_files.len()
            );

            let log_entry = format!(
                "**File {}**: `{}`",
                format_operation(operation),
                path.display()
            );
            if let Err(e) = session.append_log(&log_entry).await {
                tracing::warn!("Failed to append to log: {}", e);
            }

            // Note: State synthesis happens on AiResponse, not per-file-edit
        }
        EventType::ToolCall {
            tool_name,
            args_summary,
            success,
            ..
        } => {
            let status = if *success { "✓" } else { "✗" };

            let mut log_entry = format!("**Tool**: {} {}\n", tool_name, status);

            if !args_summary.is_empty() {
                log_entry.push_str(&format!("- **Args**: `{}`\n", args_summary));
            }

            if let Some(output) = &event.tool_output {
                let truncated = if output.chars().count() > 500 {
                    format!("{}...", truncate_str(output, 500))
                } else {
                    output.clone()
                };
                log_entry.push_str(&format!("- **Result**:\n```\n{}\n```\n", truncated));
            }

            if let Err(e) = session.append_log(&log_entry).await {
                tracing::warn!("Failed to append to log: {}", e);
            }

            session_state.record_tool_call(tool_name, *success);

            tracing::info!(
                "[processor] ToolCall {} has {} files_modified: {:?}",
                tool_name,
                event.files_modified.len(),
                event.files_modified
            );
            for path in &event.files_modified {
                session_state.record_modified_file(path.clone());
            }
            tracing::info!(
                "[processor] After recording, all_modified_files has {} entries",
                session_state.all_modified_files.len()
            );

            // Note: State synthesis happens on AiResponse, not per-tool-call
            // This avoids intermediate template updates and reduces LLM calls
        }
        EventType::UserPrompt { intent, .. } => {
            let truncated = if intent.chars().count() > 100 {
                format!("{}...", truncate_str(intent, 100))
            } else {
                intent.clone()
            };
            let log_entry = format!("**User**: {}", truncated);
            if let Err(e) = session.append_log(&log_entry).await {
                tracing::warn!("Failed to append to log: {}", e);
            }

            tracing::info!(
                "[processor] UserPrompt received, synthesis.enabled={}, backend={:?}",
                config.synthesis.enabled,
                config.synthesis.backend
            );
            if config.synthesis.enabled {
                tracing::info!("[processor] Calling synthesize_state_update for user_prompt");
                if let Err(e) = synthesize_state_update(
                    config,
                    &mut session,
                    session_state,
                    "user_prompt",
                    intent,
                )
                .await
                {
                    tracing::error!(
                        "[sidecar] LLM state synthesis failed for user prompt: {}",
                        e
                    );
                }
            } else {
                tracing::warn!(
                    "[processor] Synthesis disabled, skipping state update for user_prompt"
                );
            }
        }
        EventType::AiResponse { content, .. } => {
            let truncated = if content.chars().count() > 100 {
                format!("{}...", truncate_str(content, 100))
            } else {
                content.clone()
            };
            let log_entry = format!("**Agent**: {}", truncated);
            if let Err(e) = session.append_log(&log_entry).await {
                tracing::warn!("Failed to append to log: {}", e);
            }

            tracing::info!(
                "[processor] AiResponse received, synthesis.enabled={}, backend={:?}",
                config.synthesis.enabled,
                config.synthesis.backend
            );
            if config.synthesis.enabled {
                tracing::info!("[processor] Calling synthesize_state_update for ai_response");
                if let Err(e) = synthesize_state_update(
                    config,
                    &mut session,
                    session_state,
                    "ai_response",
                    &truncated,
                )
                .await
                {
                    tracing::error!(
                        "[sidecar] LLM state synthesis failed for AI response: {}",
                        e
                    );
                }
            } else {
                tracing::warn!(
                    "[processor] Synthesis disabled, skipping state update for ai_response"
                );
            }

            // Generate patch on AI response completion (natural boundary)
            if config.generate_patches {
                tracing::info!("[processor] AI response complete - generating patch");
                if let Err(e) = generate_patch(
                    config,
                    session_id,
                    session_state,
                    BoundaryReason::CompletionSignal,
                )
                .await
                {
                    // Only log at debug level - empty git status is normal
                    tracing::debug!("[processor] Patch generation result: {}", e);
                }
            }
        }
        _ => {
            // Other events don't need state/log updates
        }
    }

    Ok(())
}

/// Format file operation for display
fn format_operation(op: &crate::events::FileOperation) -> &'static str {
    match op {
        crate::events::FileOperation::Create => "created",
        crate::events::FileOperation::Modify => "modified",
        crate::events::FileOperation::Delete => "deleted",
        crate::events::FileOperation::Rename { .. } => "renamed",
    }
}

/// Track file changes from an event
pub(super) fn track_file_changes(event: &SessionEvent, session_state: &mut SessionProcessorState) {
    match &event.event_type {
        EventType::FileEdit { path, .. } => {
            tracing::debug!(
                "[processor] FileEdit event for path: {:?}, tracking change",
                path
            );
            session_state.file_tracker.record_change(path.clone());
        }
        EventType::ToolCall { tool_name, .. } => {
            if is_write_tool(tool_name) {
                if event.files_modified.is_empty() {
                    tracing::debug!(
                        "[processor] ToolCall {} is write tool but files_modified is empty",
                        tool_name
                    );
                } else {
                    tracing::debug!(
                        "[processor] ToolCall {} tracking {} file(s): {:?}",
                        tool_name,
                        event.files_modified.len(),
                        event.files_modified
                    );
                }
                for path in &event.files_modified {
                    session_state.file_tracker.record_change(path.clone());
                }
            }
        }
        _ => {}
    }
    tracing::debug!(
        "[processor] File tracker now has {} file(s)",
        session_state.file_tracker.get_files().len()
    );
}

/// Check if a tool is a write tool
pub(super) fn is_write_tool(tool_name: &str) -> bool {
    matches!(
        tool_name.to_lowercase().as_str(),
        "write"
            | "write_file"
            | "edit"
            | "edit_file"
            | "create_file"
            | "delete_file"
            | "ast_grep_replace"
    )
}

/// Parse boundary reason from string
pub(super) fn parse_boundary_reason(reason: &str) -> BoundaryReason {
    let lower = reason.to_lowercase();
    if lower.contains("completion") {
        BoundaryReason::CompletionSignal
    } else if lower.contains("approv") {
        BoundaryReason::UserApproval
    } else if lower.contains("session") || lower.contains("end") {
        BoundaryReason::SessionEnd
    } else if lower.contains("pause") {
        BoundaryReason::ActivityPause
    } else {
        BoundaryReason::CompletionSignal
    }
}

/// Handle session end
pub(super) async fn handle_end_session(config: &ProcessorConfig, session_id: &str) -> Result<()> {
    let mut session = Session::load(&config.sessions_dir, session_id).await?;
    session.complete().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_is_write_tool() {
        assert!(is_write_tool("write"));
        assert!(is_write_tool("Write"));
        assert!(is_write_tool("WRITE_FILE"));
        assert!(is_write_tool("edit"));
        assert!(is_write_tool("Edit_File"));
        assert!(is_write_tool("create_file"));
        assert!(is_write_tool("delete_file"));
        assert!(!is_write_tool("read_file"));
        assert!(!is_write_tool("grep"));
    }

    #[test]
    fn test_parse_boundary_reason() {
        assert!(matches!(
            parse_boundary_reason("Completion signal detected"),
            BoundaryReason::CompletionSignal
        ));
        assert!(matches!(
            parse_boundary_reason("User approved changes"),
            BoundaryReason::UserApproval
        ));
        assert!(matches!(
            parse_boundary_reason("Session ended"),
            BoundaryReason::SessionEnd
        ));
        assert!(matches!(
            parse_boundary_reason("Pause in activity detected"),
            BoundaryReason::ActivityPause
        ));
    }

    #[test]
    fn test_track_file_changes_from_file_edit_event() {
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut state = SessionProcessorState::new();

        let event = SessionEvent::file_edit(
            session_id,
            PathBuf::from("src/main.rs"),
            crate::events::FileOperation::Modify,
            Some("Update main function".to_string()),
        );

        track_file_changes(&event, &mut state);

        assert_eq!(state.file_tracker.get_files().len(), 1);
        assert!(state
            .file_tracker
            .get_files()
            .contains(&PathBuf::from("src/main.rs")));
    }

    #[test]
    fn test_track_file_changes_from_tool_call_event() {
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut state = SessionProcessorState::new();

        let mut event = SessionEvent::tool_call_with_output(
            session_id,
            "write_file".to_string(),
            Some("path=src/lib.rs".to_string()),
            None,
            true,
            None,
            None,
        );
        event.files_modified = vec![PathBuf::from("src/lib.rs")];

        track_file_changes(&event, &mut state);

        assert_eq!(state.file_tracker.get_files().len(), 1);
        assert!(state
            .file_tracker
            .get_files()
            .contains(&PathBuf::from("src/lib.rs")));
    }

    #[test]
    fn test_commit_boundary_integration_with_processor_state() {
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut state = SessionProcessorState::new();

        for i in 0..3 {
            let event = SessionEvent::file_edit(
                session_id.clone(),
                PathBuf::from(format!("src/file{}.rs", i)),
                crate::events::FileOperation::Modify,
                None,
            );
            track_file_changes(&event, &mut state);

            let _boundary = state.boundary_detector.check_boundary(&event);
        }

        let reasoning_event =
            SessionEvent::reasoning(session_id.clone(), "Implementation is complete.", None);

        let boundary = state.boundary_detector.check_boundary(&reasoning_event);

        assert!(
            boundary.is_some(),
            "Expected commit boundary to be detected after completion signal"
        );

        let boundary_info = boundary.unwrap();
        assert_eq!(
            boundary_info.files_in_scope.len(),
            3,
            "Boundary should include all 3 modified files"
        );
    }

    #[test]
    fn test_no_boundary_without_enough_files() {
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut state = SessionProcessorState::new();

        for i in 0..2 {
            let event = SessionEvent::file_edit(
                session_id.clone(),
                PathBuf::from(format!("src/file{}.rs", i)),
                crate::events::FileOperation::Modify,
                None,
            );
            let _ = state.boundary_detector.check_boundary(&event);
        }

        let reasoning_event =
            SessionEvent::reasoning(session_id.clone(), "Implementation is complete.", None);

        let boundary = state.boundary_detector.check_boundary(&reasoning_event);

        assert!(
            boundary.is_none(),
            "Should not detect boundary with fewer than min_events"
        );
    }
}
