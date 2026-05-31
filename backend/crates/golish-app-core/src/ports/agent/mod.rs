//! Agent service outbound ports (servitization S1-2e).
//!
//! The consuming platform service holds `Arc<dyn AgentLogReadPort>` instead of
//! calling `golish_db::repo::{agent_logs,search_logs}` directly. The
//! `Pg*Adapter` is the single guarded repo-calling site.

pub mod logs;

pub use logs::{AgentLogGlobal, AgentLogReadPort, PgAgentLogAdapter, SearchLogGlobal};
