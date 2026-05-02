// Included verbatim into `lib.rs` via `include!` so the `__cmd__$name`
// macros emitted by `#[tauri::command]` (which are `#[macro_export]`-ed
// to the crate root, not into sub-modules) are in scope at the call
// site of `tauri::generate_handler!`.
//
// Per ADR-0009 Phase 1: a single facade import replaces the 12
// scattered `use A::commands::*; use B::*;` globs. The facade files
// under `commands_facade/<domain>.rs` are now the authoritative
// per-domain command surface. Adding/renaming/removing a command
// means touching exactly two files: the command's home module and
// the matching facade file. Keep this block alphabetical.

use commands_facade::ai::*;
use commands_facade::findings::*;
use commands_facade::git_pty::*;
use commands_facade::indexer::*;
use commands_facade::mcp::*;
use commands_facade::pentest::*;
use commands_facade::pipeline::*;
use commands_facade::settings::*;
use commands_facade::sidecar::*;
use commands_facade::vault::*;
use commands_facade::vuln_intel::*;
use commands_facade::wiki::*;
use commands_facade::workspace::*;

/// Attach the platform-wide `invoke_handler` to a configured Tauri
/// builder. Caller chains `.build(...)` and `.run(...)` afterwards.
fn install_handlers(
    builder: tauri::Builder<tauri::Wry>,
) -> tauri::Builder<tauri::Wry> {
    builder.invoke_handler(tauri::generate_handler![
        // ── git_pty (PTY / shell / git / themes / IME) ───────────
        pty_create, pty_write, pty_resize, pty_destroy, pty_get_session,
        pty_get_foreground_process, set_active_terminal_session,
        list_path_completions, classify_input,
        shell_integration_status, shell_integration_install, shell_integration_uninstall,
        get_git_branch, git_status, git_diff, git_diff_staged,
        git_stage, git_unstage, git_commit, git_push, git_delete_worktree,
        add_command_history, add_prompt_history, load_history, search_history, clear_history,
        run_recon_pipeline, check_recon_tools_cmd,
        list_themes, read_theme, save_theme, delete_theme, save_theme_asset, get_theme_asset_path,
        write_frontend_log, ime_get_source, ime_set_source,
        // ── ai (agent init / chat / context / policies) ──────────
        init_ai_agent, init_ai_agent_vertex, init_ai_agent_openai, init_ai_agent_unified,
        send_ai_prompt, execute_ai_tool, get_available_tools,
        list_sub_agents, get_sub_agent_model, set_sub_agent_model,
        shutdown_ai_agent, is_ai_initialized, generate_commit_message,
        init_ai_session, shutdown_ai_session, cancel_ai_generation,
        is_ai_session_initialized, get_session_ai_config,
        send_ai_prompt_session, send_ai_prompt_with_attachments, get_vision_capabilities,
        clear_ai_conversation_session, get_ai_conversation_length_session, signal_frontend_ready,
        get_openrouter_api_key, get_openai_api_key, get_project_settings,
        save_project_model, get_vertex_ai_config, load_env_file, update_ai_workspace,
        clear_ai_conversation, get_ai_conversation_length, restore_ai_conversation,
        list_ai_sessions, find_ai_session, load_ai_session, export_ai_session_transcript,
        set_ai_session_persistence, is_ai_session_persistence_enabled,
        finalize_ai_session, restore_ai_session,
        get_tool_call_stats, get_db_token_usage_stats, get_usage_by_agent, get_audit_log,
        search_memories, list_recent_memories, get_memory_count,
        list_agent_definitions, read_agent_prompt, save_agent_definition, delete_agent_definition, seed_agents,
        get_approval_patterns, get_tool_approval_pattern, get_hitl_config, set_hitl_config,
        add_tool_always_allow, remove_tool_always_allow, reset_approval_patterns, respond_to_tool_approval,
        get_tool_policy_config, set_tool_policy_config, get_tool_policy, set_tool_policy,
        reset_tool_policies, enable_full_auto_mode, disable_full_auto_mode, is_full_auto_mode_enabled,
        get_agent_mode, set_agent_mode, save_project_agent_mode,
        set_use_agents, get_use_agents, set_execution_mode, get_execution_mode,
        get_api_request_stats, get_plan, get_context_summary,
        get_token_usage_stats, get_token_alert_level, get_context_utilization,
        get_remaining_tokens, reset_context_manager, get_context_trim_config, is_context_management_enabled, retry_compaction,
        get_loop_protection_config, set_loop_protection_config, get_loop_detector_stats,
        is_loop_detection_enabled, disable_loop_detection, enable_loop_detection, reset_loop_detector,
        // ── indexer ──────────────────────────────────────────────
        init_indexer, is_indexer_initialized, get_indexer_workspace,
        get_indexed_file_count, get_all_indexed_files, index_file, index_directory,
        search_code, search_files, shutdown_indexer,
        list_indexed_codebases, add_indexed_codebase, remove_indexed_codebase,
        reindex_codebase, migrate_codebase_index, update_codebase_memory_file, detect_memory_files,
        list_projects_for_home, list_recent_directories, remove_recent_directory,
        list_git_branches, create_git_worktree,
        // ── settings / models ────────────────────────────────────
        get_settings, update_settings, get_setting, set_setting, reset_settings,
        settings_file_exists, get_settings_path, reload_settings,
        save_window_state, get_window_state, is_langfuse_active, get_telemetry_stats,
        get_available_models, get_model_by_id, get_model_capabilities_command, get_providers,
        // ── sidecar ──────────────────────────────────────────────
        sidecar_status, sidecar_initialize, sidecar_start_session, sidecar_end_session,
        sidecar_current_session, sidecar_resume_session, sidecar_get_session_state,
        sidecar_get_session_log, sidecar_get_injectable_context, sidecar_get_session_meta,
        sidecar_list_sessions, sidecar_get_config, sidecar_set_config, sidecar_shutdown,
        sidecar_get_staged_patches, sidecar_get_applied_patches, sidecar_get_patch,
        sidecar_discard_patch, sidecar_get_current_staged_patches,
        sidecar_apply_patch, sidecar_apply_all_patches, sidecar_regenerate_patch, sidecar_update_patch_message,
        sidecar_get_pending_artifacts, sidecar_get_applied_artifacts, sidecar_get_artifact,
        sidecar_discard_artifact, sidecar_preview_artifact, sidecar_get_current_pending_artifacts,
        sidecar_apply_artifact, sidecar_apply_all_artifacts, sidecar_regenerate_artifacts,
        // ── pentest / ZAP ────────────────────────────────────────
        pentest_scan_tools, pentest_launch_tool, pentest_kill_tool, pentest_kill_all_tools,
        pentest_search_tools, pentest_get_command, pentest_build_command, pentest_check_runtime,
        pentest_check_tool_executable_permission, pentest_check_tools_executable_permissions,
        pentest_fix_tool_executable_permission, pentest_open_directory,
        pentest_update_config, pentest_get_config, pentest_get_categories,
        pentest_check_env_setup, pentest_install_runtime, pentest_cancel_runtime_install,
        pentest_uninstall_runtime, pentest_check_brew_outdated,
        pentest_list_installed_ruby, pentest_list_available_ruby,
        pentest_install_ruby_version, pentest_uninstall_ruby_version, pentest_set_default_ruby,
        pentest_create_file, pentest_open_url,
        pentest_fetch_github_releases, pentest_fetch_github_release, pentest_fetch_github_release_by_tag,
        pentest_check_tool_updates, pentest_fetch_github_repo_info, pentest_fetch_github_readme, pentest_analyze_github_tool,
        pentest_create_tool_package, pentest_update_tool, pentest_update_tool_executable,
        pentest_delete_backup, pentest_delete_tool, pentest_copy_to_toolpack,
        pentest_download_and_extract, pentest_cancel_download,
        pentest_find_tool_executables, pentest_list_tool_dir_files,
        pentest_rename_tool_dir, pentest_uninstall_tool_files,
        pentest_uninstall_brew_pkg, pentest_uninstall_gem_pkg,
        pentest_read_tool_config, pentest_save_tool_config,
        pentest_git_clone_tool, pentest_pip_install_tool, pentest_pip_install, pentest_pip_uninstall,
        pentest_conda_install_tool, pentest_resolve_python_path, pentest_resolve_java_path,
        pentest_install_requirements, pentest_check_requirements, pentest_list_dep_files, pentest_install_dep_file,
        pentest_browser_navigate, pentest_browser_resize, pentest_browser_hide,
        pentest_browser_show, pentest_browser_close, pentest_browser_go_back, pentest_browser_go_forward,
        pentest_set_system_proxy, pentest_clear_system_proxy, pentest_get_system_proxy,
        pentest_list_installed_java, pentest_list_available_java,
        pentest_install_java_version, pentest_uninstall_java_version, pentest_set_default_java,
        pentest_list_installed_node, pentest_list_available_node,
        pentest_install_node_version, pentest_uninstall_node_version, pentest_use_node_version,
        pentest_list_python_envs, pentest_list_available_python, pentest_create_python_env, pentest_delete_python_env,
        create_detached_window, close_detached_window,
        pentest_list_skills, pentest_read_skill, pentest_write_skill, pentest_delete_skill,
        zap_start, zap_stop, zap_status, zap_update_project, zap_detect_path, zap_set_path,
        zap_get_history, zap_get_history_count, zap_get_message,
        zap_start_scan, zap_scan_progress, zap_scan_message_count, zap_stop_scan, zap_pause_scan, zap_resume_scan,
        zap_list_scan_policies, zap_batch_scan, zap_get_alerts, zap_get_scanners,
        zap_set_scanners_enabled, zap_get_alert_count, zap_start_spider, zap_spider_progress, zap_stop_spider,
        zap_send_request, zap_get_hosts, zap_new_session, zap_save_session,
        zap_sync_to_db, zap_get_sitemap_data, zap_download_root_cert, zap_install_root_cert, zap_api_call,
        // ── workspace (targets / wiki / vault / findings / …) ───
        save_project, delete_project_config, list_project_configs, get_project_config,
        save_project_workspace, load_project_workspace,
        get_pentest_config, save_pentest_config, list_captures, list_capture_files,
        read_project_file, init_project_structure, clean_project_temp,
        list_prompts, read_prompt, list_skills, read_skill, read_skill_body,
        list_skill_files, read_skill_file, save_skill, delete_skill,
        list_rules, read_rule_body, save_rule, delete_rule,
        list_workspace_files, list_directory, read_workspace_file, write_workspace_file,
        stat_workspace_file, read_file_as_base64, watch_file, unwatch_file, unwatch_all_files,
        target_list, target_add, target_batch_add, target_update, target_update_status, target_delete, target_clear_all,
        vault_list, vault_add, vault_get_value, vault_update, vault_delete, vault_resolve, vault_validate, vault_update_status,
        project_export, project_import,
        method_list_templates, method_start_project, method_list_projects, method_load_project, method_update_item, method_delete_project,
        recording_save, recording_load, recording_list, recording_delete,
        output_parse, output_detect_tool, output_parse_and_store,
        directory_entry_list,
        findings_list, findings_add, findings_update, findings_delete,
        findings_import_parsed, findings_add_evidence, findings_remove_evidence,
        findings_evidence_path, findings_deduplicate, findings_for_host,
        // ── pipeline ─────────────────────────────────────────────
        pipeline_list, pipeline_save, pipeline_cancel, pipeline_delete,
        pipeline_load, pipeline_execute, pipeline_list_templates, pipeline_save_template, pipeline_delete_template,
        // ── vuln-intel ───────────────────────────────────────────
        intel_list_feeds, intel_add_feed, intel_toggle_feed, intel_delete_feed,
        intel_fetch, intel_fetch_page, intel_get_cached,
        intel_search, intel_search_remote, intel_search_remote_page,
        intel_match_targets, intel_search_github_poc,
        intel_search_nuclei_templates, intel_batch_search_nuclei_templates, intel_discover_all_nuclei,
        // ── misc ─────────────────────────────────────────────────
        scan_queue_list, scan_queue_upsert, scan_queue_save_all, scan_queue_remove, scan_queue_clear_completed,
        custom_rules_list, custom_rules_upsert, custom_rules_save_all, custom_rules_delete,
        notes_list, notes_add, notes_update, notes_delete,
        audit_log, audit_list, audit_clear, agent_logs_list, terminal_logs_list, search_logs_list, passive_scans_global,
        wordlist_list, wordlist_import, wordlist_delete, wordlist_deduplicate, wordlist_merge, wordlist_preview, wordlist_path,
        wiki_init, wiki_reindex, wiki_list, wiki_read, wiki_write, wiki_delete, wiki_rename, wiki_create_dir,
        wiki_search, wiki_search_db, wiki_stats, wiki_create_cve,
        kb_research_load, kb_research_save_turn, kb_research_set_status, kb_research_clear,
        vuln_link_get_all, vuln_link_get, vuln_link_add_wiki, vuln_link_remove_wiki,
        vuln_link_add_poc, vuln_link_update_poc, vuln_link_remove_poc,
        vuln_link_add_scan, vuln_link_remove_scan, vuln_link_add_poc_full,
        vuln_poc_list_cves, vuln_poc_list_unresearched, vuln_poc_stats, vuln_poc_set_verified,
        wiki_pages_grouped, wiki_pages_for_paths, wiki_suggest_for_cve,
        wiki_changelog_list, wiki_backlinks, wiki_stats_full, wiki_orphan_pages,
        conv_save, conv_delete, conv_list, conv_save_messages, conv_load_messages,
        conv_save_timeline, conv_load_timeline, conv_save_terminal_state, conv_load_terminal_states,
        conv_save_preferences, conv_load_preferences, conv_save_batch,
        plan_create, plan_get, plan_list, plan_list_active, plan_update_steps, plan_update_status, plan_update_context, plan_delete,
        oplog_list, oplog_list_by_target, oplog_list_by_type, oplog_search, oplog_count,
        target_assets_list, api_endpoints_list, api_endpoints_untested,
        fingerprints_list, js_analysis_list,
        passive_scans_list, passive_scans_by_url, passive_scans_vulnerable, passive_scans_stats, target_security_overview,
        scan_whatweb, match_pocs_for_target, scan_nuclei_targeted, nuclei_cancel, scan_feroxbuster, get_zap_discovered_paths,
        sensitive_scan_start, sensitive_scan_stop, sensitive_scan_status, sensitive_scan_results,
        sensitive_scan_clear, sensitive_scan_confirm, sensitive_scan_default_paths, sensitive_scan_apply_verdicts,
        mcp_list_servers, mcp_list_tools, mcp_get_config, mcp_is_project_trusted, mcp_trust_project_config,
        mcp_has_project_config, mcp_connect, mcp_disconnect,
    ])
}
