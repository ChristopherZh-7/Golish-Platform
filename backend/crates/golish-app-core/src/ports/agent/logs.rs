//! `AgentLogReadPort` — agent-owned activity logs as a service port (S1-2e).
//!
//! The in-proc adapter mirrors `golish_db::repo::{agent_logs,search_logs}`
//! exactly. It is the ONLY place the consuming platform service reaches these
//! agent repos; it lives under the agent port domain so the ownership guard
//! treats it as agent-owned. The repo `list_by_project<T>` is generic over the
//! caller's projection, which an object-safe trait cannot expose, so the port
//! owns its own remote-ready DTOs ([`AgentLogGlobal`] / [`SearchLogGlobal`])
//! that mirror the fixed column projection.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// Project-wide agent activity log projection (mirrors the `agent_logs`
/// list-by-project column subset).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentLogGlobal {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub initiator: String,
    pub executor: String,
    pub task: String,
    pub result: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Project-wide search log projection (mirrors the `search_logs`
/// list-by-project column subset).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SearchLogGlobal {
    pub id: Uuid,
    pub session_id: Uuid,
    pub task_id: Option<Uuid>,
    pub subtask_id: Option<Uuid>,
    pub initiator: Option<String>,
    pub engine: String,
    pub query: String,
    pub result: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Outbound port for reading agent-owned activity logs (read-only).
#[async_trait]
pub trait AgentLogReadPort: Send + Sync {
    async fn agent_logs_list_by_project(
        &self,
        project_path: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<AgentLogGlobal>>;

    async fn search_logs_list_by_project(
        &self,
        project_path: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<SearchLogGlobal>>;
}

/// In-proc adapter backed by the embedded Postgres pool.
pub struct PgAgentLogAdapter {
    pool: Arc<PgPool>,
}

impl PgAgentLogAdapter {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AgentLogReadPort for PgAgentLogAdapter {
    async fn agent_logs_list_by_project(
        &self,
        project_path: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<AgentLogGlobal>> {
        Ok(
            golish_db::repo::agent_logs::list_by_project::<AgentLogGlobal>(
                self.pool.as_ref(),
                project_path,
                limit,
            )
            .await?,
        )
    }

    async fn search_logs_list_by_project(
        &self,
        project_path: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<SearchLogGlobal>> {
        Ok(
            golish_db::repo::search_logs::list_by_project::<SearchLogGlobal>(
                self.pool.as_ref(),
                project_path,
                limit,
            )
            .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_log_read_port_is_object_safe() {
        fn _assert(_: &dyn AgentLogReadPort) {}
    }
}
