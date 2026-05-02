//! Settings, models, providers, and telemetry commands.
//!
//! Expected commands exposed here (documentation only):
//! - **Settings**: `get_settings`, `update_settings`, `get_setting`,
//!   `set_setting`, `reset_settings`, `settings_file_exists`,
//!   `get_settings_path`, `reload_settings`
//! - **Window state**: `save_window_state`, `get_window_state`
//! - **Telemetry**: `is_langfuse_active`, `get_telemetry_stats`
//! - **Models & providers**: `get_available_models`, `get_model_by_id`,
//!   `get_model_capabilities_command`, `get_providers`

pub use crate::settings::commands::*;
pub use crate::models::commands::*;
