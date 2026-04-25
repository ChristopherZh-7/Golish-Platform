//! [`ToolPolicyManager`] — runtime policy evaluation, persistence, and
//! pre-approval / full-auto state.
//!
//! Supports a two-tier policy system:
//! 1. **Global policy** (`~/.golish/tool-policy.json`) — user defaults.
//! 2. **Project policy** (`{workspace}/.golish/tool-policy.json`) — project
//!    overrides.
//!
//! Project policies override globals for the same tool. The merged config is
//! kept hot under [`ToolPolicyManager::config`] for fast lookups, while the
//! original two snapshots are preserved on the manager for `save_global` /
//! `save_project` round-trips.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use golish_core::ToolName;
use tokio::sync::RwLock;

use super::defaults::ToolPolicyConfig;
use super::types::{PolicyConstraintResult, ToolPolicy};

/// Manages tool policies for the AI agent.
pub struct ToolPolicyManager {
    /// Merged configuration (global + project, project takes precedence).
    config: RwLock<ToolPolicyConfig>,
    /// Global config snapshot (from `~/.golish/`).
    global_config: RwLock<Option<ToolPolicyConfig>>,
    /// Project config snapshot (from `{workspace}/.golish/`).
    project_config: RwLock<Option<ToolPolicyConfig>>,
    /// Path to the global policy file.
    global_config_path: PathBuf,
    /// Path to the project policy file.
    project_config_path: PathBuf,
    /// Tools that have been pre-approved this session.
    preapproved: RwLock<HashSet<String>>,
    /// Optional full-auto allowlist (when `Some`, listed tools auto-`Allow`).
    full_auto_allowlist: RwLock<Option<HashSet<String>>>,
}

impl ToolPolicyManager {
    /// Construct a manager for the given workspace by loading global +
    /// project configs and merging them.
    pub async fn new(workspace: &Path) -> Self {
        let global_config_path = Self::global_policy_path();
        let project_config_path = workspace.join(".golish").join("tool-policy.json");

        let global_config = Self::load_config_file(&global_config_path).await;
        if global_config.is_some() {
            tracing::debug!("Loaded global tool policy from {:?}", global_config_path);
        }

        let project_config = Self::load_config_file(&project_config_path).await;
        if project_config.is_some() {
            tracing::debug!("Loaded project tool policy from {:?}", project_config_path);
        }

        let merged_config = Self::merge_configs(&global_config, &project_config);

        Self {
            config: RwLock::new(merged_config),
            global_config: RwLock::new(global_config),
            project_config: RwLock::new(project_config),
            global_config_path,
            project_config_path,
            preapproved: RwLock::new(HashSet::new()),
            full_auto_allowlist: RwLock::new(None),
        }
    }

