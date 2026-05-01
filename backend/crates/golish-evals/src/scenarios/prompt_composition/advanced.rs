//! Advanced prompt composition scenarios and test helpers.

use async_trait::async_trait;

use crate::config::EvalProvider;
use crate::metrics::{LlmJudgeMetric, Metric};
use crate::outcome::EvalReport;
use crate::runner::EvalRunner;
use crate::scenarios::Scenario;

/// Tests that behavior differs when instructions are present vs absent.
///
/// This scenario uses the DEFAULT eval prompt (no custom instructions) and
/// should produce different behavior than BrevityInstructionScenario.
pub struct NoInstructionsBaselineScenario;

#[async_trait]
impl Scenario for NoInstructionsBaselineScenario {
    fn name(&self) -> &str {
        "prompt-no-instructions-baseline"
    }

    fn description(&self) -> &str {
        "Baseline: same task as brevity scenario but with default prompt"
    }

    fn testbed(&self) -> &str {
        "rust-prompt-test"
    }

    fn prompt(&self) -> &str {
        // Same prompt as BrevityInstructionScenario
        "What does the greet function in src/lib.rs do?"
    }

    // No custom system_prompt - uses default

    fn metrics(&self) -> Vec<Box<dyn Metric>> {
        vec![
            Box::new(LlmJudgeMetric::new(
                "response_is_typical_length",
                "The response should be a typical AI response - likely longer and more \
                 explanatory than a heavily constrained response. It may include context, \
                 explanation of the code structure, etc.",
                0.7,
            )),
            Box::new(LlmJudgeMetric::new(
                "task_completed",
                "The agent should successfully explain what the greet function does.",
                0.7,
            )),
        ]
    }
}

// =============================================================================
// Scenario 6: Sub-Agent Awareness
// =============================================================================

/// Tests that the agent acknowledges sub-agent capabilities when documented.
///
/// This scenario includes sub-agent documentation and asks a question that
/// could benefit from delegation, verifying the agent is aware of the option.
pub struct SubAgentAwarenessScenario;

const SUB_AGENT_AWARE_SYSTEM_PROMPT: &str = r#"You are a coding assistant being evaluated.

## Available Sub-Agents

You can delegate tasks to specialized sub-agents:

### sub_agent_analyzer
**Analyzer**: Deep semantic analysis of code structure, patterns, and dependencies.
Available tools: read_file, grep_file, indexer tools

### sub_agent_coder
**Coder**: Implements code changes based on specifications.
Available tools: read_file, write_file, edit_file

When a task would benefit from specialized analysis or implementation,
mention which sub-agent would be appropriate (even if you handle it directly).

You have access to: read_file, write_file, edit_file, grep_file, run_pty_cmd.
"#;

#[async_trait]
impl Scenario for SubAgentAwarenessScenario {
    fn name(&self) -> &str {
        "prompt-sub-agent-awareness"
    }

    fn description(&self) -> &str {
        "Tests that agent acknowledges sub-agents when documented in prompt"
    }

    fn testbed(&self) -> &str {
        "rust-prompt-test"
    }

    fn prompt(&self) -> &str {
        "I need to understand how the greet function works and then add a new function \
         that greets multiple people. Based on the capabilities described in your system prompt, \
         how would you approach this task? What sub-agents or specialized tools could help?"
    }

    fn system_prompt(&self) -> Option<&str> {
        Some(SUB_AGENT_AWARE_SYSTEM_PROMPT)
    }

    fn metrics(&self) -> Vec<Box<dyn Metric>> {
        vec![
            Box::new(LlmJudgeMetric::new(
                "mentions_sub_agents",
                "The agent should mention or reference sub-agents (analyzer, coder) \
                 as options for the task, even if it handles it directly. Look for mentions of \
                 'sub_agent', 'analyzer', 'coder', or 'delegate'.",
                0.7,
            )),
            // Note: We only test that sub-agents are mentioned, not the exact mapping.
            // The core test is whether the prompt composition system successfully
            // delivers sub-agent information to the agent's context.
        ]
    }
}

// =============================================================================
// Scenario 7: Provider Context Awareness
// =============================================================================

/// Tests that the agent uses provider-specific context from the prompt.
///
/// This scenario includes provider information and asks about capabilities.
pub struct ProviderContextScenario;

const PROVIDER_CONTEXT_SYSTEM_PROMPT: &str = r#"You are a coding assistant being evaluated.

