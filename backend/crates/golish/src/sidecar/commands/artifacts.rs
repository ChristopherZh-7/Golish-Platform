//! L3: Project artifact commands (list / get / preview / discard / apply / regenerate).

use crate::state::AppState;
use tauri::State;

use super::super::commits::PatchManager;
use super::super::events::SidecarEvent;
use super::super::session::Session;
use golish_artifacts::{ArtifactFile, ArtifactManager};

/// Resolve git_root from session meta, falling back to `git rev-parse --show-toplevel`.
fn resolve_git_root(session: &Session) -> Result<std::path::PathBuf, String> {
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
        .ok_or_else(|| "No git repository found".to_string())
}

/// Get all pending artifacts for a session
#[tauri::command]
pub async fn sidecar_get_pending_artifacts(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<ArtifactFile>, String> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let manager = ArtifactManager::new(session.dir().to_path_buf());
    manager.list_pending().await.map_err(|e| e.to_string())
}

/// Get all applied artifacts for a session
#[tauri::command]
pub async fn sidecar_get_applied_artifacts(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<ArtifactFile>, String> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let manager = ArtifactManager::new(session.dir().to_path_buf());
    manager.list_applied().await.map_err(|e| e.to_string())
}

/// Get a specific pending artifact by filename
#[tauri::command]
pub async fn sidecar_get_artifact(
    state: State<'_, AppState>,
    session_id: String,
    filename: String,
) -> Result<Option<ArtifactFile>, String> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let manager = ArtifactManager::new(session.dir().to_path_buf());
    manager
        .get_pending(&filename)
        .await
        .map_err(|e| e.to_string())
}

/// Discard a pending artifact
#[tauri::command]
pub async fn sidecar_discard_artifact(
    state: State<'_, AppState>,
    session_id: String,
    filename: String,
) -> Result<bool, String> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let manager = ArtifactManager::new(session.dir().to_path_buf());
    let discarded = manager
        .discard_artifact(&filename)
        .await
        .map_err(|e| e.to_string())?;

    if discarded {
        state
            .sidecar_state
            .emit_event(SidecarEvent::ArtifactDiscarded {
                session_id: session_id.clone(),
                filename: filename.clone(),
            });
    }

    Ok(discarded)
}

/// Preview an artifact (show diff against current target file)
#[tauri::command]
pub async fn sidecar_preview_artifact(
    state: State<'_, AppState>,
    session_id: String,
    filename: String,
) -> Result<String, String> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let manager = ArtifactManager::new(session.dir().to_path_buf());
    manager
        .preview_artifact(&filename)
        .await
        .map_err(|e| e.to_string())
}

/// Get pending artifacts for the current session
#[tauri::command]
pub async fn sidecar_get_current_pending_artifacts(
    state: State<'_, AppState>,
) -> Result<Vec<ArtifactFile>, String> {
    let session_id = state
        .sidecar_state
        .current_session_id()
        .ok_or_else(|| "No active session".to_string())?;

    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let manager = ArtifactManager::new(session.dir().to_path_buf());
    manager.list_pending().await.map_err(|e| e.to_string())
}

/// Apply a pending artifact (copy to target, git add, move to applied)
#[tauri::command]
pub async fn sidecar_apply_artifact(
    state: State<'_, AppState>,
    session_id: String,
    filename: String,
) -> Result<String, String> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let git_root = resolve_git_root(&session)?;

    let manager = ArtifactManager::new(session.dir().to_path_buf());
    let target_path = manager
        .apply_artifact(&filename, &git_root)
        .await
        .map_err(|e| e.to_string())?;

    state
        .sidecar_state
        .emit_event(SidecarEvent::ArtifactApplied {
            session_id: session_id.clone(),
            filename: filename.clone(),
            target: target_path.display().to_string(),
        });

    Ok(target_path.display().to_string())
}

/// Apply all pending artifacts
#[tauri::command]
pub async fn sidecar_apply_all_artifacts(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<(String, String)>, String> {
    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let git_root = resolve_git_root(&session)?;

    let manager = ArtifactManager::new(session.dir().to_path_buf());
    let results = manager
        .apply_all_artifacts(&git_root)
        .await
        .map_err(|e| e.to_string())?;

    for (filename, path) in &results {
        state
            .sidecar_state
            .emit_event(SidecarEvent::ArtifactApplied {
                session_id: session_id.clone(),
                filename: filename.clone(),
                target: path.display().to_string(),
            });
    }

    Ok(results
        .into_iter()
        .map(|(filename, path)| (filename, path.display().to_string()))
        .collect())
}

/// Regenerate artifacts using LLM synthesis
///
/// Triggers artifact regeneration for README.md and CLAUDE.md based on
/// applied patches and session context. Uses the configured synthesis backend.
///
/// # Arguments
/// * `session_id` - The session to regenerate artifacts for
/// * `backend_override` - Optional backend override (uses config default if None)
#[tauri::command]
pub async fn sidecar_regenerate_artifacts(
    state: State<'_, AppState>,
    session_id: String,
    backend_override: Option<String>,
) -> Result<Vec<String>, String> {
    use golish_artifacts::{ArtifactSynthesisBackend, ArtifactSynthesisConfig};

    let sessions_dir = state.sidecar_state.config().sessions_dir();
    let session = Session::load(&sessions_dir, &session_id)
        .await
        .map_err(|e| e.to_string())?;

    let git_root = resolve_git_root(&session)?;

    let session_context = session.read_state().await.unwrap_or_default();

    let patch_manager = PatchManager::new(session.dir().to_path_buf());
    let applied_patches = patch_manager
        .list_applied()
        .await
        .map_err(|e| e.to_string())?;
    let patch_subjects: Vec<String> = applied_patches.iter().map(|p| p.subject.clone()).collect();

    let settings = state.settings_manager.get().await;
    let mut config = ArtifactSynthesisConfig::from_sidecar_settings(&settings.sidecar);

    if let Some(backend_str) = backend_override {
        config.backend = backend_str
            .parse::<ArtifactSynthesisBackend>()
            .map_err(|e| e.to_string())?;
    }

    let artifact_manager = ArtifactManager::new(session.dir().to_path_buf());
    let created = artifact_manager
        .regenerate_from_patches_with_config(&git_root, &patch_subjects, &session_context, &config)
        .await
        .map_err(|e| e.to_string())?;

    // Load pending artifacts to emit ArtifactCreated events
    let pending_artifacts = artifact_manager.list_pending().await.unwrap_or_default();
    for artifact in &pending_artifacts {
        state
            .sidecar_state
            .emit_event(SidecarEvent::ArtifactCreated {
                session_id: session_id.clone(),
                filename: artifact.filename.clone(),
                target: artifact.meta.target.display().to_string(),
            });
    }

    tracing::info!(
        "Regenerated {} artifacts for session {} using {} backend",
        created.len(),
        session_id,
        config.backend
    );

    Ok(created
        .into_iter()
        .map(|p| p.display().to_string())
        .collect())
}
