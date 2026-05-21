//! PostgreSQL-backed vault storage.
//!
//! Each declared field in a group becomes one row in `vault_entries`:
//!
//! ```text
//! For (tool_id="enscan-go", group_id="tyc"):
//!   name="enscan-go.tyc.cookies.tyc"   tags=["integration-group","enscan-go","tyc"]
//!   name="enscan-go.tyc.tyc.tycid"     tags=["integration-group","enscan-go","tyc"]
//!   name="enscan-go.tyc.tyc.auth_token" tags=["integration-group","enscan-go","tyc"]
//! ```
//!
//! The `value` column is XOR-obfuscated via
//! [`golish_core::vault::obfuscate`] before storage; reads undo it.
//!
//! Backward compatibility: when no new-format row is found for a
//! single-field "default" group (the convention used by old
//! `IntelProvidersSettings` cards), we fall back to the legacy
//! `name=<tool_id>, entry_type='api_key'` row.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::{IntegrationError, IntegrationResult};
use crate::schema::{IntegrationGroup, IntegrationSchema, Storage, VaultStorage};
use crate::traits::StorageBackend;
use crate::types::FieldValue;

const TAG_NAMESPACE: &str = "integration-group";

/// Vault-backed storage. Holds a `PgPool` clone so it can be used in
/// multi-instance settings without re-acquiring the pool.
pub struct VaultBackend {
    pool: PgPool,
}

impl VaultBackend {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Inspect schema; ensure we're routed to this backend. Returns
    /// the [`VaultStorage`] block.
    fn storage(schema: &IntegrationSchema) -> IntegrationResult<&VaultStorage> {
        match &schema.storage {
            Storage::Vault { vault } => Ok(vault),
            other => Err(IntegrationError::Validation(format!(
                "VaultBackend invoked with non-Vault storage: {other:?}"
            ))),
        }
    }

    fn group<'s>(
        schema: &'s IntegrationSchema,
        group_id: &str,
    ) -> IntegrationResult<&'s IntegrationGroup> {
        schema
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or_else(|| {
                IntegrationError::SchemaNotFound(format!(
                    "group '{group_id}' not declared in schema"
                ))
            })
    }

    /// Compose the canonical vault-entry name for a given field.
    fn entry_name(tool_id: &str, group_id: &str, field_key: &str) -> String {
        format!("{tool_id}.{group_id}.{field_key}")
    }

    /// Build the JSONB tag value for inserts/updates.
    fn tags(tool_id: &str, group_id: &str, vault: &VaultStorage) -> serde_json::Value {
        let mut t = vec![
            TAG_NAMESPACE.to_string(),
            tool_id.to_string(),
            group_id.to_string(),
        ];
        t.extend(vault.extra_tags.iter().cloned());
        serde_json::Value::Array(t.into_iter().map(serde_json::Value::String).collect())
    }
}

