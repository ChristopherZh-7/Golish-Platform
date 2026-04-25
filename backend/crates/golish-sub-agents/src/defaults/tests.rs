use super::builder::create_default_sub_agents;
use super::prompts::{build_analyzer_prompt, build_coder_prompt, build_explorer_prompt, build_pentester_prompt, build_planner_prompt};

#[test]
fn test_create_default_sub_agents_count() {
    let agents = create_default_sub_agents();
    assert_eq!(agents.len(), 11);
}

#[test]
fn test_create_default_sub_agents_ids() {
    let agents = create_default_sub_agents();
    let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();

    assert!(ids.contains(&"coder"));
    assert!(ids.contains(&"analyzer"));
    assert!(ids.contains(&"explorer"));
    assert!(ids.contains(&"researcher"));
    assert!(ids.contains(&"worker"));
    assert!(ids.contains(&"pentester"));
    assert!(ids.contains(&"memorist"));
    assert!(ids.contains(&"planner"));
    assert!(ids.contains(&"reflector"));
    assert!(ids.contains(&"adviser"));
    assert!(ids.contains(&"reporter"));
    // js_harvester and js_analyzer merged into pentester
    assert!(!ids.contains(&"js_harvester"));
    assert!(!ids.contains(&"js_analyzer"));
}

#[test]
fn test_analyzer_has_read_only_tools() {
    let agents = create_default_sub_agents();
    let analyzer = agents.iter().find(|a| a.id == "analyzer").unwrap();

    assert!(analyzer.allowed_tools.contains(&"read_file".to_string()));
    assert!(!analyzer.allowed_tools.contains(&"write_file".to_string()));
    assert!(!analyzer.allowed_tools.contains(&"edit_file".to_string()));
}

#[test]
fn test_explorer_has_navigation_tools() {
    let agents = create_default_sub_agents();
    let explorer = agents.iter().find(|a| a.id == "explorer").unwrap();

    // Should have navigation and search tools
    assert!(explorer.allowed_tools.contains(&"read_file".to_string()));
    assert!(explorer.allowed_tools.contains(&"list_files".to_string()));
    assert!(explorer
        .allowed_tools
        .contains(&"list_directory".to_string()));
    assert!(explorer.allowed_tools.contains(&"grep_file".to_string()));
    assert!(explorer.allowed_tools.contains(&"find_files".to_string()));

    // Should NOT have shell access (removed for efficiency)
    assert!(!explorer.allowed_tools.contains(&"run_pty_cmd".to_string()));

    // Should NOT have write tools
    assert!(!explorer.allowed_tools.contains(&"write_file".to_string()));
    assert!(!explorer.allowed_tools.contains(&"edit_file".to_string()));

    // Should NOT have indexer tools (those are for analyzer)
    assert!(!explorer
        .allowed_tools
        .contains(&"indexer_analyze_file".to_string()));
}

#[test]
fn test_researcher_has_web_tools() {
    let agents = create_default_sub_agents();
    let researcher = agents.iter().find(|a| a.id == "researcher").unwrap();

    assert!(researcher.allowed_tools.contains(&"web_search".to_string()));
    assert!(researcher.allowed_tools.contains(&"web_fetch".to_string()));
}

#[test]
fn test_default_agents_have_reasonable_iterations() {
    let agents = create_default_sub_agents();

    for agent in &agents {
        assert!(
            agent.max_iterations >= 3,
            "{} has too few iterations: {}",
            agent.id,
            agent.max_iterations
        );
        assert!(
            agent.max_iterations <= 50,
            "{} has too many iterations: {}",
            agent.id,
            agent.max_iterations
        );
    }
}

#[test]
fn test_coder_prompt_contains_schema() {
    let prompt = build_coder_prompt();
    // Verify the schema was injected
    assert!(prompt.contains("<implementation_plan>"));
    assert!(prompt.contains("<current_content>"));
    assert!(prompt.contains("<patterns>"));
}

#[test]
fn test_analyzer_prompt_uses_natural_language() {
    let prompt = build_analyzer_prompt();
    // Verify natural language format instead of XML
    assert!(prompt.contains("**Analysis Summary**"));
    assert!(prompt.contains("**Key Findings**"));
    assert!(prompt.contains("**Implementation Guidance**"));
    // Should NOT contain XML tags
    assert!(!prompt.contains("<analysis_result>"));
}

#[test]
fn test_explorer_prompt_uses_natural_language() {
    let prompt = build_explorer_prompt();
    // Verify natural language format for the updated explorer prompt
    assert!(prompt.contains("file search agent"));
    assert!(prompt.contains("CONSTRAINTS"));
    assert!(prompt.contains("READ-ONLY"));
    assert!(prompt.contains("TOOLS"));
    assert!(prompt.contains("OUTPUT"));
    // Should NOT contain XML tags
    assert!(!prompt.contains("<exploration_result>"));
}

