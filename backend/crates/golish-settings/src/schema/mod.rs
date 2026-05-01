//! Settings schema definitions for Golish configuration.
//!
//! All settings structs use `#[serde(default)]` to allow partial configuration
//! files. Missing fields are filled with sensible defaults.
//!
//! # Module layout
//!
//! - [`enums`] — `AiProvider`, `Theme`, `LogLevel`, `IndexLocation`, …
//! - [`defaults`] — pure default-value functions used by `#[serde(default)]`
//! - [`llm`] — per-provider LLM settings (`VertexAiSettings`, `OpenAiSettings`, …)
//! - [`ai`] — `AiSettings`, `SubAgentModelConfig`, `NetworkSettings`, `ApiKeysSettings`
//! - [`ui`] — `UiSettings`, `WindowSettings`, `CaretSettings`, `TerminalSettings`,
//!   `AgentSettings`, `ToolsSettings`
//! - [`mcp_trust`] — `McpServerConfig`, `TrustSettings`, `PrivacySettings`,
//!   `AdvancedSettings`
//! - [`runtime`] — `IndexerSettings`, `ContextSettings`, `TelemetrySettings`,
//!   `LangfuseSettings`, `NotificationsSettings`, `CodebaseConfig`
//! - [`sidecar`] — `SidecarSettings` + `Synthesis*` per-backend settings
//!
//! `mod.rs` itself owns the [`GolishSettings`] root struct that aggregates
//! every sub-module's top-level settings, plus the master `Default` impl.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

mod ai;
mod defaults;
mod enums;
mod llm;
mod mcp_trust;
mod runtime;
mod sidecar;
mod ui;

#[cfg(test)]
mod tests;

pub use ai::*;
pub use enums::*;
pub use llm::*;
pub use mcp_trust::*;
pub use runtime::*;
pub use sidecar::*;
pub use ui::*;

/// Current schema version. Bump this and add a migration entry in
/// `loader::migrate_settings` whenever the settings shape changes.
pub const SCHEMA_VERSION: u32 = 1;

/// Root settings structure for Golish.
///
/// Loaded from `~/.golish/settings.toml` with environment variable
/// interpolation support. `schema_version` enables forward-compatible
/// migration: the loader detects an older version and applies a chain
/// of migration functions before deserialisation.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export, export_to = "generated/")]
pub struct GolishSettings {
    /// Schema version — must match [`SCHEMA_VERSION`] after loading.
    /// Older files are auto-migrated by `migrate_settings()` in the loader.
    #[serde(alias = "version")]
    pub schema_version: u32,

    /// AI provider configuration.
    pub ai: AiSettings,

    /// API keys for external services.
    pub api_keys: ApiKeysSettings,

    /// Tool enablement settings.
    #[serde(default)]
    pub tools: ToolsSettings,

    /// User interface preferences.
    pub ui: UiSettings,

    /// Terminal configuration.
    pub terminal: TerminalSettings,

    /// Agent behavior settings.
    pub agent: AgentSettings,

    /// MCP server definitions.
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,

    /// Repository trust levels.
    #[serde(default)]
    pub trust: TrustSettings,

    /// Privacy and telemetry settings.
    pub privacy: PrivacySettings,

    /// Advanced/debug settings.
    pub advanced: AdvancedSettings,

    /// Sidecar context capture settings.
    pub sidecar: SidecarSettings,

    /// Code indexer settings.
    pub indexer: IndexerSettings,

    /// Context window management settings.
    pub context: ContextSettings,

    /// Telemetry and observability settings.
    pub telemetry: TelemetrySettings,

    /// Network settings (proxy, etc.).
    #[serde(default)]
    pub network: NetworkSettings,

    /// Native OS notification settings.
    #[serde(default)]
    pub notifications: NotificationsSettings,

    /// List of indexed codebase paths (deprecated, migrated to `codebases`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexed_codebases: Vec<String>,

    /// Indexed codebases with configuration (new format).
    #[serde(default)]
    pub codebases: Vec<CodebaseConfig>,
}

impl Default for GolishSettings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ai: AiSettings::default(),
            api_keys: ApiKeysSettings::default(),
            tools: ToolsSettings::default(),
            ui: UiSettings::default(),
            terminal: TerminalSettings::default(),
            agent: AgentSettings::default(),
            mcp_servers: HashMap::new(),
            trust: TrustSettings::default(),
            privacy: PrivacySettings::default(),
            advanced: AdvancedSettings::default(),
            sidecar: SidecarSettings::default(),
            indexer: IndexerSettings::default(),
            context: ContextSettings::default(),
            telemetry: TelemetrySettings::default(),
            network: NetworkSettings::default(),
            notifications: NotificationsSettings::default(),
            indexed_codebases: Vec::new(),
            codebases: Vec::new(),
        }
    }
}
