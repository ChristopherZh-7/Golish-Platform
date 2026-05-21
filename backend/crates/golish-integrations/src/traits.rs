//! Core traits: how the integrations crate is plugged into the rest of
//! Golish. Implementations live in `storage::*` (Phase 2) and the IPC
//! facade in `golish/src/tools/integrations` (Phase 3).

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::IntegrationResult;
use crate::schema::IntegrationSchema;
use crate::types::{FieldValue, IntegrationHealth};

/// Reads / writes one credential group through one of the three
/// supported backends (vault / external_file / settings).
///
/// Implementations are picked per-schema by inspecting
/// [`IntegrationSchema::storage`]. The same credential group always
/// goes through exactly one backend — backends do not coordinate or
/// chain.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Read all field values for one group.
    ///
    /// Secret fields return [`FieldValue::secret_set`] (no value),
    /// non-secret fields return [`FieldValue::plain`].
    /// Blank fields return [`FieldValue::empty`].
    async fn read(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<HashMap<String, FieldValue>>;

    /// Write field values. Implementations must:
    ///
    /// 1. Validate that `fields.keys()` ⊆ schema fields.
    /// 2. Reject blank values for `required` fields.
    /// 3. Persist atomically (no partial writes visible to other
    ///    processes / agents).
    async fn write(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
        fields: HashMap<String, String>,
    ) -> IntegrationResult<()>;

    /// Delete every field belonging to this group.
    async fn clear(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<()>;

    /// Read full cleartext (including secrets) for runtime use:
    /// the tester needs the actual values, and so does the runtime
    /// when launching an external process (e.g. ENScan) that needs
    /// the credentials injected into its config file.
    ///
    /// Callers MUST NOT log or persist the returned map.
    async fn read_cleartext(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<HashMap<String, String>>;
}

/// Collects [`IntegrationSchema`] definitions from all sources:
///
/// - Tool config JSON files in `resources/toolsconfig/*.json`
///   (carrying an `integration` field).
/// - In-code `IntelProvider` implementations exposing
///   `ProviderMeta.integration_schema`.
/// - Hardcoded core integrations bundled with Golish (GitHub Token,
///   future built-ins).
///
/// The resolver is the single source of truth for the IPC facade.
#[async_trait]
pub trait SchemaResolver: Send + Sync {
    /// List every known integration. The IPC facade calls this on
    /// every `integrations_list_schemas` invocation; the resolver
    /// is responsible for caching as appropriate.
    async fn list(&self) -> IntegrationResult<Vec<ResolvedIntegration>>;

    /// Look up a single integration by `tool_id`. Returns
    /// [`crate::error::IntegrationError::SchemaNotFound`] when
    /// missing.
    async fn get(&self, tool_id: &str) -> IntegrationResult<ResolvedIntegration>;
}

/// One schema entry as surfaced to the IPC facade and frontend.
///
/// `tool_id` is duplicated from the source so callers don't need to
/// inspect the schema to know its identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedIntegration {
    pub tool_id: String,
    pub schema: IntegrationSchema,
}

/// Runs a [`crate::schema::TestKind`] and returns an [`IntegrationHealth`].
///
/// Phase 1 ships only the trait. Phase 2 ships the default
/// implementation that knows how to:
///
/// - delegate `TestKind::Builtin` to caller-provided callbacks
///   (so `IntelProvider`s can wire their existing
///   `test_connection` through here);
/// - spawn `TestKind::Exec` commands with stdout / stderr matched
///   against the configured regexes;
/// - issue `TestKind::Http` requests using `reqwest` with
///   `{{value:field_key}}` substitution.
#[async_trait]
pub trait Tester: Send + Sync {
    async fn test(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
        cleartext_fields: &HashMap<String, String>,
    ) -> IntegrationResult<IntegrationHealth>;
}
