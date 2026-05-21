//! Managed Tauri state for the Integrations facade.
//!
//! Holds the long-lived pieces (schema resolver + tester); the
//! per-call [`StorageBackend`] instance is constructed on demand by
//! [`pick_backend`] because vault needs the `PgPool` (only available
//! from `DbState`) and settings needs the shared `SettingsManager`.

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use sqlx::PgPool;

use golish_integrations::storage::{ExternalFileBackend, SettingsBackend, VaultBackend};
use golish_integrations::tester::DefaultTester;
use golish_integrations::traits::ResolvedIntegration;
use golish_integrations::{
    DefaultSchemaResolver, IntegrationError, IntegrationResult, IntegrationSchema, SchemaResolver,
    Storage, StorageBackend,
};
use golish_intel_providers::{
    fofa::FofaProvider, hunter::HunterProvider, quake::QuakeProvider, shodan::ShodanProvider,
    zone::ZoneProvider, IntelProvider,
};
use golish_settings::SettingsManager;

/// Long-lived Tauri-managed state for the Integrations IPC facade.
///
/// Construction:
///
/// 1. Pass the absolute path to the project's `resources/toolsconfig/`
///    directory so the resolver can pick up `tool.integration` blocks
///    from every tool JSON.
/// 2. Pass the in-code schemas declared by `IntelProvider`
///    implementations (one per provider that opts into Integrations).
/// 3. Pass an `Arc<SettingsManager>` so the `SettingsBackend` can
///    read / write `network.github_token` etc.
///
/// The schema resolver caches its result; calls after the first one
/// are essentially free.
pub struct IntegrationsState {
    resolver: Arc<DefaultSchemaResolver>,
    tester: Arc<DefaultTester>,
    settings_mgr: Arc<SettingsManager>,
}

impl IntegrationsState {
    /// Build an [`IntegrationsState`] ready to serve IPC.
    ///
    /// `toolsconfig_dir` is `None` only in tests where no on-disk
    /// schemas are needed.
    pub fn new<P: AsRef<Path>>(
        toolsconfig_dir: Option<P>,
        in_code_schemas: Vec<ResolvedIntegration>,
        settings_mgr: Arc<SettingsManager>,
    ) -> Self {
        let resolver = Arc::new(DefaultSchemaResolver::new(toolsconfig_dir, in_code_schemas));
        // Exec resolver: Phase 3 ships a no-op; Phase 5 will wire this
        // to the pentest ConfigManager so `{{exec}}` templates resolve
        // to the installed tool's absolute path.
        let tester =
            DefaultTester::new(Box::new(|_tool_id: &str| None)).expect("DefaultTester init");
        Self {
            resolver,
            tester: Arc::new(tester),
            settings_mgr,
        }
    }

    /// Production constructor: collect the standard in-code schemas
    /// (one per `IntelProvider` impl + the bundled `core.json`) and
    /// hand them to the resolver alongside the toolsconfig directory.
    pub fn build_default(settings_mgr: Arc<SettingsManager>) -> Self {
        Self::new(
            golish_core::paths::toolsconfig_dir(),
            collect_in_code_schemas(),
            settings_mgr,
        )
    }

    pub fn resolver(&self) -> &Arc<DefaultSchemaResolver> {
        &self.resolver
    }

    pub fn tester(&self) -> &Arc<DefaultTester> {
        &self.tester
    }

    pub fn settings_mgr(&self) -> &Arc<SettingsManager> {
        &self.settings_mgr
    }

    /// Fetch a schema by `tool_id`. Returns a typed
    /// [`IntegrationError::SchemaNotFound`] when the id is unknown.
    pub async fn get_schema(&self, tool_id: &str) -> IntegrationResult<ResolvedIntegration> {
        self.resolver.get(tool_id).await
    }

    /// Build the [`StorageBackend`] appropriate to the schema's
    /// declared storage variant. Cheap to call per-IPC because each
    /// backend is essentially a struct of `Arc`s.
    pub fn pick_backend(
        &self,
        schema: &IntegrationSchema,
        pool: PgPool,
    ) -> IntegrationResult<Box<dyn StorageBackend>> {
        match &schema.storage {
            Storage::Vault { .. } => Ok(Box::new(VaultBackend::new(pool))),
            Storage::ExternalFile { .. } => Ok(Box::new(ExternalFileBackend::new())),
            Storage::Settings { .. } => {
                Ok(Box::new(SettingsBackend::new(self.settings_mgr.clone())))
            }
        }
    }
}

