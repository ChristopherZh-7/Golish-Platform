use super::builder::create_default_sub_agents;
use super::prompts::{
    build_browser_prompt, build_coder_prompt, build_enumerator_prompt, build_pentester_prompt,
    build_planner_prompt, build_prober_prompt, build_recon_prompt, build_researcher_prompt,
};

fn has_tool(agent: &crate::SubAgentDefinition, tool: &str) -> bool {
    agent.allowed_tools.iter().any(|t| t == tool)
}

#[test]
fn test_create_default_sub_agents_count() {
    let agents = create_default_sub_agents();
    // 13 base + recon (target_intel passive collector) + prober (external_attack_surface
    // active surface-mapper) + enumerator (enumeration active content-mapper) — the three
    // stage_run per-org specialists (2026-06-13-stage-run-fanout).
    assert_eq!(agents.len(), 16);
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
    assert!(ids.contains(&"recon"));
    assert!(ids.contains(&"prober"));
    assert!(ids.contains(&"enumerator"));
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
    // pipeline tools are intentionally not exposed to agents
    assert!(!has_tool(pentester, "run_pipeline"));
    assert!(!has_tool(pentester, "flow_compose"));
    assert!(has_tool(pentester, "manage_targets"));
    assert!(has_tool(pentester, "record_finding"));
    // The pentester runs the harness target_intel stage, so it must carry the
    // passive asset-intel recon_* tools — otherwise it falls back to manual
    // `dig` via pentest_run instead of the 0.zone/quake/ENScan provider engine.
    assert!(has_tool(pentester, "recon_list_providers"));
    assert!(has_tool(pentester, "recon_discover_subsidiaries"));
    assert!(has_tool(pentester, "recon_map_assets"));
    assert!(has_tool(pentester, "recon_lookup_whois"));
    assert!(!has_tool(pentester, "js_collect"));
    assert!(has_tool(pentester, "search_knowledge_base"));
    assert!(has_tool(pentester, "read_knowledge"));
    assert!(!has_tool(pentester, "write_knowledge"));
    assert_eq!(pentester.max_iterations, 50);
    // No overall wall-clock timeout: the agent keeps working as long as it
    // makes progress, bounded only by the idle timeout + max_iterations.
    assert_eq!(pentester.timeout_secs, None);
    assert_eq!(pentester.idle_timeout_secs, Some(300));
}

#[test]
fn test_recon_has_passive_tools_only() {
    // stage_run fan-out (2026-06-13-stage-run-fanout D4): Recon is the passive
    // target_intel collector split out of the Pentester. It must carry the
    // provider/enrichment recon_* tools + the stage submit tool, and must NOT
    // carry the Pentester's offensive surface (exploits, graph writes, vault).
    let agents = create_default_sub_agents();
    let recon = agents.iter().find(|a| a.id == "recon").unwrap();

    // Passive collection + stage submission tools present.
    assert!(has_tool(recon, "recon_list_providers"));
    assert!(has_tool(recon, "recon_discover_subsidiaries"));
    assert!(has_tool(recon, "recon_map_assets"));
    assert!(has_tool(recon, "recon_lookup_whois"));
    assert!(has_tool(recon, "manage_targets"));
    assert!(has_tool(recon, "submit_stage_deliverable"));
    assert!(has_tool(recon, "record_finding"));
    assert!(has_tool(recon, "search_knowledge_base"));
    assert!(has_tool(recon, "read_knowledge"));

    // Offensive / heavy Pentester-only tools must NOT leak into Recon.
    assert!(!has_tool(recon, "list_in_scope_targets"));
    assert!(!has_tool(recon, "pentest_run"));
    assert!(!has_tool(recon, "search_exploits"));
    assert!(!has_tool(recon, "graph_add_entity"));
    assert!(!has_tool(recon, "graph_attack_paths"));
    assert!(!has_tool(recon, "vault"));
    assert!(!has_tool(recon, "write_knowledge"));
    assert!(!has_tool(recon, "run_pty_cmd"));
}

#[test]
fn test_recon_prompt_is_zero_touch() {
    let prompt = build_recon_prompt();
    assert!(prompt.contains("ZERO-TOUCH"));
    assert!(prompt.contains("recon_map_assets"));
    assert!(prompt.contains("submit_stage_deliverable"));
    // Passive identity: must not describe itself as doing exploitation.
    assert!(prompt.contains("passive"));
    assert!(!prompt.contains("list_in_scope_targets"));
    assert!(!prompt.contains("subfinder"));
    assert!(!prompt.contains("ctfr"));
    assert!(!prompt.contains("dig"));
}

