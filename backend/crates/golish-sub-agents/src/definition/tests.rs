use super::*;

// ===========================================
// SubAgentDefinition Tests
// ===========================================

#[test]
fn test_sub_agent_definition_new() {
    let agent = SubAgentDefinition::new(
        "test_agent",
        "Test Agent",
        "A test agent for unit tests",
        "You are a test agent.",
    );

    assert_eq!(agent.id, "test_agent");
    assert_eq!(agent.name, "Test Agent");
    assert_eq!(agent.description, "A test agent for unit tests");
    assert_eq!(agent.system_prompt, "You are a test agent.");
    assert!(agent.allowed_tools.is_empty());
    assert_eq!(agent.max_iterations, 50); // default
    assert!(agent.model_override.is_none()); // default
    assert_eq!(agent.timeout_secs, None); // default: no overall timeout
    assert_eq!(agent.idle_timeout_secs, Some(180)); // default: 3 minutes
    assert!(agent.prompt_template.is_none()); // default: no prompt generation
}

#[test]
fn test_sub_agent_definition_with_prompt_template() {
    let agent = SubAgentDefinition::new("test", "Test", "desc", "prompt")
        .with_prompt_template("Generate a prompt for: {task}");
    assert_eq!(
        agent.prompt_template,
        Some("Generate a prompt for: {task}".to_string())
    );
}

#[test]
fn test_sub_agent_definition_without_prompt_template() {
    let agent = SubAgentDefinition::new("test", "Test", "desc", "prompt");
    assert!(agent.prompt_template.is_none());
}

#[test]
fn test_sub_agent_definition_with_tools() {
    let agent = SubAgentDefinition::new("test", "Test", "desc", "prompt")
        .with_tools(vec!["read_file".to_string(), "write_file".to_string()]);

    assert_eq!(agent.allowed_tools.len(), 2);
    assert!(agent.allowed_tools.contains(&"read_file".to_string()));
    assert!(agent.allowed_tools.contains(&"write_file".to_string()));
}

#[test]
fn test_sub_agent_definition_with_max_iterations() {
    let agent = SubAgentDefinition::new("test", "Test", "desc", "prompt").with_max_iterations(100);

    assert_eq!(agent.max_iterations, 100);
}

#[test]
fn test_sub_agent_definition_builder_chain() {
    let agent = SubAgentDefinition::new("chained", "Chained Agent", "desc", "prompt")
        .with_tools(vec!["tool1".to_string()])
        .with_max_iterations(25);

    assert_eq!(agent.id, "chained");
    assert_eq!(agent.allowed_tools, vec!["tool1".to_string()]);
    assert_eq!(agent.max_iterations, 25);
}

#[test]
fn test_sub_agent_definition_with_model_override() {
    let agent = SubAgentDefinition::new("test", "Test", "desc", "prompt")
        .with_model_override("openai", "gpt-4o");

    assert_eq!(
        agent.model_override,
        Some(("openai".to_string(), "gpt-4o".to_string()))
    );
}

#[test]
fn test_sub_agent_definition_set_and_clear_model_override() {
    let mut agent = SubAgentDefinition::new("test", "Test", "desc", "prompt");

    // Initially no override
    assert!(agent.model_override.is_none());

    // Set override
    agent.set_model_override("anthropic", "claude-sonnet-4");
    assert_eq!(
        agent.model_override,
        Some(("anthropic".to_string(), "claude-sonnet-4".to_string()))
    );

    // Clear override
    agent.clear_model_override();
    assert!(agent.model_override.is_none());
}

// ===========================================
// SubAgentRegistry Tests
// ===========================================

#[test]
fn test_registry_new_is_empty() {
    let registry = SubAgentRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
}

#[test]
fn test_registry_default_is_empty() {
    let registry = SubAgentRegistry::default();
    assert!(registry.is_empty());
}

#[test]
fn test_registry_get_nonexistent() {
    let registry = SubAgentRegistry::new();
    assert!(registry.get("nonexistent").is_none());
}

