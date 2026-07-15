//! Render a markdown table of tools from a [`ToolSelection`].
//!
//! Two responsibilities:
//!
//! 1. Generate a deterministic markdown table that future prompt templates
//!    can drop in as a `{tool_table}` substitution. The function lives
//!    here (next to the policies) so each policy implementation can
//!    decide its own rendering style if needed without dragging the
//!    prompts crate into the new abstraction.
//! 2. Provide a `selection_to_tool_names` enumeration that the
//!    contract test in this module uses to assert that any tool name
//!    written verbatim inside the prompt template (`chat.rs`,
//!    `task.rs`) is also present in the corresponding policy's
//!    `ToolSelection` — so removing a tool from a policy without
//!    updating the prompt fails the build.

use std::collections::HashSet;

use super::policy::ToolSelection;

#[derive(Debug, Clone, Copy)]
struct ToolRow {
    name: &'static str,
    purpose: &'static str,
}

const STATIC_FILE_OPS: &[ToolRow] = &[
    ToolRow {
        name: "read_file",
        purpose: "Read file content. Always read before editing.",
    },
    ToolRow {
        name: "edit_file",
        purpose: "Targeted edits in an existing file.",
    },
    ToolRow {
        name: "create_file",
        purpose: "Create a new file (fails if it exists).",
    },
    ToolRow {
        name: "write_file",
        purpose: "Overwrite an entire file.",
    },
    ToolRow {
        name: "delete_file",
        purpose: "Remove a file.",
    },
    ToolRow {
        name: "grep_file",
        purpose: "Regex search across files.",
    },
    ToolRow {
        name: "list_files",
        purpose: "List / find files by pattern.",
    },
];

const STATIC_CORE: &[ToolRow] = &[
    ToolRow {
        name: "ast_grep",
        purpose: "Structural code search (function calls, imports).",
    },
    ToolRow {
        name: "ast_grep_replace",
        purpose: "Structural refactor / rename.",
    },
    ToolRow {
        name: "update_plan",
        purpose: "Create and track task plans.",
    },
];

const STATIC_MEMORY: &[ToolRow] = &[
    ToolRow {
        name: "search_memories",
        purpose: "Search long-term memory.",
    },
    ToolRow {
        name: "store_memory",
        purpose: "Store findings to memory.",
    },
    ToolRow {
        name: "list_memories",
        purpose: "List recent memories.",
    },
];

const STATIC_KNOWLEDGE_BASE: &[ToolRow] = &[
    ToolRow {
        name: "search_guide",
        purpose: "Search saved playbooks.",
    },
    ToolRow {
        name: "save_guide",
        purpose: "Save a successful procedure.",
    },
    ToolRow {
        name: "search_code",
        purpose: "Search saved code snippets.",
    },
    ToolRow {
        name: "save_code",
        purpose: "Save a useful code snippet.",
    },
    ToolRow {
        name: "search_knowledge_base",
        purpose: "Search vulnerability knowledge base.",
    },
    ToolRow {
        name: "read_knowledge",
        purpose: "Read a knowledge entry.",
    },
    ToolRow {
        name: "write_knowledge",
        purpose: "Append a knowledge entry.",
    },
];

const STATIC_SECURITY_ANALYSIS: &[ToolRow] = &[
    ToolRow {
        name: "log_operation",
        purpose: "Log a pentest action and outcome.",
    },
    ToolRow {
        name: "discover_apis",
        purpose: "Persist API endpoints per target.",
    },
    ToolRow {
        name: "save_js_analysis",
        purpose: "Persist JS analysis findings.",
    },
    ToolRow {
        name: "fingerprint_target",
        purpose: "Persist tech fingerprint.",
    },
    ToolRow {
        name: "log_scan_result",
        purpose: "Persist a single security test result.",
    },
    ToolRow {
        name: "query_target_data",
        purpose: "Query all known data about a target.",
    },
    ToolRow {
        name: "list_in_scope_targets",
        purpose: "List in-scope recon targets (id+value+type); call first to discover assets, then query_target_data each.",
    },
    ToolRow {
        name: "check_stage_asset_coverage",
        purpose: "Preflight DB-truth stage coverage and see pending asset-technique gaps before submit.",
    },
    ToolRow {
        name: "stage_worklist_status",
        purpose: "Read compact DB-truth status for the active stage worklist.",
    },
    ToolRow {
        name: "stage_worklist_next",
        purpose: "Return the next batch of pending/error asset-technique work items for the active stage.",
    },
    ToolRow {
        name: "list_recent_evidence",
        purpose: "List this run's REAL evidence-ledger ids with their tool/asset/technique so you can cite them in claim evidence_ids before submit.",
    },
];

