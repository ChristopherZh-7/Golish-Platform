//! YAML / JSON file backend.
//!
//! Used by integrations whose secrets must be readable by an
//! out-of-process tool (e.g. ENScan_GO reads its `~/.config/enscan/config.yaml`
//! at startup). The integrations crate **never** keeps a separate copy
//! of these values — the file on disk is the source of truth.
//!
//! ## Write algorithm
//!
//! 1. Expand `~` and `$HOME` in [`crate::schema::ExternalFileStorage::path`].
//! 2. If [`crate::schema::ExternalFileStorage::preserve_unknown_keys`]
//!    is `true` and the file exists, parse the existing document so
//!    user-added keys outside our schema survive the write.
//! 3. Set each declared field at its dotted key path
//!    (`"cookies.aqc"` → `cookies: { aqc: <value> }`). Missing
//!    intermediate maps are created.
//! 4. If [`crate::schema::ExternalFileStorage::backup_on_write`] is
//!    `true`, copy the current file to
//!    `<path>.bak.<YYYYMMDD-HHMMSS>` (max 3 rolled).
//! 5. Serialize to a temp file in the same directory, `fsync`, then
//!    atomic-rename onto the target.
//!
//! Read / clear paths are symmetrical.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_yaml::Value as YamlValue;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::{IntegrationError, IntegrationResult};
use crate::schema::{
    ExternalFileFormat, ExternalFileStorage, IntegrationGroup, IntegrationSchema, Storage,
};
use crate::traits::StorageBackend;
use crate::types::FieldValue;

const MAX_BACKUPS: usize = 3;

/// Stateless: the file path lives in the schema.
pub struct ExternalFileBackend;

impl ExternalFileBackend {
    pub fn new() -> Self {
        Self
    }

    /// Extract the [`ExternalFileStorage`] block from the schema, or
    /// return [`IntegrationError::Validation`] if the schema does not
    /// declare external-file storage.
    fn storage(schema: &IntegrationSchema) -> IntegrationResult<&ExternalFileStorage> {
        match &schema.storage {
            Storage::ExternalFile { external_file } => Ok(external_file),
            other => Err(IntegrationError::Validation(format!(
                "ExternalFileBackend invoked with non-ExternalFile storage: {other:?}"
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

    /// Expand `~` (home) at the start of `path`. Leaves the rest untouched.
    fn expand(path: &str) -> PathBuf {
        if let Some(rest) = path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest);
            }
        }
        PathBuf::from(path)
    }

    /// Read & parse the file at `path` into a [`YamlValue`], using
    /// `format` to pick the parser. Missing files → [`YamlValue::Null`]
    /// (caller starts from a fresh document).
    async fn load_existing(
        path: &Path,
        format: ExternalFileFormat,
    ) -> IntegrationResult<YamlValue> {
        match fs::read_to_string(path).await {
            Ok(text) if text.trim().is_empty() => Ok(YamlValue::Null),
            Ok(text) => match format {
                ExternalFileFormat::Yaml => serde_yaml::from_str::<YamlValue>(&text).map_err(|e| {
                    IntegrationError::ExternalFileCorrupt {
                        path: path.display().to_string(),
                        reason: format!("invalid YAML: {e}"),
                    }
                }),
                ExternalFileFormat::Json => {
                    // serde_json parses into serde_json::Value; we then
                    // convert into YamlValue so the merge code can stay
                    // format-agnostic.
                    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                        IntegrationError::ExternalFileCorrupt {
                            path: path.display().to_string(),
                            reason: format!("invalid JSON: {e}"),
                        }
                    })?;
                    serde_yaml::to_value(&v).map_err(IntegrationError::from)
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(YamlValue::Null),
            Err(e) => Err(IntegrationError::Io(e)),
        }
    }

    /// Serialize `doc` to bytes in the requested format.
    fn serialize(doc: &YamlValue, format: ExternalFileFormat) -> IntegrationResult<Vec<u8>> {
        match format {
            ExternalFileFormat::Yaml => {
                let text = serde_yaml::to_string(doc)?;
                Ok(text.into_bytes())
            }
            ExternalFileFormat::Json => {
                // Convert YamlValue → serde_json::Value first (lossless
                // for the field types we store: scalars + maps + seqs).
                let json: serde_json::Value =
                    serde_yaml::from_value(doc.clone()).map_err(IntegrationError::from)?;
                let text = serde_json::to_string_pretty(&json)?;
                Ok(text.into_bytes())
            }
        }
    }

    /// Atomic write: write a sibling temp file then `rename` it.
    ///
    /// Same-directory tmp is required so the rename stays on one FS
    /// (rename across FS = copy + delete, not atomic).
    async fn atomic_write(target: &Path, bytes: &[u8]) -> IntegrationResult<()> {
        if let Some(parent) = target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }
        let tmp = target.with_extension(format!(
            "{}.tmp.{}",
            target.extension().and_then(|s| s.to_str()).unwrap_or("dat"),
            std::process::id()
        ));
        let mut f = fs::File::create(&tmp).await?;
        f.write_all(bytes).await?;
        f.flush().await?;
        f.sync_all().await?;
        drop(f);
        fs::rename(&tmp, target).await?;
        Ok(())
    }

    /// Copy the current file to `<path>.bak.<ts>` and keep at most
    /// [`MAX_BACKUPS`] siblings. No-op if the source file doesn't
    /// exist yet (first ever write).
    async fn rolling_backup(target: &Path) -> IntegrationResult<()> {
        if !fs::try_exists(target).await? {
            return Ok(());
        }
        let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        let bak = target.with_extension(format!(
            "{}.bak.{ts}",
            target.extension().and_then(|s| s.to_str()).unwrap_or("dat")
        ));
        fs::copy(target, &bak).await?;
        prune_backups(target).await?;
        Ok(())
    }

    /// Write `value` into `root` at the dotted key path.
    ///
    /// Intermediate missing keys are filled with empty maps. If an
    /// intermediate key currently holds a non-map value, it's
    /// overwritten (the schema is authoritative for keys it claims).
    fn set_at_path(root: &mut YamlValue, dotted: &str, value: &str) {
        if !matches!(root, YamlValue::Mapping(_)) {
            *root = YamlValue::Mapping(Default::default());
        }
        let parts: Vec<&str> = dotted.split('.').collect();
        let mut cursor = root;
        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;
            // Ensure the cursor is a mapping; replace if not.
            if !matches!(cursor, YamlValue::Mapping(_)) {
                *cursor = YamlValue::Mapping(Default::default());
            }
            let map = match cursor {
                YamlValue::Mapping(m) => m,
                _ => unreachable!("just normalized"),
            };
            let key = YamlValue::String((*part).to_string());
            if is_last {
                map.insert(key, YamlValue::String(value.to_string()));
                return;
            }
            // descend / create the intermediate map
            if !map.contains_key(&key) {
                map.insert(key.clone(), YamlValue::Mapping(Default::default()));
            }
            cursor = map.get_mut(&key).expect("just inserted");
        }
    }

