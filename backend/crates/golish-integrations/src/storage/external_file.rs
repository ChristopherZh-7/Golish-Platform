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

/// Stateless except for an optional `tools_dir` hint used to expand
/// `{{tools_dir}}` templates inside the schema's `external_file.path`.
///
/// Path templates supported by [`Self::expand`]:
/// - `{{tools_dir}}` → the absolute path of the pentest tool-pack
///   directory (when the backend was built with [`Self::with_tools_dir`]).
///   When unset, the placeholder is left literal so the caller can
///   notice the misconfiguration on the first read / write.
/// - `~/` prefix → user home (via [`dirs::home_dir`]).
pub struct ExternalFileBackend {
    tools_dir: Option<PathBuf>,
}

impl ExternalFileBackend {
    pub fn new() -> Self {
        Self { tools_dir: None }
    }

    /// Attach a tools directory used to expand `{{tools_dir}}` inside
    /// the schema's `external_file.path`. Without this hint the
    /// template is left literal (which is intentional in tests that
    /// never write to a real tools layout — the corruption surfaces
    /// loudly the first time the path is touched).
    pub fn with_tools_dir(mut self, dir: PathBuf) -> Self {
        self.tools_dir = Some(dir);
        self
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

    /// Expand schema path templates.
    ///
    /// Order matters: `{{tools_dir}}` is substituted first (it may
    /// introduce a `~/`-prefixed path on some installs), then `~/` is
    /// resolved to the user home.
    fn expand(&self, path: &str) -> PathBuf {
        let mut expanded = path.to_string();
        if let Some(td) = &self.tools_dir {
            expanded = expanded.replace("{{tools_dir}}", &td.to_string_lossy());
        }
        if let Some(rest) = expanded.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest);
            }
        }
        PathBuf::from(expanded)
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
        let path = self.expand(&storage.path);
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

        let path = self.expand(&storage.path);
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
        let path = self.expand(&storage.path);
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
        let path = self.expand(&storage.path);
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
#[path = "external_file_tests.rs"]
mod tests;
