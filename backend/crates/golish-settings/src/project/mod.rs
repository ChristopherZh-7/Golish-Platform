//! Per-project settings for Golish.
//!
//! This module provides project-level settings that override global defaults
//! for specific values like AI provider, model, and agent mode.
//!
//! Settings are stored in `{workspace}/.golish/project.toml` and only contain
//! overrides - they do NOT replace the global `~/.golish/settings.toml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::schema::AiProvider;

/// Per-project settings that override global defaults.
///
/// Only fields that are Some() will override the global settings.
/// This allows projects to remember their preferred model/mode without
/// affecting other global configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectSettings {
    /// AI configuration overrides
    #[serde(default)]
    pub ai: ProjectAiSettings,
}

/// AI-specific project settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectAiSettings {
    /// Override for the AI provider (e.g., "anthropic", "openai")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AiProvider>,

    /// Override for the model name (e.g., "claude-sonnet-4-20250514")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Override for agent mode ("default", "auto-approve", "planning")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<String>,
}

/// Manages per-project settings loading and persistence.
///
/// Similar to ToolPolicyManager, this handles loading from
/// `{workspace}/.golish/project.toml` and atomic saves.
pub struct ProjectSettingsManager {
    /// Current project settings
    settings: RwLock<ProjectSettings>,
    /// Path to the project settings file
    config_path: PathBuf,
}

impl ProjectSettingsManager {
    /// Create a new ProjectSettingsManager for the given workspace.
    ///
    /// Loads settings from `{workspace}/.golish/project.toml` if it exists,
    /// otherwise uses defaults (all None).
    pub async fn new(workspace: &Path) -> Self {
        let config_path = workspace.join(".golish").join("project.toml");
        let settings = Self::load_from_path(&config_path).await;

        if settings.ai.provider.is_some()
            || settings.ai.model.is_some()
            || settings.ai.agent_mode.is_some()
        {
            tracing::debug!("Loaded project settings from {:?}", config_path);
        }

        Self {
            settings: RwLock::new(settings),
            config_path,
        }
    }

    /// Load settings from a path, returning defaults if file doesn't exist.
    async fn load_from_path(path: &PathBuf) -> ProjectSettings {
        if !path.exists() {
            return ProjectSettings::default();
        }

        match tokio::fs::read_to_string(path).await {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(settings) => settings,
                Err(e) => {
                    tracing::warn!("Failed to parse project settings {:?}: {}", path, e);
                    ProjectSettings::default()
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read project settings {:?}: {}", path, e);
                ProjectSettings::default()
            }
        }
    }

    /// Get the current project settings.
    pub async fn get(&self) -> ProjectSettings {
        self.settings.read().await.clone()
    }

    /// Update project settings and persist to disk.
    pub async fn update(&self, new_settings: ProjectSettings) -> Result<()> {
        *self.settings.write().await = new_settings.clone();
        self.save().await
    }

    /// Update just the AI settings (provider, model, agent_mode).
    pub async fn update_ai_settings(
        &self,
        provider: Option<AiProvider>,
        model: Option<String>,
        agent_mode: Option<String>,
    ) -> Result<()> {
        let mut settings = self.settings.write().await;
        settings.ai.provider = provider;
        settings.ai.model = model;
        settings.ai.agent_mode = agent_mode;
        drop(settings);

        self.save().await
    }

    /// Set just the provider and model.
    pub async fn set_model(&self, provider: AiProvider, model: String) -> Result<()> {
        let mut settings = self.settings.write().await;
        settings.ai.provider = Some(provider);
        settings.ai.model = Some(model);
        drop(settings);

        self.save().await
    }

    /// Set just the agent mode.
    pub async fn set_agent_mode(&self, agent_mode: String) -> Result<()> {
        let mut settings = self.settings.write().await;
        settings.ai.agent_mode = Some(agent_mode);
        drop(settings);

        self.save().await
    }

    /// Save current settings to disk with atomic write.
    async fn save(&self) -> Result<()> {
        let settings = self.settings.read().await;

        // Only save if there's something to save
        if settings.ai.provider.is_none()
            && settings.ai.model.is_none()
            && settings.ai.agent_mode.is_none()
        {
            return Ok(());
        }

        // Ensure .golish directory exists
        if let Some(parent) = self.config_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .context("Failed to create .golish directory")?;
        }

        // Serialize to TOML
        let toml_string =
            toml::to_string_pretty(&*settings).context("Failed to serialize project settings")?;

        // Atomic write: write to temp file, then rename
        // Use a unique temp file name to avoid conflicts with concurrent writes
        let temp_filename = format!("project.toml.{}.tmp", std::process::id());
        let temp_path = self.config_path.with_file_name(temp_filename);
        tokio::fs::write(&temp_path, &toml_string)
            .await
            .context("Failed to write temp settings file")?;
        tokio::fs::rename(&temp_path, &self.config_path)
            .await
            .context("Failed to rename temp settings file")?;

        tracing::debug!("Saved project settings to {:?}", self.config_path);
        Ok(())
    }

    /// Get the path to the project settings file.
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Reload settings from disk.
    pub async fn reload(&self) -> Result<()> {
        let settings = Self::load_from_path(&self.config_path).await;
        *self.settings.write().await = settings;
        Ok(())
    }

    /// Clear all project settings (removes the file).
    pub async fn clear(&self) -> Result<()> {
        *self.settings.write().await = ProjectSettings::default();

        if self.config_path.exists() {
            tokio::fs::remove_file(&self.config_path)
                .await
                .context("Failed to remove project settings file")?;
            tracing::debug!("Removed project settings file {:?}", self.config_path);
        }

        Ok(())
    }
}


#[cfg(test)]
mod tests;
