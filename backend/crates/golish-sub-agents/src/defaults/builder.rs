//! Constructors that assemble the default [`SubAgentDefinition`] catalogue.
//!
//! Two flavours:
//! - [`create_default_sub_agents`]: uses the hardcoded prompts from
//!   [`super::prompts`] directly.
//! - [`create_default_sub_agents_from_registry`]: prefers prompts from the
//!   template registry (`prompts/*.tera` or DB overrides), falling back to
//!   hardcoded prompts on render failure.

use crate::definition::SubAgentDefinition;
use crate::schemas::IMPLEMENTATION_PLAN_FULL_EXAMPLE;

use super::prompts::{
    build_adviser_prompt, build_browser_prompt, build_coder_prompt, build_enricher_prompt,
    build_installer_prompt, build_memorist_prompt, build_orchestrator_prompt,
    build_pentester_prompt, build_planner_prompt, build_refiner_prompt, build_reflector_prompt,
    build_reporter_prompt, build_researcher_prompt, build_researcher_prompt_fallback,
};

/// Create default sub-agents for common tasks.
pub fn create_default_sub_agents() -> Vec<SubAgentDefinition> {
    vec![
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
        .with_timeout(600)
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
        .with_timeout(600)
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
        ])
        .with_max_iterations(30)
        .with_timeout(600)
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
            "run_pipeline".to_string(),
            "flow_compose".to_string(),
            "manage_targets".to_string(),
            "record_finding".to_string(),
            "vault".to_string(),
            "pentest_list_tools".to_string(),
            "pentest_run".to_string(),
            "graph_search".to_string(),
            "graph_add_entity".to_string(),
            "graph_add_relation".to_string(),
            "graph_attack_paths".to_string(),
            "search_exploits".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
        ])
        .with_max_iterations(50)
        .with_timeout(900)
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
        .with_timeout(120)
        .with_idle_timeout(60),
        SubAgentDefinition::new(
            "planner",
            "Planner",
            "Task decomposition agent. Given a complex request, produces 3-7 ordered subtasks with agent assignments and dependencies. Use when the user's request requires multiple steps across different specializations. Returns a JSON execution plan.",
            build_planner_prompt(),
        )
        .with_tools(vec!["search_memories".to_string()])
        .with_max_iterations(5)
        .with_timeout(120)
        .with_idle_timeout(60),
        SubAgentDefinition::new(
            "reflector",
            "Reflector",
            "Correction agent invoked automatically when another agent fails to produce tool calls. Diagnoses why the agent is stuck and provides a corrective instruction. Not for direct invocation — triggered by the execution loop.",
            build_reflector_prompt(),
        )
        .with_tools(vec![])
        .with_max_iterations(3)
        .with_timeout(60)
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
        .with_timeout(300)
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
        .with_timeout(600)
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
        .with_timeout(120)
        .with_idle_timeout(60)
        .as_pipeline_only(),
        SubAgentDefinition::new(
            "browser",
            "Browser",
            "Web browser and JavaScript analysis specialist. Handles JS file collection, web content extraction, and browser-based reconnaissance. Delegate when you need to collect and analyze JavaScript files from a target, or perform deeper web interaction beyond simple HTTP fetching.",
            build_browser_prompt(),
        )
        .with_tools(vec![
            "js_collect".to_string(),
            "web_fetch".to_string(),
            "web_search".to_string(),
            "read_file".to_string(),
            "write_file".to_string(),
            "grep_file".to_string(),
            "record_finding".to_string(),
        ])
        .with_max_iterations(20)
        .with_timeout(300)
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
        .with_timeout(120)
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
        ])
        .with_max_iterations(50)
        .with_timeout(900)
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
    ]
}

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

    // Helper: render template or fall back to hardcoded
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
        .with_max_iterations(20).with_timeout(600).with_idle_timeout(180),
    );

    agents.push(
        SubAgentDefinition::new(
            "researcher", "Research Agent",
            "Researches topics by reading documentation, searching the web, and gathering information.",
            tmpl_or_fallback!("researcher", build_researcher_prompt_fallback()),
        )
        .with_tools(vec![
            "web_search".into(),
            "web_fetch".into(),
            "read_file".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
            "write_knowledge".into(),
            "ingest_cve".into(),
            "save_poc".into(),
            "list_cves_with_pocs".into(),
            "list_unresearched_cves".into(),
        ])
        .with_max_iterations(25).with_timeout(600).with_idle_timeout(180)
        .with_delegatable_agents(vec!["memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "installer", "Installer",
            "Tool installation and environment configuration specialist. Handles downloading, compiling, and configuring penetration testing tools. Manages Python virtual environments, Go builds, and dependency conflicts. Delegate when a tool needs to be installed or a complex environment needs setup.",
            tmpl_or_fallback!("installer", build_installer_prompt()),
        )
        .with_tools(vec!["read_file".into(), "write_file".into(), "web_fetch".into(), "list_directory".into(), "list_files".into(), "grep_file".into(), "pentest_list_tools".into(), "pentest_run".into()])
        .with_max_iterations(30).with_timeout(600).with_idle_timeout(300)
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
            "run_pipeline".into(),
            "flow_compose".into(),
            "manage_targets".into(),
            "record_finding".into(),
            "vault".into(),
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
        .with_timeout(900)
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
        .with_timeout(120)
        .with_idle_timeout(60),
    );

    agents.push(
        SubAgentDefinition::new(
            "planner", "Planner",
            "Task decomposition agent. Given a complex request, produces 3-7 ordered subtasks with agent assignments and dependencies.",
            tmpl_or_fallback!("planner", build_planner_prompt()),
        )
        .with_tools(vec!["search_memories".into()])
        .with_max_iterations(5).with_timeout(120).with_idle_timeout(60),
    );

    agents.push(
        SubAgentDefinition::new(
            "reflector", "Reflector",
            "Correction agent invoked automatically when another agent fails to produce tool calls.",
            tmpl_or_fallback!("reflector", build_reflector_prompt()),
        )
        .with_tools(vec![])
        .with_max_iterations(3).with_timeout(60).with_idle_timeout(30)
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
        .with_timeout(300)
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
        ])
        .with_max_iterations(20)
        .with_timeout(600)
        .with_idle_timeout(180)
        .with_delegatable_agents(vec!["memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "refiner",
            "Refiner",
            "Task plan refinement agent. Called after each subtask completes to evaluate progress and adjust the remaining plan.",
            tmpl_or_fallback!("refiner", build_refiner_prompt()),
        )
        .with_tools(vec![
            "search_memories".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
        ])
        .with_max_iterations(5)
        .with_timeout(120)
        .with_idle_timeout(60)
        .as_pipeline_only(),
    );

    agents.push(
        SubAgentDefinition::new(
            "browser",
            "Browser",
            "Web browser and JavaScript analysis specialist. Handles JS file collection and browser-based reconnaissance.",
            tmpl_or_fallback!("browser", build_browser_prompt()),
        )
        .with_tools(vec![
            "js_collect".into(),
            "web_fetch".into(),
            "web_search".into(),
            "read_file".into(),
            "write_file".into(),
            "grep_file".into(),
            "record_finding".into(),
        ])
        .with_max_iterations(20)
        .with_timeout(300)
        .with_idle_timeout(120),
    );

    agents.push(
        SubAgentDefinition::new(
            "enricher",
            "Enricher",
            "Context enrichment specialist. Gathers supplementary information from memory, knowledge base, and knowledge graph.",
            tmpl_or_fallback!("enricher", build_enricher_prompt()),
        )
        .with_tools(vec![
            "search_memories".into(),
            "read_file".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
            "graph_search".into(),
            "graph_neighbors".into(),
            "graph_attack_paths".into(),
            "search_exploits".into(),
            "list_cves_with_pocs".into(),
        ])
        .with_max_iterations(10)
        .with_timeout(120)
        .with_idle_timeout(60),
    );

    agents.push(
        SubAgentDefinition::new(
            "orchestrator",
            "Orchestrator",
            "Primary task coordinator and team orchestration manager. Analyzes complex tasks, breaks them into subtasks, and delegates to specialist agents.",
            tmpl_or_fallback!("orchestrator", build_orchestrator_prompt()),
        )
        .with_tools(vec![
            "update_plan".into(),
            "search_memories".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
            "query_target_data".into(),
        ])
        .with_max_iterations(50)
        .with_timeout(900)
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
    );

    agents
}
