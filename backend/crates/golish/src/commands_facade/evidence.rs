//! Evidence Ledger IPC commands (Phase 1b · Doc 2 §3).
//!
//! Per-domain facade re-export. The home module is `crate::tools::evidence`.
//!
//! Currently exposed:
//!   - `evidence_read`: 取 sanitize 后的 evidence summary, 供 LLM 通过
//!     `read_evidence(eid, summary_level)` 调用; 替代直接把 raw 进上下文.
//!
//! Future commands (Phase 2+):
//!   - `evidence_list_by_stage`
//!   - `evidence_list_by_target`

pub use golish_pentest_app::evidence::*;