#[test]
fn test_prober_has_active_surface_tools() {
    // stage_run fan-out (2026-06-13-stage-run-fanout · EAS rollout): Prober is the
    // active external-attack-surface mapper split out of the Pentester (mirroring
    // how Recon was split for target_intel). It must carry the active probe tools
    // (pentest_run) + target state + the stage submit tool, and must NOT carry the
    // passive provider recon_* tools (those are Recon's) nor the Pentester's
    // offensive surface (exploits, graph writes, vault).
    let agents = create_default_sub_agents();
    let prober = agents.iter().find(|a| a.id == "prober").unwrap();

    // Active surface mapping + stage submission tools present.
    assert!(has_tool(prober, "pentest_run"));
    assert!(has_tool(prober, "pentest_list_tools"));
    assert!(has_tool(prober, "wait_for_background_jobs"));
    assert!(has_tool(prober, "manage_targets"));
    assert!(has_tool(prober, "list_in_scope_targets"));
    // L1b (design 2026-06-24): Prober gets the ranked attack-surface seed worklist.
    assert!(has_tool(prober, "list_attack_surface_seeds"));
    // Repair mode relies on this read helper to inspect existing target/evidence detail.
    assert!(has_tool(prober, "query_target_data"));
    // A (design 2026-07-02-eas-worker-evidence): the real-evidence-id lister so the
    // prober can cite ids for `every claim must cite evidence` instead of dead-looping.
    assert!(has_tool(prober, "list_recent_evidence"));
    assert!(has_tool(prober, "submit_stage_deliverable"));
    assert!(has_tool(prober, "search_knowledge_base"));
    assert!(has_tool(prober, "read_knowledge"));

    // Passive provider recon_* tools belong to Recon, not Prober.
    assert!(!has_tool(prober, "recon_map_assets"));
    assert!(!has_tool(prober, "recon_discover_subsidiaries"));
    // Offensive / heavy Pentester-only tools must NOT leak into Prober.
    assert!(!has_tool(prober, "search_exploits"));
    assert!(!has_tool(prober, "graph_attack_paths"));
    assert!(!has_tool(prober, "vault"));
    assert!(!has_tool(prober, "run_pty_cmd"));
}

#[test]
fn test_prober_prompt_is_active_surface() {
    let prompt = build_prober_prompt();
    assert!(prompt.contains("attack surface"));
    assert!(prompt.contains("submit_stage_deliverable"));
    // Active surface mapping: liveness (httpx) / open ports / service fingerprint.
    assert!(prompt.contains("httpx"));
    assert!(prompt.contains("port"));
    assert!(prompt.contains("list_attack_surface_seeds"));
    assert!(prompt.contains("wait_for_background_jobs"));
    assert!(prompt.contains("found cells are credited from the database"));
    assert!(prompt.contains("Do NOT hand-copy found coverage cells"));
    assert!(prompt.contains("HTTP liveness alone"));
    assert!(prompt.contains("-list {{input_file}}"));
    assert!(prompt.contains("-iL {{input_file}}"));
    assert!(prompt.contains("--input-file={{input_file}}"));
    assert!(prompt.contains("file -f {{input_file}}"));
    // Prober is the ACTIVE counterpart of the ZERO-TOUCH Recon — it must NOT
    // describe itself as zero-touch.
    assert!(!prompt.contains("ZERO-TOUCH"));
}