const STATIC_GRAPH: &[ToolRow] = &[
    ToolRow {
        name: "graph_search",
        purpose: "Search the security knowledge graph.",
    },
    ToolRow {
        name: "graph_neighbors",
        purpose: "Walk neighbours of a graph node.",
    },
    ToolRow {
        name: "graph_attack_paths",
        purpose: "Compute attack paths.",
    },
    ToolRow {
        name: "graph_add_entity",
        purpose: "Add a graph entity.",
    },
    ToolRow {
        name: "graph_add_relation",
        purpose: "Add a graph relation.",
    },
];

const STATIC_SPLOITUS: &[ToolRow] = &[
    ToolRow {
        name: "search_exploits",
        purpose: "Search exploit database.",
    },
    ToolRow {
        name: "ingest_cve",
        purpose: "Ingest a CVE record.",
    },
    ToolRow {
        name: "save_poc",
        purpose: "Save a proof-of-concept.",
    },
    ToolRow {
        name: "list_cves_with_pocs",
        purpose: "List CVEs with PoCs.",
    },
];

const BRIDGE_ROWS: &[ToolRow] = &[
    ToolRow { name: "manage_targets", purpose: "Add / list / update pentest targets (scope in/out, link to organization)." },
    ToolRow { name: "manage_organizations", purpose: "List / create target organizations and propose candidate units for human review during scoping." },
    ToolRow { name: "recon_discover_subsidiaries", purpose: "Passively discover subsidiary/affiliate orgs of the subject via enterprise intel (ENScan) during target_intel." },
    ToolRow { name: "recon_map_assets", purpose: "Passively survey an org's domains/IPs/ASN/subdomains/certs/ICP/apps/emails via cyberspace intel providers (0.zone/quake/fofa/…) during target_intel; normal calls auto-expand bounded owned apex domains." },
    ToolRow { name: "recon_lookup_whois", purpose: "Look up domain registration (WHOIS via RDAP) for an org, once per org, during target_intel." },
    ToolRow { name: "recon_lookup_company", purpose: "Scoping step 1: resolve a raw company name to canonical registered names (以企查查为准) via enterprise-intel lookup BEFORE creating organizations." },
    ToolRow { name: "recon_list_providers", purpose: "List passive intel providers and whether each has a configured credential; call FIRST in target_intel so you only invoke usable providers and mark the rest blocked." },
    ToolRow { name: "record_finding", purpose: "Record a vulnerability finding." },
    ToolRow { name: "vault", purpose: "Store / retrieve credentials." },
    ToolRow {
        name: "browser_collect_js_api",
        purpose: "Open a SPA/lazy-loaded page in a browser, save loaded JS chunks, observe XHR/fetch API calls, and return closure/AI-assist signals.",
    },
    ToolRow {
        name: "js_extract_apis",
        purpose: "Static-analyse JS captured by `browser_collect_js_api` to enumerate REST/GraphQL endpoints + secrets.",
    },
];

const RUNTIME_PENTEST: &[ToolRow] = &[
    ToolRow {
        name: "pentest_list_tools",
        purpose: "List installed pentest tools and their skills.",
    },
    ToolRow {
        name: "pentest_run",
        purpose: "Execute a pentest tool by name with arguments.",
    },
    ToolRow {
        name: "pentest_read_skill",
        purpose: "Read a skill document for tool usage.",
    },
];

const RUNTIME_TAVILY: &[ToolRow] = &[
    ToolRow {
        name: "tavily_search",
        purpose: "Web search with source results.",
    },
    ToolRow {
        name: "tavily_search_answer",
        purpose: "Web search with AI-generated answer.",
    },
    ToolRow {
        name: "tavily_extract",
        purpose: "Extract structured content from URLs.",
    },
];