#[async_trait]
impl StorageBackend for VaultBackend {
    async fn read(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<HashMap<String, FieldValue>> {
        Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;
        let mut out = HashMap::new();
        for field in &group.fields {
            let name = Self::entry_name(tool_id, group_id, &field.key);
            let row: Option<(String, DateTime<Utc>)> = sqlx::query_as(
                r#"SELECT value, updated_at
                   FROM vault_entries
                   WHERE name = $1
                   ORDER BY updated_at DESC
                   LIMIT 1"#,
            )
            .bind(&name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| IntegrationError::Internal(format!("vault read failed: {e}")))?;
            let entry = if let Some((_obf, updated_at)) = row {
                if field.field_type.is_secret() {
                    FieldValue::secret_set(None, updated_at)
                } else {
                    // Non-secret field, but still stored as obfuscated value;
                    // surface plaintext (caller wanted to see it).
                    let plain: String = read_value_plain(&self.pool, &name)
                        .await?
                        .unwrap_or_default();
                    FieldValue::plain(plain, updated_at)
                }
            } else {
                // Backward-compat alias: a single-field "default" group
                // falls back to the legacy 'intel-provider' convention.
                if group_id == "default" && group.fields.len() == 1 {
                    if let Some((_obf, updated_at)) =
                        legacy_intel_provider_row(&self.pool, tool_id).await?
                    {
                        if field.field_type.is_secret() {
                            FieldValue::secret_set(None, updated_at)
                        } else {
                            let plain = legacy_intel_provider_value_plain(&self.pool, tool_id)
                                .await?
                                .unwrap_or_default();
                            FieldValue::plain(plain, updated_at)
                        }
                    } else {
                        FieldValue::empty()
                    }
                } else {
                    FieldValue::empty()
                }
            };
            out.insert(field.key.clone(), entry);
        }
        Ok(out)
    }

    async fn write(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
        fields: HashMap<String, String>,
    ) -> IntegrationResult<()> {
        let vault = Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;

        let declared: std::collections::HashSet<&str> =
            group.fields.iter().map(|f| f.key.as_str()).collect();
        for k in fields.keys() {
            if !declared.contains(k.as_str()) {
                return Err(IntegrationError::Validation(format!(
                    "unknown field '{k}' for group '{group_id}'"
                )));
            }
        }
        for f in &group.fields {
            if f.required {
                let blank = fields
                    .get(&f.key)
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true);
                if blank {
                    return Err(IntegrationError::Validation(format!(
                        "field '{}' is required",
                        f.key
                    )));
                }
            }
        }

        let tags_json = Self::tags(tool_id, group_id, vault);

        for f in &group.fields {
            let Some(value) = fields.get(&f.key) else {
                continue; // optional + not provided → skip
            };
            let name = Self::entry_name(tool_id, group_id, &f.key);
            let obf = golish_core::vault::obfuscate(value);

            // Try update first.
            let updated = sqlx::query(
                r#"UPDATE vault_entries
                   SET value = $1, tags = $2, updated_at = NOW()
                   WHERE name = $3"#,
            )
            .bind(&obf)
            .bind(&tags_json)
            .bind(&name)
            .execute(&self.pool)
            .await
            .map_err(|e| IntegrationError::Internal(format!("vault update failed: {e}")))?;

            if updated.rows_affected() == 0 {
                // Need INSERT. entry_type maps from field_type — anything
                // secret-ish becomes "api_key"; non-secret strings → "token".
                let entry_type = if f.field_type.is_secret() {
                    "api_key"
                } else {
                    "token"
                };
                // `project_path` was retro-actively made `NOT NULL DEFAULT ''`
                // (see migration `20260418100002_logs_project_path.sql`);
                // we rely on the schema default by omitting the column.
                sqlx::query(
                    r#"INSERT INTO vault_entries (name, entry_type, value, username, notes, project, tags)
                       VALUES ($1, $2::vault_entry_type, $3, '', $4, '', $5)"#,
                )
                .bind(&name)
                .bind(entry_type)
                .bind(&obf)
                .bind(format!(
                    "Integration credential: {tool_id} / {group_id} / {}",
                    f.key
                ))
                .bind(&tags_json)
                .execute(&self.pool)
                .await
                .map_err(|e| IntegrationError::Internal(format!("vault insert failed: {e}")))?;
            }
        }
        Ok(())
    }