#[test]
fn test_enumerator_has_content_enum_tools() {
    // stage_run fan-out (2026-06-13-stage-run-fanout · enumeration rollout): Enumerator is
    // the active content-enumeration mapper split out of the Pentester (mirroring how Prober
    // was split for external_attack_surface). It must carry the content-probe tools
    // (route_probe_paths for DIR, pentest_run for bounded crawler URL sources,
    // and js_collect/js_extract_apis for JS-API/PARAM extraction)
    // + target state + the stage submit tool, and must NOT carry the passive provider recon_*
    // tools (Recon's) nor the Pentester's offensive surface (exploits, graph writes, vault).
    let agents = create_default_sub_agents();
    let enumerator = agents.iter().find(|a| a.id == "enumerator").unwrap();

    // Active content enumeration + stage submission tools present.
    assert!(has_tool(enumerator, "pentest_run"));
    assert!(has_tool(enumerator, "pentest_list_tools"));
    assert!(has_tool(enumerator, "wait_for_background_jobs"));
    assert!(has_tool(enumerator, "browser_collect_js_api"));
    assert!(has_tool(enumerator, "js_collect"));
    assert!(has_tool(enumerator, "js_extract_apis"));
    assert!(has_tool(enumerator, "route_probe_paths"));
    assert!(has_tool(enumerator, "stage_worklist_status"));
    assert!(has_tool(enumerator, "stage_worklist_next"));
    assert!(has_tool(enumerator, "list_enumeration_web_roots"));
    assert!(!has_tool(enumerator, "manage_targets"));
    assert!(!has_tool(enumerator, "list_in_scope_targets"));
    assert!(has_tool(enumerator, "query_target_data"));
    assert!(has_tool(enumerator, "submit_stage_deliverable"));
    assert!(!has_tool(enumerator, "record_finding"));
    assert!(has_tool(enumerator, "search_knowledge_base"));
    assert!(has_tool(enumerator, "read_knowledge"));

    // Passive provider recon_* tools belong to Recon, not Enumerator.
    assert!(!has_tool(enumerator, "recon_map_assets"));
    assert!(!has_tool(enumerator, "recon_discover_subsidiaries"));
    // Offensive / heavy Pentester-only tools must NOT leak into Enumerator.
    assert!(!has_tool(enumerator, "search_exploits"));
    assert!(!has_tool(enumerator, "graph_attack_paths"));
    assert!(!has_tool(enumerator, "vault"));
    assert!(!has_tool(enumerator, "run_pty_cmd"));
}

#[test]
fn test_enumerator_prompt_is_content_enum() {
    let prompt = build_enumerator_prompt();
    assert!(prompt.contains("enumeration"));
    assert!(prompt.contains("submit_stage_deliverable"));
    assert!(prompt.contains("wait_for_background_jobs"));
    // Content enumeration: directories / parameters / JS-API extraction.
    assert!(prompt.contains("director"));
    assert!(prompt.contains("param"));
    assert!(prompt.contains("browser_collect_js_api"));
    assert!(prompt.contains("run browser_collect_js_api first"));
    assert!(prompt.contains("not as a substitute"));
    assert!(prompt.contains("ai_assist"));
    assert!(prompt.contains("recipe"));
    assert!(prompt.contains("js_extract_apis"));
    assert!(prompt.contains("route_probe_paths"));
    assert!(prompt.contains("stage_worklist_status"));
    assert!(prompt.contains("stage_worklist_next"));
    assert!(prompt.contains("ready_to_submit=true"));
    assert!(prompt.contains("work_item_id"));
    assert!(prompt.contains("full de-duplicated local/built-in wordlist"));
    assert!(prompt.contains("Do not call external directory tools"));
    assert!(prompt.contains("list_enumeration_web_roots"));
    assert!(prompt.contains("pentest_run(tool_name=..., args=...)"));
    assert!(prompt.contains("DB cannot derive"));
    assert!(prompt.contains("check_stage_asset_coverage"));
    assert!(prompt.contains("web_root_enumerated"));
    assert!(prompt.contains("api_endpoints_discovered"));
    assert!(prompt.contains("do not call record_finding"));
    assert!(prompt.contains("\"web_roots\""));
    assert!(prompt.contains("\"directories\""));
    assert!(prompt.contains("\"coverage\""));
    // Enumerator is the ACTIVE counterpart of the ZERO-TOUCH Recon — it must NOT
    // describe itself as zero-touch.
    assert!(!prompt.contains("ZERO-TOUCH"));
    assert!(!prompt.contains("list_in_scope_targets first"));
    assert!(!prompt.contains("Confirm/annotate targets via manage_targets"));
}

#[test]
fn test_browser_prompt_prefers_browser_closure_collection() {
    let prompt = build_browser_prompt();
    assert!(prompt.contains("browser_collect_js_api"));
    assert!(prompt.contains("Use `browser_collect_js_api` first"));
    assert!(prompt.contains("crawl_mode=\"standard\""));
    assert!(prompt.contains("ai_assist=true"));
    assert!(prompt.contains("same standard mode"));
    assert!(prompt.contains("bounded `recipe`"));
    assert!(prompt.contains("js_extract_apis"));
    assert!(prompt.contains("not as a replacement"));
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

    assert!(planner
        .allowed_tools
        .contains(&"search_memories".to_string()));
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
    // No overall wall-clock timeout; idle timeout + max_iterations still bound it.
    assert_eq!(reflector.timeout_secs, None);
    assert_eq!(reflector.idle_timeout_secs, Some(30));
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
