//! `PgChainPersistence`: sub-agent message-chain persistence backed by raw
//! sqlx. Moved verbatim from `tracking_bridge.rs`; re-exported by `mod.rs` so
//! `ai::tracking_bridge::PgChainPersistence` stays reachable.

use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::{
    AgentType, CheckpointBoundWorkerChain, LoadBoundWorkerChain, RuntimeMemoryRepository,
    RuntimeWorkerFence, RuntimeWorkerStatus,
};
use sqlx::PgPool;
use uuid::Uuid;

const UPDATE_CHAIN_SQL: &str =
    "UPDATE message_chains SET chain = $1, updated_at = NOW() WHERE id = $2";
const LOAD_CHAIN_BY_ID_SQL: &str = "SELECT chain FROM message_chains \
     WHERE id = $1 AND session_id = $2 AND agent = $3::agent_type";

pub struct PgChainPersistence {
    pool: Arc<PgPool>,
    runtime_memory: Option<Arc<dyn RuntimeMemoryRepository>>,
}

impl PgChainPersistence {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            pool,
            runtime_memory: None,
        }
    }

    pub fn with_runtime_memory_repository(
        mut self,
        repository: Arc<dyn RuntimeMemoryRepository>,
    ) -> Self {
        self.runtime_memory = Some(repository);
        self
    }
}

fn persistence_agent_type(agent_id: &str) -> anyhow::Result<AgentType> {
    match agent_id.trim() {
        "primary" => Ok(AgentType::Primary),
        "coder" => Ok(AgentType::Coder),
        "searcher" => Ok(AgentType::Searcher),
        "memorist" => Ok(AgentType::Memorist),
        "reporter" => Ok(AgentType::Reporter),
        "adviser" => Ok(AgentType::Adviser),
        "reflector" => Ok(AgentType::Reflector),
        "enricher" => Ok(AgentType::Enricher),
        "installer" => Ok(AgentType::Installer),
        // Stage specialists are server-owned pentest workers even when their
        // prompt-level ids are more specific than the persisted DB enum.
        "pentester"
        | "company_stage_controller"
        | "recon"
        | "prober"
        | "enumerator"
        | "vuln_scanner"
        | "attack_analyst"
        | "candidate_verifier" => Ok(AgentType::Pentester),
        other => anyhow::bail!("unsupported bound-worker persistence agent '{other}'"),
    }
}

#[async_trait]
impl golish_sub_agents::SubAgentChainPersistence for PgChainPersistence {
    async fn chain_create(
        &self,
        session_id: Uuid,
        task_id: Option<Uuid>,
        subtask_id: Option<Uuid>,
        agent_type: &str,
        _parent_chain_id: Option<Uuid>,
        _model: Option<&str>,
    ) -> anyhow::Result<Uuid> {
        let (id,): (Uuid,) = sqlx::query_as(
            r#"INSERT INTO message_chains (session_id, task_id, subtask_id, agent)
               VALUES ($1, $2, $3, $4::agent_type) RETURNING id"#,
        )
        .bind(session_id)
        .bind(task_id)
        .bind(subtask_id)
        .bind(agent_type)
        .fetch_one(self.pool.as_ref())
        .await?;
        Ok(id)
    }

    async fn chain_update(&self, id: Uuid, chain_json: &serde_json::Value) -> anyhow::Result<()> {
        let result = sqlx::query(UPDATE_CHAIN_SQL)
            .bind(chain_json)
            .bind(id)
            .execute(self.pool.as_ref())
            .await?;
        if result.rows_affected() != 1 {
            anyhow::bail!(
                "message chain {id} update affected {} rows",
                result.rows_affected()
            );
        }
        Ok(())
    }

