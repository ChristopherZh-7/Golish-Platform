//! Schema collection.
//!
//! The resolver feeds the IPC facade ([`crate::traits::SchemaResolver`]).
//! It pulls integration descriptors from two sources:
//!
//! 1. **Tool config files** in `resources/toolsconfig/*.json` whose
//!    `tool.integration` field is set.
//! 2. **In-code intel providers** — every implementation of
//!    `IntelProvider` declares its
//!    [`crate::schema::IntegrationSchema`] inline; the host crate
//!    snapshots them and hands the snapshot to [`DefaultSchemaResolver`]
//!    at construction time (so this crate stays free of a direct
//!    `golish-intel-providers` dependency).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;

use crate::error::{IntegrationError, IntegrationResult};
use crate::schema::IntegrationSchema;
use crate::traits::{ResolvedIntegration, SchemaResolver};

/// Default resolver — scans a directory of tool JSON files, plus
/// merges in any in-code schemas injected at construction time.
///
/// The result is cached behind a `tokio::sync::OnceCell` so repeated
/// IPC calls don't re-walk the filesystem.
pub struct DefaultSchemaResolver {
    /// Directory containing `*.json` tool configs. Skipped if `None`.
    toolsconfig_dir: Option<PathBuf>,
    /// Schemas declared in code (e.g. by intel providers).
    in_code: Vec<ResolvedIntegration>,
    cache: tokio::sync::OnceCell<Vec<ResolvedIntegration>>,
}

impl DefaultSchemaResolver {
    pub fn new<P: AsRef<Path>>(
        toolsconfig_dir: Option<P>,
        in_code: Vec<ResolvedIntegration>,
    ) -> Self {
        Self {
            toolsconfig_dir: toolsconfig_dir.map(|p| p.as_ref().to_path_buf()),
            in_code,
            cache: tokio::sync::OnceCell::new(),
        }
    }

    /// Build (or return cached) full integration list.
    async fn collect(&self) -> IntegrationResult<&Vec<ResolvedIntegration>> {
        self.cache
            .get_or_try_init(|| async {
                let mut by_tool: HashMap<String, ResolvedIntegration> = HashMap::new();

                // 1. Walk the toolsconfig dir.
                if let Some(dir) = &self.toolsconfig_dir {
                    if let Ok(mut entries) = fs::read_dir(dir).await {
                        while let Some(ent) = entries.next_entry().await.transpose() {
                            let ent = ent.map_err(IntegrationError::Io)?;
                            let path = ent.path();
                            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                                continue;
                            }
                            if let Some(resolved) = load_toolsconfig_file(&path).await? {
                                by_tool.insert(resolved.tool_id.clone(), resolved);
                            }
                        }
                    }
                }

                // 2. In-code schemas override toolsconfig (intel providers
                //    are authoritative for their own meta).
                for it in &self.in_code {
                    by_tool.insert(it.tool_id.clone(), it.clone());
                }

                // 3. Stable ordering by tool_id so the UI doesn't reshuffle.
                let mut out: Vec<ResolvedIntegration> = by_tool.into_values().collect();
                out.sort_by(|a, b| a.tool_id.cmp(&b.tool_id));
                Ok::<_, IntegrationError>(out)
            })
            .await
    }
}

/// Read one tool JSON; return `Some(ResolvedIntegration)` if it
/// declares an `integration` field that parses into our schema, else
/// `None`. JSON-parse failure → `IntegrationError::Validation`.
async fn load_toolsconfig_file(path: &Path) -> IntegrationResult<Option<ResolvedIntegration>> {
    let text = fs::read_to_string(path).await?;
    // The wire format wraps the config in `{ "tool": { ... } }` to
    // match the existing toolsconfig schema; we only need 2 keys.
    let outer: serde_json::Value = serde_json::from_str(&text).map_err(IntegrationError::from)?;
    let tool = match outer.get("tool") {
        Some(t) => t,
        None => return Ok(None),
    };
    let tool_id = match tool.get("id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return Ok(None),
    };
    let Some(integration_raw) = tool.get("integration").cloned() else {
        return Ok(None);
    };
    if integration_raw.is_null() {
        return Ok(None);
    }
    let schema: IntegrationSchema = serde_json::from_value(integration_raw).map_err(|e| {
        IntegrationError::Validation(format!(
            "toolsconfig {}: invalid `integration` schema: {e}",
            path.display()
        ))
    })?;
    Ok(Some(ResolvedIntegration { tool_id, schema }))
}

#[async_trait]
impl SchemaResolver for DefaultSchemaResolver {
    async fn list(&self) -> IntegrationResult<Vec<ResolvedIntegration>> {
        let list = self.collect().await?;
        Ok(list.clone())
    }

