//! Repository for `sprint_contracts` table (Doc 3 §7).
//!
//! Profile-driven sprint skeleton + planner LLM 填变量 → locked-at-stage-start.
//! status: 'active' / 'superseded' / 'expired'. supersede 链以 superseded_by 串成
//! 单向链表; 一个 stage_run 同时只能有一条 active 合同.
//!
//! 与现有 `repo/audit.rs` 同步: 自由函数 + `&PgPool`, 无 trait 抽象.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// `sprint_contracts` 行映射 (`sqlx::FromRow`).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct SprintContractRow {
    pub id: Uuid,
    pub stage_run_id: Uuid,
    pub contract_text: String,
    pub locked_after: DateTime<Utc>,
    pub superseded_by: Option<Uuid>,
    /// 'active' / 'superseded' / 'expired'
    pub status: String,
    pub planner_llm_id: String,
    pub created_at: DateTime<Utc>,
}

/// 创建一个新 sprint_contract (locked_after 是 planner 生成完毕的瞬时锚点).
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    id: Uuid,
    stage_run_id: Uuid,
    contract_text: &str,
    locked_after: DateTime<Utc>,
    planner_llm_id: &str,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO sprint_contracts
               (id, stage_run_id, contract_text, locked_after, planner_llm_id)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(id)
    .bind(stage_run_id)
    .bind(contract_text)
    .bind(locked_after)
    .bind(planner_llm_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 读 sprint_contract.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<SprintContractRow>> {
    let row = sqlx::query_as::<_, SprintContractRow>(
        r#"SELECT id, stage_run_id, contract_text, locked_after, superseded_by,
                  status, planner_llm_id, created_at
           FROM sprint_contracts
           WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 找某 stage_run 的当前 active 合同 (status='active' 且 superseded_by IS NULL).
pub async fn active_for_stage_run(
    pool: &PgPool,
    stage_run_id: Uuid,
) -> Result<Option<SprintContractRow>> {
    let row = sqlx::query_as::<_, SprintContractRow>(
        r#"SELECT id, stage_run_id, contract_text, locked_after, superseded_by,
                  status, planner_llm_id, created_at
           FROM sprint_contracts
           WHERE stage_run_id = $1 AND status = 'active' AND superseded_by IS NULL
           ORDER BY created_at DESC, id ASC
           LIMIT 1"#,
    )
    .bind(stage_run_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 列出某 stage_run 的全部 contracts (含 superseded · 按 created_at 升序).
pub async fn list_for_stage_run(
    pool: &PgPool,
    stage_run_id: Uuid,
) -> Result<Vec<SprintContractRow>> {
    let rows = sqlx::query_as::<_, SprintContractRow>(
        r#"SELECT id, stage_run_id, contract_text, locked_after, superseded_by,
                  status, planner_llm_id, created_at
           FROM sprint_contracts
           WHERE stage_run_id = $1
           ORDER BY created_at ASC, id ASC"#,
    )
    .bind(stage_run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 标记某条合同 superseded → 同步把 status 设 'superseded' (走 supersede 链).
pub async fn mark_superseded(
    pool: &PgPool,
    id: Uuid,
    superseded_by_new_contract: Uuid,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE sprint_contracts
           SET superseded_by = $2,
               status = 'superseded'
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(superseded_by_new_contract)
    .execute(pool)
    .await?;
    Ok(())
}

/// 显式过期某合同 (例如用户取消 stage). status='expired'.
pub async fn mark_expired(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query(
        r#"UPDATE sprint_contracts
           SET status = 'expired'
           WHERE id = $1"#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprint_contract_row_serde_roundtrip() {
        let row = SprintContractRow {
            id: Uuid::new_v4(),
            stage_run_id: Uuid::new_v4(),
            contract_text: "expected_findings: 1-200 subdomains".to_string(),
            locked_after: Utc::now(),
            superseded_by: None,
            status: "active".to_string(),
            planner_llm_id: "openai:gpt-4o".to_string(),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&row).expect("serialize");
        let back: SprintContractRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(row.id, back.id);
        assert_eq!(row.stage_run_id, back.stage_run_id);
        assert_eq!(row.status, back.status);
        assert_eq!(row.planner_llm_id, back.planner_llm_id);
    }
}
