//! Tool-use capability metadata for model/provider families.
//!
//! This is descriptive metadata, not executable policy. Runtime code can use
//! it to decide whether native tool calls are trusted, whether textual recovery
//! is allowed, or whether tools should be disabled for a model family.

use serde::{Deserialize, Serialize};

/// How a model/provider family is expected to express tool calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallMode {
    NativeStrict,
    NativeBestEffort,
    TextualXmlFallback,
    TextualJsonFallback,
    Disabled,
}

/// Operational reliability classification for tool use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallReliability {
    Reliable,
    NeedsAdapter,
    ChatOnly,
}

/// Explicit profile for model/provider tool-use behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUseProfile {
    pub mode: ToolCallMode,
    pub reliability: ToolCallReliability,
    pub supports_required_tool_choice: bool,
    pub supports_parallel_tool_calls: bool,
    pub max_tool_calls_per_turn: usize,
    pub requires_tool_result_balance: bool,
}

impl Default for ToolUseProfile {
    fn default() -> Self {
        Self::disabled()
    }
}

impl ToolUseProfile {
    pub const fn native_reliable() -> Self {
        Self {
            mode: ToolCallMode::NativeStrict,
            reliability: ToolCallReliability::Reliable,
            supports_required_tool_choice: true,
            supports_parallel_tool_calls: true,
            max_tool_calls_per_turn: 8,
            requires_tool_result_balance: true,
        }
    }

    pub const fn native_best_effort() -> Self {
        Self {
            mode: ToolCallMode::NativeBestEffort,
            reliability: ToolCallReliability::NeedsAdapter,
            supports_required_tool_choice: false,
            supports_parallel_tool_calls: false,
            max_tool_calls_per_turn: 1,
            requires_tool_result_balance: true,
        }
    }

    pub const fn needs_textual_xml_adapter() -> Self {
        Self {
            mode: ToolCallMode::TextualXmlFallback,
            reliability: ToolCallReliability::NeedsAdapter,
            supports_required_tool_choice: false,
            supports_parallel_tool_calls: false,
            max_tool_calls_per_turn: 1,
            requires_tool_result_balance: true,
        }
    }

    pub const fn disabled() -> Self {
        Self {
            mode: ToolCallMode::Disabled,
            reliability: ToolCallReliability::ChatOnly,
            supports_required_tool_choice: false,
            supports_parallel_tool_calls: false,
            max_tool_calls_per_turn: 0,
            requires_tool_result_balance: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_use_profile_defaults_to_disabled() {
        let profile = ToolUseProfile::default();
        assert_eq!(profile.mode, ToolCallMode::Disabled);
        assert_eq!(profile.reliability, ToolCallReliability::ChatOnly);
        assert_eq!(profile.max_tool_calls_per_turn, 0);
    }

    #[test]
    fn native_reliable_supports_strict_tool_use() {
        let profile = ToolUseProfile::native_reliable();
        assert_eq!(profile.mode, ToolCallMode::NativeStrict);
        assert_eq!(profile.reliability, ToolCallReliability::Reliable);
        assert!(profile.supports_required_tool_choice);
        assert!(profile.supports_parallel_tool_calls);
        assert!(profile.requires_tool_result_balance);
    }

    #[test]
    fn textual_xml_adapter_is_limited_to_one_call() {
        let profile = ToolUseProfile::needs_textual_xml_adapter();
        assert_eq!(profile.mode, ToolCallMode::TextualXmlFallback);
        assert_eq!(profile.reliability, ToolCallReliability::NeedsAdapter);
        assert!(!profile.supports_required_tool_choice);
        assert!(!profile.supports_parallel_tool_calls);
        assert_eq!(profile.max_tool_calls_per_turn, 1);
    }
}
