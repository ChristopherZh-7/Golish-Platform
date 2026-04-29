use super::builder::create_default_sub_agents;
use super::prompts::{
    build_coder_prompt, build_pentester_prompt, build_planner_prompt, build_researcher_prompt,
};

fn has_tool(agent: &crate::SubAgentDefinition, tool: &str) -> bool {
    agent.allowed_tools.iter().any(|t| t == tool)
}

#[test]
fn test_create_default_sub_agents_count() {
    let agents = create_default_sub_agents();
    assert_eq!(agents.len(), 13);
}

#[test]
fn test_create_default_sub_agents_ids() {
    let agents = create_default_sub_agents();
    let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();

    assert!(ids.contains(&"coder"));
    assert!(!ids.contains(&"analyzer"));
    assert!(!ids.contains(&"explorer"));
    assert!(ids.contains(&"researcher"));
    assert!(!ids.contains(&"worker"));
    assert!(ids.contains(&"pentester"));
    assert!(ids.contains(&"memorist"));
    assert!(ids.contains(&"planner"));
    assert!(ids.contains(&"reflector"));
    assert!(ids.contains(&"adviser"));
    assert!(ids.contains(&"reporter"));
    assert!(ids.contains(&"orchestrator"));
    assert!(ids.contains(&"refiner"));
    // js_harvester and js_analyzer merged into pentester
    assert!(!ids.contains(&"js_harvester"));
    assert!(!ids.contains(&"js_analyzer"));
}

#[test]
fn test_researcher_has_web_tools() {
    let agents = create_default_sub_agents();
    let researcher = agents.iter().find(|a| a.id == "researcher").unwrap();

    assert!(has_tool(researcher, "web_search"));
    assert!(has_tool(researcher, "web_fetch"));
    assert!(has_tool(researcher, "search_knowledge_base"));
    assert!(has_tool(researcher, "read_knowledge"));
    assert!(has_tool(researcher, "write_knowledge"));
    assert!(has_tool(researcher, "ingest_cve"));
    assert!(has_tool(researcher, "save_poc"));
    assert!(has_tool(researcher, "list_cves_with_pocs"));
    assert!(has_tool(researcher, "list_unresearched_cves"));
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
fn test_all_agents_have_no_prompt_template() {
    let agents = create_default_sub_agents();
    for agent in &agents {
        assert!(
            agent.prompt_template.is_none(),
            "Agent '{}' should not have a prompt_template",
            agent.id
        );
    }
}

#[test]
fn test_pentester_has_security_tools() {
    let agents = create_default_sub_agents();
    let pentester = agents.iter().find(|a| a.id == "pentester").unwrap();

    assert!(!has_tool(pentester, "run_pty_cmd"));
    assert!(!has_tool(pentester, "run_command"));
    assert!(has_tool(pentester, "pentest_run"));
    assert!(has_tool(pentester, "pentest_list_tools"));
    assert!(has_tool(pentester, "web_search"));
    assert!(has_tool(pentester, "search_memories"));
    assert!(has_tool(pentester, "run_pipeline"));
    assert!(has_tool(pentester, "manage_targets"));
    assert!(has_tool(pentester, "record_finding"));
    assert!(!has_tool(pentester, "js_collect"));
    assert!(has_tool(pentester, "search_knowledge_base"));
    assert!(has_tool(pentester, "read_knowledge"));
    assert!(!has_tool(pentester, "write_knowledge"));
    assert_eq!(pentester.max_iterations, 50);
    assert_eq!(pentester.timeout_secs, Some(900));
}

#[test]
fn test_memorist_has_memory_and_readonly_wiki_tools() {
    let agents = create_default_sub_agents();
    let memorist = agents.iter().find(|a| a.id == "memorist").unwrap();

    assert!(has_tool(memorist, "search_memories"));
    assert!(has_tool(memorist, "store_memory"));
    assert!(has_tool(memorist, "list_memories"));
    assert!(has_tool(memorist, "search_knowledge_base"));
    assert!(has_tool(memorist, "read_knowledge"));
    assert!(!has_tool(memorist, "write_knowledge"));
    assert!(!has_tool(memorist, "run_pty_cmd"));
    assert_eq!(memorist.max_iterations, 10);
}

#[test]
fn test_reporter_and_adviser_have_readonly_wiki_tools() {
    let agents = create_default_sub_agents();
    let reporter = agents.iter().find(|a| a.id == "reporter").unwrap();
    let adviser = agents.iter().find(|a| a.id == "adviser").unwrap();

    assert!(has_tool(reporter, "search_knowledge_base"));
    assert!(has_tool(reporter, "read_knowledge"));
    assert!(has_tool(reporter, "list_cves_with_pocs"));
    assert!(has_tool(reporter, "poc_stats"));
    assert!(!has_tool(reporter, "write_knowledge"));

    assert!(has_tool(adviser, "search_knowledge_base"));
    assert!(has_tool(adviser, "read_knowledge"));
    assert!(!has_tool(adviser, "write_knowledge"));
}

#[test]
fn test_planner_is_mostly_readonly() {
    let agents = create_default_sub_agents();
    let planner = agents.iter().find(|a| a.id == "planner").unwrap();

    assert!(
        planner
            .allowed_tools
            .contains(&"search_memories".to_string())
    );
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
    assert!(prompt.contains("search_knowledge_base"));
}

#[test]
fn test_researcher_prompt_instructs_wiki_writes_with_cve_id() {
    let prompt = build_researcher_prompt();
    assert!(prompt.contains("search_knowledge_base"));
    assert!(prompt.contains("write_knowledge"));
    assert!(prompt.contains("cve_id"));
    assert!(prompt.contains("save_poc"));
}

#[test]
fn test_planner_prompt_has_json_format() {
    let prompt = build_planner_prompt();
    assert!(prompt.contains("plan_summary"));
    assert!(prompt.contains("subtasks"));
    assert!(prompt.contains("depends_on"));
    assert!(prompt.contains("success_criteria"));
}
