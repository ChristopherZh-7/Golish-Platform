//! Credential vault commands (R7-aligned secrets store with auto-capture).
//!
//! Expected commands exposed here (documentation only):
//! - `vault_list`, `vault_add`, `vault_get_value`
//! - `vault_update`, `vault_delete`
//! - `vault_resolve` (lookup by name/host)
//! - `vault_validate` (probe stored creds against the target)
//! - `vault_update_status` (rotate validation timestamp)
//!
//! Extracted from `commands_facade/workspace.rs` on 2026-05-02
//! (N5) so the credential vault is no longer buried in the
//! catch-all workspace facade.

pub use golish_platform_app::vault::*;
