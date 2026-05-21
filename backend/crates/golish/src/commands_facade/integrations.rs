//! Integrations IPC facade.
//!
//! Schema-driven external-service credential management. Bridges the
//! `golish-integrations` crate into Tauri:
//!
//! - `integrations_list_schemas` · UI lists all known integrations
//! - `integrations_get`          · read one group's field values
//! - `integrations_set`          · write one group's field values
//! - `integrations_clear`        · drop one group's stored fields
//! - `integrations_test`         · run the schema's connectivity test
//!
//! See `docs/design/2026-05-21-integrations.md` for the data model.

pub use crate::tools::integrations::{
    integrations_clear, integrations_get, integrations_list_schemas, integrations_set,
    integrations_test,
};
