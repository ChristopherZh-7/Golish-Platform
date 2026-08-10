use super::builder::{create_default_sub_agents, create_default_sub_agents_from_registry};
use super::prompts::{
    build_attack_analyst_prompt, build_browser_prompt, build_candidate_hypothesis_analyst_prompt,
    build_candidate_verifier_prompt, build_coder_prompt, build_enumerator_prompt,
    build_merge_conflict_critic_prompt, build_orchestrator_prompt, build_pentester_prompt,
    build_planner_prompt, build_post_exploit_operator_prompt, build_prober_prompt,
    build_recon_prompt, build_researcher_prompt, build_vuln_scanner_prompt,
};

fn has_tool(agent: &crate::SubAgentDefinition, tool: &str) -> bool {
    agent.allowed_tools.iter().any(|t| t == tool)
}

#[test]
fn test_create_default_sub_agents_count() {
    let agents = create_default_sub_agents();
    // 13 base + recon (target_intel passive collector) + prober (external_attack_surface
    // active surface-mapper) + enumerator (enumeration active content-mapper)
    // + vuln_scanner (formulaic vuln-triage worker) + two closed AU modelers,
    // Investigation cognitive Primary + post_exploit_operator —
    // the stage_run per-org specialists (2026-06-13-stage-run-fanout / C6 P6b).
    assert_eq!(agents.len(), 40);
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
    assert!(ids.contains(&"vuln_scanner"));
    assert!(ids.contains(&"application_understanding_shard_modeler"));
    assert!(ids.contains(&"application_understanding_company_synthesizer"));
    assert!(!ids.contains(&"application_understanding"));
    assert!(ids.contains(&"investigation"));
    assert!(ids.contains(&"attack_analyst"));
    assert!(ids.contains(&"candidate_hypothesis_controller"));
    assert!(ids.contains(&"candidate_hypothesis_analyst"));
    assert!(ids.contains(&"merge_conflict_critic"));
    assert!(ids.contains(&"candidate_verifier"));
    assert!(ids.contains(&"verification_lead"));
    assert!(ids.contains(&"verification_independent_critic"));
    assert!(ids.contains(&"verification_refiner"));
    assert!(ids.contains(&"post_exploit_operator"));
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
fn unified_topology_agents_declare_only_cognitive_tools() {
    let agents = create_default_sub_agents();
    let shard_modeler = agents
        .iter()
        .find(|agent| agent.id == "application_understanding_shard_modeler")
        .expect("AU shard modeler");
    assert_eq!(shard_modeler.allowed_tools, ["submit_result"]);
    assert_eq!(shard_modeler.max_tokens, Some(32_768));
    assert!(shard_modeler.readonly);

    let company_synthesizer = agents
        .iter()
        .find(|agent| agent.id == "application_understanding_company_synthesizer")
        .expect("AU company synthesizer");
    assert_eq!(company_synthesizer.allowed_tools, ["submit_result"]);
    assert_eq!(company_synthesizer.max_tokens, Some(32_768));
    assert!(company_synthesizer.readonly);

    let investigation = agents
        .iter()
        .find(|agent| agent.id == "investigation")
        .expect("Investigation Primary");
    assert!(has_tool(investigation, "update_plan"));
    assert!(investigation
        .delegatable_agents
        .iter()
        .any(|agent| agent == "pentester"));
    assert!(!has_tool(investigation, "web_fetch"));
    assert!(!has_tool(investigation, "browser_navigate"));
    assert!(!has_tool(investigation, "pentest_run"));
    assert!(!has_tool(investigation, "record_finding"));
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
    assert!(has_tool(pentester, "check_job"));
    assert!(has_tool(pentester, "kill_job"));
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
fn every_default_pentest_runner_has_managed_process_controls() {
    let agents = create_default_sub_agents();
    let runners = agents.iter().filter(|agent| has_tool(agent, "pentest_run"));
    let mut count = 0;
    for agent in runners {
        count += 1;
        assert!(has_tool(agent, "check_job"), "{} needs check_job", agent.id);
        assert!(has_tool(agent, "kill_job"), "{} needs kill_job", agent.id);
    }
    assert!(count > 0);
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
    assert!(has_tool(recon, "check_stage_asset_coverage"));
    assert!(has_tool(recon, "stage_worklist_status"));
    assert!(has_tool(recon, "stage_worklist_next"));
    assert!(has_tool(recon, "query_target_data"));
    assert!(
        !has_tool(recon, "manage_targets"),
        "Target Intel observations must land through deterministic recon wrappers; the model cannot promote provider/DNS observations into active scope"
    );
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
    assert!(prompt.contains("ready_to_submit=true"));
    assert!(prompt.contains("terminal_exceptions_preview.coverage_to_submit"));
    assert!(prompt.contains("status `accepted` is terminal"));
    assert!(prompt.contains("do not invent"));
    assert!(prompt.contains("do not register targets yourself"));
    assert!(prompt.contains("Do not call manage_targets"));
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
    // (eas_* backend wrappers) + target state + the stage submit tool, and must NOT carry the
    // passive provider recon_* tools (those are Recon's) nor the Pentester's
    // offensive surface (exploits, graph writes, vault).
    let agents = create_default_sub_agents();
    let prober = agents.iter().find(|a| a.id == "prober").unwrap();

    // Active surface mapping + stage submission tools present.
    assert!(has_tool(prober, "eas_probe_http_liveness"));
    assert!(has_tool(prober, "eas_discover_ports"));
    assert!(has_tool(prober, "eas_fingerprint_services"));
    assert!(has_tool(prober, "eas_fingerprint_web_stack"));
    assert!(!has_tool(prober, "pentest_run"));
    assert!(!has_tool(prober, "pentest_list_tools"));
    assert!(has_tool(prober, "wait_for_background_jobs"));
    assert!(has_tool(prober, "check_job"));
    assert!(has_tool(prober, "kill_job"));
    assert!(!has_tool(prober, "manage_targets"));
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
    // Active surface mapping: liveness / open ports / service fingerprint via wrappers.
    assert!(prompt.contains("eas_probe_http_liveness"));
    assert!(prompt.contains("eas_discover_ports"));
    assert!(prompt.contains("scan_profile=\"full\""));
    assert!(prompt.contains("quick/standard"));
    assert!(prompt.contains("remain partial"));
    assert!(prompt.contains("evidence-backed policy-blocked LIVENESS/PORT"));
    assert!(!prompt.contains("scanner=\"naabu\""));
    assert!(!prompt.contains("top_ports="));
    assert!(prompt.contains("eas_fingerprint_services"));
    assert!(prompt.contains("eas_fingerprint_web_stack"));
    assert!(prompt.contains("confirmed HTTP(S)"));
    assert!(prompt.contains("port"));
    assert!(prompt.contains("list_attack_surface_seeds"));
    assert!(prompt.contains("adjustable initial yield"));
    assert!(prompt.contains("same process remains alive"));
    assert!(prompt.contains("managed `job_id`"));
    assert!(prompt.contains("never kills, detaches, or respawns it"));
    assert!(prompt.contains("typed completion"));
    assert!(prompt.contains("Usually omit `ports`"));
    assert!(prompt.contains("Do not pass or increase a timeout"));
    assert!(prompt.contains("already isolates slow IPs"));
    assert!(!prompt.contains("background:true"));
    assert!(!prompt.contains("soft timeout"));
    assert!(prompt.contains("wait_for_background_jobs"));
    assert!(prompt.contains("check_job"));
    assert!(prompt.contains("kill_job"));
    assert!(!prompt.contains("All four wrappers are forced foreground"));
    assert!(prompt.contains("found cells are credited from the database"));
    assert!(prompt.contains("Do NOT hand-copy found coverage cells"));
    assert!(prompt.contains("Never include a bare IP, IP:port, or CIDR"));
    assert!(prompt.contains("pass its ONE bare domain value exactly once"));
    assert!(prompt.contains("do not expand it into separate"));
    assert!(prompt.contains("run eas_discover_ports first"));
    assert!(prompt.contains("WEB-FINGERPRINT"));
    assert!(prompt.contains("HTTP liveness alone"));
    assert!(prompt.contains("do NOT call httpx or pentest_run directly"));
    assert!(prompt.contains("do NOT call nmap/naabu/masscan or pentest_run directly"));
    assert!(prompt.contains("terminal_exceptions_preview.coverage_to_submit"));
    assert!(prompt.contains("returns `accepted`, stop immediately"));
    assert!(prompt.contains("details.missing_origins"));
    assert!(prompt.contains("details.recommended_args.target_urls"));
    assert!(prompt.contains("Copy those exact origins unchanged"));
    assert!(prompt.contains("Never guess, infer, or rewrite the scheme"));
    assert!(!prompt.contains("via manage_targets"));
    // Prober is the ACTIVE counterpart of the ZERO-TOUCH Recon — it must NOT
    // describe itself as zero-touch.
    assert!(!prompt.contains("ZERO-TOUCH"));
}

#[tokio::test]
async fn registry_builder_keeps_managed_process_controls_for_every_pentest_runner() {
    let registry = crate::prompt_registry::PromptRegistry::new();
    let agents = create_default_sub_agents_from_registry(&registry).await;
    let prober = agents
        .iter()
        .find(|agent| agent.id == "prober")
        .expect("registry-built prober");
    for tool in [
        "eas_discover_ports",
        "wait_for_background_jobs",
        "check_job",
        "kill_job",
    ] {
        assert!(has_tool(prober, tool), "registry prober needs {tool}");
    }
    for agent in agents.iter().filter(|agent| has_tool(agent, "pentest_run")) {
        assert!(has_tool(agent, "check_job"), "{} needs check_job", agent.id);
        assert!(has_tool(agent, "kill_job"), "{} needs kill_job", agent.id);
    }
}

#[test]
fn test_enumerator_has_content_enum_tools() {
    // stage_run fan-out (2026-06-13-stage-run-fanout · enumeration rollout): Enumerator is
    // the active content-enumeration mapper split out of the Pentester (mirroring how Prober
    // was split for external_attack_surface). It must carry the content-probe tools
    // (route_probe_paths for DIR, enum_crawl_same_origin_urls for bounded crawler
    // URL sources, and browser_collect_js_api/js_extract_apis for JS-API/PARAM extraction)
    // + target state + the stage submit tool, and must NOT carry the passive provider recon_*
    // tools (Recon's) nor the Pentester's offensive surface (exploits, graph writes, vault).
    let agents = create_default_sub_agents();
    let enumerator = agents.iter().find(|a| a.id == "enumerator").unwrap();

    // Active content enumeration + stage submission tools present.
    assert!(has_tool(enumerator, "enum_preflight_web_origins"));
    assert!(has_tool(enumerator, "enum_crawl_same_origin_urls"));
    assert!(!has_tool(enumerator, "pentest_run"));
    assert!(!has_tool(enumerator, "pentest_list_tools"));
    assert!(has_tool(enumerator, "wait_for_background_jobs"));
    assert!(has_tool(enumerator, "browser_collect_js_api"));
    assert!(!has_tool(enumerator, "js_collect"));
    assert!(has_tool(enumerator, "js_extract_apis"));
    assert!(has_tool(enumerator, "enum_reduce_parameters_v2"));
    assert!(has_tool(enumerator, "route_probe_paths"));
    assert!(has_tool(enumerator, "enum_review_coverage_v2"));
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
fn resolution_worker_has_no_browser_route_or_final_submit_tools() {
    let agents = create_default_sub_agents();
    let analyst = agents
        .iter()
        .find(|agent| agent.id == "resolution_analyst")
        .expect("resolution analyst is registered");
    assert!(analyst.readonly);
    assert_eq!(
        analyst.allowed_tools,
        [
            "enum_js_get_resolution_cluster",
            "enum_js_submit_resolution"
        ]
    );
    assert!(analyst.delegatable_agents.is_empty());
    assert!(!analyst.allowed_tools.iter().any(|tool| {
        tool.contains("browser") || tool.contains("route") || tool == "submit_stage_deliverable"
    }));
    assert!(analyst.system_prompt.contains("parameter_names"));
    assert!(analyst.system_prompt.contains("source_start_byte"));
    assert!(analyst.system_prompt.contains("Do not use invented fields"));
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
    assert!(prompt.contains("once per exact {target_id,target_url}"));
    assert!(prompt.contains("ENUMERATION_V2_SINGLE_EXACT_ORIGIN_REQUIRED"));
    assert!(prompt.contains("typed lane receipt"));
    assert!(prompt.contains("full root_url"));
    assert!(prompt.contains("do NOT call query_target_data per target"));
    assert!(prompt.contains("ai_assist"));
    assert!(prompt.contains("summary.ai_used"));
    assert!(prompt.contains("ai_dialogue"));
    assert!(prompt.contains("recipe"));
    assert!(prompt.contains("js_extract_apis"));
    assert!(prompt.contains("route_probe_paths"));
    assert!(prompt.contains("stage_worklist_status"));
    assert!(prompt.contains("stage_worklist_next"));
    assert!(prompt.contains("stage_worklist_next(prefer=[\"pending\",\"error\",\"partial\"])"));
    assert!(prompt.contains("ready_to_submit=true"));
    assert!(prompt.contains("enum_preflight_web_origins"));
    assert!(prompt.contains(
        "construct a fresh enum_preflight_web_origins origin object containing only {target_id,target_url}"
    ));
    assert!(prompt.contains(
        "do not pass root_url, base_url, unfinished_techniques, or the whole page object"
    ));
    assert!(!prompt.contains("exact_origin_page verbatim into enum_preflight_web_origins"));
    assert!(prompt.contains("Any HTTP response means reachable"));
    assert!(prompt.contains("non-empty arrays are rejected"));
    assert!(prompt.contains("coverage: []"));
    assert!(prompt.contains("at most 200 cells across at most 50 roots"));
    assert!(prompt.contains("deduplicate by exact origin"));
    assert!(prompt.contains("work_item_id"));
    assert!(prompt.contains("omit both max_runtime_ms and max_requests"));
    assert!(prompt.contains("also omit batch_max_runtime_ms"));
    assert!(prompt.contains("scheduling-start ceiling, not a cancellation deadline"));
    assert!(!prompt.contains("max_runtime_ms=60000"));
    assert!(!prompt.contains("max_requests=1000"));
    assert!(prompt.contains("batch_concurrency=4"));
    assert!(prompt.contains("omit wordlist_recursion_depth"));
    assert!(prompt.contains("explicit 1..6"));
    assert!(prompt.contains("root wordlist"));
    assert!(prompt.contains("candidate-generation limit remains non-terminal"));
    assert!(prompt.contains("request_limited_partial"));
    assert!(prompt.contains("timeout_partial / request_limited_partial are non-terminal"));
    assert!(prompt.contains("v8 checkpoint"));
    assert!(prompt.contains("automatic_retry_allowed=false"));
    assert!(prompt.contains("retry_exhausted_*"));
    assert!(prompt.contains(
        "all four exact-origin axes: GOLISH-ENUM-JS, GOLISH-ENUM-JSAPI, GOLISH-ENUM-DIR, and GOLISH-ENUM-PARAM"
    ));
    assert!(prompt.contains("Business rows are discovery context only"));
    assert!(prompt.contains("Do not hand-write found, empty, blocked, or not_applicable coverage"));
    assert!(prompt.contains("Do not call external directory tools"));
    assert!(prompt.contains("list_enumeration_web_roots"));
    assert!(prompt.contains("enum_crawl_same_origin_urls"));
    assert!(prompt.contains("do NOT call katana or pentest_run directly"));
    assert!(prompt.contains("ENUMERATION_RESOLUTION_RECEIPTS_REQUIRED"));
    assert!(prompt.contains("enumeration_resolution.v1"));
    assert!(!prompt.contains("DB cannot derive"));
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
fn test_vuln_scanner_has_only_the_three_formulaic_wrappers() {
    let agents = create_default_sub_agents();
    let vuln_scanner = agents.iter().find(|a| a.id == "vuln_scanner").unwrap();

    assert!(has_tool(vuln_scanner, "stage_worklist_status"));
    assert!(has_tool(vuln_scanner, "stage_worklist_next"));
    assert!(has_tool(vuln_scanner, "vuln_nuclei_general"));
    assert!(has_tool(vuln_scanner, "vuln_nuclei_fingerprint_targeted"));
    assert!(has_tool(vuln_scanner, "vuln_probe_anonymous_access"));
    assert!(has_tool(vuln_scanner, "list_recent_evidence"));
    assert!(has_tool(vuln_scanner, "check_stage_asset_coverage"));
    assert!(has_tool(vuln_scanner, "query_target_data"));
    assert!(has_tool(vuln_scanner, "submit_stage_deliverable"));
    assert!(!has_tool(vuln_scanner, "record_finding"));
    assert!(has_tool(vuln_scanner, "search_knowledge_base"));
    assert!(has_tool(vuln_scanner, "read_knowledge"));

    assert!(!has_tool(vuln_scanner, "pentest_run"));
    assert!(!has_tool(vuln_scanner, "pentest_list_tools"));
    assert!(!has_tool(vuln_scanner, "nuclei"));
    assert!(!has_tool(vuln_scanner, "wait_for_background_jobs"));
    assert!(!has_tool(vuln_scanner, "check_job"));
    assert!(!has_tool(vuln_scanner, "kill_job"));
    assert!(!has_tool(vuln_scanner, "list_in_scope_targets"));
    assert!(!has_tool(vuln_scanner, "list_attack_surface_seeds"));
    assert!(!has_tool(vuln_scanner, "manage_targets"));
    assert!(!has_tool(vuln_scanner, "run_pty_cmd"));
    assert!(!has_tool(vuln_scanner, "run_command"));
}

#[test]
fn test_vuln_scanner_prompt_is_wrapper_based() {
    let prompt = build_vuln_scanner_prompt();
    assert!(prompt.contains("vuln_triage"));
    assert!(prompt.contains("stage_worklist_status"));
    assert!(prompt.contains("stage_worklist_next"));
    assert!(prompt.contains("vuln_nuclei_general"));
    assert!(prompt.contains("vuln_nuclei_fingerprint_targeted"));
    assert!(prompt.contains("vuln_probe_anonymous_access"));
    assert!(prompt.contains("WSTG-ATHN-04"));
    assert!(prompt.contains("reviewed_endpoint_ids"));
    assert!(prompt.contains("selected_probes"));
    assert!(prompt.contains("query_values"));
    assert!(prompt.contains("eligible_endpoint_query_contracts"));
    assert!(prompt.contains("persisted_url_query_names"));
    assert!(prompt.contains("potentially sensitive endpoint"));
    assert!(prompt.contains("Do not blindly probe every endpoint"));
    assert!(
        prompt.contains("Do not pass per-endpoint URLs, methods, headers, cookies, tokens, bodies")
    );
    assert!(prompt.contains("GOLISH-NDAY"));
    assert!(prompt.contains("foreground"));
    assert!(prompt.contains("check_stage_asset_coverage"));
    assert!(prompt.contains("ready_to_submit=true"));
    assert!(prompt.contains("Do NOT call pentest_run"));
    assert!(prompt.contains("nuclei"));
    assert!(!prompt.contains("wait_for_background_jobs"));
    assert!(prompt.contains("The gate reads the DATABASE"));
    assert!(!prompt.contains("list_in_scope_targets first"));
}

#[test]
fn candidate_verifier_has_exact_closed_tool_surface() {
    let agents = create_default_sub_agents();
    let verifier = agents
        .iter()
        .find(|agent| agent.id == "candidate_verifier")
        .unwrap();
    assert_eq!(
        verifier.allowed_tools,
        vec![
            "verify_execute_candidate_action",
            "list_recent_evidence",
            "submit_candidate_attempt",
        ]
    );
    assert!(verifier.delegatable_agents.is_empty());
    for forbidden in [
        "pentest_run",
        "record_finding",
        "vuln_nuclei_general",
        "vuln_nuclei_fingerprint_targeted",
        "wait_for_background_jobs",
        "check_job",
        "kill_job",
        "run_pty_cmd",
    ] {
        assert!(!has_tool(verifier, forbidden));
    }

    let prompt = build_candidate_verifier_prompt();
    assert!(prompt.contains("action_ordinal"));
    assert!(prompt.contains("foreground"));
    assert!(prompt.contains("outcome_unknown"));
}

#[test]
fn attack_analyst_is_reasoning_only_and_distinct_from_verifier() {
    let agents = create_default_sub_agents();
    let analyst = agents
        .iter()
        .find(|agent| agent.id == "attack_analyst")
        .unwrap();
    assert!(has_tool(analyst, "query_target_data"));
    assert!(has_tool(analyst, "list_recent_evidence"));
    assert!(has_tool(analyst, "submit_stage_deliverable"));
    assert!(!has_tool(analyst, "verify_execute_candidate_action"));
    assert!(!has_tool(analyst, "pentest_run"));
    assert!(!has_tool(analyst, "record_finding"));
    assert!(build_attack_analyst_prompt().contains("reasoning-only"));
    assert!(build_attack_analyst_prompt().contains("first response MUST call"));
    assert!(build_attack_analyst_prompt().contains("nuclei_template_ids"));
}

#[test]
fn candidate_specialist_prompts_do_not_own_the_durable_wave_cursor() {
    let analyst = build_attack_analyst_prompt();
    for required in [
        "initial vuln_triage_handoff",
        "follow-on fact_delta_consolidation",
        "zero-input",
        "durable Candidate review",
    ] {
        assert!(
            analyst.contains(required),
            "missing analyst contract: {required}"
        );
    }
    assert!(analyst.contains("never decides or opens the next Wave"));

    let verifier = build_candidate_verifier_prompt();
    for required in [
        "FactDelta proposals",
        "canonical ref/version/hash",
        "opened_next_wave",
        "closed_no_delta",
        "exhausted",
        "residual risk",
    ] {
        assert!(
            verifier.contains(required),
            "missing verifier contract: {required}"
        );
    }
    assert!(verifier.contains("never accepts, consumes, or opens"));
}

#[test]
fn candidate_hypothesis_team_has_exact_tool_free_readonly_surface() {
    let agents = create_default_sub_agents();
    let expected = [
        "candidate_hypothesis_controller",
        "candidate_hypothesis_analyst",
        "merge_conflict_critic",
    ];

    for id in expected {
        let agent = agents
            .iter()
            .find(|candidate| candidate.id == id)
            .unwrap_or_else(|| panic!("missing Candidate analysis role {id}"));
        assert!(agent.readonly, "{id} must be read-only");
        assert_eq!(agent.allowed_tools, ["submit_result"]);
        assert!(agent.delegatable_agents.is_empty());
        assert_eq!(agent.max_iterations, 8);
        assert_eq!(agent.idle_timeout_secs, Some(180));
        for forbidden in [
            "run_pty_cmd",
            "pentest_run",
            "web_search",
            "web_fetch",
            "browser_collect_js_api",
            "search_knowledge_base",
            "read_knowledge",
            "write_knowledge",
            "ingest_cve",
            "feed_refresh",
        ] {
            assert!(!has_tool(agent, forbidden), "{id} exposes {forbidden}");
        }
    }

    let controller = agents
        .iter()
        .find(|agent| agent.id == "candidate_hypothesis_controller")
        .expect("controller");
    assert!(controller.system_prompt.contains("unique final submitter"));
    assert!(controller
        .system_prompt
        .contains("instruction_authority=false"));

    let analyst = build_candidate_hypothesis_analyst_prompt();
    for required in [
        "closed frozen input",
        "instruction_authority=false",
        "knowledge_signal",
        "unknown product version",
        "stale feed",
        "must not claim proof or refutation",
        "must not invent identity or hash",
    ] {
        assert!(
            analyst.contains(required),
            "missing analyst rule: {required}"
        );
    }
}

#[test]
fn verification_campaign_team_has_exact_closed_tool_surface() {
    let agents = create_default_sub_agents();
    let expected = crate::executor::verification_campaign::VERIFICATION_CAMPAIGN_ROLE_IDS;
    for id in expected {
        let agent = agents
            .iter()
            .find(|candidate| candidate.id == id)
            .unwrap_or_else(|| panic!("missing Verification Campaign role {id}"));
        assert!(agent.readonly, "{id} must be read-only");
        assert_eq!(agent.allowed_tools, ["submit_result"]);
        assert!(agent.delegatable_agents.is_empty());
        for forbidden in [
            "verify_execute_candidate_action",
            "pentest_run",
            "record_finding",
            "web_search",
            "web_fetch",
            "browser_collect_js_api",
            "write_knowledge",
        ] {
            assert!(!has_tool(agent, forbidden), "{id} exposes {forbidden}");
        }
        for forbidden_text in ["Authorization:", "Cookie:", "curl ", "raw body"] {
            assert!(
                !agent.system_prompt.contains(forbidden_text),
                "{id} prompt contains executable/secret material {forbidden_text}"
            );
        }
    }
}

#[test]
fn merge_conflict_critic_prompt_closes_review_and_synthesis_schemas() {
    let prompt = build_merge_conflict_critic_prompt();
    for required in [
        "proposal_conflict_review.v1",
        "hypothesis_coverage_subreview.v1",
        "hypothesis_coverage_synthesis.v1",
        "attack-class",
        "trust-boundary",
        "second",
        "third",
        "designated immutable chunks",
        "cross_chunk",
        "cross_input_partition",
        "cross_input_reduce",
        "cross_dimension_reduce",
        "global_semantic_root",
        "transitive descendant-worker set",
        "page receipt is not proof of understanding",
        "sampling_omitted",
        "blocked",
        "degraded",
    ] {
        assert!(prompt.contains(required), "missing critic rule: {required}");
    }
    for forbidden_claim in [
        "page receipt proves understanding",
        "adequate means complete security coverage",
        "knowledge_signal is proof",
    ] {
        assert!(!prompt.contains(forbidden_claim));
    }
}

#[test]
fn post_exploit_operator_has_only_closed_stage_wrappers() {
    let agents = create_default_sub_agents();
    let operator = agents
        .iter()
        .find(|agent| agent.id == "post_exploit_operator")
        .unwrap();
    for tool in [
        "post_exploit_validate_access",
        "post_exploit_record_internal_observation",
        "post_exploit_build_objective_path",
        "post_exploit_execute_action",
        "list_recent_evidence",
        "submit_stage_deliverable",
    ] {
        assert!(has_tool(operator, tool), "missing {tool}");
    }
    for forbidden in [
        "pentest_run",
        "run_command",
        "run_pty_cmd",
        "record_finding",
        "wait_for_background_jobs",
        "check_job",
        "kill_job",
        "sub_agent_pentester",
    ] {
        assert!(!has_tool(operator, forbidden), "leaked {forbidden}");
    }
    let prompt = build_post_exploit_operator_prompt();
    assert!(prompt.contains("approval_required"));
    assert!(prompt.contains("never bypass or blindly replay"));
}

#[test]
fn test_browser_prompt_prefers_browser_closure_collection() {
    let prompt = build_browser_prompt();
    assert!(prompt.contains("browser_collect_js_api"));
    assert!(prompt.contains("Use `browser_collect_js_api` first"));
    assert!(prompt.contains("crawl_mode=\"standard\""));
    assert!(prompt.contains("ai_assist=false"));
    assert!(prompt.contains("same standard mode"));
    assert!(prompt.contains("bounded `recipe`"));
    assert!(prompt.contains("js_extract_apis"));
    assert!(prompt.contains("browser_collect_js_api` is the collector"));
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
    assert!(prompt.contains("yield-time-ms"));
    assert!(prompt.contains("check_job"));
    assert!(prompt.contains("kill_job"));
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

#[test]
fn test_orchestrator_prompt_respects_stage_run_override() {
    let prompt = build_orchestrator_prompt();
    assert!(prompt.contains("Stage-specialist override"));
    assert!(prompt.contains("stage_run"));
    assert!(prompt.contains("Do not call `sub_agent_pentester`"));
    assert!(prompt.contains("outside active stage-specialist delegation"));
}