    /// Read the string at the dotted key path. Returns `None` for
    /// missing keys or for non-string leaf values.
    fn get_at_path(root: &YamlValue, dotted: &str) -> Option<String> {
        let mut cursor = root;
        for part in dotted.split('.') {
            match cursor {
                YamlValue::Mapping(m) => {
                    cursor = m.get(YamlValue::String(part.to_string()))?;
                }
                _ => return None,
            }
        }
        match cursor {
            YamlValue::String(s) => Some(s.clone()),
            YamlValue::Bool(b) => Some(b.to_string()),
            YamlValue::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    /// Delete the value at the dotted key path. Empty intermediate
    /// maps are NOT pruned (the user might have other unrelated keys
    /// in the same map, and pruning would risk data loss).
    fn delete_at_path(root: &mut YamlValue, dotted: &str) {
        let parts: Vec<&str> = dotted.split('.').collect();
        let mut cursor = root;
        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;
            let map = match cursor {
                YamlValue::Mapping(m) => m,
                _ => return,
            };
            let key = YamlValue::String((*part).to_string());
            if is_last {
                map.remove(&key);
                return;
            }
            match map.get_mut(&key) {
                Some(next) => cursor = next,
                None => return,
            }
        }
    }
}

impl Default for ExternalFileBackend {
    fn default() -> Self {
        Self::new()
    }
}

async fn prune_backups(target: &Path) -> IntegrationResult<()> {
    let Some(parent) = target.parent() else {
        return Ok(());
    };
    let file_name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let mut entries = fs::read_dir(parent).await?;
    // collect sibling paths matching `<file_name>.bak.<ts>`
    let prefix = format!("{file_name}.bak.");
    // Compare on the full extension chain (target.with_extension result
    // is `<stem>.<ts>`), so match against the full file name.
    let mut backups: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix)
            || name.contains(".bak.")
                && name.starts_with(target.file_stem().and_then(|s| s.to_str()).unwrap_or(""))
        {
            let meta = entry.metadata().await?;
            let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            backups.push((modified, entry.path()));
        }
    }
    if backups.len() <= MAX_BACKUPS {
        return Ok(());
    }
    // Sort by modified time descending (newest first); delete the tail.
    backups.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));
    for (_ts, path) in backups.into_iter().skip(MAX_BACKUPS) {
        let _ = fs::remove_file(&path).await;
    }
    Ok(())
}

