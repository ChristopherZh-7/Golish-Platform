//! Loop detection and protection for the AI agent system.
//!
//! This module provides mechanisms to detect and prevent infinite loops
//! and runaway agent behavior by tracking:
//! - Total turn count per request
//! - Tool calls per turn (inner loops)
//! - Repeated identical tool calls with the same arguments

mod monitor;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use monitor::{ExecutionMonitor, ExecutionMonitorMode};

/// Configuration for loop protection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopProtectionConfig {
    /// Maximum number of tool call iterations per turn.
    #[serde(default = "LoopProtectionConfig::default_max_tool_loops")]
    pub max_tool_loops: usize,

    /// Maximum number of times the same tool can be called with identical
    /// arguments within a single turn.
    #[serde(default = "LoopProtectionConfig::default_max_repeated_tool_calls")]
    pub max_repeated_tool_calls: usize,

    /// Threshold at which to warn the user about potential loops.
    #[serde(default = "LoopProtectionConfig::default_warning_threshold")]
    pub warning_threshold: f64,

    /// Whether loop detection is enabled.
    #[serde(default = "LoopProtectionConfig::default_enabled")]
    pub enabled: bool,
}

impl LoopProtectionConfig {
    pub const DEFAULT_MAX_TOOL_LOOPS: usize = 100;
    pub const DEFAULT_MAX_REPEATED_CALLS: usize = 5;
    pub const DEFAULT_WARNING_THRESHOLD: f64 = 0.6;

    const fn default_max_tool_loops() -> usize {
        Self::DEFAULT_MAX_TOOL_LOOPS
    }

    const fn default_max_repeated_tool_calls() -> usize {
        Self::DEFAULT_MAX_REPEATED_CALLS
    }

    const fn default_warning_threshold() -> f64 {
        Self::DEFAULT_WARNING_THRESHOLD
    }

    const fn default_enabled() -> bool {
        true
    }
}

impl Default for LoopProtectionConfig {
    fn default() -> Self {
        Self {
            max_tool_loops: Self::DEFAULT_MAX_TOOL_LOOPS,
            max_repeated_tool_calls: Self::DEFAULT_MAX_REPEATED_CALLS,
            warning_threshold: Self::DEFAULT_WARNING_THRESHOLD,
            enabled: true,
        }
    }
}

/// Response from loop detection check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoopDetectionResult {
    Allowed,

    Warning {
        tool_name: String,
        current_count: usize,
        max_count: usize,
        message: String,
    },

    Blocked {
        tool_name: String,
        repeat_count: usize,
        max_count: usize,
        message: String,
    },

    MaxIterationsReached {
        iterations: usize,
        max_iterations: usize,
        message: String,
    },
}

impl LoopDetectionResult {
    #[cfg(test)]
    pub fn is_allowed(&self) -> bool {
        matches!(
            self,
            LoopDetectionResult::Allowed | LoopDetectionResult::Warning { .. }
        )
    }

    #[cfg(test)]
    pub fn is_blocked(&self) -> bool {
        matches!(
            self,
            LoopDetectionResult::Blocked { .. } | LoopDetectionResult::MaxIterationsReached { .. }
        )
    }
}

/// Creates a signature for a tool call based on name and arguments.
fn make_signature(tool_name: &str, args: &serde_json::Value) -> String {
    let args_str = serde_json::to_string(args).unwrap_or_default();
    format!("{}:{}", tool_name, args_str)
}

/// Detects potential model loops from repetitive tool calls.
#[derive(Debug)]
pub struct LoopDetector {
    repeated_calls: HashMap<String, usize>,
    iteration_count: usize,
    config: LoopProtectionConfig,
    disabled_for_session: bool,
}

impl LoopDetector {
    pub fn new(config: LoopProtectionConfig) -> Self {
        Self {
            repeated_calls: HashMap::new(),
            iteration_count: 0,
            config,
            disabled_for_session: false,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(LoopProtectionConfig::default())
    }

    pub fn record_tool_call(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> LoopDetectionResult {
        if self.disabled_for_session || !self.config.enabled {
            self.iteration_count += 1;
            return LoopDetectionResult::Allowed;
        }

        self.iteration_count += 1;
        if self.iteration_count > self.config.max_tool_loops {
            return LoopDetectionResult::MaxIterationsReached {
                iterations: self.iteration_count,
                max_iterations: self.config.max_tool_loops,
                message: format!(
                    "Maximum tool call limit ({}) reached for this turn. \
                     The agent may be stuck in a loop. Consider adjusting \
                     loop_protection.max_tool_loops in settings if more iterations are needed.",
                    self.config.max_tool_loops
                ),
            };
        }

        let signature = make_signature(tool_name, args);
        let count = self.repeated_calls.entry(signature).or_insert(0);
        *count += 1;

        let max = self.config.max_repeated_tool_calls;
        let warning_at = (max as f64 * self.config.warning_threshold).ceil() as usize;

        if *count > max {
            LoopDetectionResult::Blocked {
                tool_name: tool_name.to_string(),
                repeat_count: *count,
                max_count: max,
                message: format!(
                    "Tool '{}' has been called {} times with identical arguments. \
                     This appears to be a loop. Consider adjusting \
                     loop_protection.max_repeated_tool_calls in settings if this is intentional.",
                    tool_name, count
                ),
            }
        } else if *count >= warning_at {
            LoopDetectionResult::Warning {
                tool_name: tool_name.to_string(),
                current_count: *count,
                max_count: max,
                message: format!(
                    "Tool '{}' has been called {} times with identical arguments. \
                     {} more calls will trigger loop protection.",
                    tool_name,
                    count,
                    max - *count
                ),
            }
        } else {
            LoopDetectionResult::Allowed
        }
    }

    #[cfg(test)]
    pub fn iteration_count(&self) -> usize {
        self.iteration_count
    }

    pub fn reset(&mut self) {
        self.repeated_calls.clear();
        self.iteration_count = 0;
    }

    pub fn disable_for_session(&mut self) {
        self.disabled_for_session = true;
    }

    pub fn enable(&mut self) {
        self.disabled_for_session = false;
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled && !self.disabled_for_session
    }

    pub fn config(&self) -> &LoopProtectionConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: LoopProtectionConfig) {
        self.config = config;
    }

    pub fn stats(&self) -> LoopDetectorStats {
        let most_repeated = self
            .repeated_calls
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(sig, count)| {
                let tool_name = sig.split(':').next().unwrap_or("unknown").to_string();
                (tool_name, *count)
            });

        let (most_repeated_tool, most_repeated_count) = match most_repeated {
            Some((name, count)) => (Some(name), count),
            None => (None, 0),
        };

        LoopDetectorStats {
            iteration_count: self.iteration_count,
            max_iterations: self.config.max_tool_loops,
            unique_signatures: self.repeated_calls.len(),
            most_repeated_tool,
            most_repeated_count,
            is_enabled: self.is_enabled(),
        }
    }
}

/// Statistics about the current loop detection state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopDetectorStats {
    pub iteration_count: usize,
    pub max_iterations: usize,
    pub unique_signatures: usize,
    pub most_repeated_tool: Option<String>,
    pub most_repeated_count: usize,
    pub is_enabled: bool,
}