    async fn clear(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<()> {
        Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;
        let mut names: Vec<String> = group
            .fields
            .iter()
            .map(|f| Self::entry_name(tool_id, group_id, &f.key))
            .collect();
        // Also remove the legacy single-key row for "default" groups so
        // a "Clear" button in the new UI actually empties everything.
        if group_id == "default" && group.fields.len() == 1 {
            names.push(tool_id.to_string());
        }
        sqlx::query(r#"DELETE FROM vault_entries WHERE name = ANY($1)"#)
            .bind(&names[..])
            .execute(&self.pool)
            .await
            .map_err(|e| IntegrationError::Internal(format!("vault clear failed: {e}")))?;
        Ok(())
    }

    async fn read_cleartext(
        &self,
        tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<HashMap<String, String>> {
        Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;
        let mut out = HashMap::new();
        for f in &group.fields {
            let name = Self::entry_name(tool_id, group_id, &f.key);
            if let Some(plain) = read_value_plain(&self.pool, &name).await? {
                out.insert(f.key.clone(), plain);
                continue;
            }
            // legacy fallback
            if group_id == "default" && group.fields.len() == 1 {
                if let Some(plain) = legacy_intel_provider_value_plain(&self.pool, tool_id).await? {
                    out.insert(f.key.clone(), plain);
                }
            }
        }
        Ok(out)
    }
}

/// Read the obfuscated value at `name` and decode it.
async fn read_value_plain(pool: &PgPool, name: &str) -> IntegrationResult<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"SELECT value FROM vault_entries WHERE name = $1 ORDER BY updated_at DESC LIMIT 1"#,
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e| IntegrationError::Internal(format!("vault read failed: {e}")))?;
    match row {
        None => Ok(None),
        Some((obf,)) => match golish_core::vault::deobfuscate(&obf) {
            Ok(p) => Ok(Some(p)),
            Err(e) => Err(IntegrationError::Internal(format!(
                "vault decode failed: {e}"
            ))),
        },
    }
}

/// Read the legacy "intel-provider" single-row alias.
async fn legacy_intel_provider_row(
    pool: &PgPool,
    tool_id: &str,
) -> IntegrationResult<Option<(String, DateTime<Utc>)>> {
    let row: Option<(String, DateTime<Utc>)> = sqlx::query_as(
        r#"SELECT value, updated_at
           FROM vault_entries
           WHERE name = $1 AND entry_type = 'api_key'
           ORDER BY updated_at DESC
           LIMIT 1"#,
    )
    .bind(tool_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| IntegrationError::Internal(format!("vault legacy read failed: {e}")))?;
    Ok(row)
}

async fn legacy_intel_provider_value_plain(
    pool: &PgPool,
    tool_id: &str,
) -> IntegrationResult<Option<String>> {
    match legacy_intel_provider_row(pool, tool_id).await? {
        None => Ok(None),
        Some((obf, _)) => match golish_core::vault::deobfuscate(&obf) {
            Ok(p) => Ok(Some(p)),
            Err(e) => Err(IntegrationError::Internal(format!(
                "vault legacy decode failed: {e}"
            ))),
        },
    }
}

#[cfg(test)]
mod tests {
    //! Integration-level tests live in the IPC facade once Phase 3
    //! lands (they need a real `PgPool`). Here we cover the
    //! pure-function helpers + schema dispatch only.

    use super::*;
    use crate::schema::{Field, FieldType, IntegrationGroup, IntegrationSchema};

    fn schema_default() -> IntegrationSchema {
        IntegrationSchema {
            category: "asm".into(),
            display_name: "0.zone".into(),
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
        }
    }

    #[test]
    fn entry_name_matches_convention() {
        assert_eq!(
            VaultBackend::entry_name("0.zone", "default", "api_key"),
            "0.zone.default.api_key"
        );
        assert_eq!(
            VaultBackend::entry_name("enscan-go", "tyc", "cookies.tyc"),
            "enscan-go.tyc.cookies.tyc"
        );
    }

    #[test]
    fn tags_default_namespace() {
        let vault = VaultStorage::default();
        let tags = VaultBackend::tags("0.zone", "default", &vault);
        let arr = tags.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str(), Some(TAG_NAMESPACE));
        assert_eq!(arr[1].as_str(), Some("0.zone"));
        assert_eq!(arr[2].as_str(), Some("default"));
    }

    #[test]
    fn tags_with_extras() {
        let vault = VaultStorage {
            extra_tags: vec!["data-source".into()],
        };
        let tags = VaultBackend::tags("0.zone", "default", &vault);
        let arr = tags.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[3].as_str(), Some("data-source"));
    }

    #[test]
    fn schema_dispatch_rejects_non_vault() {
        let mut schema = schema_default();
        // swap to ExternalFile to trigger the validation branch
        schema.storage = Storage::ExternalFile {
            external_file: crate::schema::ExternalFileStorage {
                path: "/tmp/x".into(),
                format: crate::schema::ExternalFileFormat::Yaml,
                preserve_unknown_keys: true,
                backup_on_write: true,
            },
        };
        let err = VaultBackend::storage(&schema).unwrap_err();
        assert!(matches!(err, IntegrationError::Validation(_)));
    }

    #[test]
    fn group_lookup_returns_not_found_for_missing_group() {
        let schema = schema_default();
        let err = VaultBackend::group(&schema, "nonexistent").unwrap_err();
        assert!(matches!(err, IntegrationError::SchemaNotFound(_)));
    }
}
