//! Integrations IPC facade.
//!
//! Schema-driven external-service credential management. Bridges the
//! `golish-integrations` crate into Tauri:
//!
//! - `integrations_list_schemas`     · UI lists all known integrations
//! - `integrations_get`              · read one group's field values
//! - `integrations_set`              · write one group's field values
//! - `integrations_clear`            · drop one group's stored fields
//! - `integrations_test`             · run the schema's connectivity test
//! - `integrations_capture_start`    · open ⚡ auto-capture session
//! - `integrations_capture_status`   · poll one session (event-channel
//!   fallback for reconnect)
//! - `integrations_capture_cancel`   · cancel an in-flight session
//! - `integrations_capture_clear_profile` · clear saved capture browser login state
//!
//! See `docs/design/2026-05-21-integrations.md` for the data model
//! and `docs/design/2026-05-21-credential-capture-engine.md` for the
//! capture-specific architecture.

// Extracted to the golish-recon-app crate (crate-per-service split M2c).
// Functions come from the module root; the `__cmd__$name` macros are pulled
// from their defining submodules so the aggregate `generate_handler!` resolves
// them.
pub use golish_recon_app::integrations::capture_commands::{
    __cmd__integrations_capture_cancel, __cmd__integrations_capture_clear_profile,
    __cmd__integrations_capture_start, __cmd__integrations_capture_status,
};
pub use golish_recon_app::integrations::commands::{
    __cmd__integrations_clear, __cmd__integrations_get, __cmd__integrations_list_schemas,
    __cmd__integrations_set, __cmd__integrations_test,
};
pub use golish_recon_app::integrations::{
    integrations_capture_cancel, integrations_capture_clear_profile, integrations_capture_start,
    integrations_capture_status, integrations_clear, integrations_get, integrations_list_schemas,
    integrations_set, integrations_test,
};
