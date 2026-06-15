//! Repository for `org_stage_completions` — the per-(organization, stage)
//! completion ledger that `stage_run` consults to resume-skip an org whose stage
//! already passed its gate within a freshness window.
//!
//! 与现有 `repo/stage_runs.rs` 同款: 自由函数 + `&PgPool`, 无 trait 抽象。
//! 关键语义: 一行一个 `(organization_id, stage_kind)`，`upsert` 把它刷新到最新
//! 一次通过；`passed_at` 是新鲜度时钟（"上次啥时候测的"），跳过判定由调用方按
//! TTL 比较 `passed_at`（策略不落在 SQL 层，便于调参/单测）。

use crate::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// `org_stage_completions` 行映射。
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OrgStageCompletion {
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub passed_at: DateTime<Utc>,
    pub stage_run_id: Option<String>,
}

/// 记录/刷新一个 org 在某 stage 的通过（org 过自己那关 gate 时调用）。
/// UNIQUE(organization_id, stage_kind) → 已有则把 `passed_at` 刷到 NOW()。
pub async fn upsert(
    pool: &PgPool,
    organization_id: Uuid,
    stage_kind: &str,
    stage_run_id: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO org_stage_completions
               (organization_id, stage_kind, passed_at, stage_run_id, updated_at)
           VALUES ($1, $2, NOW(), $3, NOW())
           ON CONFLICT (organization_id, stage_kind)
           DO UPDATE SET passed_at = NOW(),
                         stage_run_id = EXCLUDED.stage_run_id,
                         updated_at = NOW()"#,
    )
    .bind(organization_id)
    .bind(stage_kind)
    .bind(stage_run_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 读某 org 某 stage 的最近一次通过（无记录 = 从未完成）。TTL 比较由调用方做。
pub async fn get(
    pool: &PgPool,
    organization_id: Uuid,
    stage_kind: &str,
) -> Result<Option<OrgStageCompletion>> {
    let row = sqlx::query_as::<_, OrgStageCompletion>(
        r#"SELECT organization_id, stage_kind, passed_at, stage_run_id
           FROM org_stage_completions
           WHERE organization_id = $1 AND stage_kind = $2"#,
    )
    .bind(organization_id)
    .bind(stage_kind)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_row_serde_roundtrip() {
        let row = OrgStageCompletion {
            organization_id: Uuid::new_v4(),
            stage_kind: "target_intel".to_string(),
            passed_at: Utc::now(),
            stage_run_id: Some("call_00_abc::org::xyz".to_string()),
        };
        let json = serde_json::to_string(&row).expect("serialize");
        let back: OrgStageCompletion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(row.organization_id, back.organization_id);
        assert_eq!(row.stage_kind, back.stage_kind);
        assert_eq!(row.stage_run_id, back.stage_run_id);
    }
}
