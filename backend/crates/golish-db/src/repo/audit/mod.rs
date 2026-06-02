//! `audit_log` repository.
//!
//! Split by concern: this file holds the core insert (`log` /
//! `log_operation` / `log_operation_with_lineage`) and the startup reclaim
//! helpers; the sibling submodules hold reads (`queries`), the pentest
//! lineage writer (`pentest`), and the cross-table activity timeline
//! (`timeline`). Their public items are re-exported here so callers keep
//! using `repo::audit::*` unchanged.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::AuditEntry;
use crate::Result;

mod pentest;
mod queries;
mod timeline;

pub use pentest::PentestAudit;
pub use queries::{
    clear, clear_by_project_exact, count, list, list_by_category, list_by_project_exact,
    list_by_session, list_by_target, search,
};
pub use timeline::{target_timeline, TimelineEntry};

/// 默认 startup reclaim 阈值: 超过 1 小时未到终态的 `status='started'` 行被
/// 视为 abandoned (process crash / OOM / wait_message timeout 导致的孤儿行).
///
/// Doc 1 §5.3 fire-and-forget reclaim 规则.
pub const DEFAULT_RECLAIM_THRESHOLD_HOURS: i64 = 1;

/// 把 audit_log 中超过 `threshold` 仍处于 `status='started'` 的孤儿行标 'abandoned'.
///
/// 防止后续 evidence_classifications 误引用 abandoned 行 (§5.3 不补的后果).
///
/// 返回被 reclaim 的行数. 失败时通过 anyhow::Error 暴露, 调用方决定是否
/// fatal (`GolishDb::start` 选 log + continue, 不 panic).
///
/// audit_log 没有 started_at 字段, 用 created_at (insert 时间) 做时间锚.
pub async fn reclaim_abandoned_audits(pool: &PgPool, threshold: Duration) -> Result<u64> {
    let cutoff = reclaim_cutoff(threshold);
    let result = sqlx::query(
        r#"UPDATE audit_log
           SET status = 'abandoned'
           WHERE status = 'started'
             AND created_at < $1"#,
    )
    .bind(cutoff)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// 计算 reclaim cutoff 时间锚: NOW() - threshold.
///
/// 抽出来纯函数方便单元测试; 真正的 DB-aware reclaim 需 pg-embed 跑集成测试
/// (推 Phase 2 加).
pub(crate) fn reclaim_cutoff(threshold: Duration) -> DateTime<Utc> {
    Utc::now() - threshold
}

pub async fn log(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    project_path: Option<&str>,
    source: &str,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO audit_log (action, category, details, entity_type, entity_id, project_path, source)
           VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(entity_type)
    .bind(entity_id)
    .bind(project_path)
    .bind(source)
    .execute(pool)
    .await?;
    Ok(())
}

/// Extended log with pentest operation fields
pub async fn log_operation(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    project_path: Option<&str>,
    source: &str,
    target_id: Option<Uuid>,
    session_id: Option<&str>,
    tool_name: Option<&str>,
    status: &str,
    detail: &serde_json::Value,
) -> Result<AuditEntry> {
    log_operation_with_lineage(
        pool,
        action,
        category,
        details,
        project_path,
        source,
        target_id,
        session_id,
        tool_name,
        status,
        detail,
        None,
        None,
    )
    .await
}

/// Internal helper that supports parent_id (self-ref) + run_id (correlation UUID).
/// All audit_log writers ultimately route through this single SQL.
#[allow(clippy::too_many_arguments)]
pub async fn log_operation_with_lineage(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    project_path: Option<&str>,
    source: &str,
    target_id: Option<Uuid>,
    session_id: Option<&str>,
    tool_name: Option<&str>,
    status: &str,
    detail: &Value,
    parent_id: Option<i64>,
    run_id: Option<Uuid>,
) -> Result<AuditEntry> {
    let row = sqlx::query_as::<_, AuditEntry>(
        r#"INSERT INTO audit_log
               (action, category, details, project_path, source,
                target_id, session_id, tool_name, status, detail,
                parent_id, run_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
           RETURNING *"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(project_path)
    .bind(source)
    .bind(target_id)
    .bind(session_id)
    .bind(tool_name)
    .bind(status)
    .bind(detail)
    .bind(parent_id)
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// 写一条 evidence 行 (`audit_role='evidence'`). 返回带 id 的 [`AuditEntry`],
/// 调用方 (`EvidenceLedger::append`) 取 `.id` 包成 `EvidenceAuditId`.
///
/// 与 `log_operation_with_lineage` 的区别: `status` 固定 `'completed'`,
/// `audit_role` 固定 `'evidence'` (migration `20260601000001` 加的列, 默认
/// `'action'`, 老行不破). `RETURNING *` 多出的 `audit_role` 列被 `FromRow`
/// 忽略 (AuditEntry 不含该字段, 读路径走 detail JSON).
#[allow(clippy::too_many_arguments)]
pub async fn log_evidence(
    pool: &PgPool,
    action: &str,
    category: &str,
    details: &str,
    project_path: Option<&str>,
    source: &str,
    target_id: Option<Uuid>,
    session_id: Option<&str>,
    tool_name: Option<&str>,
    detail: &Value,
    run_id: Option<Uuid>,
) -> Result<AuditEntry> {
    let row = sqlx::query_as::<_, AuditEntry>(
        r#"INSERT INTO audit_log
               (action, category, details, project_path, source,
                target_id, session_id, tool_name, status, detail,
                run_id, audit_role)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'completed', $9, $10, 'evidence')
           RETURNING *"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(project_path)
    .bind(source)
    .bind(target_id)
    .bind(session_id)
    .bind(tool_name)
    .bind(detail)
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// 给一组 `audit_log.id`, 返回其中真实存在且 `audit_role='evidence'` 的子集.
///
/// harness gate 用它拒绝引用了不存在 evidence id 的交付物 (防 agent 伪造
/// `evidence_refs`). 空入参直接返回空, 避免无谓 query.
pub async fn existing_evidence_ids(pool: &PgPool, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM audit_log
           WHERE audit_role = 'evidence' AND id = ANY($1)"#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

#[cfg(test)]
mod tests;
