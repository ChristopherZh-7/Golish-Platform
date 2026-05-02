//! L2: Staged patch commands (list / get / discard / apply / regenerate).

use crate::error::GolishError;
use crate::state::{AppState, SidecarManaged};
use tauri::State;

use super::super::commits::{PatchManager, StagedPatch};
use super::super::events::SidecarEvent;
use super::super::session::Session;
use golish_artifacts::ArtifactManager;

/// Resolve git_root from session meta, falling back to `git rev-parse --show-toplevel`.
fn resolve_git_root(session: &Session) -> Result<std::path::PathBuf, GolishError> {
    session
        .meta()
        .git_root
        .clone()
        .or_else(|| {
            std::process::Command::new("git")
                .args(["rev-parse", "--show-toplevel"])
                .current_dir(&session.meta().cwd)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| {
                    std::path::PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string())
                })
        })
        .ok_or_else(|| GolishError::Internal("No git repository found".to_string()))
}

/// Get all staged patches for a session
#[tauri::command]
pub async fn sidecar_get_staged_patches(
    state: State<'_, SidecarManaged>,
    session_id: String,
) -> Result<Vec<StagedPatch>, GolishError> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let manager = PatchManager::new(session.dir().to_path_buf());
    manager.list_staged().await.map_err(GolishError::from)
}

/// Get all applied patches for a session
#[tauri::command]
pub async fn sidecar_get_applied_patches(
    state: State<'_, SidecarManaged>,
    session_id: String,
) -> Result<Vec<StagedPatch>, GolishError> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let manager = PatchManager::new(session.dir().to_path_buf());
    manager.list_applied().await.map_err(GolishError::from)
}

/// Get a specific patch by ID
#[tauri::command]
pub async fn sidecar_get_patch(
    state: State<'_, SidecarManaged>,
    session_id: String,
    patch_id: u32,
) -> Result<Option<StagedPatch>, GolishError> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let manager = PatchManager::new(session.dir().to_path_buf());
    manager
        .get_staged(patch_id)
        .await
        .map_err(GolishError::from)
}

/// Discard a staged patch
#[tauri::command]
pub async fn sidecar_discard_patch(
    state: State<'_, SidecarManaged>,
    session_id: String,
    patch_id: u32,
) -> Result<bool, GolishError> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let manager = PatchManager::new(session.dir().to_path_buf());
    let discarded = manager
        .discard_patch(patch_id)
        .await
?;

    if discarded {
        state
            .sidecar_state
            .emit_event(SidecarEvent::PatchDiscarded {
                session_id: session_id.clone(),
                patch_id,
            });
    }

    Ok(discarded)
}

/// Apply a staged patch using git am
///
/// After successful application, triggers L3 artifact regeneration (L2 -> L3 cascade).
/// Uses the configured artifact synthesis backend from settings.
#[tauri::command]
pub async fn sidecar_apply_patch(
    state: State<'_, AppState>,
    session_id: String,
    patch_id: u32,
) -> Result<String, GolishError> {
    use golish_artifacts::ArtifactSynthesisConfig;

    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let git_root = resolve_git_root(&session)?;

    let patch_manager = PatchManager::new(session.dir().to_path_buf());

    // Get the patch subject before applying (for artifact regeneration)
    let patch = patch_manager
        .get_staged(patch_id)
        .await
?
        .ok_or_else(|| format!("Patch {} not found", patch_id))?;
    let patch_subject = patch.subject.clone();

    let sha = patch_manager
        .apply_patch(patch_id, &git_root)
        .await
?;

    state.sidecar_state.emit_event(SidecarEvent::PatchApplied {
        session_id: session_id.clone(),
        patch_id,
        commit_sha: sha.clone(),
    });

    // L2 -> L3 Cascade: Trigger artifact regeneration with configured backend
    let artifact_manager = ArtifactManager::new(session.dir().to_path_buf());
    let session_context = session.read_state().await.unwrap_or_default();

    let settings = state.settings_manager.get().await;
    let artifact_config = ArtifactSynthesisConfig::from_sidecar_settings(&settings.sidecar);

    if let Err(e) = artifact_manager
        .regenerate_from_patches_with_config(
            &git_root,
            &[patch_subject],
            &session_context,
            &artifact_config,
        )
        .await
    {
        // Log but don't fail - artifact regeneration is non-critical
        tracing::warn!("Failed to regenerate artifacts after patch apply: {}", e);
    }

    Ok(sha)
}

