//! Workspace tools — the remaining catch-all after vault / wiki /
//! findings were extracted in N5 (2026-05-02). Each sub-block below
//! is still a candidate for its own facade if it grows further.
//!
//! Already extracted (see neighbouring files):
//! - `commands_facade/vault.rs`     — credential vault
//! - `commands_facade/wiki.rs`      — wiki / KB research / vuln links
//! - `commands_facade/findings.rs`  — findings + evidence
//! - `commands_facade/mcp.rs`       — MCP server/tool wrappers
//!
//! Expected command groups still in this file (documentation only):
//! - **Workspace files** (from `crate::commands::fs`):
//!   `list_workspace_files`, `list_directory`,
//!   `read_workspace_file`, `write_workspace_file`,
//!   `stat_workspace_file`, `read_file_as_base64`,
//!   `watch_file`, `unwatch_file`, `unwatch_all_files`,
//!   `list_path_completions`
//! - **Project assets** (from `crate::commands::project`):
//!   `list_prompts`, `read_prompt`, `*_skill*`, `*_rule*`
//! - **Projects (DB-backed)** (from `crate::projects::commands`):
//!   `save_project`, `*_project_config*`, `*_project_workspace`,
//!   `*_pentest_config`, `list_captures*`, `read_project_file`,
//!   `init_project_structure`, `clean_project_temp`
//! - **Targets**: `target_{list,add,batch_add,update,update_status,
//!   delete,clear_all}`, `directory_entry_list`
//! - **Project I/O**: `project_export`, `project_import`
//! - **Methodology**: `method_*`
//! - **Recordings**: `recording_*`
//! - **Output parser**: `output_*`
//! - **Scan queue**: `scan_queue_*`
//! - **Custom rules**: `custom_rules_*`
//! - **Notes**: `notes_*`
//! - **Audit log**: `audit_*`, `*_logs_list`, `passive_scans_global`
//! - **Wordlists**: `wordlist_*`
//! - **Conversation store**: `conv_*`
//! - **Execution plans**: `plan_*`
//! - **Security analysis**: `oplog_*`, `target_assets_list`,
//!   `api_endpoints_{list,untested}`, `fingerprints_list`,
//!   `js_analysis_list`, `passive_scans_*`, `target_security_overview`
//! - **Scan runner**: `scan_whatweb`, `match_pocs_for_target`,
//!   `scan_nuclei_targeted`, `nuclei_cancel`, `scan_feroxbuster`,
//!   `get_zap_discovered_paths`
//! - **Sensitive scan**: `sensitive_scan_*`

pub use crate::commands::fs::*;
pub use crate::commands::project::*;
pub use crate::projects::commands::*;
pub use crate::tools::audit::*;
pub use crate::tools::conversation_store::batch::*;
pub use crate::tools::conversation_store::*;
pub use crate::tools::custom_rules::*;
pub use crate::tools::engagements::*;
pub use crate::tools::execution_plans::*;
pub use crate::tools::organizations::*;
pub use crate::tools::methodology::*;
pub use crate::tools::notes::*;
pub use crate::tools::output_parser::*;
pub use crate::tools::project_io::*;
pub use crate::tools::recordings::*;
pub use crate::tools::scan_queue::*;
pub use crate::tools::scan_runner::*;
pub use crate::tools::security_analysis::*;
pub use crate::tools::sensitive_scan::*;
pub use crate::tools::targets::*;
pub use crate::tools::wordlists::*;
