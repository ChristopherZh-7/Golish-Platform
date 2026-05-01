//! Sidecar session and artifact management commands.

pub use crate::sidecar::commands::lifecycle::{sidecar_initialize, sidecar_shutdown};
pub use crate::sidecar::commands::status::{sidecar_status, sidecar_current_session, sidecar_get_session_state};
pub use crate::sidecar::commands::content::{sidecar_start_session, sidecar_end_session, sidecar_resume_session, sidecar_get_session_log, sidecar_get_injectable_context, sidecar_get_session_meta, sidecar_list_sessions};
pub use crate::sidecar::commands::config::{sidecar_get_config, sidecar_set_config};
pub use crate::sidecar::commands::patches::{sidecar_get_staged_patches, sidecar_get_applied_patches, sidecar_get_patch, sidecar_discard_patch, sidecar_get_current_staged_patches, sidecar_apply_patch, sidecar_apply_all_patches, sidecar_regenerate_patch, sidecar_update_patch_message};
pub use crate::sidecar::commands::artifacts::{sidecar_get_pending_artifacts, sidecar_get_applied_artifacts, sidecar_get_artifact, sidecar_discard_artifact, sidecar_preview_artifact, sidecar_get_current_pending_artifacts, sidecar_apply_artifact, sidecar_apply_all_artifacts, sidecar_regenerate_artifacts};