    /// Path to the global policy file (`~/.golish/tool-policy.json`).
    pub fn global_policy_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".golish")
            .join("tool-policy.json")
    }

    /// Load a config file if it exists.
    async fn load_config_file(path: &PathBuf) -> Option<ToolPolicyConfig> {
        if !path.exists() {
            return None;
        }

        match tokio::fs::read_to_string(path).await {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(config) => Some(config),
                Err(e) => {
                    tracing::warn!("Failed to parse tool policy config {:?}: {}", path, e);
                    None
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read tool policy config {:?}: {}", path, e);
                None
            }
        }
    }

    /// Merge global and project configs.
    ///
    /// Order:
    /// 1. Start from defaults.
    /// 2. Apply global config.
    /// 3. Apply project config (overrides global).
    pub fn merge_configs(
        global: &Option<ToolPolicyConfig>,
        project: &Option<ToolPolicyConfig>,
    ) -> ToolPolicyConfig {
        let mut merged = ToolPolicyConfig::default();

        if let Some(global_cfg) = global {
            for (tool, policy) in &global_cfg.policies {
                merged.policies.insert(tool.clone(), policy.clone());
            }
            for (tool, constraints) in &global_cfg.constraints {
                merged.constraints.insert(tool.clone(), constraints.clone());
            }
            merged.default_policy = global_cfg.default_policy.clone();
            for tool in &global_cfg.available_tools {
                if !merged.available_tools.contains(tool) {
                    merged.available_tools.push(tool.clone());
                }
            }
        }

        if let Some(project_cfg) = project {
            for (tool, policy) in &project_cfg.policies {
                merged.policies.insert(tool.clone(), policy.clone());
            }
            for (tool, constraints) in &project_cfg.constraints {
                merged.constraints.insert(tool.clone(), constraints.clone());
            }
            merged.default_policy = project_cfg.default_policy.clone();
            for tool in &project_cfg.available_tools {
                if !merged.available_tools.contains(tool) {
                    merged.available_tools.push(tool.clone());
                }
            }
        }

        merged
    }

    /// Construct a manager with an explicit config (for testing).
    pub fn with_config(config: ToolPolicyConfig, project_config_path: PathBuf) -> Self {
        Self {
            config: RwLock::new(config.clone()),
            global_config: RwLock::new(None),
            project_config: RwLock::new(Some(config)),
            global_config_path: Self::global_policy_path(),
            project_config_path,
            preapproved: RwLock::new(HashSet::new()),
            full_auto_allowlist: RwLock::new(None),
        }
    }

    pub fn has_global_policy(&self) -> bool {
        self.global_config_path.exists()
    }

    pub fn has_project_policy(&self) -> bool {
        self.project_config_path.exists()
    }

    pub async fn get_global_config(&self) -> Option<ToolPolicyConfig> {
        self.global_config.read().await.clone()
    }

    pub async fn get_project_config(&self) -> Option<ToolPolicyConfig> {
        self.project_config.read().await.clone()
    }

    // ========================================================================
    // Policy queries
    // ========================================================================

    /// Get the policy for a tool by name.
    ///
    /// Honors full-auto allowlist (auto-`Allow`), then falls back to the
    /// merged config, then to `default_policy`.
    pub async fn get_policy(&self, tool_name: &str) -> ToolPolicy {
        let config = self.config.read().await;

        if let Some(ref allowlist) = *self.full_auto_allowlist.read().await {
            if allowlist.contains(tool_name) {
                return ToolPolicy::Allow;
            }
        }

        config
            .policies
            .get(tool_name)
            .cloned()
            .unwrap_or_else(|| config.default_policy.clone())
    }

    /// Get the policy for a known tool (type-safe).
    pub async fn get_policy_for_tool(&self, tool: ToolName) -> ToolPolicy {
        self.get_policy(tool.as_str()).await
    }

    /// Set the policy for a tool and persist.
    pub async fn set_policy(&self, tool_name: &str, policy: ToolPolicy) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.policies.insert(tool_name.to_string(), policy);
        }
        self.save().await
    }

    /// Set the policy for a known tool (type-safe).
    pub async fn set_policy_for_tool(&self, tool: ToolName, policy: ToolPolicy) -> Result<()> {
        self.set_policy(tool.as_str(), policy).await
    }

    pub async fn get_constraints(&self, tool_name: &str) -> Option<super::types::ToolConstraints> {
        let config = self.config.read().await;
        config.constraints.get(tool_name).cloned()
    }

    pub async fn set_constraints(
        &self,
        tool_name: &str,
        constraints: super::types::ToolConstraints,
    ) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config
                .constraints
                .insert(tool_name.to_string(), constraints);
        }
        self.save().await
    }

    /// Apply policy constraints to tool arguments.
    ///
    /// Returns whether constraints pass, fail, or modify the args (e.g. when
    /// `max_items` reduces a request `limit`).
    pub async fn apply_constraints(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> PolicyConstraintResult {
        let config = self.config.read().await;

        let constraints = match config.constraints.get(tool_name) {
            Some(c) => c,
            None => return PolicyConstraintResult::Allowed,
        };

        // URL-based
        if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
            if let Some(reason) = constraints.is_url_blocked(url) {
                return PolicyConstraintResult::Violated(reason);
            }
        }

        // Path-based — try common field names
        for path_field in &["path", "file_path", "file", "target"] {
            if let Some(path) = args.get(*path_field).and_then(|v| v.as_str()) {
                if let Some(reason) = constraints.is_path_blocked(path) {
                    return PolicyConstraintResult::Violated(reason);
                }
            }
        }

        // Mode
        if let Some(mode) = args.get("mode").and_then(|v| v.as_str()) {
            if !constraints.is_mode_allowed(mode) {
                return PolicyConstraintResult::Violated(format!(
                    "Mode '{}' is not allowed",
                    mode
                ));
            }
        }

        // Item count → modify args to comply (cap `limit` at `max_items`).
        if let Some(max_items) = constraints.max_items {
            if let Some(limit) = args.get("limit").and_then(|v| v.as_u64()) {
                if limit > max_items as u64 {
                    let mut modified_args = args.clone();
                    if let Some(obj) = modified_args.as_object_mut() {
                        obj.insert("limit".to_string(), serde_json::json!(max_items));
                    }
                    return PolicyConstraintResult::Modified(
                        modified_args,
                        format!(
                            "Limit reduced from {} to {} per policy constraint",
                            limit, max_items
                        ),
                    );
                }
            }
        }

        PolicyConstraintResult::Allowed
    }

    /// True iff the tool can execute (Allow policy or pre-approved).
    pub async fn should_execute(&self, tool_name: &str) -> bool {
        if self.preapproved.read().await.contains(tool_name) {
            return true;
        }
        matches!(self.get_policy(tool_name).await, ToolPolicy::Allow)
    }

    pub async fn should_execute_tool(&self, tool: ToolName) -> bool {
        self.should_execute(tool.as_str()).await
    }

    pub async fn requires_prompt(&self, tool_name: &str) -> bool {
        if self.preapproved.read().await.contains(tool_name) {
            return false;
        }
        matches!(self.get_policy(tool_name).await, ToolPolicy::Prompt)
    }

    pub async fn requires_prompt_for_tool(&self, tool: ToolName) -> bool {
        self.requires_prompt(tool.as_str()).await
    }

    pub async fn is_denied(&self, tool_name: &str) -> bool {
        matches!(self.get_policy(tool_name).await, ToolPolicy::Deny)
    }

    pub async fn is_tool_denied(&self, tool: ToolName) -> bool {
        self.is_denied(tool.as_str()).await
    }

    // ========================================================================
    // Pre-approval / full-auto state
    // ========================================================================

    /// Pre-approve a tool for this session (one-time approval).
    pub async fn preapprove(&self, tool_name: &str) {
        self.preapproved.write().await.insert(tool_name.to_string());
    }

    /// Consume pre-approval status (one-time use).
    pub async fn take_preapproved(&self, tool_name: &str) -> bool {
        self.preapproved.write().await.remove(tool_name)
    }

    /// Enable full-auto mode with the given allowlist.
    pub async fn enable_full_auto(&self, allowed_tools: Vec<String>) {
        let allowlist: HashSet<String> = allowed_tools.into_iter().collect();
        *self.full_auto_allowlist.write().await = Some(allowlist);
        tracing::info!("Full-auto mode enabled");
    }

    pub async fn disable_full_auto(&self) {
        *self.full_auto_allowlist.write().await = None;
        tracing::info!("Full-auto mode disabled");
    }

    pub async fn is_full_auto_enabled(&self) -> bool {
        self.full_auto_allowlist.read().await.is_some()
    }

    pub async fn is_allowed_in_full_auto(&self, tool_name: &str) -> bool {
        if let Some(ref allowlist) = *self.full_auto_allowlist.read().await {
            allowlist.contains(tool_name)
        } else {
            false
        }
    }

    // ========================================================================
    // Bulk config operations
    // ========================================================================

    /// Update the list of available tools (synced from the tool registry).
    pub async fn sync_available_tools(&self, tools: Vec<String>) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.available_tools = tools;
        }
        self.save().await
    }

    pub async fn get_config(&self) -> ToolPolicyConfig {
        self.config.read().await.clone()
    }

    pub async fn set_config(&self, config: ToolPolicyConfig) -> Result<()> {
        *self.config.write().await = config;
        self.save().await
    }

    /// Set every tool to `Allow` and bump `default_policy` to `Allow`.
    pub async fn allow_all(&self) -> Result<()> {
        {
            let mut config = self.config.write().await;
            for tool in &config.available_tools.clone() {
                config.policies.insert(tool.clone(), ToolPolicy::Allow);
            }
            config.default_policy = ToolPolicy::Allow;
        }
        self.save().await
    }

    /// Set every tool to `Deny` and bump `default_policy` to `Deny`.
    pub async fn deny_all(&self) -> Result<()> {
        {
            let mut config = self.config.write().await;
            for tool in &config.available_tools.clone() {
                config.policies.insert(tool.clone(), ToolPolicy::Deny);
            }
            config.default_policy = ToolPolicy::Deny;
        }
        self.save().await
    }

    /// Drop all per-tool policies and revert to a `Prompt` default.
    pub async fn reset_to_prompt(&self) -> Result<()> {
        {
            let mut config = self.config.write().await;
            config.policies.clear();
            config.default_policy = ToolPolicy::Prompt;
        }
        self.save().await
    }

    pub async fn reset_to_defaults(&self) -> Result<()> {
        *self.config.write().await = ToolPolicyConfig::default();
        self.save().await
    }

    // ========================================================================
    // Persistence
    // ========================================================================

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
