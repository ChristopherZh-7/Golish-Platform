//! Constructors that assemble the default [`SubAgentDefinition`] catalogue.
//!
//! Two flavours:
//! - [`create_default_sub_agents`]: uses the hardcoded prompts from
//!   [`super::prompts`] directly.
//! - [`create_default_sub_agents_from_registry`]: prefers prompts from the
//!   template registry (`prompts/*.tera` or DB overrides), falling back to
//!   hardcoded prompts on render failure.

mod registry;

pub use registry::create_default_sub_agents_from_registry;

use crate::definition::SubAgentDefinition;

use super::prompts::{
    build_adviser_prompt, build_application_understanding_company_synthesizer_prompt,
    build_application_understanding_shard_modeler_prompt, build_attack_analyst_prompt,
    build_browser_prompt, build_candidate_hypothesis_analyst_prompt,
    build_candidate_hypothesis_controller_prompt, build_candidate_verifier_prompt,
    build_coder_prompt, build_enricher_prompt, build_enumerator_prompt, build_installer_prompt,
    build_memorist_prompt, build_merge_conflict_critic_prompt, build_orchestrator_prompt,
    build_pentester_prompt, build_planner_prompt, build_post_exploit_operator_prompt,
    build_prober_prompt, build_recon_prompt, build_refiner_prompt, build_reflector_prompt,
    build_reporter_prompt, build_researcher_prompt, build_resolution_analyst_prompt,
    build_verification_campaign_prompt, build_vuln_scanner_prompt, VerificationCampaignRole,
};

pub(super) fn verification_campaign_agent_definitions() -> Vec<SubAgentDefinition> {
    use VerificationCampaignRole::*;
    [
        ("verification_lead", "Verification Lead", Lead),
        (
            "verification_pentester",
            "Verification Pentester",
            Pentester,
        ),
        (
            "verification_researcher",
            "Verification Researcher",
            Researcher,
        ),
        (
            "verification_poc_designer",
            "Verification PoC Designer",
            PocDesigner,
        ),
        (
            "verification_auth_specialist",
            "Verification Auth Specialist",
            AuthSpecialist,
        ),
        (
            "verification_api_specialist",
            "Verification API Specialist",
            ApiSpecialist,
        ),
        (
            "verification_business_logic_specialist",
            "Verification Business Logic Specialist",
            BusinessLogicSpecialist,
        ),
        (
            "verification_injection_specialist",
            "Verification Injection Specialist",
            InjectionSpecialist,
        ),
        (
            "verification_evidence_analyst",
            "Verification Evidence Analyst",
            EvidenceAnalyst,
        ),
        (
            "verification_independent_critic",
            "Verification Independent Critic",
            IndependentCritic,
        ),
        ("verification_refiner", "Verification Refiner", Refiner),
        ("verification_adviser", "Verification Adviser", Adviser),
        (
            "verification_reflector",
            "Verification Reflector",
            Reflector,
        ),
    ]
    .into_iter()
    .map(|(id, name, role)| {
        SubAgentDefinition::new(
            id,
            name,
            "Closed, durable Verification Campaign reasoning role; no execution authority.",
            build_verification_campaign_prompt(role),
        )
        .with_tools(vec!["submit_result".to_string()])
        .with_readonly(true)
        .with_max_iterations(8)
        .with_idle_timeout(180)
    })
    .collect()
}

