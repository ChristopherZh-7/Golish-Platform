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
    WORKER_PROMPT_TEMPLATE, build_adviser_prompt, build_analyzer_prompt, build_coder_prompt,
    build_explorer_prompt, build_installer_prompt, build_memorist_prompt, build_pentester_prompt,
    build_planner_prompt, build_reflector_prompt, build_reporter_prompt, build_researcher_prompt,
    build_researcher_prompt_fallback, build_worker_prompt, build_worker_prompt_fallback,
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
            "analyzer",
            "Analyzer",
            "Performs deep semantic analysis of code: traces data flow, identifies dependencies, and explains complex logic. Returns structured analysis for implementation planning.",
            build_analyzer_prompt(),
        )
        .with_tools(vec![
            "read_file".to_string(),
            "grep_file".to_string(),
            "ast_grep".to_string(),
            "list_directory".to_string(),
            "find_files".to_string(),
            "indexer_search_code".to_string(),
            "indexer_search_files".to_string(),
            "indexer_analyze_file".to_string(),
            "indexer_extract_symbols".to_string(),
            "indexer_get_metrics".to_string(),
            "indexer_detect_language".to_string(),
        ])
        .with_max_iterations(30)
        .with_timeout(300)
        .with_idle_timeout(120),
        SubAgentDefinition::new(
            "explorer",
            "Explorer",
            "Fast, read-only file search agent. Delegates to find relevant file paths — does not analyze or explain code. Use when you need to:\n- Find files by name, pattern, or extension\n- Locate files containing specific keywords, symbols, or code patterns\n- Map out project structure or directory layout\nWhen calling, provide: (1) what you're looking for, (2) any known context like paths or patterns, (3) thoroughness level: \"quick\", \"medium\", or \"thorough\". Act on the returned file paths yourself — this agent only finds files, it does not read or interpret them.",
            build_explorer_prompt(),
        )
        .with_tools(vec![
            "read_file".to_string(),
            "list_files".to_string(),
            "list_directory".to_string(),
            "grep_file".to_string(),
            "ast_grep".to_string(),
            "find_files".to_string(),
        ])
        .with_max_iterations(15)
        .with_timeout(180)
        .with_idle_timeout(90),
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
            "run_pty_cmd".into(),
            "read_file".into(),
            "write_file".into(),
            "web_fetch".into(),
            "list_directory".into(),
            "list_files".into(),
            "grep_file".into(),
        ])
        .with_max_iterations(30)
        .with_timeout(600)
        .with_idle_timeout(300)
        .with_delegatable_agents(vec!["researcher".into(), "memorist".into()]),
        // js_harvester and js_analyzer removed — JS collection (via js_collect tool)
        // and JS security analysis (as prompt knowledge) are now integrated into pentester.
        SubAgentDefinition::new(
            "worker",
            "Worker",
            "A general-purpose agent that can handle any task with access to all standard tools. Use when the task doesn't fit a specialized agent, or when you need to run multiple independent tasks concurrently.",
            build_worker_prompt(),
        )
        .with_tools(vec![
            "read_file".to_string(),
            "write_file".to_string(),
            "create_file".to_string(),
            "edit_file".to_string(),
            "delete_file".to_string(),
            "list_files".to_string(),
            "list_directory".to_string(),
            "grep_file".to_string(),
            "ast_grep".to_string(),
            "ast_grep_replace".to_string(),
            "run_pty_cmd".to_string(),
            "web_search".to_string(),
            "web_fetch".to_string(),
            "search_knowledge_base".to_string(),
            "read_knowledge".to_string(),
            "write_knowledge".to_string(),
            "ingest_cve".to_string(),
            "save_poc".to_string(),
            "list_cves_with_pocs".to_string(),
            "list_unresearched_cves".to_string(),
            "poc_stats".to_string(),
        ])
        .with_max_iterations(30)
        .with_timeout(600)
        .with_idle_timeout(180)
        .with_delegatable_agents(vec![
            "explorer".into(),
            "researcher".into(),
            "memorist".into(),
        ])
        .with_prompt_template(WORKER_PROMPT_TEMPLATE),
        // ── New Phase 1 agents ─────────────────────────────────────────
        SubAgentDefinition::new(
            "pentester",
            "Pentester",
            "Penetration testing specialist for security assessments. Handles network scanning, web app testing, vulnerability assessment, and exploitation. Delegate security-related tasks that require tool expertise (nmap, gobuster, sqlmap, etc.).",
            build_pentester_prompt(),
        )
        .with_tools(vec![
            "run_pty_cmd".to_string(),
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
            "js_collect".to_string(),
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
            "explorer".to_string(),
            "installer".to_string(),
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
        .with_idle_timeout(30),
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
            "analyzer", "Analyzer",
            "Performs deep semantic analysis of code: traces data flow, identifies dependencies, and explains complex logic. Returns structured analysis for implementation planning.",
            tmpl_or_fallback!("analyzer", build_analyzer_prompt()),
        )
        .with_tools(vec!["read_file".into(), "grep_file".into(), "ast_grep".into(), "list_directory".into(), "find_files".into(), "indexer_search_code".into(), "indexer_search_files".into(), "indexer_analyze_file".into(), "indexer_extract_symbols".into(), "indexer_get_metrics".into(), "indexer_detect_language".into()])
        .with_max_iterations(30).with_timeout(300).with_idle_timeout(120),
    );

    agents.push(
        SubAgentDefinition::new(
            "explorer", "Explorer",
            "Fast, read-only file search agent. Delegates to find relevant file paths — does not analyze or explain code.",
            tmpl_or_fallback!("explorer", build_explorer_prompt()),
        )
        .with_tools(vec!["read_file".into(), "list_files".into(), "list_directory".into(), "grep_file".into(), "ast_grep".into(), "find_files".into()])
        .with_max_iterations(15).with_timeout(180).with_idle_timeout(90),
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
        .with_tools(vec!["run_pty_cmd".into(), "read_file".into(), "write_file".into(), "web_fetch".into(), "list_directory".into(), "list_files".into(), "grep_file".into()])
        .with_max_iterations(30).with_timeout(600).with_idle_timeout(300)
        .with_delegatable_agents(vec!["researcher".into(), "memorist".into()]),
    );

    agents.push(
        SubAgentDefinition::new(
            "worker",
            "Worker",
            "A general-purpose agent that can handle any task with access to all standard tools.",
            tmpl_or_fallback!("worker", build_worker_prompt_fallback()),
        )
        .with_tools(vec![
            "read_file".into(),
            "write_file".into(),
            "create_file".into(),
            "edit_file".into(),
            "delete_file".into(),
            "list_files".into(),
            "list_directory".into(),
            "grep_file".into(),
            "ast_grep".into(),
            "ast_grep_replace".into(),
            "run_pty_cmd".into(),
            "web_search".into(),
            "web_fetch".into(),
            "search_knowledge_base".into(),
            "read_knowledge".into(),
            "write_knowledge".into(),
            "ingest_cve".into(),
            "save_poc".into(),
            "list_cves_with_pocs".into(),
            "list_unresearched_cves".into(),
            "poc_stats".into(),
        ])
        .with_max_iterations(30)
        .with_timeout(600)
        .with_idle_timeout(180)
        .with_delegatable_agents(vec![
            "explorer".into(),
            "researcher".into(),
            "memorist".into(),
        ])
        .with_prompt_template(WORKER_PROMPT_TEMPLATE),
    );

    agents.push(
        SubAgentDefinition::new(
            "pentester",
            "Pentester",
            "Penetration testing specialist for security assessments.",
            tmpl_or_fallback!("pentester", build_pentester_prompt()),
        )
        .with_tools(vec![
            "run_pty_cmd".into(),
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
            "js_collect".into(),
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
            "explorer".into(),
            "installer".into(),
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
        .with_max_iterations(3).with_timeout(60).with_idle_timeout(30),
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

    agents
}