// A DB-template reload rebuilds definitions from defaults (no model_override /
// LLM params) and must NOT wipe runtime overrides applied after construction
// (the `apply_sub_agent_model_settings` / `set_sub_agent_model` writers). The
// reload's only job is to refresh the prompt text.
#[test]
fn test_register_preserving_overrides_keeps_runtime_overrides() {
    let mut registry = SubAgentRegistry::new();

    // Simulate startup: register a fresh "coder", then apply runtime overrides.
    registry.register(SubAgentDefinition::new(
        "coder",
        "Coder",
        "desc",
        "old prompt",
    ));
    {
        let coder = registry.get_mut("coder").unwrap();
        coder.set_model_override("xiaomi", "mimo-v2.5-pro");
        coder.temperature = Some(0.2);
        coder.max_tokens = Some(4096);
        coder.top_p = Some(0.9);
    }

    // Simulate the DB-template reload: fresh defs (no overrides), new prompt,
    // plus a brand-new agent that has no prior entry to inherit from.
    let reloaded = vec![
        SubAgentDefinition::new("coder", "Coder", "desc", "new prompt from DB"),
        SubAgentDefinition::new("newbie", "Newbie", "desc", "fresh"),
    ];
    registry.register_preserving_overrides(reloaded);

    // The prompt refresh landed...
    let coder = registry.get("coder").unwrap();
    assert_eq!(coder.system_prompt, "new prompt from DB");
    // ...but the four runtime override fields survived the reload.
    assert_eq!(
        coder.model_override,
        Some(("xiaomi".to_string(), "mimo-v2.5-pro".to_string()))
    );
    assert_eq!(coder.temperature, Some(0.2));
    assert_eq!(coder.max_tokens, Some(4096));
    assert_eq!(coder.top_p, Some(0.9));

    // A newly introduced agent has nothing to inherit → stays unoverridden.
    let newbie = registry.get("newbie").unwrap();
    assert!(newbie.model_override.is_none());
    assert!(newbie.temperature.is_none());
}

// Reverse order (reload runs BEFORE overrides are applied): the reload finds
// no existing override to carry, and a later override write must still stick.
#[test]
fn test_register_preserving_overrides_no_existing_is_noop() {
    let mut registry = SubAgentRegistry::new();
    registry.register_preserving_overrides(vec![SubAgentDefinition::new(
        "coder", "Coder", "desc", "prompt",
    )]);
    assert!(registry.get("coder").unwrap().model_override.is_none());

    // Override applied after the reload survives (no further reload clobbers it).
    registry
        .get_mut("coder")
        .unwrap()
        .set_model_override("xiaomi", "mimo-v2.5-pro");
    assert_eq!(
        registry.get("coder").unwrap().model_override,
        Some(("xiaomi".to_string(), "mimo-v2.5-pro".to_string()))
    );
}

// ===========================================
// SubAgentContext Tests
// ===========================================

#[test]
fn test_context_default() {
    let context = SubAgentContext::default();
    assert_eq!(context.original_request, "");
    assert!(context.conversation_summary.is_none());
    assert!(context.variables.is_empty());
    assert_eq!(context.depth, 0);
    assert!(context.parent_agent.is_none());
    assert!(context.task_id.is_none());
    assert!(context.subtask_id.is_none());
    assert!(context.execution_history.is_empty());
}

#[test]
fn test_context_with_values() {
    let mut variables = HashMap::new();
    variables.insert("key".to_string(), serde_json::json!("value"));

    let context = SubAgentContext {
        original_request: "Do something".to_string(),
        conversation_summary: Some("Previous context".to_string()),
        variables,
        depth: 2,
        parent_agent: Some("main-agent".to_string()),
        task_id: Some("task-001".to_string()),
        subtask_id: Some("subtask-001".to_string()),
        execution_history: vec!["Step 1: Scanned ports".to_string()],
    };

    assert_eq!(context.original_request, "Do something");
    assert_eq!(
        context.conversation_summary,
        Some("Previous context".to_string())
    );
    assert_eq!(
        context.variables.get("key").unwrap(),
        &serde_json::json!("value")
    );
    assert_eq!(context.depth, 2);
    assert_eq!(context.parent_agent, Some("main-agent".to_string()));
    assert_eq!(context.task_id, Some("task-001".to_string()));
    assert_eq!(context.execution_history.len(), 1);
}

// ===========================================
// SubAgentResult Tests
// ===========================================

#[test]
fn test_result_construction() {
    let result = SubAgentResult {
        agent_id: "test_agent".to_string(),
        response: "Task completed".to_string(),
        context: SubAgentContext::default(),
        success: true,
        duration_ms: 1500,
        files_modified: vec!["main.go".to_string()],
    };

    assert_eq!(result.agent_id, "test_agent");
    assert_eq!(result.response, "Task completed");
    assert!(result.success);
    assert_eq!(result.duration_ms, 1500);
    assert_eq!(result.files_modified, vec!["main.go".to_string()]);
}

// ===========================================
// Constants Tests
// ===========================================

#[test]
fn test_max_agent_depth() {
    // Capped at 2 = single-level nesting (primary → sub-agent; no deeper).
    assert_eq!(MAX_AGENT_DEPTH, 2);
}
