//! Default tool catalog and the [`ToolPolicyConfig`] structure persisted to
//! `tool-policy.json`.
//!
//! The default catalog encodes the safe-by-default policy:
//! - **Allow**: read-only operations (read_file, grep, list, indexer queries,
//!   knowledge tools, planning, ...). Auto-approved and also allowed in
//!   planning mode.
//! - **Prompt**: file modifications, command execution, web fetch — anything
//!   with side effects.
//! - **Deny**: outright dangerous operations (delete_file, execute_code).
//!
//! The catalog is split into two parallel sets per policy:
//! - `*_TYPED`  — tools backed by [`ToolName`] enum variants.
//! - `*_DYNAMIC` — names without enum variants (plugin tools, sub-agents).

use std::collections::HashMap;

use golish_core::ToolName;
use serde::{Deserialize, Serialize};

use super::types::{ToolConstraints, ToolPolicy};

/// Configuration for tool policies loaded from `.golish/tool-policy.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicyConfig {
    /// Version for future migrations.
    #[serde(default = "default_version")]
    pub version: u32,

    /// List of all known/available tools.
    #[serde(default)]
    pub available_tools: Vec<String>,

    /// Per-tool policies.
    #[serde(default)]
    pub policies: HashMap<String, ToolPolicy>,

    /// Per-tool constraints.
    #[serde(default)]
    pub constraints: HashMap<String, ToolConstraints>,

    /// Default policy for unknown tools.
    #[serde(default)]
    pub default_policy: ToolPolicy,
}

fn default_version() -> u32 {
    1
}

/// Type-safe list of allowed tools that have enum variants.
const ALLOW_TOOLS_TYPED: &[ToolName] = &[
    ToolName::ReadFile,
    ToolName::GrepFile,
    ToolName::ListFiles,
    ToolName::ListDirectory,
    ToolName::IndexerSearchCode,
    ToolName::IndexerSearchFiles,
    ToolName::IndexerAnalyzeFile,
    ToolName::IndexerExtractSymbols,
    ToolName::IndexerGetMetrics,
    ToolName::IndexerDetectLanguage,
    ToolName::UpdatePlan,
    ToolName::AstGrep,
    // Graph knowledge base (read-only)
    ToolName::GraphSearch,
    ToolName::GraphNeighbors,
    ToolName::GraphAttackPaths,
    // Vulnerability database (read-only)
    ToolName::SearchExploits,
];

/// Additional allowed tools without enum variants (dynamic/plugin tools).
const ALLOW_TOOLS_DYNAMIC: &[&str] = &[
    "debug_agent",
    "analyze_agent",
    "get_errors",
    "list_skills",
    "search_skills",
    "load_skill",
    "search_tools",
];

/// Default allowed tools (safe read-only operations).
/// Auto-approved and also allowed in planning mode.
pub const ALLOW_TOOLS: &[&str] = &[
    "read_file",
    "grep_file",
    "list_files",
    "list_directory",
    "indexer_search_code",
    "indexer_search_files",
    "indexer_analyze_file",
    "indexer_extract_symbols",
    "indexer_get_metrics",
    "indexer_detect_language",
    "debug_agent",
    "analyze_agent",
    "get_errors",
    "update_plan",
    "list_skills",
    "search_skills",
    "load_skill",
    "search_tools",
    "ast_grep",
    "graph_search",
    "graph_neighbors",
    "graph_attack_paths",
    "search_exploits",
];

/// Type-safe list of prompt tools that have enum variants.
const PROMPT_TOOLS_TYPED: &[ToolName] = &[
    ToolName::WriteFile,
    ToolName::CreateFile,
    ToolName::EditFile,
    ToolName::WebFetch,
    ToolName::RunPtyCmd,
    ToolName::RunCommand,
    ToolName::AstGrepReplace,
    // Graph knowledge base (mutating)
    ToolName::GraphAddEntity,
    ToolName::GraphAddRelation,
];

/// Additional prompt tools without enum variants.
const PROMPT_TOOLS_DYNAMIC: &[&str] = &[
    "apply_patch",
    "save_skill",
    "create_pty_session",
    "send_pty_input",
];

/// Default prompt tools (file modifications + side-effecting commands).
const PROMPT_TOOLS: &[&str] = &[
    "write_file",
    "create_file",
    "edit_file",
    "apply_patch",
    "save_skill",
    "web_fetch",
    "run_pty_cmd",
    "run_command",
    "create_pty_session",
    "send_pty_input",
    "ast_grep_replace",
    "graph_add_entity",
    "graph_add_relation",
];

