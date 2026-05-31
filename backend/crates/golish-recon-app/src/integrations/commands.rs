//! IPC entry points for the Integrations facade.
//!
//! Five `#[tauri::command]` functions, one per surface:
//!
//! | Command                          | UI use                                              |
//! |----------------------------------|-----------------------------------------------------|
//! | [`integrations_list_schemas`]    | Render the Settings → Integrations grid             |
//! | [`integrations_get`]             | Hydrate one card's form with current values         |
//! | [`integrations_set`]             | "Save" button                                       |
//! | [`integrations_clear`]           | "Clear" button                                      |
//! | [`integrations_test`]            | "Test Connection" button                            |
//!
//! Error mapping follows `docs/design/2026-05-21-integrations.md §4`:
//! `Validation` → 400, `SchemaNotFound` → 404, `ExternalFileCorrupt`
//! → 409, everything else → 500. The HTTP-style codes themselves
//! don't surface here — they live in the frontend `mapErr()` helper.

use std::collections::HashMap;

use golish_integrations::traits::ResolvedIntegration;
use golish_integrations::types::{FieldValue, IntegrationHealth};
use golish_integrations::SchemaResolver;
use golish_integrations::Tester;

use golish_app_core::DbState;
use golish_app_core::GolishError;

use super::state::{map_err, IntegrationsState};

/// List every integration schema known to this Golish install.
///
/// Sources merged in [`golish_integrations::DefaultSchemaResolver`]:
///
/// 1. `resources/toolsconfig/*.json` files whose `tool.integration`
///    field is set.
/// 2. In-code declarations from `IntelProvider::meta().integration_schema`.
///
/// Sorted by `tool_id` so the UI doesn't reshuffle between calls.
#[tauri::command]
pub async fn integrations_list_schemas(
    state: tauri::State<'_, IntegrationsState>,
) -> Result<Vec<ResolvedIntegration>, GolishError> {
    state.resolver().list().await.map_err(map_err)
}

/// Read the current `FieldValue` map for one group.
///
/// Secret fields surface with `has_value=true`, `value=None` so the
/// UI can show a "configured" badge without ever holding the secret
/// in memory longer than necessary.
#[tauri::command]
pub async fn integrations_get(
    state: tauri::State<'_, IntegrationsState>,
    db: tauri::State<'_, DbState>,
    tool_id: String,
    group_id: String,
) -> Result<HashMap<String, FieldValue>, GolishError> {
    let resolved = state.get_schema(&tool_id).await.map_err(map_err)?;
    let pool = db.pool_ready().await?.clone();
    let backend = state
        .pick_backend(&resolved.schema, pool)
        .map_err(map_err)?;
    backend
        .read(&tool_id, &group_id, &resolved.schema)
        .await
        .map_err(map_err)
}

/// Persist the user's field values for one group through the
/// appropriate backend.
///
/// Backend-specific behaviour:
/// - **Vault**: one row per field, aggregated by
///   `tags=["integration-group", <tool>, <group>]`.
/// - **ExternalFile**: rendered into a YAML / JSON file, with
///   `preserve_unknown_keys=true` merging on top of the user's
///   existing file.
/// - **Settings**: written through the shared `SettingsManager`.
///
/// Validation (required-fields / unknown-keys) happens inside the
/// backend so the contract is identical no matter which storage the
/// schema picks.
#[tauri::command]
pub async fn integrations_set(
    state: tauri::State<'_, IntegrationsState>,
    db: tauri::State<'_, DbState>,
    tool_id: String,
    group_id: String,
    fields: HashMap<String, String>,
) -> Result<(), GolishError> {
    let resolved = state.get_schema(&tool_id).await.map_err(map_err)?;
    let pool = db.pool_ready().await?.clone();
    let backend = state
        .pick_backend(&resolved.schema, pool)
        .map_err(map_err)?;
    backend
        .write(&tool_id, &group_id, &resolved.schema, fields)
        .await
        .map_err(map_err)
}

/// Delete every field belonging to a group.
///
/// For Vault this drops every row matching the per-field name plus
/// the legacy single-key row used by the old `IntelProvidersSettings`
/// UI (so "Clear" really empties everything). ExternalFile removes
/// the schema-declared keys but leaves user-added keys in place.
/// Settings sets the field to the empty string (the typed schema
/// requires the field to exist).
#[tauri::command]
pub async fn integrations_clear(
    state: tauri::State<'_, IntegrationsState>,
    db: tauri::State<'_, DbState>,
    tool_id: String,
    group_id: String,
) -> Result<(), GolishError> {
    let resolved = state.get_schema(&tool_id).await.map_err(map_err)?;
    let pool = db.pool_ready().await?.clone();
    let backend = state
        .pick_backend(&resolved.schema, pool)
        .map_err(map_err)?;
    backend
        .clear(&tool_id, &group_id, &resolved.schema)
        .await
        .map_err(map_err)
}

