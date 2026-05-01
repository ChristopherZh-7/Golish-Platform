//! Tool Policy System for AI Agent.
//!
//! This module provides policy-based access control for AI tool execution:
//! - [`ToolPolicy`] enum: allow/prompt/deny policies.
//! - [`ToolPolicyConfig`]: configuration loaded from `.golish/tool-policy.json`.
//! - [`ToolConstraints`]: per-tool execution limits.
//! - [`ToolPolicyManager`]: manages policy loading, saving, and evaluation.
//!
//! Based on the VTCode implementation pattern.
//!
//! # Submodules
//!
//! - [`types`]: core types — `ToolPolicy`, `ToolConstraints`, the simple glob
//!   matcher, and `PolicyConstraintResult`.
//! - [`defaults`]: the default tool catalog (typed + dynamic), `is_known_tool`
//!   helpers, and `ToolPolicyConfig` + its `Default` impl.
//! - [`manager`]: [`ToolPolicyManager`] — runtime evaluation, two-tier
//!   global/project config, persistence, pre-approval, and full-auto state.

// Public API for future tool policy integration
#![allow(dead_code)]

mod defaults;
mod manager;
mod types;

#[cfg(test)]
mod tests;

pub use defaults::{
    get_known_tool, get_typed_allow_tools, get_typed_deny_tools, get_typed_prompt_tools,
    is_known_tool, ToolPolicyConfig, ALLOW_TOOLS,
};
pub use manager::ToolPolicyManager;
pub use types::{PolicyConstraintResult, ToolConstraints, ToolPolicy};
