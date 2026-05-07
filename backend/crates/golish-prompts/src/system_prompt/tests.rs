//! System prompt tests.

use super::*;
use golish_core::{AgentMode, PromptContext};
use std::path::{Path, PathBuf};

#[test]
fn test_build_system_prompt_contains_required_sections() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let prompt = build_system_prompt(&workspace, AgentMode::Default, None);

    assert!(prompt.contains("# Tone and style"));
    assert!(prompt.contains("# Tool Reference"));
    assert!(prompt.contains("## TEAM COLLABORATION & DELEGATION"));
    assert!(prompt.contains("# Security Boundaries"));
    assert!(prompt.contains("# Before Claiming Completion"));
    assert!(prompt.contains("## Project Instructions"));
    assert!(prompt.contains("## AUTHORIZATION FRAMEWORK"));
    assert!(prompt.contains("## SENIOR MENTOR SUPERVISION"));
    assert!(prompt.contains("## SUMMARIZATION AWARENESS"));
}

#[test]
fn test_build_system_prompt_planning_mode() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let prompt = build_system_prompt(&workspace, AgentMode::Planning, None);

    assert!(prompt.contains("<planning_mode>"));
    assert!(prompt.contains("Planning Mode Active"));
    assert!(prompt.contains("READ-ONLY mode"));
    assert!(prompt.contains("**Forbidden**"));
}

#[test]
fn test_build_system_prompt_auto_approve_mode() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let prompt = build_system_prompt(&workspace, AgentMode::AutoApprove, None);

    assert!(prompt.contains("<autoapprove_mode>"));
    assert!(prompt.contains("AutoApprove Mode Active"));
}

#[test]
fn test_read_project_instructions_returns_empty_when_no_memory_file() {
    let workspace = PathBuf::from("/nonexistent/path");
    let instructions = read_project_instructions(&workspace, None);

    assert!(instructions.is_empty());
}

#[test]
fn test_read_project_instructions_returns_error_for_missing_configured_file() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let memory_file = PathBuf::from("NONEXISTENT.md");
    let instructions = read_project_instructions(&workspace, Some(&memory_file));

    assert!(instructions.contains("not found"));
    assert!(instructions.contains("NONEXISTENT.md"));
}

#[test]
fn test_read_project_instructions_reads_configured_file() {
    // Create a temp directory with a memory file
    let temp_dir = std::env::temp_dir().join("golish_test_memory_file");
    std::fs::create_dir_all(&temp_dir).unwrap();
    let memory_file_path = temp_dir.join("TEST_MEMORY.md");
    std::fs::write(&memory_file_path, "Test project instructions content").unwrap();

    let instructions = read_project_instructions(&temp_dir, Some(Path::new("TEST_MEMORY.md")));

    assert_eq!(instructions, "Test project instructions content");

    // Cleanup
    std::fs::remove_dir_all(&temp_dir).ok();
}

#[test]
fn test_prompt_with_contributions_same_as_base() {
    // Since we no longer append contributions, both functions should return the same result
    let workspace = PathBuf::from("/tmp/test");

    let base_prompt = build_system_prompt(&workspace, AgentMode::Default, None);
    let composed_prompt =
        build_system_prompt_with_contributions(&workspace, AgentMode::Default, None, None, None);

    assert_eq!(
        base_prompt, composed_prompt,
        "Both functions should return identical prompts"
    );
}

#[test]
fn test_use_agents_true_includes_delegation() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4").with_sub_agents(true);

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    assert!(prompt.contains("## TEAM COLLABORATION & DELEGATION"));
    assert!(prompt.contains("<team_specialists>"));
    assert!(prompt.contains("sub_agent_pentester"));
    assert!(prompt.contains("<delegation_rules>"));
}