/// Map a domain-level [`IntegrationError`] to the application-level
/// [`crate::error::GolishError`]. This is kept in `state.rs` so both
/// commands can reuse one canonical mapping.
pub fn map_err(e: IntegrationError) -> crate::error::GolishError {
    match e {
        IntegrationError::Validation(m) => crate::error::GolishError::Validation(m),
        IntegrationError::SchemaNotFound(m) => crate::error::GolishError::NotFound(m),
        IntegrationError::ExternalFileCorrupt { path, reason } => {
            crate::error::GolishError::Internal(format!(
                "external file corrupt at {path}: {reason}"
            ))
        }
        IntegrationError::Io(e) => crate::error::GolishError::Io(e),
        IntegrationError::Yaml(e) => crate::error::GolishError::Internal(format!("yaml: {e}")),
        IntegrationError::Json(e) => crate::error::GolishError::Json(e),
        IntegrationError::Internal(m) => crate::error::GolishError::Internal(m),

        // -- Capture engine errors ---------------------------------------------
        // The `[CAPTURE_*]` / `[WEBVIEW_*]` prefix in each Display is preserved
        // so the frontend `mapErr()` can dispatch on prefix alone. The chosen
        // GolishError variant only affects HTTP-style status grouping
        // (validation vs not-found vs internal).
        e @ (IntegrationError::CaptureNoRecipe
        | IntegrationError::CaptureAlreadyRunning { .. }
        | IntegrationError::CaptureInvalidUrl(_)
        | IntegrationError::CaptureInvalidTargetField { .. }) => {
            crate::error::GolishError::Validation(e.to_string())
        }
        e @ IntegrationError::CaptureSessionNotFound(_) => {
            crate::error::GolishError::NotFound(e.to_string())
        }
        e @ (IntegrationError::WebviewCreateFailed(_)
        | IntegrationError::CaptureTimeout { .. }
        | IntegrationError::CaptureRuleFailed { .. }) => {
            crate::error::GolishError::Internal(e.to_string())
        }
    }
}

/// Wire-format for `resources/integrations/core.json`. The file is
/// thin on purpose — each entry is just a `tool_id` + the same
/// `IntegrationSchema` we use everywhere else.
#[derive(Debug, Deserialize)]
struct CoreIntegrationsFile {
    integrations: Vec<ResolvedIntegration>,
}

/// Gather every schema that ships with the binary:
///
/// 1. Every `IntelProvider` impl that declared
///    `integration_schema: Some(_)` in its [`ProviderMeta`].
/// 2. The JSON entries inside `resources/integrations/core.json`
///    (GitHub Token, future bundled built-ins).
///
/// Returned list is what we feed to `DefaultSchemaResolver` as the
/// `in_code` set. Errors loading the JSON file are logged and the
/// loop continues — a corrupted bundle file mustn't block startup.
fn collect_in_code_schemas() -> Vec<ResolvedIntegration> {
    let mut out: Vec<ResolvedIntegration> = Vec::new();

    // -- 1. Intel providers -------------------------------------------------
    let providers: Vec<Arc<dyn IntelProvider>> = vec![
        Arc::new(ZoneProvider::default()),
        Arc::new(FofaProvider::default()),
        Arc::new(QuakeProvider::default()),
        Arc::new(HunterProvider::default()),
        Arc::new(ShodanProvider::default()),
    ];
    for p in providers {
        let meta = p.meta();
        if let Some(schema) = meta.integration_schema {
            out.push(ResolvedIntegration {
                tool_id: meta.id,
                schema,
            });
        }
    }

    // -- 2. Bundled core.json ----------------------------------------------
    if let Some(path) = golish_core::paths::integrations_core_file() {
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<CoreIntegrationsFile>(&text) {
                Ok(file) => out.extend(file.integrations),
                Err(e) => tracing::warn!("[integrations] failed to parse {}: {e}", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Bundle file optional in tests / packaging stages
                // where resources aren't laid out yet.
                tracing::debug!("[integrations] no core.json at {}", path.display());
            }
            Err(e) => tracing::warn!("[integrations] reading {}: {e}", path.display()),
        }
    }

    out
}

// Allow constructing IntegrationSchema bare in case someone wants to
// extend the schema in code without a JSON file.
#[allow(dead_code)]
fn _api_key_schema_placeholder_does_not_warn(_s: IntegrationSchema) {}
