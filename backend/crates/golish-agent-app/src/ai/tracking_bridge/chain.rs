//! `PgChainPersistence`: sub-agent message-chain persistence backed by raw
//! sqlx. Moved verbatim from `tracking_bridge.rs`; re-exported by `mod.rs` so
//! `ai::tracking_bridge::PgChainPersistence` stays reachable.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

pub struct PgChainPersistence {
    pool: Arc<PgPool>,
}

impl PgChainPersistence {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
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
        sqlx::query("UPDATE message_chains SET chain = $1, updated_at = NOW() WHERE id = $2")
            .bind(chain_json)
            .bind(id)
            .execute(self.pool.as_ref())
            .await?;
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

    async fn load_prompt_template_overrides(&self) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT template_name, content FROM prompt_templates WHERE is_active = true",
        )
        .fetch_all(self.pool.as_ref())
        .await
        .unwrap_or_default()
    }
}