    async fn get(&self, tool_id: &str) -> IntegrationResult<ResolvedIntegration> {
        let list = self.collect().await?;
        list.iter()
            .find(|r| r.tool_id == tool_id)
            .cloned()
            .ok_or_else(|| IntegrationError::SchemaNotFound(tool_id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{
        ExternalFileFormat, ExternalFileStorage, Field, FieldType, IntegrationGroup,
        IntegrationSchema, Storage,
    };
    use tempfile::TempDir;

    fn sample_integration_json(tool_id: &str) -> serde_json::Value {
        serde_json::json!({
            "tool": {
                "id": tool_id,
                "integration": {
                    "category": "enterprise-intel",
                    "display_name": format!("{tool_id} demo"),
                    "storage": {
                        "type": "external_file",
                        "external_file": {
                            "path": "~/.config/demo/config.yaml",
                            "format": "yaml",
                            "preserve_unknown_keys": true,
                            "backup_on_write": true
                        }
                    },
                    "groups": [{
                        "id": "default",
                        "name": "Default",
                        "fields": [{
                            "key": "api_key",
                            "label": "API Key",
                            "type": "secret_text",
                            "required": true
                        }]
                    }]
                }
            }
        })
    }

    fn in_code_provider(tool_id: &str) -> ResolvedIntegration {
        ResolvedIntegration {
            tool_id: tool_id.to_string(),
            schema: IntegrationSchema {
                category: "asm".into(),
                display_name: format!("{tool_id} in-code"),
                description: None,
                storage: Storage::vault_default(),
                help_url: None,
                groups: vec![IntegrationGroup {
                    id: "default".into(),
                    name: "API Key".into(),
                    description: None,
                    icon: None,
                    help_url: None,
                    test: None,
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
            },
        }
    }

    #[tokio::test]
    async fn collects_from_toolsconfig_only() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("enscan-go.json"),
            serde_json::to_string(&sample_integration_json("enscan-go")).unwrap(),
        )
        .unwrap();
        let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
        let list = resolver.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].tool_id, "enscan-go");
    }

    #[tokio::test]
    async fn collects_from_in_code_only() {
        let resolver: DefaultSchemaResolver =
            DefaultSchemaResolver::new(None::<&Path>, vec![in_code_provider("0.zone")]);
        let list = resolver.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].tool_id, "0.zone");
    }

    #[tokio::test]
    async fn in_code_overrides_toolsconfig_for_same_id() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("0.zone.json"),
            serde_json::to_string(&sample_integration_json("0.zone")).unwrap(),
        )
        .unwrap();
        let resolver =
            DefaultSchemaResolver::new(Some(dir.path()), vec![in_code_provider("0.zone")]);
        let list = resolver.list().await.unwrap();
        assert_eq!(list.len(), 1);
        // in-code wins → schema display_name should match the in-code version
        assert_eq!(list[0].schema.display_name, "0.zone in-code");
    }

    #[tokio::test]
    async fn stable_ordering_by_id() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("zzz.json"),
            serde_json::to_string(&sample_integration_json("zzz")).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("aaa.json"),
            serde_json::to_string(&sample_integration_json("aaa")).unwrap(),
        )
        .unwrap();
        let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
        let list = resolver.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].tool_id, "aaa");
        assert_eq!(list[1].tool_id, "zzz");
    }

    #[tokio::test]
    async fn skips_tools_without_integration_field() {
        let dir = TempDir::new().unwrap();
        // Tool without `integration` field — should be silently skipped.
        std::fs::write(
            dir.path().join("plain.json"),
            r#"{"tool":{"id":"plain","name":"Plain"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("with_int.json"),
            serde_json::to_string(&sample_integration_json("with-int")).unwrap(),
        )
        .unwrap();
        let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
        let list = resolver.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].tool_id, "with-int");
    }

    #[tokio::test]
    async fn get_missing_returns_schema_not_found() {
        let resolver: DefaultSchemaResolver = DefaultSchemaResolver::new(None::<&Path>, vec![]);
        let err = resolver.get("not-here").await.unwrap_err();
        assert!(matches!(err, IntegrationError::SchemaNotFound(_)));
    }

    #[tokio::test]
    async fn malformed_integration_schema_errors() {
        let dir = TempDir::new().unwrap();
        // `integration.storage` missing → invalid schema → Validation err
        std::fs::write(
            dir.path().join("bad.json"),
            r#"{"tool":{"id":"bad","integration":{"category":"x","display_name":"x","groups":[]}}}"#,
        )
        .unwrap();
        let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
        let err = resolver.list().await.unwrap_err();
        assert!(matches!(err, IntegrationError::Validation(_)));
    }

    #[tokio::test]
    async fn schema_can_be_round_tripped_through_resolver() {
        // Sanity: a fully populated schema serialized to JSON should
        // load back identically (proves wire format stability).
        let original = IntegrationSchema {
            category: "enterprise-intel".into(),
            display_name: "ENScan".into(),
            description: Some("Demo".into()),
            storage: Storage::ExternalFile {
                external_file: ExternalFileStorage {
                    path: "~/.config/enscan/config.yaml".into(),
                    format: ExternalFileFormat::Yaml,
                    preserve_unknown_keys: true,
                    backup_on_write: true,
                },
            },
            help_url: None,
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
        };
        let json = serde_json::json!({
            "tool": { "id": "enscan-go", "integration": original.clone() }
        });
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("enscan.json"),
            serde_json::to_string(&json).unwrap(),
        )
        .unwrap();
        let resolver = DefaultSchemaResolver::new(Some(dir.path()), vec![]);
        let resolved = resolver.get("enscan-go").await.unwrap();
        assert_eq!(resolved.schema, original);
    }
}