#[test]
fn test_use_agents_false_excludes_delegation() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4").with_sub_agents(false);

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    assert!(!prompt.contains("## TEAM COLLABORATION & DELEGATION"));
    assert!(!prompt.contains("<team_specialists>"));
    assert!(!prompt.contains("sub_agent_pentester"));
    // Core sections should still be present
    assert!(prompt.contains("# Tone and style"));
    assert!(prompt.contains("## AUTHORIZATION FRAMEWORK"));
    assert!(prompt.contains("## Pentest Bridge Tools"));
}

#[test]
fn test_use_agents_false_includes_single_agent_note() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4").with_sub_agents(false);

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    assert!(prompt.contains("## SINGLE-AGENT MODE"));
    assert!(prompt.contains("<single_agent_mode>"));
    assert!(prompt.contains("There are NO `sub_agent_*` tools"));
}

#[test]
fn test_use_agents_false_strips_delegate_phrasing_from_workflow() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4").with_sub_agents(false);

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    // The Memory-Aware Workflow's step 3 must NOT advertise delegation when
    // sub-agents are off — otherwise the LLM is told to "Delegate to specialists"
    // while no `sub_agent_*` tool exists.
    assert!(!prompt.contains("Delegate to appropriate specialist(s) or handle directly"));
    assert!(prompt.contains("Handle the task yourself with the available file"));
}

#[test]
fn test_use_agents_false_strips_adviser_request_line() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4").with_sub_agents(false);

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    // "request mentor advice using the `adviser` sub-agent" should disappear
    // when sub-agents are off; otherwise the LLM tries to call a non-existent
    // `sub_agent_adviser`.
    assert!(!prompt.contains("request mentor advice using the `adviser` sub-agent"));
    assert!(prompt.contains("cannot request it explicitly in single-agent mode"));
}

#[test]
fn test_use_agents_true_keeps_delegate_phrasing() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4").with_sub_agents(true);

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    // Task-mode prompt keeps the delegation language and adviser sub-agent
    // line — task mode is the multi-agent orchestration path.
    assert!(prompt.contains("Delegate to appropriate specialist(s) or handle directly"));
    assert!(prompt.contains("request mentor advice using the `adviser` sub-agent"));
    assert!(!prompt.contains("## SINGLE-AGENT MODE"));
}

#[test]
fn test_no_context_defaults_to_agents_enabled() {
    let workspace = PathBuf::from("/tmp/test-workspace");

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        None, // No context -> defaults to use_agents=true
    );

    assert!(prompt.contains("## TEAM COLLABORATION & DELEGATION"));
}

#[test]
fn test_pipeline_not_forced() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let prompt = build_system_prompt(&workspace, AgentMode::Default, None);

    // Old behavior: "ALWAYS prefer run_pipeline" should NOT be present
    assert!(!prompt.contains("ALWAYS prefer `run_pipeline`"));
    // New: pipeline is available but not forced
    assert!(prompt.contains("run_pipeline"));
    assert!(prompt.contains("Use when the user explicitly requests"));
}

#[test]
fn test_is_openai_provider() {
    assert!(is_openai_provider("openai"));
    assert!(is_openai_provider("openai_responses"));
    assert!(is_openai_provider("openai_reasoning"));
    assert!(!is_openai_provider("anthropic"));
    assert!(!is_openai_provider("vertex_ai"));
    assert!(!is_openai_provider("gemini"));
    assert!(!is_openai_provider(""));
}

#[test]
fn test_openai_provider_uses_codex_prompt() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("openai", "gpt-4o");

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    // Codex prompt uses "Core Principles" instead of "Tone and style"
    assert!(prompt.contains("Core Principles"));
    assert!(!prompt.contains("# Tone and style"));
}

#[test]
fn test_openai_responses_provider_uses_codex_prompt() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("openai_responses", "o3-mini");

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    // Codex prompt uses "Core Principles" instead of "Tone and style"
    assert!(prompt.contains("Core Principles"));
    assert!(!prompt.contains("# Tone and style"));
}

