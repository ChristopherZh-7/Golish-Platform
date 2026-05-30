use super::*;
use crate::execution_mode::policy::*;

#[test]
fn render_includes_js_collect_when_enabled() {
    let s = ToolSelection {
        static_groups: StaticGroupSelection::all_enabled(),
        bridge_tools: BridgeToolSelection::all_enabled(),
        runtime_tools: RuntimeToolSelection {
            pentest_runtime: true,
            tavily: true,
        },
        agent_tools: AgentToolSelection::none(),
        include_run_command: true,
        include_ask_human: true,
        deny_overrides: vec![],
    };
    let table = render_tool_table_for_prompt(&s);
    assert!(table.contains("`js_collect`"));
    assert!(table.contains("`manage_targets`"));
    assert!(table.contains("`run_pipeline`"));
    assert!(table.contains("`run_pty_cmd`"));
    assert!(table.contains("`ask_human`"));
}

#[test]
fn render_omits_disabled_bridge_tools() {
    let mut bridge = BridgeToolSelection::none();
    bridge.manage_targets = true;
    let s = ToolSelection {
        bridge_tools: bridge,
        ..Default::default()
    };
    let table = render_tool_table_for_prompt(&s);
    assert!(table.contains("`manage_targets`"));
    assert!(!table.contains("`js_collect`"));
    assert!(!table.contains("`run_pipeline`"));
}

#[test]
fn deny_overrides_remove_tools() {
    let s = ToolSelection {
        static_groups: StaticGroupSelection::all_enabled(),
        deny_overrides: vec!["update_plan".into()],
        ..Default::default()
    };
    let table = render_tool_table_for_prompt(&s);
    assert!(!table.contains("`update_plan`"));
    assert!(table.contains("`read_file`"));
}

#[test]
fn selection_to_tool_names_matches_render() {
    let s = ToolSelection {
        static_groups: StaticGroupSelection::all_enabled(),
        bridge_tools: BridgeToolSelection::all_enabled(),
        runtime_tools: RuntimeToolSelection {
            pentest_runtime: true,
            tavily: true,
        },
        agent_tools: AgentToolSelection::none(),
        include_run_command: true,
        include_ask_human: true,
        deny_overrides: vec!["update_plan".into()],
    };
    let names = selection_to_tool_names(&s);
    assert!(names.contains("js_collect"));
    assert!(names.contains("manage_targets"));
    assert!(names.contains("read_file"));
    assert!(names.contains("ask_human"));
    assert!(!names.contains("update_plan"));
}

/// **Contract test**: every tool name written verbatim inside
/// `system_prompt/chat.rs` (as `` `tool_name` ``) must also be
/// reachable through the live `ChatModePolicy.primary_tools()`
/// selection. Catches the original 2026-05-06 bug shape: chat.rs
/// listing `manage_targets` while the runtime filter would never
/// expose it.
#[tokio::test]
async fn chat_prompt_template_tools_subset_of_chat_policy() {
    use crate::execution_mode::context::PolicyContext;
    use crate::execution_mode::modes::chat::ChatModePolicy;
    use crate::execution_mode::policy::ExecutionModePolicy;
    use std::path::Path;

    const CHAT_RS: &str = include_str!("../../../golish-prompts/src/system_prompt/chat.rs");

    let s = ChatModePolicy
        .primary_tools(&PolicyContext::new(
            Path::new("/tmp"),
            golish_core::AgentMode::default(),
        ))
        .await;
    let allowed = selection_to_tool_names(&s);

    // Tool names that historically appear in the chat prompt
    // template's `## Pentest Bridge Tools (Direct)` and `##
    // Security Analysis & Data Persistence Tools (Direct)` sections.
    // If any of them goes missing from ChatModePolicy in the
    // future, this assertion fires and forces a sync update.
    const CRITICAL_NAMES: &[&str] = &[
        "manage_targets",
        "run_pipeline",
        "record_finding",
        "vault",
        "log_operation",
        "discover_apis",
        "save_js_analysis",
        "fingerprint_target",
        "log_scan_result",
        "query_target_data",
        "read_file",
        "edit_file",
        "write_file",
        "create_file",
        "delete_file",
        "grep_file",
        "list_files",
        "ast_grep",
        "ast_grep_replace",
        "update_plan",
        "search_memories",
        "store_memory",
        "list_memories",
        "search_guide",
        "save_guide",
        "search_code",
        "save_code",
    ];

    let mut missing: Vec<&str> = Vec::new();
    for name in CRITICAL_NAMES {
        let backticked = format!("`{}`", name);
        if CHAT_RS.contains(&backticked) && !allowed.contains(*name) {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "chat.rs prompt mentions tool(s) that ChatModePolicy doesn't expose: {:?}. \
             Either remove the mention or add the flag to ChatModePolicy.primary_tools()",
        missing
    );
}