/// Run the schema-declared connectivity test against the currently
/// stored credentials.
///
/// Three test recipes are supported (see
/// [`golish_integrations::schema::TestKind`]):
///
/// - `Exec` — spawn the tool with `{{exec}}` substituted by the
///   installed executable path; match stdout against `ok_regex` /
///   `fail_regex`.
/// - `Http` — fire an HTTP request with `{{value:field_key}}`
///   substitution; expect a status in `ok_status_range`.
/// - `Builtin` — defer to the provider's own `test_connection`.
///   **Phase 3 returns `Unknown` for Builtin** because the dispatch
///   wiring to `intel_test_connection` is planned for Phase 5.
///   Until then, schemas that need a real test should declare
///   `Http` (already covered) or wait for the dispatch hook.
#[tauri::command]
pub async fn integrations_test(
    state: tauri::State<'_, IntegrationsState>,
    db: tauri::State<'_, DbState>,
    tool_id: String,
    group_id: String,
) -> Result<IntegrationHealth, GolishError> {
    let resolved = state.get_schema(&tool_id).await.map_err(map_err)?;
    let pool = db.pool_ready().await?.clone();
    let backend = state
        .pick_backend(&resolved.schema, pool)
        .map_err(map_err)?;
    let cleartext = backend
        .read_cleartext(&tool_id, &group_id, &resolved.schema)
        .await
        .map_err(map_err)?;
    state
        .tester()
        .test(&tool_id, &group_id, &resolved.schema, &cleartext)
        .await
        .map_err(map_err)
}

#[cfg(test)]
mod tests {
    //! Unit tests here are state-pure: they exercise the resolver +
    //! pick_backend dispatch using in-memory schemas, without
    //! spinning up a real PgPool / SettingsManager. Integration-level
    //! tests (writing real vault rows, etc.) live with the IPC E2E
    //! suite in Phase 5.
    //!
    //! Because `SettingsManager` only exposes `::new()` that reads
    //! from a global path, we don't build a full [`IntegrationsState`]
    //! here. Instead we drive the underlying `DefaultSchemaResolver` +
    //! `DefaultTester` directly — the IPC functions are thin
    //! adapters over these primitives.

    use golish_integrations::tester::DefaultTester;
    use golish_integrations::traits::{ResolvedIntegration, SchemaResolver, Tester};
    use golish_integrations::{
        schema::{
            ExternalFileFormat, ExternalFileStorage, Field, FieldType, IntegrationGroup,
            IntegrationSchema, SettingsStorage, Storage, TestKind,
        },
        types::HealthStatus,
        DefaultSchemaResolver, IntegrationError,
    };
    use std::path::Path;