#[async_trait]
impl StorageBackend for ExternalFileBackend {
    async fn read(
        &self,
        _tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<HashMap<String, FieldValue>> {
        let storage = Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;
        let path = Self::expand(&storage.path);
        let doc = Self::load_existing(&path, storage.format).await?;
        let mut out = HashMap::new();
        let updated_at: Option<chrono::DateTime<chrono::Utc>> = if path.exists() {
            fs::metadata(&path)
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .map(chrono::DateTime::<chrono::Utc>::from)
        } else {
            None
        };
        for field in &group.fields {
            let entry = match Self::get_at_path(&doc, &field.key) {
                Some(value) if !value.is_empty() => {
                    if field.field_type.is_secret() {
                        // Don't surface the secret value, but indicate
                        // it's configured.
                        FieldValue::secret_set(None, updated_at.unwrap_or_else(chrono::Utc::now))
                    } else {
                        FieldValue {
                            has_value: true,
                            value: Some(value),
                            display_hint: None,
                            updated_at,
                        }
                    }
                }
                _ => FieldValue::empty(),
            };
            out.insert(field.key.clone(), entry);
        }
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

        // 1. Validate field keys against schema and check required.
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

        let path = Self::expand(&storage.path);
        // 2. Start from existing document if preserving unknown keys.
        let mut doc = if storage.preserve_unknown_keys {
            Self::load_existing(&path, storage.format).await?
        } else {
            YamlValue::Null
        };

        // 3. Set each field.
        for f in &group.fields {
            if let Some(v) = fields.get(&f.key) {
                Self::set_at_path(&mut doc, &f.key, v);
            }
        }

        // 4. Backup before overwrite (if enabled and the file exists).
        if storage.backup_on_write {
            Self::rolling_backup(&path).await?;
        }

        // 5. Serialize + atomic write.
        let bytes = Self::serialize(&doc, storage.format)?;
        Self::atomic_write(&path, &bytes).await?;
        Ok(())
    }

    async fn clear(
        &self,
        _tool_id: &str,
        group_id: &str,
        schema: &IntegrationSchema,
    ) -> IntegrationResult<()> {
        let storage = Self::storage(schema)?;
        let group = Self::group(schema, group_id)?;
        let path = Self::expand(&storage.path);
        if !path.exists() {
            return Ok(());
        }
        let mut doc = Self::load_existing(&path, storage.format).await?;
        for f in &group.fields {
            Self::delete_at_path(&mut doc, &f.key);
        }
        if storage.backup_on_write {
            Self::rolling_backup(&path).await?;
        }
        let bytes = Self::serialize(&doc, storage.format)?;
        Self::atomic_write(&path, &bytes).await?;
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
        let path = Self::expand(&storage.path);
        let doc = Self::load_existing(&path, storage.format).await?;
        let mut out = HashMap::new();
        for f in &group.fields {
            if let Some(v) = Self::get_at_path(&doc, &f.key) {
                if !v.is_empty() {
                    out.insert(f.key.clone(), v);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Field, FieldType, IntegrationGroup, IntegrationSchema};
    use tempfile::TempDir;

    fn enscan_tyc_schema(path: &Path) -> IntegrationSchema {
        IntegrationSchema {
            category: "enterprise-intel".into(),
            display_name: "ENScan_GO".into(),
            description: None,
            storage: Storage::ExternalFile {
                external_file: ExternalFileStorage {
                    path: path.to_string_lossy().into(),
                    format: ExternalFileFormat::Yaml,
                    preserve_unknown_keys: true,
                    backup_on_write: true,
                },
            },
            help_url: None,
            groups: vec![IntegrationGroup {
                id: "tyc".into(),
                name: "TYC".into(),
                description: None,
                icon: None,
                help_url: None,
                test: None,
                capture: None,
                fields: vec![
                    Field {
                        key: "cookies.tyc".into(),
                        label: "Cookie".into(),
                        field_type: FieldType::SecretTextarea,
                        placeholder: None,
                        required: true,
                        rows: None,
                        options: vec![],
                        pattern: None,
                    },
                    Field {
                        key: "tyc.tycid".into(),
                        label: "tycid".into(),
                        field_type: FieldType::SecretText,
                        placeholder: None,
                        required: true,
                        rows: None,
                        options: vec![],
                        pattern: None,
                    },
                    Field {
                        key: "tyc.auth_token".into(),
                        label: "auth_token".into(),
                        field_type: FieldType::SecretText,
                        placeholder: None,
                        required: true,
                        rows: None,
                        options: vec![],
                        pattern: None,
                    },
                ],
            }],
        }
    }

    fn aqc_schema(path: &Path, preserve: bool) -> IntegrationSchema {
        IntegrationSchema {
            category: "enterprise-intel".into(),
            display_name: "ENScan_GO".into(),
            description: None,
            storage: Storage::ExternalFile {
                external_file: ExternalFileStorage {
                    path: path.to_string_lossy().into(),
                    format: ExternalFileFormat::Yaml,
                    preserve_unknown_keys: preserve,
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
        }
    }

    #[tokio::test]
    async fn write_creates_yaml_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        let mut fields = HashMap::new();
        fields.insert("cookies.aqc".into(), "BAIDUID=1;BDUSS=2".into());
        backend
            .write("enscan-go", "aqc", &schema, fields)
            .await
            .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("cookies:"));
        assert!(text.contains("aqc:"));
        assert!(text.contains("BAIDUID=1;BDUSS=2"));
    }

    #[tokio::test]
    async fn write_preserves_unknown_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            r#"
# user's hand-written config
log_level: debug
proxy:
  enabled: true
  url: http://127.0.0.1:8080
cookies:
  aqc: old_cookie
"#,
        )
        .unwrap();

        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        let mut fields = HashMap::new();
        fields.insert("cookies.aqc".into(), "new_cookie".into());
        backend
            .write("enscan-go", "aqc", &schema, fields)
            .await
            .unwrap();

        let parsed: YamlValue =
            serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        // Schema key updated:
        assert_eq!(
            parsed
                .get("cookies")
                .and_then(|v| v.get("aqc"))
                .and_then(|v| v.as_str()),
            Some("new_cookie")
        );
        // Unrelated user keys preserved:
        assert_eq!(
            parsed.get("log_level").and_then(|v| v.as_str()),
            Some("debug")
        );
        assert_eq!(
            parsed
                .get("proxy")
                .and_then(|v| v.get("enabled"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            parsed
                .get("proxy")
                .and_then(|v| v.get("url"))
                .and_then(|v| v.as_str()),
            Some("http://127.0.0.1:8080")
        );
    }

    #[tokio::test]
    async fn write_three_field_group_makes_nested_yaml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let schema = enscan_tyc_schema(&path);
        let backend = ExternalFileBackend::new();
        let mut fields = HashMap::new();
        fields.insert("cookies.tyc".into(), "TYC_COOKIE".into());
        fields.insert("tyc.tycid".into(), "TYCID123".into());
        fields.insert("tyc.auth_token".into(), "AUTH456".into());
        backend
            .write("enscan-go", "tyc", &schema, fields)
            .await
            .unwrap();

        let parsed: YamlValue =
            serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed
                .get("cookies")
                .and_then(|v| v.get("tyc"))
                .and_then(|v| v.as_str()),
            Some("TYC_COOKIE")
        );
        assert_eq!(
            parsed
                .get("tyc")
                .and_then(|v| v.get("tycid"))
                .and_then(|v| v.as_str()),
            Some("TYCID123")
        );
        assert_eq!(
            parsed
                .get("tyc")
                .and_then(|v| v.get("auth_token"))
                .and_then(|v| v.as_str()),
            Some("AUTH456")
        );
    }

    #[tokio::test]
    async fn read_returns_secret_set_without_value() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "cookies:\n  aqc: BAIDUID=1\n").unwrap();
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        let result = backend.read("enscan-go", "aqc", &schema).await.unwrap();
        let v = result.get("cookies.aqc").expect("field present");
        assert!(v.has_value);
        assert_eq!(v.value, None, "secret field MUST NOT surface plaintext");
    }

