//! Workspace tools — targets, wiki, vault, findings, notes, scans, recordings, etc.

pub use crate::projects::commands::{
    save_project, delete_project_config, list_project_configs,
    get_project_config, save_project_workspace, load_project_workspace,
    get_pentest_config, save_pentest_config,
    list_captures, list_capture_files, read_project_file,
    init_project_structure, clean_project_temp,
    list_prompts, read_prompt,
    list_skills, read_skill, read_skill_body, list_skill_files, read_skill_file,
    save_skill, delete_skill,
    list_rules, read_rule_body, save_rule, delete_rule,
    list_workspace_files, list_directory, read_workspace_file,
    write_workspace_file, stat_workspace_file, read_file_as_base64,
    watch_file, unwatch_file, unwatch_all_files,
};
pub use crate::tools::targets::{
    target_list, target_add, target_batch_add, target_update,
    target_update_status, target_delete, target_clear_all, directory_entry_list,
};
pub use crate::tools::vault::{
    vault_list, vault_add, vault_get_value, vault_update,
    vault_delete, vault_resolve, vault_validate, vault_update_status,
};
pub use crate::tools::project_io::{project_export, project_import};
pub use crate::tools::methodology::{
    method_list_templates, method_start_project, method_list_projects,
    method_load_project, method_update_item, method_delete_project,
};
pub use crate::tools::recordings::{recording_save, recording_load, recording_list, recording_delete};
pub use crate::tools::output_parser::{output_parse, output_detect_tool, output_parse_and_store};
pub use crate::tools::findings::{
    findings_list, findings_add, findings_update, findings_delete,
    findings_import_parsed, findings_add_evidence, findings_remove_evidence,
    findings_evidence_path, findings_deduplicate, findings_for_host,
};
pub use crate::tools::scan_queue::{scan_queue_list, scan_queue_upsert, scan_queue_save_all, scan_queue_remove, scan_queue_clear_completed};
pub use crate::tools::custom_rules::{custom_rules_list, custom_rules_upsert, custom_rules_save_all, custom_rules_delete};
pub use crate::tools::notes::{notes_list, notes_add, notes_update, notes_delete};
pub use crate::tools::audit::{audit_log, audit_list, audit_clear, agent_logs_list, terminal_logs_list, search_logs_list, passive_scans_global};
pub use crate::tools::wordlists::{wordlist_list, wordlist_import, wordlist_delete, wordlist_deduplicate, wordlist_merge, wordlist_preview, wordlist_path};
pub use crate::tools::wiki::{
    wiki_init, wiki_reindex, wiki_list, wiki_read, wiki_write, wiki_delete,
    wiki_rename, wiki_create_dir, wiki_search, wiki_search_db, wiki_stats,
    wiki_create_cve, kb_research_load, kb_research_save_turn,
    kb_research_set_status, kb_research_clear,
    vuln_link_get_all, vuln_link_get, vuln_link_add_wiki, vuln_link_remove_wiki,
    vuln_link_add_poc, vuln_link_update_poc, vuln_link_remove_poc,
    vuln_link_add_scan, vuln_link_remove_scan, vuln_link_add_poc_full,
    vuln_poc_list_cves, vuln_poc_list_unresearched, vuln_poc_stats, vuln_poc_set_verified,
    wiki_pages_grouped, wiki_pages_for_paths, wiki_suggest_for_cve,
    wiki_changelog_list, wiki_backlinks, wiki_stats_full, wiki_orphan_pages,
};
pub use crate::tools::conversation_store::{
    conv_save, conv_delete, conv_list,
    conv_save_messages, conv_load_messages,
    conv_save_timeline, conv_load_timeline,
    conv_save_terminal_state, conv_load_terminal_states,
    conv_save_preferences, conv_load_preferences,
    batch::conv_save_batch,
};
pub use crate::tools::execution_plans::{
    plan_create, plan_get, plan_list, plan_list_active,
    plan_update_steps, plan_update_status, plan_update_context, plan_delete,
};
pub use crate::tools::security_analysis::{
    oplog_list, oplog_list_by_target, oplog_list_by_type, oplog_search, oplog_count,
    target_assets_list, api_endpoints_list, api_endpoints_untested,
    fingerprints_list, js_analysis_list,
    passive_scans_list, passive_scans_by_url, passive_scans_vulnerable, passive_scans_stats,
    target_security_overview,
};
pub use crate::tools::scan_runner::{
    scan_whatweb, match_pocs_for_target, scan_nuclei_targeted, nuclei_cancel,
    scan_feroxbuster, get_zap_discovered_paths,
};
pub use crate::tools::sensitive_scan::{
    sensitive_scan_start, sensitive_scan_stop, sensitive_scan_status,
    sensitive_scan_results, sensitive_scan_clear, sensitive_scan_confirm,
    sensitive_scan_default_paths, sensitive_scan_apply_verdicts,
};
pub use crate::mcp::commands::{
    mcp_list_servers, mcp_list_tools, mcp_get_config,
    mcp_is_project_trusted, mcp_trust_project_config, mcp_has_project_config,
    mcp_connect, mcp_disconnect,
};
