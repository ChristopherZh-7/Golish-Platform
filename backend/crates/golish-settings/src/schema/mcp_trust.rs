//! MCP server registry and trust/privacy/advanced toggles.
//!
//! Bundled together because they collectively define the agent's *security
//! posture*: which MCP servers it can spawn, which paths it can touch,
//! whether it can call out for telemetry, and which experimental knobs are
//! switched on.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::enums::LogLevel;

/// MCP (Model Context Protocol) server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(default)]
#[ts(export, export_to = "generated/")]
pub struct McpServerConfig {
    /// Command to start the server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Arguments for the command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables for the server.
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// URL for HTTP-based MCP servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Repository trust settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(default)]
#[ts(export, export_to = "generated/")]
pub struct TrustSettings {
    /// Paths with full trust (all tools allowed).
    #[serde(default)]
    pub full_trust: Vec<String>,

    /// Paths with read-only trust.
    #[serde(default)]
    pub read_only_trust: Vec<String>,

    /// Paths that are never trusted.
    #[serde(default)]
    pub never_trust: Vec<String>,

    /// Additional paths accessible outside workspace (supports glob patterns).
    /// Example: `["~/Documents/*", "/tmp/scratch"]`.
    #[serde(default)]
    pub allowed_paths: Vec<String>,

    /// Disable workspace path restrictions entirely (use with caution).
    #[serde(default)]
    pub disable_path_restrictions: bool,
}

/// Privacy and telemetry settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(default)]
#[ts(export, export_to = "generated/")]
pub struct PrivacySettings {
    /// Enable anonymous usage statistics.
    pub usage_statistics: bool,

    /// Log prompts for debugging (local only).
    pub log_prompts: bool,
}

/// Advanced/debug settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default, TS)]
#[serde(default)]
#[ts(export, export_to = "generated/")]
pub struct AdvancedSettings {
    /// Enable experimental features.
    pub enable_experimental: bool,

    /// Log level.
    pub log_level: LogLevel,

    /// Enable LLM API request/response logging to `./logs/api/`.
    /// When enabled, raw JSON request/response data is logged per session.
    pub enable_llm_api_logs: bool,

    /// Extract and parse the raw SSE JSON instead of logging escaped strings.
    /// When enabled, SSE chunks are logged as parsed JSON objects.
    pub extract_raw_sse: bool,
}
