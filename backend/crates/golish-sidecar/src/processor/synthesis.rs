//! LLM/template synthesis for state.md updates, session titles, and commit messages.

use anyhow::Result;
use std::path::PathBuf;

use golish_synthesis::{
    create_state_synthesizer, create_title_synthesizer, generate_template_message,
    SessionTitleInput, StateSynthesisInput, SynthesisBackend, SynthesisInput,
};

use crate::events::SidecarEvent;
use crate::session::Session;

use super::git::{get_diff_for_files, get_git_changes};
use super::session_state::SessionProcessorState;
use super::ProcessorConfig;

/// Synthesize an updated state.md using LLM
pub(super) async fn synthesize_state_update(
    config: &ProcessorConfig,
    session: &mut Session,
    session_state: &SessionProcessorState,
    event_type: &str,
    event_details: &str,
) -> Result<()> {
    tracing::info!(
        "[sidecar] Synthesizing state update via LLM (event_type={}, backend={:?})",
        event_type,
        config.synthesis.backend
    );

    let current_state = session.read_state().await.unwrap_or_default();

    // Get files from git diff (includes diffs for context)
    let git_changes = get_git_changes(&session.meta().cwd).await;
    let mut files: Vec<String> = git_changes
        .iter()
        .map(|gc| {
            if gc.diff.is_empty() {
                gc.path.clone()
            } else {
                let diff_preview = if gc.diff.len() > 200 {
                    format!("{} (+{} lines)", gc.path, gc.diff.lines().count())
                } else {
                    format!("{}\n{}", gc.path, gc.diff)
                };
                diff_preview
            }
        })
        .collect();

    // Also include files tracked from tool calls (works even without git)
    // This ensures Changes section is populated even in non-git workspaces
    let tracked_files: std::collections::HashSet<String> = session_state
        .all_modified_files
        .iter()
        .map(|p| p.display().to_string())
        .collect();

    let git_paths: std::collections::HashSet<String> =
        git_changes.iter().map(|gc| gc.path.clone()).collect();

    for tracked in &tracked_files {
        if !git_paths.contains(tracked) {
            files.push(tracked.clone());
        }
    }

    if !files.is_empty() {
        tracing::info!(
            "[sidecar] Detected {} modified files (git: {}, tracked: {}): {:?}",
            files.len(),
            git_changes.len(),
            tracked_files.len(),
            files
                .iter()
                .map(|f| f.lines().next().unwrap_or(f))
                .collect::<Vec<_>>()
        );
    }

    let input = StateSynthesisInput::new(
        current_state,
        event_type.to_string(),
        event_details.to_string(),
        files,
    );

    let synthesizer = create_state_synthesizer(&config.synthesis)?;
    let result = synthesizer.synthesize_state(&input).await?;

    session.update_state(&result.state_body).await?;

    tracing::info!(
        "[sidecar] State synthesized successfully using {} backend",
        result.backend
    );

    config.emit_event(SidecarEvent::StateUpdated {
        session_id: session.meta().session_id.clone(),
        backend: result.backend.clone(),
    });

    if session.meta().title.is_none() {
        match generate_session_title(config, session, &result.state_body).await {
            Ok(title) => {
                tracing::info!("[sidecar] Generated session title: {}", title);
                config.emit_event(SidecarEvent::TitleGenerated {
                    session_id: session.meta().session_id.clone(),
                    title: title.clone(),
                });
            }
            Err(e) => {
                tracing::warn!("[sidecar] Failed to generate session title: {}", e);
            }
        }
    }

    Ok(())
}

/// Generate a session title using the configured synthesis backend
async fn generate_session_title(
    config: &ProcessorConfig,
    session: &mut Session,
    state_body: &str,
) -> Result<String> {
    let input = SessionTitleInput::new(session.meta().initial_request.clone())
        .with_state(state_body.to_string());

    let synthesizer = create_title_synthesizer(&config.synthesis)?;
    let result = synthesizer.synthesize_title(&input).await?;

    session.set_title(result.title.clone()).await?;

    Ok(result.title)
}

/// Generate a commit message using the configured synthesis backend
pub(super) async fn generate_commit_message(
    config: &ProcessorConfig,
    session: &Session,
    files: &[PathBuf],
    git_root: &PathBuf,
) -> String {
    if !config.synthesis.enabled || config.synthesis.backend == SynthesisBackend::Template {
        let diff = get_diff_for_files(git_root, files)
            .await
            .unwrap_or_default();
        return generate_template_message(files, &diff);
    }

    match generate_llm_commit_message(config, session, files, git_root).await {
        Ok(message) => message,
        Err(e) => {
            tracing::warn!("LLM synthesis failed, falling back to template: {}", e);
            let diff = get_diff_for_files(git_root, files)
                .await
                .unwrap_or_default();
            generate_template_message(files, &diff)
        }
    }
}

/// Generate commit message using LLM synthesis
async fn generate_llm_commit_message(
    config: &ProcessorConfig,
    session: &Session,
    files: &[PathBuf],
    git_root: &PathBuf,
) -> Result<String> {
    use golish_synthesis::create_synthesizer;

    let diff = get_diff_for_files(git_root, files).await?;

    let session_context = session.read_state().await.ok();

    let synthesizer = create_synthesizer(&config.synthesis)?;

    let mut input = SynthesisInput::new(diff, files.to_vec());
    if let Some(ctx) = session_context {
        input = input.with_context(ctx);
    }

    let result = synthesizer.synthesize(&input).await?;
    tracing::debug!("Generated commit message using {} backend", result.backend);

    Ok(result.message)
}
