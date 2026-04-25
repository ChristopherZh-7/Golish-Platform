//! System prompt tests.

use super::*;
use std::path::PathBuf;

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
    let composed_prompt = build_system_prompt_with_contributions(
        &workspace,
        AgentMode::Default,
        None,
        None,
        None,
    );

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