    async fn chain_update_usage(
        &self,
        id: Uuid,
        input_tokens: i32,
        output_tokens: i32,
        _cache_read_tokens: i32,
        _input_cost: f64,
        _output_cost: f64,
        duration_ms: i32,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"UPDATE message_chains
               SET tokens_in = COALESCE(tokens_in, 0) + $1,
                   tokens_out = COALESCE(tokens_out, 0) + $2,
                   duration_ms = COALESCE(duration_ms, 0) + $3,
                   updated_at = NOW()
               WHERE id = $4"#,
        )
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(duration_ms)
        .bind(id)
        .execute(self.pool.as_ref())
        .await?;
        Ok(())
    }

    async fn chain_load_latest(
        &self,
        session_id: Uuid,
        _task_id: Option<Uuid>,
        agent_type: &str,
    ) -> anyhow::Result<Option<(Uuid, serde_json::Value)>> {
        // Most recently updated persisted chain for this (session, agent). The
        // `chain IS NOT NULL` filter skips freshly-created rows that never got
        // a saved conversation, so resume only picks a chain with real content.
        let row: Option<(Uuid, Option<serde_json::Value>)> = sqlx::query_as(
            r#"SELECT id, chain FROM message_chains
               WHERE session_id = $1 AND agent = $2::agent_type AND chain IS NOT NULL
               ORDER BY updated_at DESC
               LIMIT 1"#,
        )
        .bind(session_id)
        .bind(agent_type)
        .fetch_optional(self.pool.as_ref())
        .await?;
        Ok(row.and_then(|(id, chain)| chain.map(|c| (id, c))))
    }

    async fn chain_load_by_id(
        &self,
        chain_id: Uuid,
        session_id: Uuid,
        agent_type: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let row: Option<(Option<serde_json::Value>,)> = sqlx::query_as(LOAD_CHAIN_BY_ID_SQL)
            .bind(chain_id)
            .bind(session_id)
            .bind(agent_type)
            .fetch_optional(self.pool.as_ref())
            .await?;
        Ok(row.and_then(|(chain,)| chain))
    }

    async fn chain_load_bound_worker(
        &self,
        bound: &golish_sub_agents::BoundWorkerChainContext,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let repository = self.runtime_memory.as_ref().ok_or_else(|| {
            anyhow::anyhow!("runtime-memory repository is unavailable for bound worker load")
        })?;
        let loaded = repository
            .load_bound_worker_chain(LoadBoundWorkerChain {
                operation_id: bound.operation_id,
                stage_execution_id: bound.stage_execution_id,
                stage_run_unit_id: bound.worker_lease.stage_run_unit_id,
                worker_run_id: bound.worker_lease.worker_run_id,
                message_chain_id: bound.chain_id,
                session_id: bound.session_id,
                agent: persistence_agent_type(&bound.agent_type)?,
                selected_source: bound.runtime_memory_source.map(|source| match source {
                    golish_sub_agents::BoundWorkerRuntimeMemorySource::Legacy => {
                        golish_agent_kit::db_traits::RuntimeMemoryRecordSource::Legacy
                    }
                    golish_sub_agents::BoundWorkerRuntimeMemorySource::V2 => {
                        golish_agent_kit::db_traits::RuntimeMemoryRecordSource::V2
                    }
                    golish_sub_agents::BoundWorkerRuntimeMemorySource::LegacyFallback => {
                        golish_agent_kit::db_traits::RuntimeMemoryRecordSource::LegacyFallback
                    }
                }),
            })
            .await?;
        anyhow::ensure!(
            loaded.worker.lease_token == Some(bound.worker_lease.lease_token)
                && loaded.worker.attempt_epoch == bound.worker_lease.attempt_epoch,
            "bound worker load returned a different lease fence"
        );
        anyhow::ensure!(
            loaded.worker.status == RuntimeWorkerStatus::Running
                && loaded
                    .worker
                    .lease_expires_at
                    .is_some_and(|expires_at| expires_at > chrono::Utc::now()),
            "bound worker load returned a non-running or expired lease"
        );
        anyhow::ensure!(
            loaded.worker.checkpoint_version == bound.current_checkpoint_version(),
            "bound worker load returned checkpoint version {}, expected {}",
            loaded.worker.checkpoint_version,
            bound.current_checkpoint_version()
        );
        Ok(Some(loaded.chain))
    }

    async fn chain_checkpoint_bound_worker(
        &self,
        bound: &golish_sub_agents::BoundWorkerChainContext,
        chain_id: Uuid,
        chain_json: &serde_json::Value,
        expected_checkpoint_version: i64,
    ) -> anyhow::Result<i64> {
        anyhow::ensure!(chain_id == bound.chain_id, "bound chain identity mismatch");
        let repository = self.runtime_memory.as_ref().ok_or_else(|| {
            anyhow::anyhow!("runtime-memory repository is unavailable for bound checkpoint")
        })?;
        let worker = repository
            .checkpoint_bound_worker_chain(CheckpointBoundWorkerChain {
                fence: RuntimeWorkerFence {
                    operation_id: bound.operation_id,
                    stage_execution_id: bound.stage_execution_id,
                    stage_run_unit_id: bound.worker_lease.stage_run_unit_id,
                    worker_run_id: bound.worker_lease.worker_run_id,
                    lease_token: bound.worker_lease.lease_token,
                    attempt_epoch: bound.worker_lease.attempt_epoch,
                    expected_checkpoint_version,
                },
                message_chain_id: chain_id,
                chain: chain_json.clone(),
                checkpoint: chain_json.clone(),
            })
            .await?;
        Ok(worker.checkpoint_version)
    }

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT template_name, content FROM prompt_templates WHERE is_active = true",
        )
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{persistence_agent_type, LOAD_CHAIN_BY_ID_SQL, UPDATE_CHAIN_SQL};
    use golish_agent_kit::db_traits::AgentType;

    #[test]
    fn exact_chain_load_is_scoped_to_id_session_and_agent() {
        assert!(LOAD_CHAIN_BY_ID_SQL.contains("id = $1"));
        assert!(LOAD_CHAIN_BY_ID_SQL.contains("session_id = $2"));
        assert!(LOAD_CHAIN_BY_ID_SQL.contains("agent = $3::agent_type"));
    }

    #[test]
    fn chain_update_sql_targets_one_exact_id() {
        assert!(UPDATE_CHAIN_SQL.contains("WHERE id = $2"));
    }

    #[test]
    fn chain_bound_worker_maps_stage_specialists_to_persisted_pentester_type() {
        for specialist in [
            "company_stage_controller",
            "recon",
            "prober",
            "enumerator",
            "vuln_scanner",
            "attack_analyst",
            "candidate_verifier",
            "pentester",
        ] {
            assert_eq!(
                persistence_agent_type(specialist).expect("known specialist"),
                AgentType::Pentester
            );
        }
        assert!(persistence_agent_type("model_invented_agent").is_err());
    }
}