## Environment
- Provider: Anthropic Claude
- Model: claude-sonnet-4
- Workspace: /test/project

## Provider-Specific Features
- Web search is available via the web_search tool
- Extended thinking is enabled for complex reasoning
- This model excels at code analysis and generation

When asked about your capabilities, reference these provider-specific features.

You have access to: read_file, write_file, edit_file, grep_file, run_pty_cmd, web_search.
"#;

#[async_trait]
impl Scenario for ProviderContextScenario {
    fn name(&self) -> &str {
        "prompt-provider-context"
    }

    fn description(&self) -> &str {
        "Tests that agent uses provider context from the prompt"
    }

    fn testbed(&self) -> &str {
        "rust-prompt-test"
    }

    fn prompt(&self) -> &str {
        "According to your system prompt, what tools and provider-specific capabilities \
         do you have available? Please list them, including any special features like \
         web search or extended thinking. I want to understand a codebase and then make changes."
    }

    fn system_prompt(&self) -> Option<&str> {
        Some(PROVIDER_CONTEXT_SYSTEM_PROMPT)
    }

    fn metrics(&self) -> Vec<Box<dyn Metric>> {
        vec![
            Box::new(LlmJudgeMetric::new(
                "mentions_web_search",
                "The agent should mention web_search as an available capability.",
                0.7,
            )),
            Box::new(LlmJudgeMetric::new(
                "mentions_provider_features",
                "The agent should reference provider-specific features like extended thinking \
                 or code analysis capabilities mentioned in the prompt.",
                0.6,
            )),
        ]
    }
}

// =============================================================================
// Scenario 8: Instruction Specificity
// =============================================================================

/// Tests that specific instructions override general behavior.
///
/// This scenario provides very specific file naming conventions and verifies
/// the agent follows them exactly.
pub struct SpecificInstructionsScenario;

const SPECIFIC_INSTRUCTIONS_SYSTEM_PROMPT: &str = r#"You are a coding assistant being evaluated.

## MANDATORY FILE NAMING CONVENTION
When creating new files, you MUST follow this EXACT pattern:
- All new Rust files MUST be named with the prefix "golish_"
- Example: golish_helpers.rs, golish_utils.rs, golish_config.rs
- This is a hard requirement - files without this prefix will be rejected

You have access to: read_file, write_file, create_file, edit_file, list_files, run_pty_cmd.
"#;

#[async_trait]
impl Scenario for SpecificInstructionsScenario {
    fn name(&self) -> &str {
        "prompt-specific-instructions"
    }

    fn description(&self) -> &str {
        "Tests that specific naming instructions are followed exactly"
    }

    fn testbed(&self) -> &str {
        "rust-prompt-test"
    }

    fn prompt(&self) -> &str {
        "Create a new Rust file with helper functions for string manipulation. \
         Add a function to reverse a string and another to count vowels."
    }

    fn system_prompt(&self) -> Option<&str> {
        Some(SPECIFIC_INSTRUCTIONS_SYSTEM_PROMPT)
    }

    fn metrics(&self) -> Vec<Box<dyn Metric>> {
        vec![
            Box::new(
                LlmJudgeMetric::new(
                    "follows_naming_convention",
                    "Any new file created should follow the golish_ prefix convention. \
                     Use list_files to check the src/ directory and verify a file like \
                     'golish_helpers.rs' or 'golish_string.rs' was created (not 'helpers.rs').",
                    0.8,
                )
                .with_tools(),
            ),
            Box::new(
                LlmJudgeMetric::new(
                    "creates_requested_functions",
                    "The agent should create the requested functions (reverse string, count vowels). \
                     Use read_file to check the actual file content.",
                    0.7,
                )
                .with_tools(),
            ),
        ]
    }
}

// =============================================================================
// Testbed Files
// =============================================================================

/// Testbed files for prompt composition scenarios.
pub fn testbed_files() -> Vec<(String, String)> {
    vec![
        (
            "Cargo.toml".to_string(),
            r#"[package]
name = "prompt-test"
version = "0.1.0"
edition = "2021"

[dependencies]
"#
            .to_string(),
        ),
        (
            "src/lib.rs".to_string(),
            r#"/// A simple greeting module.

/// Returns a greeting for the given name.
pub fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greet() {
        assert_eq!(greet("World"), "Hello, World!");
    }

    #[test]
    fn test_greet_name() {
        assert_eq!(greet("Alice"), "Hello, Alice!");
    }
}
"#
            .to_string(),
        ),
    ]
}
