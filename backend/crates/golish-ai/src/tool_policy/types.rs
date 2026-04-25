//! Core types: [`ToolPolicy`], [`ToolConstraints`], and
//! [`PolicyConstraintResult`].
//!
//! Includes the simple glob matcher used by `ToolConstraints::is_path_blocked`
//! to evaluate `*.env`/`**/secrets/*`-style patterns without pulling in a full
//! glob crate.

use serde::{Deserialize, Serialize};

/// Policy for a tool determining whether it can be executed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolPolicy {
    /// Execute without prompting.
    Allow,
    /// Request user confirmation (HITL).
    #[default]
    Prompt,
    /// Prevent execution entirely.
    Deny,
}

impl std::fmt::Display for ToolPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolPolicy::Allow => write!(f, "allow"),
            ToolPolicy::Prompt => write!(f, "prompt"),
            ToolPolicy::Deny => write!(f, "deny"),
        }
    }
}

/// Constraints that can be applied to tool execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolConstraints {
    /// Maximum number of items/results (e.g., for list operations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,

    /// Maximum bytes for content operations (e.g., file read/write).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,

    /// Allowed modes for the tool (e.g., `["read", "write"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_modes: Option<Vec<String>>,

    /// Blocked URL schemes (e.g., `["file://", "data://"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_schemes: Option<Vec<String>>,

    /// Blocked domains/hosts (e.g., `["127.0.0.1", "localhost"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_hosts: Option<Vec<String>>,

    /// Allowed file extensions (e.g., `[".rs", ".ts", ".py"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_extensions: Option<Vec<String>>,

    /// Blocked file patterns (e.g., `["*.env", "*.key"]`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_patterns: Option<Vec<String>>,

    /// Maximum command execution time in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
}

impl ToolConstraints {
    /// Check if a URL is blocked based on schemes and hosts.
    pub fn is_url_blocked(&self, url: &str) -> Option<String> {
        if let Some(schemes) = &self.blocked_schemes {
            for scheme in schemes {
                if url.starts_with(scheme) {
                    return Some(format!("URL scheme '{}' is blocked", scheme));
                }
            }
        }

        if let Some(hosts) = &self.blocked_hosts {
            // Extract host from URL using simple parsing: look for `://` then
            // take everything until the next `/`, `:`, or `?`.
            if let Some(scheme_end) = url.find("://") {
                let after_scheme = &url[scheme_end + 3..];
                let host_end = after_scheme
                    .find(['/', ':', '?'])
                    .unwrap_or(after_scheme.len());
                let host = &after_scheme[..host_end];

                for blocked in hosts {
                    if host == blocked.as_str()
                        || host.ends_with(&format!(".{}", blocked))
                        || (blocked.starts_with('.') && host.ends_with(blocked))
                    {
                        return Some(format!("Host '{}' is blocked", host));
                    }
                }
            }
        }

        None
    }

    /// Check if a file path is blocked based on extensions and patterns.
    pub fn is_path_blocked(&self, path: &str) -> Option<String> {
        if let Some(patterns) = &self.blocked_patterns {
            for pattern in patterns {
                if simple_glob_match(pattern, path) {
                    return Some(format!("Path matches blocked pattern '{}'", pattern));
                }
            }
        }

        // Check allowed extensions (if specified, only these are allowed).
        if let Some(extensions) = &self.allowed_extensions {
            if !extensions.is_empty() {
                let has_valid_ext = extensions
                    .iter()
                    .any(|ext| path.ends_with(ext) || path.ends_with(&ext[1..]));
                if !has_valid_ext {
                    return Some(format!(
                        "File extension not in allowed list: {:?}",
                        extensions
                    ));
                }
            }
        }

        None
    }

    /// Check if a mode is allowed.
    pub fn is_mode_allowed(&self, mode: &str) -> bool {
        match &self.allowed_modes {
            Some(modes) => modes.iter().any(|m| m == mode),
            None => true,
        }
    }

    /// Check if an item count exceeds the limit.
    pub fn exceeds_max_items(&self, count: u32) -> bool {
        self.max_items.map(|max| count > max).unwrap_or(false)
    }

    /// Check if a byte size exceeds the limit.
    pub fn exceeds_max_bytes(&self, bytes: u64) -> bool {
        self.max_bytes.map(|max| bytes > max).unwrap_or(false)
    }
}

/// Simple glob pattern matching (supports `*`, `**`, and exact match).
///
/// Not a full glob implementation — only the patterns we actually need
/// (`*.env`, `**/secrets/*`, plain prefixes/suffixes).
pub(super) fn simple_glob_match(pattern: &str, path: &str) -> bool {
    // Handle ** patterns (match any path segment)
    if pattern.contains("**") {
        let parts: Vec<&str> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let suffix = parts[1];

            // If pattern is like "**/*.env", check if path ends with suffix pattern
            if prefix.is_empty() && suffix.starts_with('/') {
                let suffix_pattern = &suffix[1..];
                return simple_glob_match(suffix_pattern, path)
                    || path
                        .split('/')
                        .any(|segment| simple_glob_match(suffix_pattern, segment));
            }

            // Otherwise: path starts with prefix and ends with suffix
            let matches_prefix = prefix.is_empty() || path.starts_with(prefix);
            let matches_suffix = suffix.is_empty() || simple_glob_match(suffix, path);
            return matches_prefix && matches_suffix;
        }
    }

    // Simple `*` matching (matches any characters except `/`)
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 2 {
            let prefix = parts[0];
            let suffix = parts[1];
            return path.starts_with(prefix) && path.ends_with(suffix);
        }
    }

    pattern == path
}

/// Result of applying policy constraints to a tool call.
#[derive(Debug, Clone)]
pub enum PolicyConstraintResult {
    /// Constraints passed, tool can execute.
    Allowed,
    /// A constraint was violated.
    Violated(String),
    /// Arguments were modified to comply with constraints.
    Modified(serde_json::Value, String),
}
