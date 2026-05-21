//! Managed Tauri state for the Integrations facade.
//!
//! Holds the long-lived pieces (schema resolver + tester); the
//! per-call [`StorageBackend`] instance is constructed on demand by
//! [`pick_backend`] because vault needs the `PgPool` (only available
//! from `DbState`) and settings needs the shared `SettingsManager`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use sqlx::PgPool;

use golish_integrations::storage::{ExternalFileBackend, SettingsBackend, VaultBackend};
use golish_integrations::tester::{BuiltinDispatcher, DefaultTester, ExecResolver};
use golish_integrations::traits::ResolvedIntegration;
use golish_integrations::{
    DefaultSchemaResolver, IntegrationError, IntegrationResult, IntegrationSchema, SchemaResolver,
    Storage, StorageBackend,
};
use golish_integrations::{Field, IntegrationGroup, IntegrationHealth};
use golish_intel_providers::types::ConnectionStatus;
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
/// 4. Pass a real [`ExecResolver`] (resolves `{{exec}}` in
///    `TestKind::Exec` recipes) and optional
///    [`BuiltinDispatcher`] (routes `TestKind::Builtin` to the
///    matching `IntelProvider::test_connection`). Both default to a
///    no-op for the test constructor.
///
/// The schema resolver caches its result; calls after the first one
/// are essentially free.
pub struct IntegrationsState {
    resolver: Arc<DefaultSchemaResolver>,
    tester: Arc<DefaultTester>,
    settings_mgr: Arc<SettingsManager>,
}

impl IntegrationsState {
    /// Low-level constructor. Tests use this with a no-op
    /// `exec_resolver` and no builtin dispatcher;
    /// [`Self::build_default`] is what production code calls.
    pub fn new<P: AsRef<Path>>(
        toolsconfig_dir: Option<P>,
        in_code_schemas: Vec<ResolvedIntegration>,
        settings_mgr: Arc<SettingsManager>,
        exec_resolver: ExecResolver,
        builtin_dispatcher: Option<Arc<dyn BuiltinDispatcher>>,
    ) -> Self {
        let resolver = Arc::new(DefaultSchemaResolver::new(toolsconfig_dir, in_code_schemas));
        let mut tester = DefaultTester::new(exec_resolver).expect("DefaultTester init");
        if let Some(d) = builtin_dispatcher {
            tester = tester.with_builtin_dispatcher(d);
        }
        Self {
            resolver,
            tester: Arc::new(tester),
            settings_mgr,
        }
    }