/// Render a markdown `| Tool | Purpose |` table containing every tool
/// the `selection` enables. Order is deterministic: file_ops, core,
/// memory, knowledge_base, security_analysis, graph, sploitus,
/// bridge tools (in `enabled_tool_names` order), pentest_runtime,
/// tavily, run_command, ask_human. Tools listed in `deny_overrides`
/// are filtered out at the end.
pub fn render_tool_table_for_prompt(selection: &ToolSelection) -> String {
    let mut rows: Vec<ToolRow> = Vec::new();

    if selection.static_groups.file_ops {
        rows.extend_from_slice(STATIC_FILE_OPS);
    }
    if selection.static_groups.core {
        rows.extend_from_slice(STATIC_CORE);
    }
    if selection.static_groups.memory {
        rows.extend_from_slice(STATIC_MEMORY);
    }
    if selection.static_groups.knowledge_base {
        rows.extend_from_slice(STATIC_KNOWLEDGE_BASE);
    }
    if selection.static_groups.security_analysis {
        rows.extend_from_slice(STATIC_SECURITY_ANALYSIS);
    }
    if selection.static_groups.graph {
        rows.extend_from_slice(STATIC_GRAPH);
    }
    if selection.static_groups.sploitus {
        rows.extend_from_slice(STATIC_SPLOITUS);
    }

    for name in selection.bridge_tools.enabled_tool_names() {
        if let Some(row) = BRIDGE_ROWS.iter().find(|r| r.name == name) {
            rows.push(*row);
        }
    }

    if selection.runtime_tools.pentest_runtime {
        rows.extend_from_slice(RUNTIME_PENTEST);
    }
    if selection.runtime_tools.tavily {
        rows.extend_from_slice(RUNTIME_TAVILY);
    }

    if selection.include_run_command {
        rows.push(ToolRow {
            name: "run_pty_cmd",
            purpose: "Execute shell commands with PTY support.",
        });
    }
    if selection.include_ask_human {
        rows.push(ToolRow {
            name: "ask_human",
            purpose: "Ask the user a clarifying question.",
        });
    }

    let denied: HashSet<&str> = selection
        .deny_overrides
        .iter()
        .map(|s| s.as_str())
        .collect();
    rows.retain(|r| !denied.contains(r.name));

    let mut out = String::from("| Tool | Purpose |\n|---|---|\n");
    for r in rows {
        out.push_str(&format!("| `{}` | {} |\n", r.name, r.purpose));
    }
    out
}

/// Enumerate every tool name the selection enables (after
/// `deny_overrides`). Used by contract tests to assert prompt-template
/// vs policy consistency without parsing markdown.
pub fn selection_to_tool_names(selection: &ToolSelection) -> HashSet<&'static str> {
    let mut names: HashSet<&'static str> = HashSet::new();

    macro_rules! add_group {
        ($flag:expr, $group:ident) => {
            if $flag {
                for r in $group {
                    names.insert(r.name);
                }
            }
        };
    }

    add_group!(selection.static_groups.file_ops, STATIC_FILE_OPS);
    add_group!(selection.static_groups.core, STATIC_CORE);
    add_group!(selection.static_groups.memory, STATIC_MEMORY);
    add_group!(
        selection.static_groups.knowledge_base,
        STATIC_KNOWLEDGE_BASE
    );
    add_group!(
        selection.static_groups.security_analysis,
        STATIC_SECURITY_ANALYSIS
    );
    add_group!(selection.static_groups.graph, STATIC_GRAPH);
    add_group!(selection.static_groups.sploitus, STATIC_SPLOITUS);

    for n in selection.bridge_tools.enabled_tool_names() {
        names.insert(n);
    }
    add_group!(selection.runtime_tools.pentest_runtime, RUNTIME_PENTEST);
    add_group!(selection.runtime_tools.tavily, RUNTIME_TAVILY);

    if selection.include_run_command {
        names.insert("run_pty_cmd");
    }
    if selection.include_ask_human {
        names.insert("ask_human");
    }

    for d in &selection.deny_overrides {
        names.remove(d.as_str());
    }

    names
}

#[cfg(test)]
#[path = "prompt_render_tests.rs"]
mod tests;
