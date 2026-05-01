use std::path::PathBuf;

use anyhow::Result;

use super::ToolPolicyManager;
use crate::tool_policy::ToolPolicy;

impl ToolPolicyManager {
    /// Save to the project policy file.
    /// All current settings are persisted (the merged config is what we save).
    pub async fn save(&self) -> Result<()> {
        self.save_project().await
    }

    pub async fn save_project(&self) -> Result<()> {
        let config = self.config.read().await;

        if let Some(parent) = self.project_config_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let json = serde_json::to_string_pretty(&*config)?;
        tokio::fs::write(&self.project_config_path, json).await?;

        *self.project_config.write().await = Some(config.clone());

        tracing::debug!(
            "Saved project tool policy config to {:?}",
            self.project_config_path
        );
        Ok(())
    }

    pub async fn save_global(&self) -> Result<()> {
        let config = self.config.read().await;

        if let Some(parent) = self.global_config_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let json = serde_json::to_string_pretty(&*config)?;
        tokio::fs::write(&self.global_config_path, json).await?;

        *self.global_config.write().await = Some(config.clone());

        tracing::debug!(
            "Saved global tool policy config to {:?}",
            self.global_config_path
        );
        Ok(())
    }

    /// Reload configuration from both global and project files.
    pub async fn reload(&self) -> Result<()> {
        let global_config = Self::load_config_file(&self.global_config_path).await;
        *self.global_config.write().await = global_config.clone();

        let project_config = Self::load_config_file(&self.project_config_path).await;
        *self.project_config.write().await = project_config.clone();

        let merged = Self::merge_configs(&global_config, &project_config);
        *self.config.write().await = merged;

        tracing::debug!(
            "Reloaded tool policy configs (global: {}, project: {})",
            global_config.is_some(),
            project_config.is_some()
        );
        Ok(())
    }

    pub fn project_policy_path(&self) -> &PathBuf {
        &self.project_config_path
    }

    pub fn global_policy_path_ref(&self) -> &PathBuf {
        &self.global_config_path
    }

    /// Print a one-line-per-field summary of policy state. Useful when
    /// debugging why a tool decision came out unexpectedly.
    pub async fn print_status(&self) {
        let config = self.config.read().await;
        let preapproved = self.preapproved.read().await;
        let full_auto = self.full_auto_allowlist.read().await;

        tracing::info!("=== Tool Policy Status ===");
        tracing::info!("Default policy: {}", config.default_policy);
        tracing::info!("Available tools: {}", config.available_tools.len());
        tracing::info!("Configured policies: {}", config.policies.len());
        tracing::info!("Configured constraints: {}", config.constraints.len());
        tracing::info!("Pre-approved this session: {}", preapproved.len());
        tracing::info!(
            "Full-auto mode: {}",
            if full_auto.is_some() {
                "enabled"
            } else {
                "disabled"
            }
        );

        let allow_count = config
            .policies
            .values()
            .filter(|p| **p == ToolPolicy::Allow)
            .count();
        let prompt_count = config
            .policies
            .values()
            .filter(|p| **p == ToolPolicy::Prompt)
            .count();
        let deny_count = config
            .policies
            .values()
            .filter(|p| **p == ToolPolicy::Deny)
            .count();

        tracing::info!(
            "Policy distribution: {} allow, {} prompt, {} deny",
            allow_count,
            prompt_count,
            deny_count
        );
    }
}
