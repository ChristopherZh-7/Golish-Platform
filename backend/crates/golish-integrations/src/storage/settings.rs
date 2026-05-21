//! Golish `settings.toml`-backed storage.
//!
//! Used by integrations whose credentials live inside Golish's own
//! settings file at a dotted path (e.g. `network.github_token`). Reads
//! and writes go through the shared [`SettingsManager`] so the
//! existing in-memory cache stays consistent.
//!
//! Note: the value is stored **in plaintext** inside `settings.toml`
//! today — same as the existing GitHub Token entry — so this backend
//! does not encrypt. OS keychain integration is tracked as a future
//! upgrade in `docs/design/2026-05-21-integrations.md §7`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use golish_settings::SettingsManager;

use crate::error::{IntegrationError, IntegrationResult};
use crate::schema::{IntegrationGroup, IntegrationSchema, SettingsStorage, Storage};
use crate::traits::StorageBackend;
use crate::types::FieldValue;

/// Wraps the shared `SettingsManager` (already used everywhere else
/// in the codebase as a `tauri::State<'_, Arc<SettingsManager>>`).
pub struct SettingsBackend {
    manager: Arc<SettingsManager>,
}

impl SettingsBackend {
    pub fn new(manager: Arc<SettingsManager>) -> Self {
        Self { manager }
    }

    fn storage(schema: &IntegrationSchema) -> IntegrationResult<&SettingsStorage> {
        match &schema.storage {
            Storage::Settings { settings } => Ok(settings),
            other => Err(IntegrationError::Validation(format!(
                "SettingsBackend invoked with non-Settings storage: {other:?}"
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

    /// For settings storage we expect **single-field groups** (e.g.
    /// the GitHub Token group has one field `token` mapped onto
    /// `network.github_token`). Multi-field is rejected — declare
    /// one schema-key per setting path instead.
    fn single_field(group: &IntegrationGroup) -> IntegrationResult<&crate::schema::Field> {
        if group.fields.len() != 1 {
            return Err(IntegrationError::Validation(format!(
                "SettingsBackend requires single-field groups, but '{}' declares {}",
                group.id,
                group.fields.len()
            )));
        }
        Ok(&group.fields[0])
    }
}

#[async_trait]
impl StorageBackend for SettingsBackend {
    async fn read(
        &self,
        _tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<HashMap<String, FieldValue>> {
        let storage = Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;
        let field = Self::single_field(group)?;

        let value = self
            .manager
            .get_value(&storage.key)
            .await
            .unwrap_or(serde_json::Value::Null);

        let mut out = HashMap::new();
        let entry = match value.as_str() {
            Some(s) if !s.is_empty() => {
                if field.field_type.is_secret() {
                    FieldValue::secret_set(None, Utc::now())
                } else {
                    FieldValue::plain(s, Utc::now())
                }
            }
            _ => FieldValue::empty(),
        };
        out.insert(field.key.clone(), entry);
        Ok(out)
    }

    async fn write(
        &self,
        _tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
        fields: HashMap<String, String>,
    ) -> IntegrationResult<()> {
        let storage = Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;
        let field = Self::single_field(group)?;

        // Validate
        let Some(raw) = fields.get(&field.key) else {
            if field.required {
                return Err(IntegrationError::Validation(format!(
                    "field '{}' is required",
                    field.key
                )));
            }
            return Ok(()); // nothing to write
        };
        if field.required && raw.trim().is_empty() {
            return Err(IntegrationError::Validation(format!(
                "field '{}' is required",
                field.key
            )));
        }
        for k in fields.keys() {
            if k != &field.key {
                return Err(IntegrationError::Validation(format!(
                    "unknown field '{k}' for group '{group_id}'"
                )));
            }
        }

        self.manager
            .set_value(&storage.key, serde_json::Value::String(raw.clone()))
            .await
            .map_err(|e| IntegrationError::Internal(format!("settings.set_value failed: {e}")))?;
        Ok(())
    }

    async fn clear(
        &self,
        _tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<()> {
        let storage = Self::storage(schema)?;
        let _group = Self::group(schema, group_id)?;

        // "Clear" = write empty string. We can't actually unset a
        // typed `String` field on the in-memory settings struct, so
        // this preserves the schema invariant (field always exists).
        self.manager
            .set_value(&storage.key, serde_json::Value::String(String::new()))
            .await
            .map_err(|e| IntegrationError::Internal(format!("settings.set_value failed: {e}")))?;
        Ok(())
    }

    async fn read_cleartext(
        &self,
        _tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<HashMap<String, String>> {
        let storage = Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;
        let field = Self::single_field(group)?;

        let value = self
            .manager
            .get_value(&storage.key)
            .await
            .unwrap_or(serde_json::Value::Null);
        let mut out = HashMap::new();
        if let Some(s) = value.as_str() {
            if !s.is_empty() {
                out.insert(field.key.clone(), s.to_string());
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    //! Settings round-trip tests need an on-disk `~/.golish/settings.toml`
    //! the manager can write into; they live with the IPC facade where
    //! we already spin up a test settings dir. Here we only cover the
    //! pure dispatch / validation branches.

    use super::*;
    use crate::schema::{Field, FieldType, IntegrationGroup, IntegrationSchema};

    fn github_token_schema() -> IntegrationSchema {
        IntegrationSchema {
            category: "code-host".into(),
            display_name: "GitHub".into(),
            description: None,
            storage: Storage::Settings {
                settings: SettingsStorage {
                    key: "network.github_token".into(),
                },
            },
            help_url: None,
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

    #[test]
    fn rejects_non_settings_storage() {
        let mut schema = github_token_schema();
        schema.storage = Storage::vault_default();
        let err = SettingsBackend::storage(&schema).unwrap_err();
        assert!(matches!(err, IntegrationError::Validation(_)));
    }

    #[test]
    fn rejects_multi_field_group() {
        let mut schema = github_token_schema();
        // Add a bogus second field.
        schema.groups[0].fields.push(Field {
            key: "token2".into(),
            label: "Token2".into(),
            field_type: FieldType::SecretText,
            placeholder: None,
            required: false,
            rows: None,
            options: vec![],
            pattern: None,
        });
        let group = &schema.groups[0];
        let err = SettingsBackend::single_field(group).unwrap_err();
        assert!(matches!(err, IntegrationError::Validation(_)));
    }

    #[test]
    fn accepts_single_field_group() {
        let schema = github_token_schema();
        let group = &schema.groups[0];
        let field = SettingsBackend::single_field(group).unwrap();
        assert_eq!(field.key, "token");
    }
}
