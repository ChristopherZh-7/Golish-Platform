//! Repository for `stage_runs` table (Doc 3 §7 prerequisite for sprint_contracts).
//!
//! 每个 stage 的运行实例. 一个 operation 下可能多次跑同一 stage_kind (例如
//! external_attack_surface 第一次 gate fail → 第二次 retry).
//!
//! 与现有 `repo/audit.rs` 同步: 自由函数 + `&PgPool`, 无 trait 抽象.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// `stage_runs` 行映射 (`sqlx::FromRow`).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct StageRunRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_kind: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    /// 'started' / 'completed' / 'failed' / 'paused_needs_user'
    pub status: String,
    pub active_sprint_contract_id: Option<Uuid>,
}

/// 创建一个新 stage_run · 调用方负责生成 UUID 并保证唯一.
pub async fn insert(pool: &PgPool, id: Uuid, operation_id: Uuid, stage_kind: &str) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO stage_runs (id, operation_id, stage_kind)
           VALUES ($1, $2, $3)"#,
    )
    .bind(id)
    .bind(operation_id)
    .bind(stage_kind)
    .execute(pool)
    .await?;
    Ok(())
}

/// 读 stage_run.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<StageRunRow>> {
    let row = sqlx::query_as::<_, StageRunRow>(
        r#"SELECT id, operation_id, stage_kind, started_at, completed_at,
                  status, active_sprint_contract_id
           FROM stage_runs
           WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 列出某 operation 全部 stage_runs (按 started_at 升序 · 含 retry 多行).
pub async fn list_for_operation(pool: &PgPool, operation_id: Uuid) -> Result<Vec<StageRunRow>> {
    let rows = sqlx::query_as::<_, StageRunRow>(
        r#"SELECT id, operation_id, stage_kind, started_at, completed_at,
                  status, active_sprint_contract_id
           FROM stage_runs
           WHERE operation_id = $1
           ORDER BY started_at ASC, id ASC"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// stage 结束 (status + completed_at NOW()) · 终态: completed / failed / paused_needs_user.
pub async fn mark_terminal(pool: &PgPool, id: Uuid, new_status: &str) -> Result<()> {
    sqlx::query(
        r#"UPDATE stage_runs
           SET status = $2,
               completed_at = NOW()
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(new_status)
    .execute(pool)
    .await?;
    Ok(())
}

/// 在 stage_run 上挂当前 active sprint_contract (insert 后调一次).
pub async fn set_active_sprint_contract(
    pool: &PgPool,
    id: Uuid,
    sprint_contract_id: Uuid,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE stage_runs
           SET active_sprint_contract_id = $2
           WHERE id = $1"#,
    )
    .bind(id)
    .bind(sprint_contract_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_run_row_serde_roundtrip() {
        let row = StageRunRow {
            id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            stage_kind: "external_attack_surface".to_string(),
            started_at: Utc::now(),
            completed_at: None,
            status: "started".to_string(),
            active_sprint_contract_id: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_string(&row).expect("serialize");
        let back: StageRunRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(row.id, back.id);
        assert_eq!(row.stage_kind, back.stage_kind);
        assert_eq!(row.status, back.status);
    }
}
