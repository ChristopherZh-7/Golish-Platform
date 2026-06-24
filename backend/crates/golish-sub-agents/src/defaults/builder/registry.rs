//! Template registry-based sub-agent construction.

use crate::definition::SubAgentDefinition;
use crate::schemas::IMPLEMENTATION_PLAN_FULL_EXAMPLE;

use super::super::prompts::{
    build_adviser_prompt, build_browser_prompt, build_coder_prompt, build_enricher_prompt,
    build_enumerator_prompt, build_installer_prompt, build_memorist_prompt,
    build_orchestrator_prompt, build_pentester_prompt, build_planner_prompt, build_prober_prompt,
    build_recon_prompt, build_refiner_prompt, build_reflector_prompt, build_reporter_prompt,
    build_researcher_prompt_fallback,
};

/// Create default sub-agents with prompts loaded from the template registry.
///
/// This is the preferred constructor — it uses templates from `prompts/*.tera`
/// (or DB overrides loaded into the registry) instead of hardcoded strings.
/// Falls back to hardcoded prompts if template rendering fails.
pub async fn create_default_sub_agents_from_registry(
    registry: &crate::prompt_registry::PromptRegistry,
) -> Vec<SubAgentDefinition> {
    let ctx = crate::prompt_registry::PromptContext::new().set(
        "implementation_plan_example",
        IMPLEMENTATION_PLAN_FULL_EXAMPLE,
    );

    let mut agents = Vec::new();

    macro_rules! tmpl_or_fallback {
        ($name:expr, $fallback:expr) => {
            match registry.render($name, &ctx).await {
                Ok(rendered) => rendered,
                Err(e) => {
                    tracing::warn!(
                        "[defaults] Template '{}' render failed, using hardcoded: {e}",
                        $name
                    );
                    $fallback
                }
            }
        };
    }

    agents.push(
        SubAgentDefinition::new(
            "coder", "Coder",
            "Applies surgical code edits using unified diff format. Use for precise multi-hunk edits. Outputs standard git-style diffs that are parsed and applied automatically.",
            tmpl_or_fallback!("coder", build_coder_prompt()),
        )
        .with_tools(vec!["read_file".into(), "list_files".into(), "grep_file".into(), "ast_grep".into(), "ast_grep_replace".into()])
        .with_max_iterations(20).with_idle_timeout(180),
    );

    agents.push(
        SubAgentDefinition::new(
            "researcher", "Research Agent",
            "Researches topics by reading documentation, searching the web, and gathering information.",
            tmpl_or_fallback!("researcher", build_researcher_prompt_fallback()),
        )
        .with_tools(vec![
            "web_search".into(), "web_fetch".into(), "read_file".into(),
            "search_knowledge_base".into(), "read_knowledge".into(), "write_knowledge".into(),
            "ingest_cve".into(), "save_poc".into(), "list_cves_with_pocs".into(),
            "list_unresearched_cves".into(),
        ])
        .with_max_iterations(25).with_idle_timeout(180)
        .with_delegatable_agents(vec!["memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "installer", "Installer",
            "Tool installation and environment configuration specialist. Handles downloading, compiling, and configuring penetration testing tools. Manages Python virtual environments, Go builds, and dependency conflicts. Delegate when a tool needs to be installed or a complex environment needs setup.",
            tmpl_or_fallback!("installer", build_installer_prompt()),
        )
        .with_tools(vec!["read_file".into(), "write_file".into(), "web_fetch".into(), "list_directory".into(), "list_files".into(), "grep_file".into(), "pentest_list_tools".into(), "pentest_run".into()])
        .with_max_iterations(30).with_idle_timeout(300)
        .with_delegatable_agents(vec!["researcher".into(), "memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "pentester",
            "Pentester",
            "Penetration testing specialist for security assessments.",
            tmpl_or_fallback!("pentester", build_pentester_prompt()),
        )
        .with_tools(vec![
            "read_file".into(),
            "write_file".into(),
            "web_fetch".into(),
            "web_search".into(),
            "list_directory".into(),
            "list_files".into(),
            "grep_file".into(),
            "search_memories".into(),
            "manage_targets".into(),
            "list_in_scope_targets".into(),
            "recon_list_providers".into(),
            "recon_discover_subsidiaries".into(),
            "recon_map_assets".into(),
            "recon_lookup_whois".into(),
            "record_finding".into(),
            "vault".into(),
            "js_extract_apis".into(),
            "auth_probe".into(),
            "pentest_list_tools".into(),
            "pentest_run".into(),
            "graph_search".into(),
            "graph_add_entity".into(),
            "graph_add_relation".into(),
            "graph_attack_paths".into(),
            "search_exploits".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
        ])
        .with_max_iterations(50)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec![
            "coder".into(),
            "researcher".into(),
            "memorist".into(),
            "installer".into(),
            "enricher".into(),
            "browser".into(),
        ]),
    );

    agents.push(
        SubAgentDefinition::new(
            "recon",
            "Recon",
            "Passive target-intelligence collector for the target_intel stage. Enriches one organization's footprint via providers (0.zone/quake/ENScan) plus passive subdomain + URL history, and registers discovered assets as in-scope targets. ZERO-TOUCH: no live probing or exploitation — that stays with the Pentester. The stage_run tool fans one Recon out per org.",
            tmpl_or_fallback!("recon", build_recon_prompt()),
        )
        .with_tools(vec![
            "recon_list_providers".into(),
            "recon_discover_subsidiaries".into(),
            "recon_map_assets".into(),
            "recon_lookup_whois".into(),
            "manage_targets".into(),
            "submit_stage_deliverable".into(),
            "record_finding".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
        ])
        .with_max_iterations(40)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["enricher".into(), "memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "prober",
            "Prober",
            "Active external-attack-surface mapper for the external_attack_surface stage. Turns one organization's passively-discovered footprint (inherited from target_intel) into a confirmed attack surface — liveness (httpx), open ports (naabu/masscan), and service/version fingerprints — by actively but lightly probing the target. NON-EXPLOIT: no exploitation or vulnerability scanning — that stays with the Pentester. The stage_run tool fans one Prober out per org.",
            tmpl_or_fallback!("prober", build_prober_prompt()),
        )
        .with_tools(vec![
            "list_in_scope_targets".into(),
            "list_attack_surface_seeds".into(),
            "manage_targets".into(),
            "pentest_list_tools".into(),
            "pentest_run".into(),
            "submit_stage_deliverable".into(),
            "record_finding".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
        ])
        .with_max_iterations(40)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["enricher".into(), "memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "enumerator",
            "Enumerator",
            "Active content-enumeration mapper for the enumeration stage. Turns the live web services external_attack_surface mapped (host + ports + service) into concrete testable units by enumerating directories/paths, request parameters, and JS/API endpoints — actively but without exploitation. NON-EXPLOIT: no vulnerability scanning or exploitation — that stays with the Pentester. The stage_run tool fans one Enumerator out per org.",
            tmpl_or_fallback!("enumerator", build_enumerator_prompt()),
        )
        .with_tools(vec![
            "list_in_scope_targets".into(),
            "manage_targets".into(),
            "pentest_list_tools".into(),
            "pentest_run".into(),
            "js_collect".into(),
            "js_extract_apis".into(),
            "submit_stage_deliverable".into(),
            "record_finding".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
        ])
        .with_max_iterations(40)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["enricher".into(), "memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "memorist",
            "Memorist",
            "Memory management agent for long-term knowledge persistence.",
            tmpl_or_fallback!("memorist", build_memorist_prompt()),
        )
        .with_tools(vec![
            "search_memories".into(),
            "store_memory".into(),
            "list_memories".into(),
            "graph_add_entity".into(),
            "graph_add_relation".into(),
            "graph_search".into(),
            "graph_neighbors".into(),
            "graph_attack_paths".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
        ])
        .with_max_iterations(10)
        .with_idle_timeout(60),
    );

    agents.push(
        SubAgentDefinition::new(
            "planner", "Planner",
            "Task decomposition agent. Given a complex request, produces 3-7 ordered subtasks with agent assignments and dependencies.",
            tmpl_or_fallback!("planner", build_planner_prompt()),
        )
        .with_tools(vec!["search_memories".into()])
        .with_max_iterations(5).with_idle_timeout(60),
    );

    agents.push(
        SubAgentDefinition::new(
            "reflector", "Reflector",
            "Correction agent invoked automatically when another agent fails to produce tool calls.",
            tmpl_or_fallback!("reflector", build_reflector_prompt()),
        )
        .with_tools(vec![])
        .with_max_iterations(3).with_idle_timeout(30)
        .as_pipeline_only(),
    );

    agents.push(
        SubAgentDefinition::new(
            "adviser",
            "Adviser",
            "Security expert consultant for complex findings.",
            tmpl_or_fallback!("adviser", build_adviser_prompt()),
        )
        .with_tools(vec![
            "web_search".into(),
            "web_fetch".into(),
            "read_file".into(),
            "search_memories".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
        ])
        .with_max_iterations(15)
        .with_idle_timeout(120)
        .with_delegatable_agents(vec!["researcher".into(), "memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "reporter",
            "Reporter",
            "Generates structured security assessment reports.",
            tmpl_or_fallback!("reporter", build_reporter_prompt()),
        )
        .with_tools(vec![
            "read_file".into(),
            "search_memories".into(),
            "list_memories".into(),
            "write_file".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
            "list_cves_with_pocs".into(),
            "poc_stats".into(),
            // C2c · the reporter is the agent that produces the StageDeliverable
            // when delegated to; it must submit it via the deterministic tool.
            "submit_stage_deliverable".into(),
        ])
        .with_max_iterations(20)
        .with_idle_timeout(180)
        .with_delegatable_agents(vec!["memorist".into()]),
    );

    // `refiner.tera` exists in `prompts/` and is registered with the
    // registry, but it uses Tera variables `{{ execution_context }}` and
    // `{{ remaining_subtasks }}` that only have values at runtime when
    // the refiner is invoked with a specific subtask result. The static
    // `SubAgentDefinition::system_prompt` we're building here is the
    // base-line prompt — there's no plan or context yet. The actual
    // runtime path (`bridge_executor::trait_impl::*` →
    // `task_orchestrator::prompts::refiner_prompt`) builds the full
    // templated prompt independently per call, so this base prompt only
    // needs the static portion.
    agents.push(
        SubAgentDefinition::new(
            "refiner", "Refiner",
            "Task plan refinement agent. Called after each subtask completes to evaluate progress and adjust the remaining plan.",
            build_refiner_prompt(),
        )
        .with_tools(vec![
            "search_memories".into(), "search_knowledge_base".into(), "read_knowledge".into(),
        ])
        .with_max_iterations(5).with_idle_timeout(60)
        .as_pipeline_only(),
    );

    agents.push(
        SubAgentDefinition::new(
            "browser", "Browser",
            "Web browser and JavaScript analysis specialist. Handles JS file collection and browser-based reconnaissance.",
            tmpl_or_fallback!("browser", build_browser_prompt()),
        )
        .with_tools(vec![
            "js_collect".into(), "js_extract_apis".into(),
            "web_fetch".into(), "web_search".into(),
            "read_file".into(), "write_file".into(), "grep_file".into(), "record_finding".into(),
        ])
        .with_max_iterations(20).with_idle_timeout(120),
    );

    agents.push(
        SubAgentDefinition::new(
            "enricher", "Enricher",
            "Context enrichment specialist. Gathers supplementary information from memory, knowledge base, and knowledge graph.",
            tmpl_or_fallback!("enricher", build_enricher_prompt()),
        )
        .with_tools(vec![
            "search_memories".into(), "read_file".into(), "search_knowledge_base".into(),
            "read_knowledge".into(), "graph_search".into(), "graph_neighbors".into(),
            "graph_attack_paths".into(), "search_exploits".into(), "list_cves_with_pocs".into(),
        ])
        .with_max_iterations(10).with_idle_timeout(60),
    );

    // `orchestrator.tera` wraps `{{execution_context}}` in `{% raw %}` so
    // Tera renders it back as a literal placeholder; the agent runtime
    // (`bridge_executor::trait_impl::*`) then substitutes the real
    // execution-context XML via `String::replace`.
    agents.push(
        SubAgentDefinition::new(
            "orchestrator", "Orchestrator",
            "Primary task coordinator and team orchestration manager. Analyzes complex tasks, breaks them into subtasks, and delegates to specialist agents.",
            tmpl_or_fallback!("orchestrator", build_orchestrator_prompt()),
        )
        .with_tools(vec![
            "update_plan".into(), "search_memories".into(), "search_knowledge_base".into(),
            "read_knowledge".into(), "query_target_data".into(),
            "list_in_scope_targets".into(),
            // C2c · orchestrator may also submit the stage deliverable directly.
            "submit_stage_deliverable".into(),
        ])
        .with_max_iterations(50).with_idle_timeout(300)
        .with_delegatable_agents(vec![
            "researcher".into(), "pentester".into(), "coder".into(), "memorist".into(),
            "installer".into(), "adviser".into(), "reporter".into(), "enricher".into(),
            "browser".into(),
        ])
        .as_pipeline_only(),
    );

    agents
}
