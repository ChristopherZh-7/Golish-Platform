//! Settings loading, saving, and environment variable interpolation.
//!
//! The `SettingsManager` handles:
//! - Loading settings from `~/.golish/settings.toml`
//! - Resolving `$VAR` and `${VAR}` environment variable references
//!   (delegated to [`env`])
//! - Atomic file writes with temp file + rename
//! - First-run template generation
//! - Forward-compatible schema migration (delegated to [`migration`])

use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::sync::RwLock;

use crate::schema::GolishSettings;

mod env;
mod migration;

pub use env::{apply_proxy_env, get_with_env_fallback};
pub use migration::migrate_settings;

/// Embedded template for first-run generation.
const TEMPLATE: &str = include_str!("../template.toml");

/// Get the path to the global settings file.
pub fn settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".golish")
        .join("settings.toml")
}

/// Manages settings loading, interpolation, and persistence.
pub struct SettingsManager {
    /// Cached settings (with env vars resolved)
    settings: RwLock<GolishSettings>,

    /// Path to the settings file
    path: PathBuf,

    /// Serializes file write operations to prevent concurrent temp file conflicts
    write_mutex: tokio::sync::Mutex<()>,
}

impl SettingsManager {
    /// Create a new SettingsManager, loading from disk if available.
    pub async fn new() -> Result<Self> {
        let path = settings_path();
        let settings = Self::load_from_path(&path).await?;

        Ok(Self {
            settings: RwLock::new(settings),
            path,
            write_mutex: tokio::sync::Mutex::new(()),
        })
    }

    /// Load settings without Tauri state (for CLI/eval use).
    ///
    /// This is useful when you need settings outside of the Tauri app context,
    /// such as in CLI commands or evaluation scenarios.
    #[allow(dead_code)] // Used by evals feature
    pub async fn load_standalone() -> Result<GolishSettings> {
        let path = settings_path();
        Self::load_from_path(&path).await
    }

    /// Load settings from a specific path.
    async fn load_from_path(path: &PathBuf) -> Result<GolishSettings> {
        if !path.exists() {
            tracing::debug!("Settings file not found at {:?}, using defaults", path);
            return Ok(GolishSettings::default());
        }

        let contents = tokio::fs::read_to_string(path)
            .await
            .context("Failed to read settings file")?;

        // Run forward-compatible migration on raw TOML before deserialising.
        let mut raw: toml::Value =
            toml::from_str(&contents).context("Failed to parse settings TOML")?;
        migrate_settings(&mut raw)?;

        let toml_str = toml::to_string(&raw)?;
        let mut settings: GolishSettings =
            toml::from_str(&toml_str).context("Failed to deserialize settings")?;

        env::resolve_env_vars(&mut settings);

        tracing::info!("Loaded settings from {:?}", path);
        Ok(settings)
    }

    /// Get the current settings (read-only).
    pub async fn get(&self) -> GolishSettings {
        self.settings.read().await.clone()
    }

    /// Update settings and persist to disk.
    pub async fn update(&self, new_settings: GolishSettings) -> Result<()> {
        *self.settings.write().await = new_settings.clone();

        // Serialize file writes to prevent concurrent temp file race conditions.
        // Without this, rapid onChange calls (e.g. typing in an input) can cause
        // one rename to move the temp file before another rename reads it.
        let _file_guard = self.write_mutex.lock().await;

        let toml_string =
            toml::to_string_pretty(&new_settings).context("Failed to serialize settings")?;

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let temp_path = self.path.with_extension("toml.tmp");
        tokio::fs::write(&temp_path, &toml_string).await?;
        tokio::fs::rename(&temp_path, &self.path).await?;

        tracing::debug!("Saved settings to {:?}", self.path);
        Ok(())
    }

    /// Get a specific setting by dot-notation key (e.g., "ai.vertex_ai.project_id").
    pub async fn get_value(&self, key: &str) -> Result<serde_json::Value> {
        let settings = self.settings.read().await;
        let json = serde_json::to_value(&*settings)?;

        let mut current = &json;
        for part in key.split('.') {
            current = current
                .get(part)
                .ok_or_else(|| anyhow::anyhow!("Setting '{}' not found", key))?;
        }

        Ok(current.clone())
    }

    /// Set a specific setting by dot-notation key.
    pub async fn set_value(&self, key: &str, value: serde_json::Value) -> Result<()> {
        let mut settings = self.settings.write().await;
        let mut json = serde_json::to_value(&*settings)?;

        let parts: Vec<&str> = key.split('.').collect();
        set_nested_value(&mut json, &parts, value)?;

        *settings = serde_json::from_value(json)?;
        drop(settings);

        self.update(self.get().await).await
    }

    /// Reset to defaults and persist.
    pub async fn reset(&self) -> Result<()> {
        self.update(GolishSettings::default()).await
    }

    /// Check if settings file exists.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Get the settings file path.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Ensure settings file exists, creating from template if needed.
    ///
    /// Returns `true` if a new file was created.
    pub async fn ensure_settings_file(&self) -> Result<bool> {
        if self.path.exists() {
            return Ok(false);
        }

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&self.path, TEMPLATE).await?;
        tracing::info!("Generated settings template at {:?}", self.path);
        Ok(true)
    }

    /// Reload settings from disk.
    pub async fn reload(&self) -> Result<()> {
        let settings = Self::load_from_path(&self.path).await?;
        *self.settings.write().await = settings;
        Ok(())
    }
}

/// Set a value in a nested JSON object using a key path.
fn set_nested_value(
    json: &mut serde_json::Value,
    parts: &[&str],
    value: serde_json::Value,
) -> Result<()> {
    if parts.is_empty() {
        return Err(anyhow::anyhow!("Empty key path"));
    }

    let mut current = json;
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let Some(obj) = current.as_object_mut() {
                obj.insert((*part).to_string(), value);
                return Ok(());
            } else {
                return Err(anyhow::anyhow!("Cannot set value on non-object"));
            }
        } else {
            current = current
                .get_mut(*part)
                .ok_or_else(|| anyhow::anyhow!("Setting path '{}' not found", parts.join(".")))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::SCHEMA_VERSION;

    #[tokio::test]
    async fn test_settings_manager_defaults() {
        let manager = SettingsManager {
            settings: RwLock::new(GolishSettings::default()),
            path: PathBuf::from("/nonexistent/settings.toml"),
            write_mutex: tokio::sync::Mutex::new(()),
        };

        let settings = manager.get().await;
        assert_eq!(settings.schema_version, SCHEMA_VERSION);
        assert_eq!(
            settings.ai.default_provider,
            crate::schema::AiProvider::VertexAi
        );
    }

    #[tokio::test]
    async fn test_settings_manager_get_value() {
        let manager = SettingsManager {
            settings: RwLock::new(GolishSettings::default()),
            path: PathBuf::from("/nonexistent/settings.toml"),
            write_mutex: tokio::sync::Mutex::new(()),
        };

        let value = manager.get_value("ai.default_provider").await.unwrap();
        assert_eq!(value, serde_json::json!("vertex_ai"));

        let value = manager.get_value("terminal.font_size").await.unwrap();
        assert_eq!(value, serde_json::json!(14));
    }
}
