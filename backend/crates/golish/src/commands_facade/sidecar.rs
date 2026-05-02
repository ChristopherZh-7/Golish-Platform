//! Sidecar session, patch, and artifact management commands.
//!
//! Expected commands exposed here (documentation only):
//! - **Lifecycle**: `sidecar_initialize`, `sidecar_shutdown`
//! - **Status**: `sidecar_status`, `sidecar_current_session`,
//!   `sidecar_get_session_state`
//! - **Session content**: `sidecar_start_session`, `sidecar_end_session`,
//!   `sidecar_resume_session`, `sidecar_get_session_log`,
//!   `sidecar_get_injectable_context`, `sidecar_get_session_meta`,
//!   `sidecar_list_sessions`
//! - **Config**: `sidecar_get_config`, `sidecar_set_config`
//! - **Patches**: `sidecar_{get,discard,apply,regenerate}_*_patch*`,
//!   `sidecar_update_patch_message`
//! - **Artifacts**: `sidecar_{get,discard,apply,preview,regenerate}_*_artifact*`

pub use crate::sidecar::commands::*;
