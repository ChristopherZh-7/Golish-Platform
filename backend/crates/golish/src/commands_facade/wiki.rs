//! Wiki / KB research / vulnerability links commands (~30 commands).
//!
//! Expected commands exposed here (documentation only):
//! - **Wiki pages**: `wiki_init`, `wiki_reindex`, `wiki_list`, `wiki_read`,
//!   `wiki_write`, `wiki_delete`, `wiki_rename`, `wiki_create_dir`,
//!   `wiki_search`, `wiki_search_db`, `wiki_stats`, `wiki_create_cve`
//! - **Wiki indexes / metadata**: `wiki_pages_grouped`, `wiki_pages_for_paths`,
//!   `wiki_suggest_for_cve`, `wiki_changelog_list`, `wiki_backlinks`,
//!   `wiki_stats_full`, `wiki_orphan_pages`
//! - **KB research log**: `kb_research_load`, `kb_research_save_turn`,
//!   `kb_research_set_status`, `kb_research_clear`
//! - **Vuln links** (CVE ↔ wiki / poc / scan): `vuln_link_get_all`,
//!   `vuln_link_get`, `vuln_link_add_wiki`, `vuln_link_remove_wiki`,
//!   `vuln_link_add_poc`, `vuln_link_update_poc`, `vuln_link_remove_poc`,
//!   `vuln_link_add_scan`, `vuln_link_remove_scan`, `vuln_link_add_poc_full`
//! - **Vuln POC catalog**: `vuln_poc_list_cves`, `vuln_poc_list_unresearched`,
//!   `vuln_poc_stats`, `vuln_poc_set_verified`
//!
//! Extracted from `commands_facade/workspace.rs` on 2026-05-02 (N5).

// Extracted to the `golish-vuln-app` crate (crate-per-service split M1b).
// The glob re-exports both the command fns and their `__cmd__$name` macros so
// the aggregate `generate_handler!` in `commands_registry.rs` resolves them.
pub use golish_vuln_app::wiki::*;