#[test]
fn test_worker_has_broad_tool_access() {
    let agents = create_default_sub_agents();
    let worker = agents.iter().find(|a| a.id == "worker").unwrap();

    // Should have file read/write tools
    assert!(worker.allowed_tools.contains(&"read_file".to_string()));
    assert!(worker.allowed_tools.contains(&"write_file".to_string()));
    assert!(worker.allowed_tools.contains(&"edit_file".to_string()));
    assert!(worker.allowed_tools.contains(&"create_file".to_string()));
    assert!(worker.allowed_tools.contains(&"delete_file".to_string()));

    // Should have search tools
    assert!(worker.allowed_tools.contains(&"grep_file".to_string()));
    assert!(worker.allowed_tools.contains(&"ast_grep".to_string()));
    assert!(worker
        .allowed_tools
        .contains(&"ast_grep_replace".to_string()));

    // Should have shell access
    assert!(worker.allowed_tools.contains(&"run_pty_cmd".to_string()));

    // Should have web tools
    assert!(worker.allowed_tools.contains(&"web_search".to_string()));
    assert!(worker.allowed_tools.contains(&"web_fetch".to_string()));
}

#[test]
fn test_worker_has_prompt_template() {
    let agents = create_default_sub_agents();
    let worker = agents.iter().find(|a| a.id == "worker").unwrap();
    assert!(
        worker.prompt_template.is_some(),
        "Worker should have a prompt_template"
    );
    let template = worker.prompt_template.as_ref().unwrap();
    // Template is a system prompt for the prompt generator, not a string template
    assert!(
        template.contains("agent architect"),
        "Template should describe the architect role"
    );
    assert!(
        template.contains("Return ONLY the system prompt text"),
        "Template should instruct plain text output"
    );
    // Should NOT contain substitution placeholders — task/context go as user message
    assert!(
        !template.contains("{task}"),
        "Template should not contain {{task}} placeholder"
    );
}

#[test]
fn test_specialized_agents_do_not_have_prompt_template() {
    let agents = create_default_sub_agents();
    for agent in &agents {
        if agent.id == "worker" {
            continue;
        }
        assert!(
            agent.prompt_template.is_none(),
            "Specialized agent '{}' should not have a prompt_template",
            agent.id
        );
    }
}

#[test]
fn test_pentester_has_security_tools() {
    let agents = create_default_sub_agents();
    let pentester = agents.iter().find(|a| a.id == "pentester").unwrap();

    assert!(pentester.allowed_tools.contains(&"run_pty_cmd".to_string()));
    assert!(pentester.allowed_tools.contains(&"web_search".to_string()));
    assert!(pentester.allowed_tools.contains(&"search_memories".to_string()));
    assert!(pentester.allowed_tools.contains(&"run_pipeline".to_string()));
    assert!(pentester.allowed_tools.contains(&"manage_targets".to_string()));
    assert!(pentester.allowed_tools.contains(&"record_finding".to_string()));
    assert!(pentester.allowed_tools.contains(&"js_collect".to_string()));
    assert_eq!(pentester.max_iterations, 50);
    assert_eq!(pentester.timeout_secs, Some(900));
}

#[test]
fn test_memorist_has_memory_tools_only() {
    let agents = create_default_sub_agents();
    let memorist = agents.iter().find(|a| a.id == "memorist").unwrap();

    assert!(memorist.allowed_tools.contains(&"search_memories".to_string()));
    assert!(memorist.allowed_tools.contains(&"store_memory".to_string()));
    assert!(memorist.allowed_tools.contains(&"list_memories".to_string()));
    assert!(!memorist.allowed_tools.contains(&"run_pty_cmd".to_string()));
    assert_eq!(memorist.max_iterations, 10);
}

#[test]
fn test_planner_is_mostly_readonly() {
    let agents = create_default_sub_agents();
    let planner = agents.iter().find(|a| a.id == "planner").unwrap();

    assert!(planner.allowed_tools.contains(&"search_memories".to_string()));
    assert!(!planner.allowed_tools.contains(&"run_pty_cmd".to_string()));
    assert!(!planner.allowed_tools.contains(&"write_file".to_string()));
    assert_eq!(planner.max_iterations, 5);
}

#[test]
fn test_reflector_has_no_tools() {
    let agents = create_default_sub_agents();
    let reflector = agents.iter().find(|a| a.id == "reflector").unwrap();

    assert!(reflector.allowed_tools.is_empty());
    assert_eq!(reflector.max_iterations, 3);
    assert_eq!(reflector.timeout_secs, Some(60));
}

#[test]
fn test_pentester_prompt_has_core_identity() {
    let prompt = build_pentester_prompt();
    assert!(prompt.contains("penetration testing specialist"));
    assert!(prompt.contains("<expertise>"));
    assert!(prompt.contains("<constraints>"));
}

#[test]
fn test_planner_prompt_has_json_format() {
    let prompt = build_planner_prompt();
    assert!(prompt.contains("plan_summary"));
    assert!(prompt.contains("subtasks"));
    assert!(prompt.contains("depends_on"));
    assert!(prompt.contains("success_criteria"));
}
