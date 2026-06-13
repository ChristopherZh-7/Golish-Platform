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
    build_adviser_prompt, build_browser_prompt, build_coder_prompt, build_enricher_prompt,
    build_installer_prompt, build_memorist_prompt, build_orchestrator_prompt,
    build_pentester_prompt, build_planner_prompt, build_recon_prompt, build_refiner_prompt,
    build_reflector_prompt, build_reporter_prompt, build_researcher_prompt,
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
            "recon_enrich_assets".to_string(),
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
            "Passive target-intelligence collector for the target_intel stage. Enriches one organization's footprint via providers (0.zone/quake/ENScan) plus passive subdomain + URL history, and registers discovered assets as in-scope targets. ZERO-TOUCH: no live probing or exploitation — that stays with the Pentester. The stage_run tool fans one Recon out per org.",
            build_recon_prompt(),
        )
        .with_tools(vec![
            "recon_list_providers".to_string(),
            "recon_discover_subsidiaries".to_string(),
            "recon_enrich_assets".to_string(),
            "manage_targets".to_string(),
            "list_in_scope_targets".to_string(),
            // Passive collection only (subfinder / amass -passive / gau); the
            // stage tool-type allowlist (recon/*) keeps this zero-touch.
            "pentest_run".to_string(),
            "submit_stage_deliverable".to_string(),
            "record_finding".to_string(),
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
            "js_collect".to_string(),
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
    ]
}
