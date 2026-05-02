//! Workspace tools — targets, wiki, vault, findings, notes, scans,
//! conversation store, execution plans, security analysis, recordings,
//! sensitive scans, and so on.
//!
//! This is the "catch-all" facade for workspace-scoped operations. If a
//! sub-domain grows large enough to deserve its own facade, extract it
//! (the way `mcp` was extracted on 2026-05-02). Target size is no more
//! than ~30 re-exports.
//!
//! Expected command groups exposed here (documentation only):
//! - **Projects**: `save_project`, `*_project_config*`,
//!   `*_project_workspace`, `*_pentest_config`, `list_captures*`,
//!   `read_project_file`, `init_project_structure`, `clean_project_temp`
//! - **Prompts / skills / rules** (from `projects::commands`):
//!   `list_prompts`, `read_prompt`, `*_skill*`, `*_rule*`
//! - **Files**: `list_workspace_files`, `list_directory`,
//!   `read_workspace_file`, `write_workspace_file`, `stat_workspace_file`,
//!   `read_file_as_base64`, `watch_file`, `unwatch_file`,
//!   `unwatch_all_files`
//! - **Targets**: `target_{list,add,batch_add,update,update_status,
//!   delete,clear_all}`, `directory_entry_list`
//! - **Vault**: `vault_{list,add,get_value,update,delete,resolve,
//!   validate,update_status}`
//! - **Project I/O**: `project_export`, `project_import`
//! - **Methodology**: `method_{list_templates,start_project,
//!   list_projects,load_project,update_item,delete_project}`
//! - **Recordings**: `recording_{save,load,list,delete}`
//! - **Output parser**: `output_{parse,detect_tool,parse_and_store}`
//! - **Findings**: `findings_{list,add,update,delete,import_parsed,
//!   add_evidence,remove_evidence,evidence_path,deduplicate,for_host}`
//! - **Scan queue**: `scan_queue_{list,upsert,save_all,remove,
//!   clear_completed}`
//! - **Custom rules**: `custom_rules_{list,upsert,save_all,delete}`
//! - **Notes**: `notes_{list,add,update,delete}`
//! - **Audit**: `audit_{log,list,clear}`, `*_logs_list`,
//!   `passive_scans_global`
//! - **Wordlists**: `wordlist_{list,import,delete,deduplicate,merge,
//!   preview,path}`
//! - **Wiki / KB research / vuln links**: `wiki_*`, `kb_research_*`,
//!   `vuln_link_*`, `vuln_poc_*`
//! - **Conversation store**: `conv_*`
//! - **Execution plans**: `plan_*`
//! - **Security analysis**: `oplog_*`, `target_assets_list`,
//!   `api_endpoints_{list,untested}`, `fingerprints_list`,
//!   `js_analysis_list`, `passive_scans_*`, `target_security_overview`
//! - **Scan runner**: `scan_whatweb`, `match_pocs_for_target`,
//!   `scan_nuclei_targeted`, `nuclei_cancel`, `scan_feroxbuster`,
//!   `get_zap_discovered_paths`
//! - **Sensitive scan**: `sensitive_scan_*`
//!
//! MCP has been extracted to `commands_facade/mcp.rs`.

pub use crate::commands::fs::*;
pub use crate::commands::project::*;
pub use crate::projects::commands::*;
pub use crate::tools::targets::*;
pub use crate::tools::vault::*;
pub use crate::tools::project_io::*;
pub use crate::tools::methodology::*;
pub use crate::tools::recordings::*;
pub use crate::tools::output_parser::*;
pub use crate::tools::findings::*;
pub use crate::tools::scan_queue::*;
pub use crate::tools::custom_rules::*;
pub use crate::tools::notes::*;
pub use crate::tools::audit::*;
pub use crate::tools::wordlists::*;
pub use crate::tools::wiki::*;
pub use crate::tools::conversation_store::*;
pub use crate::tools::conversation_store::batch::*;
pub use crate::tools::execution_plans::*;
pub use crate::tools::security_analysis::*;
pub use crate::tools::scan_runner::*;
pub use crate::tools::sensitive_scan::*;