/// Create default sub-agents for common tasks.
pub fn create_default_sub_agents() -> Vec<SubAgentDefinition> {
    let mut agents = vec![
        SubAgentDefinition::new(
            "coder",
            "Coder",
            "Applies surgical code edits using unified diff format. Use for precise multi-hunk edits. Outputs standard git-style diffs that are parsed and applied automatically.",
            build_coder_prompt(),
        )
        .with_tools(vec![
            "read_file".to_string(),
            "list_files".to_string(),
            "grep_file".to_string(),
            "ast_grep".to_string(),
            "ast_grep_replace".to_string(),
        ])
        .with_max_iterations(20)
        .with_idle_timeout(180),
        SubAgentDefinition::new(
            "researcher",
            "Research Agent",
            "Researches topics by reading documentation, searching the web, and gathering information. Use this agent when you need to understand APIs, libraries, or gather external information.",
            build_researcher_prompt(),
        )
        .with_tools(vec![
            "web_search".to_string(),
            "web_fetch".to_string(),
            "read_file".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
            "write_knowledge".to_string(),
            "ingest_cve".to_string(),
            "save_poc".to_string(),
            "list_cves_with_pocs".to_string(),
            "list_unresearched_cves".to_string(),
        ])
        .with_max_iterations(25)
        .with_idle_timeout(180)
        .with_delegatable_agents(vec!["memorist".into()]),
        SubAgentDefinition::new(
            "installer",
            "Installer",
            "Tool installation and environment configuration specialist. Handles downloading, compiling, and configuring penetration testing tools. Manages Python virtual environments, Go builds, and dependency conflicts. Delegate when a tool needs to be installed or a complex environment needs setup.",
            build_installer_prompt(),
        )
        .with_tools(vec![
            "read_file".into(),
            "write_file".into(),
            "web_fetch".into(),
            "list_directory".into(),
            "list_files".into(),
            "grep_file".into(),
            "pentest_list_tools".into(),
            "pentest_run".into(),
            "check_job".into(),
            "kill_job".into(),
        ])
        .with_max_iterations(30)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["researcher".into(), "memorist".into()]),
        SubAgentDefinition::new(
            "pentester",
            "Pentester",
            "Penetration testing specialist for security assessments. Handles network scanning, web app testing, vulnerability assessment, and exploitation. Delegate security-related tasks that require tool expertise (nmap, gobuster, sqlmap, etc.).",
            build_pentester_prompt(),
        )
        .with_tools(vec![
            "read_file".to_string(),
            "write_file".to_string(),
            "web_fetch".to_string(),
            "web_search".to_string(),
            "list_directory".to_string(),
            "list_files".to_string(),
            "grep_file".to_string(),
            "search_memories".to_string(),
            "manage_targets".to_string(),
            "recon_list_providers".to_string(),
            "recon_discover_subsidiaries".to_string(),
            "recon_map_assets".to_string(),
            "recon_lookup_whois".to_string(),
            "record_finding".to_string(),
            "vault".to_string(),
            "pentest_list_tools".to_string(),
            "pentest_run".to_string(),
            "check_job".to_string(),
            "kill_job".to_string(),
            "list_recent_evidence".to_string(),
            "graph_search".to_string(),
            "graph_add_entity".to_string(),
            "graph_add_relation".to_string(),
            "graph_attack_paths".to_string(),
            "search_exploits".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(50)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec![
            "coder".to_string(),
            "researcher".to_string(),
            "memorist".to_string(),
            "installer".to_string(),
            "enricher".to_string(),
            "browser".to_string(),
        ]),
        SubAgentDefinition::new(
            "recon",
            "Recon",
            "Passive target-intelligence collector for the target_intel stage. Enriches one organization's footprint via providers (0.zone/quake/ENScan) plus passive subdomain + URL history. Deterministic backend landing registers only authorized domain identities; DNS/provider IP observations remain relationships, not scan targets. ZERO-TOUCH: no live probing or exploitation — that stays with the Pentester. The stage_run tool fans one Recon out per org.",
            build_recon_prompt(),
        )
        .with_tools(vec![
            "recon_list_providers".to_string(),
            "recon_discover_subsidiaries".to_string(),
            "recon_map_assets".to_string(),
            "recon_lookup_whois".to_string(),
            "check_stage_asset_coverage".to_string(),
            "stage_worklist_status".to_string(),
            "stage_worklist_next".to_string(),
            "query_target_data".to_string(),
            "submit_stage_deliverable".to_string(),
            "record_finding".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(40)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["enricher".to_string(), "memorist".to_string()]),
        SubAgentDefinition::new(
            "investigation",
            "Investigation Primary",
            "Organization-isolated Investigation Primary. It plans bounded analysis and verification cognition, dynamically delegates specialists, and submits typed strategy and lineage; host-owned operators alone perform external actions.",
            format!(
                "{}\n\n{}",
                build_orchestrator_prompt(),
                "You are the unique Primary for one organization-bound Investigation task. Use update_plan to create and revise a bounded plan, and dynamically delegate only the specialists needed for current evidence gaps. Workers may return strategy, evidence interpretation, and typed action intent only. Neither you nor nested workers may directly perform HTTP, browser, CLI, credential, pentest, action execution, Finding writes, or canonical hypothesis mutation. Database facts and Evidence Ledger are authoritative; RAG/KG/methodology are advisory. A hypothesis click is observe-only and cannot schedule work. Finish only through typed host receipts and the deterministic Investigation gate."
            ),
        )
        .with_tools(vec![
            "update_plan".to_string(),
            "query_target_data".to_string(),
            "list_in_scope_targets".to_string(),
            "list_recent_evidence".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
            "graph_search".to_string(),
            "graph_neighbors".to_string(),
            "graph_attack_paths".to_string(),
            "submit_stage_deliverable".to_string(),
        ])
        .with_readonly(true)
        .with_max_iterations(50)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec![
            "pentester".to_string(),
            "researcher".to_string(),
            "browser".to_string(),
            "coder".to_string(),
            "installer".to_string(),
            "enricher".to_string(),
            "memorist".to_string(),
            "adviser".to_string(),
        ]),
        SubAgentDefinition::new(
            "resolution_analyst",
            "Resolution Analyst",
            "Bounded evidence-anchored analyst for one server-assigned unresolved JS/API cluster. It cannot browse, probe, read arbitrary files, publish canonical truth, or submit a stage deliverable.",
            build_resolution_analyst_prompt(),
        )
        .with_tools(vec![
            "enum_js_get_resolution_cluster".to_string(),
            "enum_js_submit_resolution".to_string(),
        ])
        .with_readonly(true)
        .with_max_iterations(4)
        .with_idle_timeout(120),
        SubAgentDefinition::new(
            "attack_analyst",
            "Attack Analyst",
            "Reasoning-only attack_candidate specialist. Produces bounded Candidate plans from durable facts and evidence; it never executes verification actions.",
            build_attack_analyst_prompt(),
        )
        .with_tools(vec![
            "query_target_data".to_string(),
            "list_recent_evidence".to_string(),
            "submit_stage_deliverable".to_string(),
        ])
        .with_max_iterations(30)
        .with_idle_timeout(180),
        SubAgentDefinition::new(
            "application_understanding_shard_modeler",
            "Application Understanding Shard Modeler",
            "Closed semantic modeler for one host-frozen application shard. It cannot collect data or cross identity boundaries.",
            build_application_understanding_shard_modeler_prompt(),
        )
        .with_tools(vec!["submit_result".to_string()])
        .with_readonly(true)
        .with_max_iterations(8)
        .with_max_tokens(32_768)
        .with_idle_timeout(180),
        SubAgentDefinition::new(
            "application_understanding_company_synthesizer",
            "Application Understanding Company Synthesizer",
            "Closed company-level synthesizer over host-validated shard outputs. It cannot collect data or cross organization boundaries.",
            build_application_understanding_company_synthesizer_prompt(),
        )
        .with_tools(vec!["submit_result".to_string()])
        .with_readonly(true)
        .with_max_iterations(8)
        .with_max_tokens(32_768)
        .with_idle_timeout(180),
        SubAgentDefinition::new(
            "candidate_hypothesis_controller",
            "Candidate Hypothesis Controller",
            "Read-only Controller over one server-frozen Candidate snapshot; the unique final submitter for the closed analysis team.",
            build_candidate_hypothesis_controller_prompt(),
        )
        .with_tools(vec!["submit_result".to_string()])
        .with_readonly(true)
        .with_max_iterations(8)
        .with_idle_timeout(180),
        SubAgentDefinition::new(
            "candidate_hypothesis_analyst",
            "Candidate Hypothesis Analyst",
            "Read-only analyst over one server-frozen Candidate microbatch.",
            build_candidate_hypothesis_analyst_prompt(),
        )
        .with_tools(vec!["submit_result".to_string()])
        .with_readonly(true)
        .with_max_iterations(8)
        .with_idle_timeout(180),
        SubAgentDefinition::new(
            "merge_conflict_critic",
            "Merge Conflict Critic",
            "Read-only critic for closed proposal-conflict, coverage-subreview, and synthesis artifacts.",
            build_merge_conflict_critic_prompt(),
        )
        .with_tools(vec!["submit_result".to_string()])
        .with_readonly(true)
        .with_max_iterations(8)
        .with_idle_timeout(180),
        SubAgentDefinition::new(
            "candidate_verifier",
            "Candidate Verifier",
            "Foreground-only verifier for one scheduler-bound CandidateAttempt. Executes only canonical DB-authorized action ordinals.",
            build_candidate_verifier_prompt(),
        )
        .with_tools(vec![
            "verify_execute_candidate_action".to_string(),
            "list_recent_evidence".to_string(),
            "submit_candidate_attempt".to_string(),
        ])
        .with_max_iterations(30)
        .with_idle_timeout(300),
        SubAgentDefinition::new(
            "post_exploit_operator",
            "Post-Exploit Operator",
            "Lease-fenced specialist for the four typed Post-Exploit V2 stages. It records canonical facts and prepares cleanup-bound actions without raw shell or scanner access.",
            build_post_exploit_operator_prompt(),
        )
        .with_tools(vec![
            "query_target_data".to_string(),
            "list_recent_evidence".to_string(),
            "post_exploit_validate_access".to_string(),
            "post_exploit_record_internal_observation".to_string(),
            "post_exploit_build_objective_path".to_string(),
            "post_exploit_execute_action".to_string(),
            "submit_stage_deliverable".to_string(),
        ])
        .with_max_iterations(30)
        .with_idle_timeout(300),
        SubAgentDefinition::new(
            "prober",
            "Prober",
            "Active external-attack-surface mapper for the external_attack_surface stage. Turns one organization's passively-discovered footprint (inherited from target_intel) into a confirmed attack surface — domain/URL liveness, concrete IP/CIDR open ports first, service/version fingerprints for every open IP:port, and WhatWeb fingerprints for confirmed web origins — by actively but lightly probing the target. NON-EXPLOIT: no exploitation or vulnerability scanning — that stays with the Pentester. The stage_run tool fans one Prober out per org.",
            build_prober_prompt(),
        )
        .with_tools(vec![
            "list_in_scope_targets".to_string(),
            "list_attack_surface_seeds".to_string(),
            "query_target_data".to_string(),
            // Active EAS probing is exposed as backend-owned capability wrappers
            // so the model chooses business actions, not raw CLI flags.
            "eas_probe_http_liveness".to_string(),
            "eas_discover_ports".to_string(),
            "eas_fingerprint_services".to_string(),
            "eas_fingerprint_web_stack".to_string(),
            "wait_for_background_jobs".to_string(),
            "check_job".to_string(),
            "kill_job".to_string(),
            "list_recent_evidence".to_string(),
            "submit_stage_deliverable".to_string(),
            "record_finding".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(40)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["enricher".to_string(), "memorist".to_string()]),
        SubAgentDefinition::new(
            "enumerator",
            "Enumerator",
            "Active content-enumeration mapper for the enumeration stage. Turns the live web services external_attack_surface mapped (host + ports + service) into concrete testable units by enumerating directories/paths, request parameters, and JS/API endpoints — actively but without exploitation. NON-EXPLOIT: no vulnerability scanning or exploitation — that stays with the Pentester. The stage_run tool fans one Enumerator out per org.",
            build_enumerator_prompt(),
        )
        .with_tools(vec![
            "stage_worklist_status".to_string(),
            "stage_worklist_next".to_string(),
            "list_enumeration_web_roots".to_string(),
            "query_target_data".to_string(),
            "enum_preflight_web_origins".to_string(),
            // Active content enumeration. Katana is exposed through a backend
            // wrapper so the model chooses browser seed discovery, not raw CLI flags.
            "enum_crawl_same_origin_urls".to_string(),
            "wait_for_background_jobs".to_string(),
            // JS/API extraction (GOLISH-ENUM-JSAPI): collect a host's JS, then pull
            // endpoints/paths out of it.
            "browser_collect_js_api".to_string(),
            "js_extract_apis".to_string(),
            "enum_reduce_parameters_v2".to_string(),
            "route_probe_paths".to_string(),
            "enum_review_coverage_v2".to_string(),
            "list_recent_evidence".to_string(),
            "submit_stage_deliverable".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(40)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["enricher".to_string(), "memorist".to_string()]),
        SubAgentDefinition::new(
            "vuln_scanner",
            "Vuln Scanner",
            "Formulaic vulnerability-observation specialist for the vuln_triage stage. Closes WSTG/GOLISH cells through two guarded Nuclei wrappers plus one server-owned anonymous-access wrapper rather than raw CLI/request commands. The stage_run tool fans one Vuln Scanner out per org.",
            build_vuln_scanner_prompt(),
        )
        .with_tools(vec![
            "stage_worklist_status".to_string(),
            "stage_worklist_next".to_string(),
            "query_target_data".to_string(),
            "vuln_nuclei_general".to_string(),
            "vuln_nuclei_fingerprint_targeted".to_string(),
            "vuln_probe_anonymous_access".to_string(),
            "list_recent_evidence".to_string(),
            "check_stage_asset_coverage".to_string(),
            "submit_stage_deliverable".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(40)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["enricher".to_string(), "memorist".to_string()]),
        SubAgentDefinition::new(
            "memorist",
            "Memorist",
            "Memory management agent for long-term knowledge persistence. Call after significant findings to store them, or before new tasks to retrieve relevant past context. Handles deduplication, categorization, and semantic search across session history.",
            build_memorist_prompt(),
        )
        .with_tools(vec![
            "search_memories".to_string(),
            "store_memory".to_string(),
            "list_memories".to_string(),
            "graph_add_entity".to_string(),
            "graph_add_relation".to_string(),
            "graph_search".to_string(),
            "graph_neighbors".to_string(),
            "graph_attack_paths".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(10)
        .with_idle_timeout(60),
        SubAgentDefinition::new(
            "planner",
            "Planner",
            "Task decomposition agent. Given a complex request, produces 3-7 ordered subtasks with agent assignments and dependencies. Use when the user's request requires multiple steps across different specializations. Returns a JSON execution plan.",
            build_planner_prompt(),
        )
        .with_tools(vec!["search_memories".to_string()])
        .with_max_iterations(5)
        .with_idle_timeout(60),
        SubAgentDefinition::new(
            "reflector",
            "Reflector",
            "Correction agent invoked automatically when another agent fails to produce tool calls. Diagnoses why the agent is stuck and provides a corrective instruction. Not for direct invocation — triggered by the execution loop.",
            build_reflector_prompt(),
        )
        .with_tools(vec![])
        .with_max_iterations(3)
        .with_idle_timeout(30)
        .as_pipeline_only(),
        SubAgentDefinition::new(
            "adviser",
            "Adviser",
            "Security expert consultant for complex findings. Delegate to this agent when a vulnerability or configuration requires deeper analysis, risk assessment, or when the pentester needs guidance on exploitation strategy, prioritization, or remediation recommendations.",
            build_adviser_prompt(),
        )
        .with_tools(vec![
            "web_search".to_string(),
            "web_fetch".to_string(),
            "read_file".to_string(),
            "search_memories".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(15)
        .with_idle_timeout(120)
        .with_delegatable_agents(vec!["researcher".into(), "memorist".into()]),
        SubAgentDefinition::new(
            "reporter",
            "Reporter",
            "Generates structured security assessment reports. Delegate to this agent after scanning or penetration testing is complete. It reads findings from memory, organizes them by severity, and produces reports in standard formats (OWASP, executive summary).",
            build_reporter_prompt(),
        )
        .with_tools(vec![
            "read_file".to_string(),
            "search_memories".to_string(),
            "list_memories".to_string(),
            "write_file".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
            "list_cves_with_pocs".to_string(),
            "poc_stats".to_string(),
        ])
        .with_max_iterations(20)
        .with_idle_timeout(180)
        .with_delegatable_agents(vec!["memorist".into()]),
        SubAgentDefinition::new(
            "refiner",
            "Refiner",
            "Task plan refinement agent. Called after each subtask completes to evaluate progress and adjust the remaining plan. Can add, remove, modify, or reorder subtasks based on new discoveries. Not for direct invocation — triggered by the task orchestrator.",
            build_refiner_prompt(),
        )
        .with_tools(vec![
            "search_memories".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(5)
        .with_idle_timeout(60)
        .as_pipeline_only(),
        SubAgentDefinition::new(
            "browser",
            "Browser",
            "Web browser and JavaScript analysis specialist. Handles JS file collection, web content extraction, and browser-based reconnaissance. Delegate when you need to collect and analyze JavaScript files from a target, or perform deeper web interaction beyond simple HTTP fetching.",
            build_browser_prompt(),
        )
        .with_tools(vec![
            "browser_collect_js_api".to_string(),
            "js_extract_apis".to_string(),
            "web_fetch".to_string(),
            "web_search".to_string(),
            "read_file".to_string(),
            "write_file".to_string(),
            "grep_file".to_string(),
            "record_finding".to_string(),
        ])
        .with_max_iterations(20)
        .with_idle_timeout(120),
        SubAgentDefinition::new(
            "enricher",
            "Enricher",
            "Context enrichment specialist. Gathers supplementary information from memory, knowledge base, and knowledge graph before or during task execution. Delegate to this agent when another agent needs additional context about targets, vulnerabilities, or past findings to perform better.",
            build_enricher_prompt(),
        )
        .with_tools(vec![
            "search_memories".to_string(),
            "read_file".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
            "graph_search".to_string(),
            "graph_neighbors".to_string(),
            "graph_attack_paths".to_string(),
            "search_exploits".to_string(),
            "list_cves_with_pocs".to_string(),
        ])
        .with_max_iterations(10)
        .with_idle_timeout(60),
        SubAgentDefinition::new(
            "orchestrator",
            "Orchestrator",
            "Primary task coordinator and team orchestration manager. Analyzes complex tasks, breaks them into subtasks, and delegates to specialist agents. Manages the overall workflow, integrates results, and ensures task completion. The top-level agent for task mode execution.",
            build_orchestrator_prompt(),
        )
        .with_tools(vec![
            "update_plan".to_string(),
            "search_memories".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
            "query_target_data".to_string(),
            "list_in_scope_targets".to_string(),
        ])
        .with_max_iterations(50)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec![
            "researcher".into(),
            "pentester".into(),
            "coder".into(),
            "memorist".into(),
            "installer".into(),
            "adviser".into(),
            "reporter".into(),
            "enricher".into(),
            "browser".into(),
        ])
        .as_pipeline_only(),
    ];
    agents.extend(verification_campaign_agent_definitions());
    agents
}
