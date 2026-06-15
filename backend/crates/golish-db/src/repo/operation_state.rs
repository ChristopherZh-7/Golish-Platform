//! Repository for `operation_state` cursor table (Doc 1 §3.4).
//!
//! 注意: 这不是 operations 表; 没有 valid_until / authz_level / scope (那些走
//! targets / organizations). 这是用户 2026-05-17 删 engagements 后唯一可接受的新表形状.
//!
//! 与现有 `repo/audit.rs` 同步: 自由函数 + `&PgPool`, 无 trait 抽象.

use crate::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// `operation_state` 行映射 (`sqlx::FromRow`).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct OperationStateRow {
    pub operation_id: Uuid,
    pub profile: String,
    pub current_stage: String,
    pub stage_started_at: DateTime<Utc>,
    pub last_evidence_audit_id: Option<i64>,
    pub last_classification_id: Option<i64>,
    pub last_scope_version: Option<i64>,
    pub state_blob: serde_json::Value,
    pub superseded_by: Option<Uuid>,
    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): the
    /// scoping-confirmed root organization id this operation is bound to. Fan-out
    /// / in-scope reads confine to its subtree (root + subsidiaries). `None` = not
    /// yet bound (legacy whole-DB axis).
    pub engagement_org_id: Option<Uuid>,
}

/// 创建一个新 operation_state 行 (新 operation 入口).
pub async fn insert(
    pool: &PgPool,
    operation_id: Uuid,
    profile: &str,
    current_stage: &str,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO operation_state
               (operation_id, profile, current_stage)
           VALUES ($1, $2, $3)"#,
    )
    .bind(operation_id)
    .bind(profile)
    .bind(current_stage)
    .execute(pool)
    .await?;
    Ok(())
}

/// 读 operation_state · 主 lookup.
pub async fn get(pool: &PgPool, operation_id: Uuid) -> Result<Option<OperationStateRow>> {
    let row = sqlx::query_as::<_, OperationStateRow>(
        r#"SELECT operation_id, profile, current_stage, stage_started_at,
                  last_evidence_audit_id, last_classification_id,
                  last_scope_version, state_blob, superseded_by, engagement_org_id
           FROM operation_state
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 推进 cursor (resume 时更新最新 evidence + classification + scope_version 锚).
pub async fn advance_cursor(
    pool: &PgPool,
    operation_id: Uuid,
    last_evidence_audit_id: Option<i64>,
    last_classification_id: Option<i64>,
    last_scope_version: Option<i64>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE operation_state
           SET last_evidence_audit_id = $2,
               last_classification_id = $3,
               last_scope_version = $4
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .bind(last_evidence_audit_id)
    .bind(last_classification_id)
    .bind(last_scope_version)
    .execute(pool)
    .await?;
    Ok(())
}

/// 切换 current_stage + 写新 stage_started_at = NOW().
pub async fn advance_stage(pool: &PgPool, operation_id: Uuid, new_stage: &str) -> Result<()> {
    sqlx::query(
        r#"UPDATE operation_state
           SET current_stage = $2,
               stage_started_at = NOW()
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .bind(new_stage)
    .execute(pool)
    .await?;
    Ok(())
}

/// cross-profile transition (assessment → pentest 等) · 标 superseded_by 但不删原行.
///
/// 调用方应已经先插入新 operation_state(new_operation_id), 再调本 fn 把
/// 老 operation 的 superseded_by 指向新 operation.
pub async fn supersede(
    pool: &PgPool,
    operation_id: Uuid,
    superseded_by_new_operation: Uuid,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE operation_state
           SET superseded_by = $2
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .bind(superseded_by_new_operation)
    .execute(pool)
    .await?;
    Ok(())
}

/// 写入 harness 私有 resume 状态 (state_blob JSONB · 整段覆盖).
pub async fn write_state_blob(
    pool: &PgPool,
    operation_id: Uuid,
    state_blob: serde_json::Value,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE operation_state
           SET state_blob = $2
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .bind(state_blob)
    .execute(pool)
    .await?;
    Ok(())
}

/// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): bind this
/// operation to its scoping-confirmed engagement root org (or clear with `None`).
/// Read back via [`get`] → `OperationStateRow::engagement_org_id`.
pub async fn set_engagement_org(
    pool: &PgPool,
    operation_id: Uuid,
    engagement_org_id: Option<Uuid>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE operation_state
           SET engagement_org_id = $2
           WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .bind(engagement_org_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_state_row_serde_roundtrip() {
        let row = OperationStateRow {
            operation_id: Uuid::new_v4(),
            profile: "assessment".to_string(),
            current_stage: "external_attack_surface".to_string(),
            stage_started_at: Utc::now(),
            last_evidence_audit_id: Some(42),
            last_classification_id: Some(7),
            last_scope_version: Some(3),
            state_blob: serde_json::json!({"sprint_id": "abc"}),
            superseded_by: None,
            engagement_org_id: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_string(&row).expect("serialize");
        let back: OperationStateRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(row.operation_id, back.operation_id);
        assert_eq!(row.current_stage, back.current_stage);
        assert_eq!(row.state_blob, back.state_blob);
    }
}
