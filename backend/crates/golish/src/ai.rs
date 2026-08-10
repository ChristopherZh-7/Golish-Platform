//! AI / agent command surface — re-exported from `golish-agent-app`.
//!
//! The entire `ai/` subtree (the `ai/commands/*` Tauri handlers, the
//! AppState-free agent bridges, and the agent-runtime re-exports) was moved
//! into the `golish-agent-app` crate (crate-per-service M4-proper). This shim
//! keeps the historical `crate::ai::*` paths working for golish-staying
//! consumers (`state`, `app/mcp_bootstrap`, `mcp/commands`, `cli/bootstrap`)
//! and for the `commands_facade::ai` re-export feeding `generate_handler!`.
pub use golish_agent_app::ai::*;

/// Single desktop composition seam for the process-owned Investigation
/// projector and its commit-visible refresh publisher.
pub fn compose_investigation_projection_event_bridge(
    pool: std::sync::Arc<sqlx::PgPool>,
) -> InvestigationProjectionEventBridge {
    InvestigationProjectionEventBridge::new(pool)
}