#[test]
fn test_openai_reasoning_provider_uses_codex_prompt() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("openai_reasoning", "o1");

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    // Codex prompt uses "Core Principles" instead of "Tone and style"
    assert!(prompt.contains("Core Principles"));
    assert!(!prompt.contains("# Tone and style"));
}

#[test]
fn test_anthropic_provider_uses_default_prompt() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4-20250514");

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    // Default prompt uses "Tone and style"
    assert!(prompt.contains("# Tone and style"));
    assert!(!prompt.contains("Core Principles"));
}

#[test]
fn test_no_context_uses_default_prompt() {
    let workspace = PathBuf::from("/tmp/test-workspace");

    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        None, // No context
    );

    // Default prompt uses "Tone and style"
    assert!(prompt.contains("# Tone and style"));
}

// ─────────────────────────────────────────────────────────────────────────
// Chat ↔ Task template ISOLATION tests
//
// After Batch 2 split, the chat prompt is built from `chat.rs` and the task
// prompt from `task.rs`. These tests guard against leakage in both
// directions:
//   - chat template must contain ZERO `sub_agent_*` references
//   - task template must contain the full `<team_specialists>` block
// They are stricter than the older `use_agents_*_excludes/includes_delegation`
// pair because they assert on raw substring presence regardless of section
// title — protecting against future drifts where a `sub_agent_xxx` line
// sneaks back into chat.rs (e.g. via copy-paste from task.rs).
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn test_chat_template_contains_no_sub_agent_references() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4").with_sub_agents(false);
    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    // No CONCRETE specialist tool name should appear anywhere in chat-mode
    // prompt. We deliberately list each one (rather than match `sub_agent_`
    // as a prefix) because the `<single_agent_mode>` block legitimately
    // mentions the WILDCARD `sub_agent_*` to tell the model "these tools
    // do not exist" — that reverse-declaration is intentional and must
    // stay; it's only the specific names that would trick the model into
    // hallucinating delegation calls.
    for tool in [
        "sub_agent_explorer",
        "sub_agent_analyzer",
        "sub_agent_researcher",
        "sub_agent_pentester",
        "sub_agent_memorist",
        "sub_agent_adviser",
        "sub_agent_planner",
        "sub_agent_reflector",
        "sub_agent_reporter",
        "sub_agent_installer",
        "sub_agent_worker",
    ] {
        assert!(
            !prompt.contains(tool),
            "chat-mode prompt must not reference {tool}, but it does"
        );
    }

    // Specific specialist role names from team_delegation should also be gone.
    assert!(!prompt.contains("<team_specialists>"));
    assert!(!prompt.contains("<delegation_rules>"));
    assert!(!prompt.contains("Concurrent Sub-Agents"));
    assert!(!prompt.contains("Security-Specific Routing"));
}

#[test]
fn test_task_template_contains_full_team_specialists_block() {
    let workspace = PathBuf::from("/tmp/test-workspace");
    let context = PromptContext::new("anthropic", "claude-sonnet-4").with_sub_agents(true);
    let prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        Some(&context),
    );

    assert!(prompt.contains("<team_specialists>"));
    assert!(prompt.contains("</team_specialists>"));
    assert!(prompt.contains("<delegation_rules>"));

    // Every specialist role declared in team_delegation.rs must be present.
    for tool in [
        "sub_agent_explorer",
        "sub_agent_analyzer",
        "sub_agent_researcher",
        "sub_agent_pentester",
        "sub_agent_memorist",
        "sub_agent_adviser",
        "sub_agent_planner",
        "sub_agent_reflector",
        "sub_agent_reporter",
        "sub_agent_installer",
        "sub_agent_worker",
    ] {
        assert!(
            prompt.contains(tool),
            "task-mode prompt should advertise {tool}"
        );
    }

    // No SINGLE-AGENT MODE block in task mode.
    assert!(!prompt.contains("## SINGLE-AGENT MODE"));
    assert!(!prompt.contains("<single_agent_mode>"));
}