    #[tokio::test]
    async fn read_returns_empty_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        let result = backend.read("enscan-go", "aqc", &schema).await.unwrap();
        let v = result.get("cookies.aqc").expect("field declared");
        assert!(!v.has_value);
    }

    #[tokio::test]
    async fn read_cleartext_surfaces_actual_value() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "cookies:\n  aqc: BAIDUID=1\n").unwrap();
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        let result = backend
            .read_cleartext("enscan-go", "aqc", &schema)
            .await
            .unwrap();
        assert_eq!(
            result.get("cookies.aqc").map(String::as_str),
            Some("BAIDUID=1")
        );
    }

    #[tokio::test]
    async fn clear_removes_only_declared_fields() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            r#"
log_level: debug
cookies:
  aqc: keep_user_other  # this will be wiped (it's our schema key)
  unrelated: untouched
"#,
        )
        .unwrap();
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        backend.clear("enscan-go", "aqc", &schema).await.unwrap();

        let parsed: YamlValue =
            serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed.get("log_level").and_then(|v| v.as_str()),
            Some("debug"),
            "non-schema keys must survive clear()"
        );
        assert!(
            parsed.get("cookies").and_then(|v| v.get("aqc")).is_none(),
            "schema key must be removed"
        );
        assert_eq!(
            parsed
                .get("cookies")
                .and_then(|v| v.get("unrelated"))
                .and_then(|v| v.as_str()),
            Some("untouched"),
            "sibling user keys in the same map must survive"
        );
    }

    #[tokio::test]
    async fn write_validates_required_fields() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        // empty value for required field — should reject
        let mut fields = HashMap::new();
        fields.insert("cookies.aqc".into(), "   ".into());
        let err = backend
            .write("enscan-go", "aqc", &schema, fields)
            .await
            .unwrap_err();
        assert!(matches!(err, IntegrationError::Validation(_)));
        // file must NOT be created
        assert!(!path.exists(), "must not write when validation fails");
    }

    #[tokio::test]
    async fn write_rejects_unknown_field_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        let mut fields = HashMap::new();
        fields.insert("cookies.aqc".into(), "ok".into());
        fields.insert("not.in.schema".into(), "bad".into());
        let err = backend
            .write("enscan-go", "aqc", &schema, fields)
            .await
            .unwrap_err();
        assert!(matches!(err, IntegrationError::Validation(_)));
    }

    #[tokio::test]
    async fn backup_keeps_max_three_rolling() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        for i in 0..5 {
            let mut fields = HashMap::new();
            fields.insert("cookies.aqc".into(), format!("cookie_{i}"));
            backend
                .write("enscan-go", "aqc", &schema, fields)
                .await
                .unwrap();
            // bump mtime so backups don't get lumped into the same sort bucket
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        // After 5 writes: 1 main file + at most 3 backups
        let bak_count = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.contains(".bak.")
            })
            .count();
        assert!(
            bak_count <= MAX_BACKUPS,
            "expected at most {MAX_BACKUPS} backups, got {bak_count}"
        );
    }

    #[tokio::test]
    async fn atomic_write_no_leftover_tmp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        let mut fields = HashMap::new();
        fields.insert("cookies.aqc".into(), "v".into());
        backend
            .write("enscan-go", "aqc", &schema, fields)
            .await
            .unwrap();
        let leftover = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".tmp."));
        assert!(!leftover, "tmp file must be renamed away, not left behind");
    }

    #[tokio::test]
    async fn json_format_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.json");
        let mut schema = aqc_schema(&path, true);
        if let Storage::ExternalFile { external_file } = &mut schema.storage {
            external_file.format = ExternalFileFormat::Json;
        }
        let backend = ExternalFileBackend::new();
        let mut fields = HashMap::new();
        fields.insert("cookies.aqc".into(), "JSON_COOKIE".into());
        backend
            .write("enscan-go", "aqc", &schema, fields)
            .await
            .unwrap();

        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed.pointer("/cookies/aqc").and_then(|v| v.as_str()),
            Some("JSON_COOKIE")
        );

        // read back works
        let result = backend
            .read_cleartext("enscan-go", "aqc", &schema)
            .await
            .unwrap();
        assert_eq!(
            result.get("cookies.aqc").map(String::as_str),
            Some("JSON_COOKIE")
        );
    }

    #[tokio::test]
    async fn corrupt_yaml_returns_external_file_corrupt_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "not: valid: yaml: [unclosed").unwrap();
        let schema = aqc_schema(&path, true);
        let backend = ExternalFileBackend::new();
        let err = backend.read("enscan-go", "aqc", &schema).await.unwrap_err();
        match err {
            IntegrationError::ExternalFileCorrupt { reason, .. } => {
                assert!(reason.contains("YAML"));
            }
            other => panic!("expected ExternalFileCorrupt, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_non_external_file_storage() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.yaml");
        let mut schema = aqc_schema(&path, true);
        schema.storage = Storage::vault_default();
        let backend = ExternalFileBackend::new();
        let err = backend.read("enscan-go", "aqc", &schema).await.unwrap_err();
        assert!(matches!(err, IntegrationError::Validation(_)));
    }
}
