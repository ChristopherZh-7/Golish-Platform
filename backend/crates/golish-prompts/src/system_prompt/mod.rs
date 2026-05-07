//! System prompt building for the Golish agent.
//!
//! This module is a **dispatcher** that picks the right prompt template
//! based on the runtime mode:
//!
//! - **OpenAI providers** → [`crate::codex_prompt::build_codex_style_prompt`]
//!   (concise reasoning-model-friendly prompt; not affected by chat/task split)
//! - **Task mode** (`has_sub_agents == true`) → [`task::build_task_prompt`]
//!   — the multi-agent orchestration prompt with the full team-delegation
//!   section, specialist routing tables, and `adviser` sub-agent guidance
//! - **Chat mode** (`has_sub_agents == false`) → [`chat::build_chat_prompt`]
//!   — the single-agent prompt with `<single_agent_mode>` block, no
//!   `sub_agent_*` references anywhere
//!
//! Public surface stays the same; chat/task split is opaque to callers.
//!
//! Shared helpers (rules discovery, agent-mode instructions, project
//! memory file reading) live here and are re-used by both child templates.

use std::path::Path;

mod chat;
mod instructions;
mod task;
mod team_delegation;

#[cfg(test)]
mod tests;

pub use instructions::{get_agent_mode_instructions, read_project_instructions};

use golish_core::PromptContext;

use crate::codex_prompt::build_codex_style_prompt;
use crate::prompt_registry::PromptContributorRegistry;
use golish_core::AgentMode;

/// Build the system prompt for the agent.
///
/// This is a convenience wrapper that calls `build_system_prompt_with_contributions`
/// without any contributors. Use this for backward compatibility or when dynamic
/// contributions are not needed.
///
/// # Arguments
/// * `workspace_path` - The current workspace directory
/// * `agent_mode` - The current agent mode (affects available operations)
/// * `memory_file_path` - Optional path to a memory file (from codebase settings)
///
/// # Returns
/// The complete system prompt string
pub fn build_system_prompt(
    workspace_path: &Path,
    agent_mode: AgentMode,
    memory_file_path: Option<&Path>,
) -> String {
    build_system_prompt_with_contributions(workspace_path, agent_mode, memory_file_path, None, None)
}

/// Build the system prompt with optional context.
///
/// Dispatches to the right template based on the prompt context:
///
/// 1. If `context.provider` is an OpenAI variant → Codex-style prompt
/// 2. Else if `context.has_sub_agents == true` → task-mode (multi-agent) prompt
/// 3. Else (no context **or** `has_sub_agents == false`) → chat-mode
///    (single-agent) prompt
///
/// **Backward compatibility note**: when `context` is `None`, the original
/// implementation defaulted to "agents enabled". We keep that behavior for
/// existing callers (legacy tests + any direct caller of
/// [`build_system_prompt`]) by treating no context as task mode.
///
/// # Arguments
/// * `workspace_path` - The current workspace directory
/// * `agent_mode` - The current agent mode (affects available operations)
/// * `memory_file_path` - Optional path to a memory file (from codebase settings)
/// * `_registry` - Unused, kept for API compatibility
/// * `context` - Optional prompt context containing provider/model info
///
/// # Returns
/// The complete system prompt string
pub fn build_system_prompt_with_contributions(
    workspace_path: &Path,
    agent_mode: AgentMode,
    memory_file_path: Option<&Path>,
    _registry: Option<&PromptContributorRegistry>,
    context: Option<&PromptContext>,
) -> String {
    if let Some(ctx) = context {
        if is_openai_provider(&ctx.provider) {
            return build_codex_style_prompt(workspace_path, agent_mode, memory_file_path);
        }
    }

    let use_agents = context.is_none_or(|ctx| ctx.has_sub_agents);

    if use_agents {
        task::build_task_prompt(workspace_path, agent_mode, memory_file_path)
    } else {
        chat::build_chat_prompt(workspace_path, agent_mode, memory_file_path)
    }
}

/// Discover and concatenate `alwaysApply: true` rule files from
/// `~/.golish/rules/` and `<workspace>/.golish/rules/`.
///
/// Each rule file must start with a YAML frontmatter block fenced by `---`
/// lines that contains `alwaysApply: true`; the body after the frontmatter is
/// emitted as `<rule name="...">...</rule>` and concatenated.
///
/// Returns an empty string if no rules are present or all rules opt out of
/// `alwaysApply`.
pub(super) fn build_rules_section(workspace_path: &Path) -> String {
    let rules_dir_global = dirs::home_dir().map(|h| h.join(".golish").join("rules"));
    let rules_dir_local = workspace_path.join(".golish").join("rules");
    let mut rules_text = String::new();

    for dir in [rules_dir_global, Some(rules_dir_local)]
        .into_iter()
        .flatten()
    {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let trimmed = content.trim_start();
                    if !trimmed.starts_with("---") {
                        continue;
                    }
                    let after = &trimmed[3..];
                    if let Some(end) = after.find("\n---") {
                        let yaml = &after[..end];
                        if yaml.contains("alwaysApply: true") {
                            let body = after[end + 4..].trim();
                            if !body.is_empty() {
                                let name =
                                    path.file_stem().and_then(|s| s.to_str()).unwrap_or("rule");
                                rules_text.push_str(&format!(
                                    "\n<rule name=\"{name}\">\n{body}\n</rule>\n"
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    rules_text
}

/// Check if the provider is an OpenAI provider.
///
/// OpenAI providers use the Codex-style system prompt which is more concise
/// and uses less structured formatting.
pub(super) fn is_openai_provider(provider: &str) -> bool {
    matches!(provider, "openai" | "openai_responses" | "openai_reasoning")
}
