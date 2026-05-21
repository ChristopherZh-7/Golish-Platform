//! Integrations IPC facade — schema-driven external-service credentials.
//!
//! Bridges the `golish-integrations` crate's [`SchemaResolver`] /
//! [`StorageBackend`] / [`Tester`] traits into Tauri-callable commands:
//!
//! - [`integrations_list_schemas`] · UI lists every known integration
//!   + its `IntegrationSchema` (so the form is rendered client-side)
//! - [`integrations_get`] · UI reads the current `FieldValue` map for a
//!   single group (secrets surface as `has_value=true` + `value=None`)
//! - [`integrations_set`] · UI writes a group's field map, persisting
//!   through whichever backend the schema declares
//! - [`integrations_clear`] · UI clears every field in the group
//! - [`integrations_test`] · UI runs the schema's declared test
//!
//! See `docs/design/2026-05-21-integrations.md` for the architecture.
//!
//! [`SchemaResolver`]: golish_integrations::SchemaResolver
//! [`StorageBackend`]: golish_integrations::StorageBackend
//! [`Tester`]: golish_integrations::Tester

pub mod capture;
pub mod capture_commands;
pub mod commands;
pub mod state;

pub use capture_commands::{
    integrations_capture_cancel, integrations_capture_start, integrations_capture_status,
};
pub use commands::{
    integrations_clear, integrations_get, integrations_list_schemas, integrations_set,
    integrations_test,
};
pub use state::IntegrationsState;
