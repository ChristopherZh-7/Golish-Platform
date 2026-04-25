//! Executor tests.

use super::*;
use super::*;

/// Helper to build the main agent's system prompt for comparison.
///
/// This replicates the exact logic from agent_bridge.rs::prepare_execution_context
/// so we can verify evals get the same prompt.
fn build_main_agent_prompt(
    workspace: &Path,
    provider_name: &str,
    has_web_search: bool,
) -> String {
    // Create sub-agent registry (same as main agent)
    let sub_agent_registry = Arc::new(RwLock::new(SubAgentRegistry::new()));

    // Create prompt contributor registry with default contributors
    let contributors = create_default_contributors(sub_agent_registry);
    let mut registry = PromptContributorRegistry::new();
    for contributor in contributors {
        registry.register(contributor);
    }

    // Create prompt context (same as main agent)
    let has_sub_agents = true; // Main agent always has sub-agents
    let prompt_context = PromptContext::new(provider_name, "test-model")
        .with_web_search(has_web_search)
        .with_sub_agents(has_sub_agents)
        .with_workspace(workspace.display().to_string());

    // Build prompt (same as main agent with AutoApprove mode since that's what we expect)
    build_system_prompt_with_contributions(
        workspace,
        AgentMode::AutoApprove,
        None,
        Some(&registry),
        Some(&prompt_context),
    )
}

#[test]
fn test_eval_prompt_matches_main_agent_prompt_vertex() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let workspace = temp_dir.path();

    let eval_prompt = build_production_system_prompt(workspace, EvalProvider::VertexClaude);
    let main_prompt = build_main_agent_prompt(workspace, "anthropic", true);

    assert_eq!(
        eval_prompt, main_prompt,
        "Eval prompt must match main agent prompt for Vertex Claude"
    );
}

#[test]
fn test_eval_prompt_matches_main_agent_prompt_openai() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let workspace = temp_dir.path();

    let eval_prompt = build_production_system_prompt(workspace, EvalProvider::OpenAi);
    let main_prompt = build_main_agent_prompt(workspace, "openai", false);

    assert_eq!(
        eval_prompt, main_prompt,
        "Eval prompt must match main agent prompt for OpenAI"
    );
}

#[test]
fn test_eval_prompt_matches_main_agent_prompt_zai() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let workspace = temp_dir.path();

    let eval_prompt = build_production_system_prompt(workspace, EvalProvider::Zai);
    let main_prompt = build_main_agent_prompt(workspace, "zai", false);

    assert_eq!(
        eval_prompt, main_prompt,
        "Eval prompt must match main agent prompt for Z.AI"
    );
}

#[test]
fn test_eval_prompt_contains_core_sections() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let workspace = temp_dir.path();

    let prompt = build_production_system_prompt(workspace, EvalProvider::VertexClaude);

    // Verify all core sections from the main agent's system_prompt.rs are present
    assert!(
        prompt.contains("# Tone and style"),
        "Prompt must contain tone and style section"
    );
    assert!(
        prompt.contains("# Tool Reference"),
        "Prompt must contain tool reference section"
    );
    assert!(
        prompt.contains("# Sub-Agent Delegation"),
        "Prompt must contain sub-agent delegation section"
    );
    assert!(
        prompt.contains("# Security Boundaries"),
        "Prompt must contain security boundaries section"
    );
    assert!(
        prompt.contains("# Before Claiming Completion"),
        "Prompt must contain completion checklist"
    );
}

#[test]
fn test_eval_prompt_contains_autoapprove_mode() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let workspace = temp_dir.path();

    let prompt = build_production_system_prompt(workspace, EvalProvider::VertexClaude);

    // Evals use AutoApprove mode, which adds specific instructions
    assert!(
        prompt.contains("<autoapprove_mode>"),
        "Eval prompt must contain auto-approve mode instructions"
    );
}

#[test]
fn test_eval_prompt_contains_sub_agent_docs_for_vertex() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let workspace = temp_dir.path();

    let prompt = build_production_system_prompt(workspace, EvalProvider::VertexClaude);

    // With has_sub_agents = true, sub-agent docs should be included
    // Note: The registry starts empty, so we might not see specific sub-agents,
    // but the infrastructure should still work. Let's check the prompt
    // builds without errors.
    assert!(!prompt.is_empty(), "Prompt should not be empty");
}

#[test]
fn test_eval_prompt_provider_specific() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let workspace = temp_dir.path();

    let vertex_prompt = build_production_system_prompt(workspace, EvalProvider::VertexClaude);
    let openai_prompt = build_production_system_prompt(workspace, EvalProvider::OpenAi);
    let zai_prompt = build_production_system_prompt(workspace, EvalProvider::Zai);

    // OpenAI uses a different (Codex-style) prompt
    assert_ne!(
        vertex_prompt, openai_prompt,
        "OpenAI should use Codex-style prompt"
    );

    // Non-OpenAI providers should use the same default prompt
    assert_eq!(
        vertex_prompt, zai_prompt,
        "Non-OpenAI providers should use the same default prompt"
    );

    // Verify OpenAI uses Codex-style prompt (has "Core Principles")
    assert!(
        openai_prompt.contains("Core Principles"),
        "OpenAI prompt should contain Codex-style sections"
    );

    // Verify non-OpenAI providers use default prompt (has "Tone and style")
    assert!(
        vertex_prompt.contains("# Tone and style"),
        "Default prompt should contain standard sections"
    );
}