    /// Production constructor.
    ///
    /// `tools_dir` and `toolsconfig_dir` come from the pentest
    /// `ConfigManager`; the caller is responsible for awaiting them
    /// once at startup (they're `PathBuf` values, not `&Path`, so the
    /// closure can move-own a snapshot). New tools installed at
    /// runtime require a Golish restart to surface here — acceptable
    /// for the Test Connection path, which only runs when the user
    /// clicks the button.
    pub fn build_default(
        settings_mgr: Arc<SettingsManager>,
        tools_dir: PathBuf,
        toolsconfig_dir: PathBuf,
    ) -> Self {
        // 1. Snapshot every ToolConfig in `toolsconfig_dir`. Used by
        //    both the exec resolver (to look up `executable` /
        //    `runtime`) and any future per-tool lookups.
        let scan = golish_pentest::scan_toolsconfig(&toolsconfig_dir);
        if !scan.success {
            tracing::warn!(
                "[integrations] toolsconfig scan failed at {}: {}",
                toolsconfig_dir.display(),
                scan.error.as_deref().unwrap_or("<unknown>")
            );
        }
        let configs = Arc::new(scan.tools);
        let tools_dir_arc = Arc::new(tools_dir);

        // 2. Real exec resolver: closure moves the snapshot in so the
        //    integrations tester crate doesn't need to depend on
        //    `golish-pentest`. Closure is sync — works because the
        //    underlying helper just does `which_executable` + path
        //    joins, no IO awaits.
        let configs_for_resolver = configs.clone();
        let tools_dir_for_resolver = tools_dir_arc.clone();
        let exec_resolver: ExecResolver = Box::new(move |tool_id: &str| {
            golish_pentest::resolve_tool_executable(
                tool_id,
                &configs_for_resolver,
                &tools_dir_for_resolver,
            )
        });

        // 3. Real builtin dispatcher: routes `TestKind::Builtin` to
        //    the matching `IntelProvider::test_connection`. The
        //    provider registry is built once here and lives inside
        //    the dispatcher's `Arc`.
        let (in_code_schemas, providers) = collect_in_code_schemas_and_providers();
        let dispatcher: Arc<dyn BuiltinDispatcher> = Arc::new(IntelBuiltinDispatcher {
            providers: Arc::new(providers),
        });

        Self::new(
            Some(toolsconfig_dir),
            in_code_schemas,
            settings_mgr,
            exec_resolver,
            Some(dispatcher),
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

/// Build the in-code schema list **and** the provider registry used
/// by [`IntelBuiltinDispatcher`]. The two are computed together so we
/// don't construct the five `IntelProvider` instances twice.
fn collect_in_code_schemas_and_providers() -> (
    Vec<ResolvedIntegration>,
    HashMap<String, Arc<dyn IntelProvider>>,
) {
    let providers: Vec<Arc<dyn IntelProvider>> = vec![
        Arc::new(ZoneProvider::default()),
        Arc::new(FofaProvider::default()),
        Arc::new(QuakeProvider::default()),
        Arc::new(HunterProvider::default()),
        Arc::new(ShodanProvider::default()),
    ];

    let mut schemas: Vec<ResolvedIntegration> = Vec::new();
    let mut registry: HashMap<String, Arc<dyn IntelProvider>> = HashMap::new();

    for p in &providers {
        let meta = p.meta();
        if let Some(schema) = meta.integration_schema {
            schemas.push(ResolvedIntegration {
                tool_id: meta.id.clone(),
                schema,
            });
        }
        registry.insert(meta.id, p.clone());
    }

    // -- Bundled core.json (no providers behind it) ------------------------
    if let Some(path) = golish_core::paths::integrations_core_file() {
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<CoreIntegrationsFile>(&text) {
                Ok(file) => schemas.extend(file.integrations),
                Err(e) => tracing::warn!("[integrations] failed to parse {}: {e}", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("[integrations] no core.json at {}", path.display());
            }
            Err(e) => tracing::warn!("[integrations] reading {}: {e}", path.display()),
        }
    }

    (schemas, registry)
}

/// `BuiltinDispatcher` impl backed by the `IntelProvider` registry.
///
/// Strategy:
/// - Find the registered provider for `tool_id`. Missing id → Unknown
///   with explanatory message (legacy callers without a registry
///   never reach this path — they pass `None` to `IntegrationsState`).
/// - Pick the first secret field declared in the matching group as
///   the credential. The five intel providers all use a single
///   `api_key` field today; if a future schema declares multiple
///   secret fields we'll need a dedicated `credential_field` hint on
///   `TestKind::Builtin`.
/// - Call `provider.test_connection(key)` and map [`ConnectionStatus`]
///   onto [`crate::IntegrationHealth`].
struct IntelBuiltinDispatcher {
    providers: Arc<HashMap<String, Arc<dyn IntelProvider>>>,
}

#[async_trait]
impl BuiltinDispatcher for IntelBuiltinDispatcher {
    async fn dispatch(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
        cleartext_fields: &HashMap<String, String>,
    ) -> IntegrationResult<IntegrationHealth> {
        let Some(provider) = self.providers.get(tool_id) else {
            return Ok(IntegrationHealth::unknown(format!(
                "no IntelProvider registered for '{tool_id}' — builtin test cannot run"
            )));
        };

        let group = schema
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| {
                IntegrationError::Validation(format!(
                    "builtin dispatch: group '{group_id}' not in schema"
                ))
            })?;

        let key = pick_credential_value(group, cleartext_fields)
            .unwrap_or_default();

        match provider.test_connection(&key).await {
            Ok(status) => Ok(connection_status_to_health(status)),
            Err(e) => Ok(IntegrationHealth::unknown(format!(
                "provider error: {e}"
            ))),
        }
    }
}

/// Pull out the value for the first declared `secret_text` /
/// `secret_textarea` field. Falls back to the first field overall if
/// no secret is declared (defensive — wouldn't be a sane builtin
/// schema, but better than panicking).
fn pick_credential_value(
    group: &IntegrationGroup,
    cleartext: &HashMap<String, String>,
) -> Option<String> {
    let first_secret = group
        .fields
        .iter()
        .find(|f: &&Field| f.field_type.is_secret());
    let field = first_secret.or_else(|| group.fields.first())?;
    cleartext.get(&field.key).cloned()
}

fn connection_status_to_health(s: ConnectionStatus) -> IntegrationHealth {
    match s {
        ConnectionStatus::Ok {
            message,
            quota_remaining,
            quota_total,
        } => {
            let mut msg = message;
            if let (Some(remaining), Some(total)) = (quota_remaining, quota_total) {
                msg.push_str(&format!(" · quota {remaining}/{total}"));
            } else if let Some(remaining) = quota_remaining {
                msg.push_str(&format!(" · quota_remaining {remaining}"));
            }
            IntegrationHealth::healthy(msg)
        }
        ConnectionStatus::AuthFailed { message } => IntegrationHealth::invalid(message),
        ConnectionStatus::QuotaExhausted { message } => IntegrationHealth {
            status: golish_integrations::HealthStatus::RateLimited,
            message,
            tested_at: chrono::Utc::now(),
        },
        ConnectionStatus::NetworkError { message } => IntegrationHealth::unknown(message),
    }
}

// Allow constructing IntegrationSchema bare in case someone wants to
// extend the schema in code without a JSON file.
#[allow(dead_code)]
fn _api_key_schema_placeholder_does_not_warn(_s: IntegrationSchema) {}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_integrations::schema::{
        Field, FieldType, IntegrationGroup, IntegrationSchema, Storage, TestKind, VaultStorage,
    };
    use std::collections::HashMap;

    fn fake_schema_one_secret() -> IntegrationSchema {
        IntegrationSchema {
            category: "asm".into(),
            display_name: "demo".into(),
            description: None,
            help_url: None,
            storage: Storage::Vault {
                vault: VaultStorage::default(),
            },
            groups: vec![IntegrationGroup {
                id: "default".into(),
                name: "API Key".into(),
                description: None,
                icon: None,
                help_url: None,
                test: Some(TestKind::Builtin),
                capture: None,
                fields: vec![Field {
                    key: "api_key".into(),
                    label: "API Key".into(),
                    field_type: FieldType::SecretText,
                    placeholder: None,
                    required: true,
                    rows: None,
                    options: vec![],
                    pattern: None,
                }],
            }],
        }
    }

    fn pick_value_test_group() -> IntegrationGroup {
        IntegrationGroup {
            id: "default".into(),
            name: "x".into(),
            description: None,
            icon: None,
            help_url: None,
            test: None,
            capture: None,
            fields: vec![
                Field {
                    key: "label_only".into(),
                    label: "Label".into(),
                    field_type: FieldType::Text,
                    placeholder: None,
                    required: false,
                    rows: None,
                    options: vec![],
                    pattern: None,
                },
                Field {
                    key: "api_key".into(),
                    label: "API Key".into(),
                    field_type: FieldType::SecretText,
                    placeholder: None,
                    required: true,
                    rows: None,
                    options: vec![],
                    pattern: None,
                },
            ],
        }
    }

    #[test]
    fn pick_credential_prefers_secret_field_over_first_field() {
        let group = pick_value_test_group();
        let mut cleartext = HashMap::new();
        cleartext.insert("label_only".into(), "ignored-label".into());
        cleartext.insert("api_key".into(), "real-secret".into());
        let v = pick_credential_value(&group, &cleartext);
        assert_eq!(v.as_deref(), Some("real-secret"));
    }

    #[test]
    fn pick_credential_returns_none_when_secret_missing_from_cleartext() {
        let group = pick_value_test_group();
        let cleartext = HashMap::new();
        let v = pick_credential_value(&group, &cleartext);
        assert!(v.is_none(), "no cleartext entry → None, not empty string");
    }

    #[test]
    fn connection_status_ok_with_quota_appended_to_message() {
        let s = ConnectionStatus::Ok {
            message: "0.zone validated".into(),
            quota_remaining: Some(42),
            quota_total: Some(100),
        };
        let h = connection_status_to_health(s);
        assert_eq!(h.status, golish_integrations::HealthStatus::Healthy);
        assert!(h.message.contains("0.zone validated"));
        assert!(h.message.contains("42/100"));
    }

    #[test]
    fn connection_status_auth_failed_maps_to_invalid() {
        let s = ConnectionStatus::AuthFailed {
            message: "bad key".into(),
        };
        let h = connection_status_to_health(s);
        assert_eq!(h.status, golish_integrations::HealthStatus::Invalid);
        assert_eq!(h.message, "bad key");
    }

    #[test]
    fn connection_status_quota_exhausted_maps_to_rate_limited() {
        let s = ConnectionStatus::QuotaExhausted {
            message: "quota out".into(),
        };
        let h = connection_status_to_health(s);
        assert_eq!(h.status, golish_integrations::HealthStatus::RateLimited);
        assert_eq!(h.message, "quota out");
    }

    #[test]
    fn connection_status_network_error_maps_to_unknown() {
        let s = ConnectionStatus::NetworkError {
            message: "dns failure".into(),
        };
        let h = connection_status_to_health(s);
        assert_eq!(h.status, golish_integrations::HealthStatus::Unknown);
        assert_eq!(h.message, "dns failure");
    }

    #[tokio::test]
    async fn intel_dispatcher_unknown_tool_returns_unknown_health() {
        let dispatcher = IntelBuiltinDispatcher {
            providers: Arc::new(HashMap::new()),
        };
        let schema = fake_schema_one_secret();
        let cleartext = HashMap::new();
        let h = dispatcher
            .dispatch("missing-id", "default", &schema, &cleartext)
            .await
            .unwrap();
        assert_eq!(h.status, golish_integrations::HealthStatus::Unknown);
        assert!(h.message.contains("missing-id"));
    }

    #[tokio::test]
    async fn intel_dispatcher_bad_group_returns_validation_error() {
        // Register a real provider so the dispatcher gets past the
        // "missing-id" early return and reaches the group lookup.
        let p: Arc<dyn IntelProvider> = Arc::new(ZoneProvider::default());
        let mut providers = HashMap::new();
        providers.insert("0.zone".to_string(), p);
        let dispatcher = IntelBuiltinDispatcher {
            providers: Arc::new(providers),
        };
        let schema = fake_schema_one_secret();
        let cleartext = HashMap::new();
        let err = dispatcher
            .dispatch("0.zone", "nonexistent", &schema, &cleartext)
            .await
            .unwrap_err();
        match err {
            IntegrationError::Validation(m) => assert!(m.contains("nonexistent")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }
}