/// Type-safe list of denied tools that have enum variants.
const DENY_TOOLS_TYPED: &[ToolName] = &[ToolName::DeleteFile];

/// Additional denied tools without enum variants.
const DENY_TOOLS_DYNAMIC: &[&str] = &["execute_code"];

/// Default deny tools (dangerous operations).
const DENY_TOOLS: &[&str] = &["delete_file", "execute_code"];

/// Default blocked hosts for network operations.
const BLOCKED_HOSTS: &[&str] = &[
    "127.0.0.1",
    "::1",
    "localhost",
    ".local",
    ".internal",
    ".lan",
];

/// Default blocked file patterns for write operations.
const BLOCKED_FILE_PATTERNS: &[&str] =
    &["*.env", "*.key", "*.pem", "**/credentials*", "**/secrets*"];

/// Check if a tool name corresponds to a known tool with an enum variant.
pub fn is_known_tool(tool_name: &str) -> bool {
    ToolName::from_str(tool_name).is_some()
}

/// Get the typed tool name if it exists.
pub fn get_known_tool(tool_name: &str) -> Option<ToolName> {
    ToolName::from_str(tool_name)
}

/// Get all typed allowed tools.
pub fn get_typed_allow_tools() -> &'static [ToolName] {
    ALLOW_TOOLS_TYPED
}

/// Get all typed prompt tools.
pub fn get_typed_prompt_tools() -> &'static [ToolName] {
    PROMPT_TOOLS_TYPED
}

/// Get all typed denied tools.
pub fn get_typed_deny_tools() -> &'static [ToolName] {
    DENY_TOOLS_TYPED
}

impl Default for ToolPolicyConfig {
    fn default() -> Self {
        // Build policies using typed tools first, then dynamic tools.
        let policies: HashMap<String, ToolPolicy> = ALLOW_TOOLS_TYPED
            .iter()
            .map(|t| (t.as_str().to_string(), ToolPolicy::Allow))
            .chain(
                ALLOW_TOOLS_DYNAMIC
                    .iter()
                    .map(|&t| (t.to_string(), ToolPolicy::Allow)),
            )
            .chain(
                PROMPT_TOOLS_TYPED
                    .iter()
                    .map(|t| (t.as_str().to_string(), ToolPolicy::Prompt)),
            )
            .chain(
                PROMPT_TOOLS_DYNAMIC
                    .iter()
                    .map(|&t| (t.to_string(), ToolPolicy::Prompt)),
            )
            .chain(
                DENY_TOOLS_TYPED
                    .iter()
                    .map(|t| (t.as_str().to_string(), ToolPolicy::Deny)),
            )
            .chain(
                DENY_TOOLS_DYNAMIC
                    .iter()
                    .map(|&t| (t.to_string(), ToolPolicy::Deny)),
            )
            .collect();

        let web_fetch_constraints = ToolConstraints {
            max_bytes: Some(65536), // 64KB max response
            blocked_hosts: Some(BLOCKED_HOSTS.iter().map(|&s| s.to_string()).collect()),
            ..Default::default()
        };

        let write_file_constraints = ToolConstraints {
            blocked_patterns: Some(
                BLOCKED_FILE_PATTERNS
                    .iter()
                    .map(|&s| s.to_string())
                    .collect(),
            ),
            ..Default::default()
        };

        let constraints: HashMap<String, ToolConstraints> = [
            ("web_fetch".to_string(), web_fetch_constraints),
            ("write_file".to_string(), write_file_constraints.clone()),
            ("edit_file".to_string(), write_file_constraints),
        ]
        .into_iter()
        .collect();

        Self {
            version: 1,
            available_tools: Vec::new(),
            policies,
            constraints,
            default_policy: ToolPolicy::Prompt,
        }
    }
}

// Suppress dead-code warnings for the parallel string lists kept alongside the
// typed catalogs. They're part of the public API surface in case callers need
// the raw `&str` form.
#[allow(dead_code)]
const _UNUSED_KEEP_PROMPT_TOOLS: &[&str] = PROMPT_TOOLS;
#[allow(dead_code)]
const _UNUSED_KEEP_DENY_TOOLS: &[&str] = DENY_TOOLS;
