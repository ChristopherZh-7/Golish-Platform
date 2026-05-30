//! [`Storage`] — where an integration's credentials are persisted
//! (vault row, external config file, or golish settings.toml path).

use serde::{Deserialize, Serialize};

/// Where credentials are persisted.
///
/// Tag-discriminated union with a **nested** payload per variant
/// (easier to read in JSON / YAML hand-edited by humans):
///
/// ```jsonc
/// { "type": "vault", "vault": { "extra_tags": [...] } }
/// { "type": "external_file",
///   "external_file": { "path": "...", "format": "yaml", ... } }
/// { "type": "settings", "settings": { "key": "network.github_token" } }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Storage {
    /// Stored encrypted in the `vault_entries` table. Each field in a
    /// group becomes one vault row, aggregated by
    /// `tags=["integration-group", <tool>, <group>]`.
    Vault {
        #[serde(default)]
        vault: VaultStorage,
    },

    /// Rendered into a file the external process reads (e.g.
    /// `~/.config/enscan/config.yaml`). The integration crate never
    /// keeps a separate copy — the file is authoritative.
    ExternalFile { external_file: ExternalFileStorage },

    /// Written through [`crate::traits::StorageBackend`] into the
    /// existing `golish settings.toml` at the given dotted path
    /// (e.g. `network.github_token`).
    Settings { settings: SettingsStorage },
}

impl Storage {
    /// Convenience constructor for `Storage::Vault` with default tags.
    pub fn vault_default() -> Self {
        Self::Vault {
            vault: VaultStorage::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct VaultStorage {
    /// Extra tags to attach to vault rows on top of the default
    /// `["integration-group", <tool>, <group>]`. Optional, useful for
    /// migration markers (`"data-source"` / `"intel-provider"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalFileStorage {
    /// File path with `~` expanded at write time. Required.
    pub path: String,

    /// File format. Determines parser / serializer used.
    #[serde(default = "default_yaml_format")]
    pub format: ExternalFileFormat,

    /// When true, parse existing file and merge new values into it
    /// rather than overwriting (so user-added keys outside our schema
    /// survive a Golish write).
    #[serde(default = "default_true")]
    pub preserve_unknown_keys: bool,

    /// When true, copy the existing file to
    /// `<path>.bak.<YYYYMMDD-HHMMSS>` before writing the new one.
    /// At most 3 backups are kept (oldest rotated out).
    #[serde(default = "default_true")]
    pub backup_on_write: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ExternalFileFormat {
    Yaml,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettingsStorage {
    /// Dotted setting path (e.g. `"network.github_token"`). Resolved
    /// via the existing `SettingsManager`.
    pub key: String,
}

fn default_yaml_format() -> ExternalFileFormat {
    ExternalFileFormat::Yaml
}

fn default_true() -> bool {
    true
}
