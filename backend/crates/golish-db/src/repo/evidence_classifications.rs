//! Repository for `evidence_classifications` table (Doc 1 §3.2).
//!
//! bitemporal · append-only. 当前活跃分类 = WHERE valid_to IS NULL (partial
//! unique index 保唯一). Re-label 时关闭老行 (UPDATE valid_to=NOW()) + 插新行
//! (INSERT valid_from=NOW(), valid_to=NULL), **必须在同一事务**避免 partial
//! state.
//!
//! 与现有 `repo/audit.rs` 同步: 自由函数 + `&PgPool`, 无 trait 抽象.

use crate::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

/// `evidence_classifications` 行映射 (`sqlx::FromRow`).
#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize)]
pub struct ClassificationRow {
    pub id: i64,
    pub evidence_audit_id: i64,
    /// 'in_scope' / 'out_of_scope' / 'derived_from_out_of_scope'
    pub classification: String,
    pub scope_version: i64,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub reason: String,
    pub relabel_decision: Option<String>,
    pub classified_by_session: String,
    pub producing_stage_run_id: Option<Uuid>,
    pub schema_v: i32,
}

/// 插一行 classification (使用方负责保证 partial unique index 不冲突 ·
/// 通常通过 close_current_open_new 走事务).
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    evidence_audit_id: i64,
    classification: &str,
    scope_version: i64,
    reason: &str,
    classified_by_session: &str,
    producing_stage_run_id: Option<Uuid>,
    relabel_decision: Option<&str>,
) -> Result<i64> {
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO evidence_classifications
               (evidence_audit_id, classification, scope_version, reason,
                classified_by_session, producing_stage_run_id, relabel_decision)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#,
    )
    .bind(evidence_audit_id)
    .bind(classification)
    .bind(scope_version)
    .bind(reason)
    .bind(classified_by_session)
    .bind(producing_stage_run_id)
    .bind(relabel_decision)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Insert the initial classification on an existing caller-owned transaction.
///
/// Evidence creation uses this variant so the `audit_log` evidence row and its
/// first active classification either both commit or both roll back. Keep this
/// SQL aligned with [`insert`].
#[allow(clippy::too_many_arguments)]
pub async fn insert_in_transaction(
    connection: &mut PgConnection,
    evidence_audit_id: i64,
    classification: &str,
    scope_version: i64,
    reason: &str,
    classified_by_session: &str,
    producing_stage_run_id: Option<Uuid>,
    relabel_decision: Option<&str>,
) -> Result<i64> {
    let id: i64 = sqlx::query_scalar(
        r#"INSERT INTO evidence_classifications
               (evidence_audit_id, classification, scope_version, reason,
                classified_by_session, producing_stage_run_id, relabel_decision)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#,
    )
    .bind(evidence_audit_id)
    .bind(classification)
    .bind(scope_version)
    .bind(reason)
    .bind(classified_by_session)
    .bind(producing_stage_run_id)
    .bind(relabel_decision)
    .fetch_one(connection)
    .await?;
    Ok(id)
}

/// 读当前活跃 (valid_to IS NULL) 的分类行.
pub async fn current_for(
    pool: &PgPool,
    evidence_audit_id: i64,
) -> Result<Option<ClassificationRow>> {
    let row = sqlx::query_as::<_, ClassificationRow>(
        r#"SELECT id, evidence_audit_id, classification, scope_version,
                  valid_from, valid_to, reason, relabel_decision,
                  classified_by_session, producing_stage_run_id, schema_v
           FROM evidence_classifications
           WHERE evidence_audit_id = $1 AND valid_to IS NULL
           LIMIT 1"#,
    )
    .bind(evidence_audit_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// 关闭当前活跃行 + 插新行 (事务边界 · Doc 1 §7).
///
/// 单 partial unique index 在并发时一边 INSERT 时另一边触发约束冲突 → 调用方需重读
/// latest 并决定是否再次 INSERT (参考 §21.7.6).
#[allow(clippy::too_many_arguments)]
pub async fn close_current_open_new(
    pool: &PgPool,
    evidence_audit_id: i64,
    new_classification: &str,
    new_scope_version: i64,
    new_reason: &str,
    new_classified_by_session: &str,
    new_producing_stage_run_id: Option<Uuid>,
    new_relabel_decision: Option<&str>,
) -> Result<i64> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"UPDATE evidence_classifications
           SET valid_to = NOW()
           WHERE evidence_audit_id = $1 AND valid_to IS NULL"#,
    )
    .bind(evidence_audit_id)
    .execute(&mut *tx)
    .await?;

    let new_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO evidence_classifications
               (evidence_audit_id, classification, scope_version, reason,
                classified_by_session, producing_stage_run_id, relabel_decision)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#,
    )
    .bind(evidence_audit_id)
    .bind(new_classification)
    .bind(new_scope_version)
    .bind(new_reason)
    .bind(new_classified_by_session)
    .bind(new_producing_stage_run_id)
    .bind(new_relabel_decision)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(new_id)
}

/// 列出某条 evidence 全部 re-label 历史 (按 valid_from 升序).
pub async fn list_supersedes_chain(
    pool: &PgPool,
    evidence_audit_id: i64,
) -> Result<Vec<ClassificationRow>> {
    let rows = sqlx::query_as::<_, ClassificationRow>(
        r#"SELECT id, evidence_audit_id, classification, scope_version,
                  valid_from, valid_to, reason, relabel_decision,
                  classified_by_session, producing_stage_run_id, schema_v
           FROM evidence_classifications
           WHERE evidence_audit_id = $1
           ORDER BY valid_from ASC, id ASC"#,
    )
    .bind(evidence_audit_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 按 stage_run_id 列出该 stage 内所有 evidence 的当前分类 (gate 用).
pub async fn list_current_for_stage_run(
    pool: &PgPool,
    stage_run_id: Uuid,
) -> Result<Vec<ClassificationRow>> {
    let rows = sqlx::query_as::<_, ClassificationRow>(
        r#"SELECT id, evidence_audit_id, classification, scope_version,
                  valid_from, valid_to, reason, relabel_decision,
                  classified_by_session, producing_stage_run_id, schema_v
           FROM evidence_classifications
           WHERE producing_stage_run_id = $1 AND valid_to IS NULL
           ORDER BY id ASC"#,
    )
    .bind(stage_run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_row_serde_roundtrip() {
        // Pure-data test: 确保 Row 字段名与序列化兼容
        let row = ClassificationRow {
            id: 1,
            evidence_audit_id: 100,
            classification: "in_scope".to_string(),
            scope_version: 7,
            valid_from: Utc::now(),
            valid_to: None,
            reason: "ScopeService classify".to_string(),
            relabel_decision: None,
            classified_by_session: "session-abc".to_string(),
            producing_stage_run_id: Some(Uuid::new_v4()),
            schema_v: 1,
        };
        let json = serde_json::to_string(&row).expect("serialize");
        let back: ClassificationRow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(row.id, back.id);
        assert_eq!(row.classification, back.classification);
        assert_eq!(row.scope_version, back.scope_version);
    }
}
