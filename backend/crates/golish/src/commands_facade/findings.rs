//! Findings (vulnerability/issue records) IPC commands.
//!
//! Expected commands exposed here (documentation only):
//! - `findings_list`, `findings_for_host`
//! - `findings_add`, `findings_update`, `findings_delete`
//! - `findings_import_parsed` (bulk-insert from output-parser)
//! - `findings_add_evidence`, `findings_remove_evidence`,
//!   `findings_evidence_path` (binary attachments — screenshots,
//!   request/response captures, raw tool stdout)
//! - `findings_deduplicate` (collapse near-identical findings)
//!
//! Extracted from `commands_facade/workspace.rs` on 2026-05-02 (N5).

pub use crate::tools::findings::*;
