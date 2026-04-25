//! Staged-patch generation orchestration triggered by the processor.

use anyhow::{Context, Result};
use std::path::PathBuf;

use golish_artifacts::ArtifactManager;

use crate::commits::{BoundaryReason, PatchManager};
use crate::events::SidecarEvent;
use crate::session::Session;

use super::git::get_git_changes;
use super::session_state::SessionProcessorState;
use super::synthesis::generate_commit_message;
use super::ProcessorConfig;

/// Generate a staged patch from current git state and emit related events.
pub(super) async fn generate_patch(
    config: &ProcessorConfig,
    session_id: &str,
    session_state: &mut SessionProcessorState,
    reason: BoundaryReason,
) -> Result<()> {
    tracing::info!(
        "[processor] generate_patch called for session {} with reason {:?}",
        session_id,
        reason
    );

    let session = Session::load(&config.sessions_dir, session_id)
        .await
        .context("Failed to load session")?;

    let git_root = session
        .meta()
        .git_root
        .clone()
        .unwrap_or_else(|| session.meta().cwd.clone());

    tracing::debug!(
        "[processor] Using git_root: {:?} for patch generation",
        git_root
    );

    let manager = PatchManager::new(session.dir().to_path_buf());

    // Use git to detect modified files (more reliable than file_tracker)
    let git_changes = get_git_changes(&session.meta().cwd).await;
    let files: Vec<PathBuf> = git_changes
        .iter()
        .map(|gc| PathBuf::from(&gc.path))
        .collect();

    if files.is_empty() {
        tracing::debug!("[processor] No files detected by git, skipping patch creation");
        return Ok(());
    }

    tracing::info!(
        "[processor] Creating patch with {} file(s): {:?}",
        files.len(),
        files
    );

    let message = generate_commit_message(config, &session, &files, &git_root).await;

    tracing::debug!("[processor] Generated commit message: {}", message);

    let patch = manager
        .create_patch_from_changes(&git_root, &files, &message, reason)
        .await?;

    tracing::info!(
        "[processor] Patch {} created successfully for session {}",
        patch.meta.id,
        session_id
    );

    config.emit_event(SidecarEvent::PatchCreated {
        session_id: session_id.to_string(),
        patch_id: patch.meta.id,
        subject: patch.subject.clone(),
    });

    // Auto-generate artifacts (README.md, CLAUDE.md) if they exist
    let readme_exists = git_root.join("README.md").exists();
    let claude_md_exists = git_root.join("CLAUDE.md").exists();

    if readme_exists || claude_md_exists {
        tracing::info!(
            "[processor] Triggering artifact generation (README.md={}, CLAUDE.md={})",
            readme_exists,
            claude_md_exists
        );

        let session_context = session.read_state().await.unwrap_or_default();
        let patch_subjects = vec![patch.subject.clone()];

        let artifact_manager = ArtifactManager::new(session.dir().to_path_buf());
        match artifact_manager
            .regenerate_from_patches(&git_root, &patch_subjects, &session_context)
            .await
        {
            Ok(created) => {
                if !created.is_empty() {
                    tracing::info!(
                        "[processor] Created {} artifact(s): {:?}",
                        created.len(),
                        created
                    );
                    for path in &created {
                        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                            // Determine target file (README.md or CLAUDE.md)
                            let target = if filename.contains("README") {
                                git_root.join("README.md").display().to_string()
                            } else if filename.contains("CLAUDE") {
                                git_root.join("CLAUDE.md").display().to_string()
                            } else {
                                path.display().to_string()
                            };

                            config.emit_event(SidecarEvent::ArtifactCreated {
                                session_id: session_id.to_string(),
                                filename: filename.to_string(),
                                target,
                            });
                        }
                    }
                } else {
                    tracing::debug!("[processor] No artifact changes needed");
                }
            }
            Err(e) => {
                tracing::warn!("[processor] Artifact generation failed: {}", e);
            }
        }
    } else {
        tracing::debug!(
            "[processor] Skipping artifact generation - no README.md or CLAUDE.md found"
        );
    }

    session_state.file_tracker.clear();
    session_state.boundary_detector.clear();

    Ok(())
}