    fn vault_schema() -> IntegrationSchema {
        IntegrationSchema {
            category: "asm".into(),
            display_name: "0.zone".into(),
            description: None,
            help_url: None,
            storage: Storage::vault_default(),
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

    fn external_file_schema() -> IntegrationSchema {
        IntegrationSchema {
            category: "enterprise-intel".into(),
            display_name: "ENScan_GO".into(),
            description: None,
            help_url: None,
            storage: Storage::ExternalFile {
                external_file: ExternalFileStorage {
                    path: "~/.config/enscan/config.yaml".into(),
                    format: ExternalFileFormat::Yaml,
                    preserve_unknown_keys: true,
                    backup_on_write: true,
                },
            },
            groups: vec![IntegrationGroup {
                id: "aqc".into(),
                name: "AQC".into(),
                description: None,
                icon: None,
                help_url: None,
                test: None,
                capture: None,
                fields: vec![Field {
                    key: "cookies.aqc".into(),
                    label: "Cookie".into(),
                    field_type: FieldType::SecretTextarea,
                    placeholder: None,
                    required: true,
                    rows: None,
                    options: vec![],
                    pattern: None,
                }],
            }],
        }
    }

    fn settings_schema() -> IntegrationSchema {
        IntegrationSchema {
            category: "code-host".into(),
            display_name: "GitHub".into(),
            description: None,
            help_url: None,
            storage: Storage::Settings {
                settings: SettingsStorage {
                    key: "network.github_token".into(),
                },
            },
            groups: vec![IntegrationGroup {
                id: "default".into(),
                name: "Token".into(),
                description: None,
                icon: None,
                help_url: None,
                test: None,
                capture: None,
                fields: vec![Field {
                    key: "token".into(),
                    label: "Token".into(),
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

    fn empty_tester() -> DefaultTester {
        DefaultTester::new(Box::new(|_| None)).unwrap()
    }

    #[tokio::test]
    async fn resolver_lists_in_code_schemas_in_stable_order() {
        // The IPC `integrations_list_schemas` ultimately calls
        // `resolver.list()` — pin that the ordering is stable so the
        // settings UI doesn't reshuffle cards between mounts.
        let zone = ResolvedIntegration {
            tool_id: "0.zone".into(),
            schema: vault_schema(),
        };
        let github = ResolvedIntegration {
            tool_id: "github".into(),
            schema: settings_schema(),
        };
        let resolver = DefaultSchemaResolver::new::<&Path>(None, vec![zone, github]);
        let list = resolver.list().await.unwrap();
        let ids: Vec<&str> = list.iter().map(|r| r.tool_id.as_str()).collect();
        assert_eq!(ids, vec!["0.zone", "github"]);
    }

    #[tokio::test]
    async fn resolver_get_returns_not_found_for_unknown_id() {
        // Mirrors what `integrations_get` / _set / _clear / _test
        // do internally before constructing a backend.
        let resolver = DefaultSchemaResolver::new::<&Path>(None, vec![]);
        let err = resolver.get("nope").await.unwrap_err();
        assert!(matches!(err, IntegrationError::SchemaNotFound(_)));
    }

    #[tokio::test]
    async fn tester_returns_unknown_for_builtin_until_phase5() {
        // Phase 3 explicitly defers Builtin dispatch; the default
        // tester returns Unknown + "builtin test path" hint. We pin
        // that behaviour here so the Phase 5 hookup is easy to spot
        // (search for "builtin test path" string).
        let tester = empty_tester();
        let schema = vault_schema();
        let mut cleartext = std::collections::HashMap::new();
        cleartext.insert("api_key".to_string(), "abc".to_string());
        let h = tester
            .test("0.zone", "default", &schema, &cleartext)
            .await
            .unwrap();
        assert_eq!(h.status, HealthStatus::Unknown);
        assert!(h.message.to_lowercase().contains("builtin"));
    }

    #[tokio::test]
    async fn tester_returns_unknown_for_missing_test_recipe() {
        // No `test` declared on the group → Unknown with an
        // explanatory message. The Test Connection button in the UI
        // should be disabled / hidden when this status comes back.
        let tester = empty_tester();
        let mut schema = external_file_schema();
        schema.groups[0].test = None;
        let h = tester
            .test(
                "enscan-go",
                "aqc",
                &schema,
                &std::collections::HashMap::new(),
            )
            .await
            .unwrap();
        assert_eq!(h.status, HealthStatus::Unknown);
        assert!(h.message.contains("no test recipe"));
    }

    #[test]
    fn map_err_routes_each_variant_to_the_right_golish_error() {
        // The error mapping defined in state::map_err is the
        // single source of truth for IPC error codes
        // (see docs/design/2026-05-21-integrations.md §4).
        use super::super::state::map_err;
        use golish_app_core::GolishError;

        let e = map_err(IntegrationError::Validation("bad field".into()));
        assert!(matches!(e, GolishError::Validation(_)));

        let e = map_err(IntegrationError::SchemaNotFound("0.zone".into()));
        assert!(matches!(e, GolishError::NotFound(_)));

        let e = map_err(IntegrationError::ExternalFileCorrupt {
            path: "/tmp/x".into(),
            reason: "bad yaml".into(),
        });
        assert!(matches!(e, GolishError::Internal(_)));

        let e = map_err(IntegrationError::Internal("oops".into()));
        assert!(matches!(e, GolishError::Internal(_)));
    }

    #[test]
    fn schema_serialization_round_trips_through_json() {
        // `integrations_list_schemas` returns `Vec<ResolvedIntegration>`
        // straight to the frontend; pin that all three storage
        // variants round-trip through JSON cleanly (catches any
        // future change to ResolvedIntegration's derives that would
        // break IPC serialization silently).
        for schema in [vault_schema(), external_file_schema(), settings_schema()] {
            let resolved = ResolvedIntegration {
                tool_id: "demo".into(),
                schema,
            };
            let json = serde_json::to_string(&resolved).unwrap();
            let back: ResolvedIntegration = serde_json::from_str(&json).unwrap();
            assert_eq!(back, resolved);
        }
    }
}