/// Apply all staged patches in order
///
/// After successful application of all patches, triggers L3 artifact regeneration (L2 -> L3 cascade).
/// Uses the configured artifact synthesis backend from settings.
#[tauri::command]
pub async fn sidecar_apply_all_patches(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<(u32, String)>, GolishError> {
    use golish_artifacts::ArtifactSynthesisConfig;

    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let git_root = resolve_git_root(&session)?;

    let patch_manager = PatchManager::new(session.dir().to_path_buf());

    let staged = patch_manager
        .list_staged()
        .await
?;
    let patch_subjects: Vec<String> = staged.iter().map(|p| p.subject.clone()).collect();

    let results = patch_manager
        .apply_all_patches(&git_root)
        .await
?;

    for (patch_id, sha) in &results {
        state.sidecar_state.emit_event(SidecarEvent::PatchApplied {
            session_id: session_id.clone(),
            patch_id: *patch_id,
            commit_sha: sha.clone(),
        });
    }

    // L2 -> L3 Cascade: Trigger artifact regeneration if patches were applied
    if !results.is_empty() {
        let artifact_manager = ArtifactManager::new(session.dir().to_path_buf());
        let session_context = session.read_state().await.unwrap_or_default();

        let settings = state.settings_manager.get().await;
        let artifact_config = ArtifactSynthesisConfig::from_sidecar_settings(&settings.sidecar);

        if let Err(e) = artifact_manager
            .regenerate_from_patches_with_config(
                &git_root,
                &patch_subjects,
                &session_context,
                &artifact_config,
            )
            .await
        {
            tracing::warn!(
                "Failed to regenerate artifacts after applying {} patches: {}",
                results.len(),
                e
            );
        }
    }

    Ok(results)
}

/// Get staged patches for the current session
#[tauri::command]
pub async fn sidecar_get_current_staged_patches(
    state: State<'_, SidecarManaged>,
) -> Result<Vec<StagedPatch>, GolishError> {
    let session_id = state
        .sidecar_state
        .current_session_id()
        .ok_or_else(|| GolishError::Internal("No active session".into()))?;

    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let manager = PatchManager::new(session.dir().to_path_buf());
    manager.list_staged().await.map_err(GolishError::from)
}

/// Regenerate a patch's commit message using LLM synthesis
///
/// Uses the configured synthesis backend to generate a new commit message
/// based on the patch diff and session context.
#[tauri::command]
pub async fn sidecar_regenerate_patch(
    state: State<'_, AppState>,
    session_id: String,
    patch_id: u32,
) -> Result<StagedPatch, GolishError> {
    use golish_synthesis::{create_synthesizer, SynthesisConfig, SynthesisInput};

    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let manager = PatchManager::new(session.dir().to_path_buf());

    let patch = manager
        .get_staged(patch_id)
        .await
?
        .ok_or_else(|| format!("Patch {} not found", patch_id))?;

    let diff = manager
        .get_patch_diff(patch_id)
        .await
?;

    let session_context = session.read_state().await.ok();

    let settings = state.settings_manager.get().await;
    let synthesis_config = SynthesisConfig::from_sidecar_settings(&settings.sidecar);

    let synthesizer = create_synthesizer(&synthesis_config)
        .map_err(|e| format!("Failed to create synthesizer: {}", e))?;

    let files: Vec<std::path::PathBuf> = patch.files.iter().map(std::path::PathBuf::from).collect();
    let mut input = SynthesisInput::new(diff, files);
    if let Some(ctx) = session_context {
        input = input.with_context(ctx);
    }

    let result = synthesizer
        .synthesize(&input)
        .await
        .map_err(|e| format!("Synthesis failed: {}", e))?;

    let updated_patch = manager
        .update_patch_message(patch_id, &result.message)
        .await
?;

    tracing::info!(
        "Regenerated patch {} message using {} backend",
        patch_id,
        result.backend
    );

    Ok(updated_patch)
}

/// Update a patch's commit message manually (without LLM)
#[tauri::command]
pub async fn sidecar_update_patch_message(
    state: State<'_, SidecarManaged>,
    session_id: String,
    patch_id: u32,
    new_message: String,
) -> Result<StagedPatch, GolishError> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
?;

    let manager = PatchManager::new(session.dir().to_path_buf());
    let updated_patch = manager
        .update_patch_message(patch_id, &new_message)
        .await
?;

    state
        .sidecar_state
        .emit_event(SidecarEvent::PatchMessageUpdated {
            session_id: session_id.clone(),
            patch_id,
            new_subject: updated_patch.subject.clone(),
        });

    Ok(updated_patch)
}
