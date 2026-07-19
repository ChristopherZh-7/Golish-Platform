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
use sqlx::{PgConnection, PgPool};
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

pub(crate) fn audit_project_path(project_path: Option<&str>) -> &str {
    project_path.unwrap_or("")
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
    .bind(audit_project_path(project_path))
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
    .bind(audit_project_path(project_path))
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

/// Transaction-owned counterpart of [`log_operation_with_lineage`]. Active
/// target producers use this after locking a [`crate::repo::scoped::TargetWriteGuard`]
/// so the timeline row cannot cross an ownership/scope change between a Rust
/// revalidation and the actual insert.
#[allow(clippy::too_many_arguments)]
pub async fn log_operation_with_lineage_in_transaction(
    connection: &mut PgConnection,
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
    .bind(audit_project_path(project_path))
    .bind(source)
    .bind(target_id)
    .bind(session_id)
    .bind(tool_name)
    .bind(status)
    .bind(detail)
    .bind(parent_id)
    .bind(run_id)
    .fetch_one(connection)
    .await?;
    Ok(row)
}

/// 写一条 evidence 行 (`audit_role='evidence'`). 返回带 id 的 [`AuditEntry`],
/// 调用方 (`EvidenceLedger::append`) 取 `.id` 包成 `EvidenceAuditId`.
///
/// 与 `log_operation_with_lineage` 的区别: `status` 固定 `'completed'`,
/// `audit_role` 固定 `'evidence'` (migration `20260601000001` 加的列, 默认
/// `'action'`, 老行不破). `AuditEntry` 保留这些 typed evidence 列，让只读 UI
/// 可以按 ledger outcome 展示状态，而不从 `detail.raw_output` 猜测成功/空结果。
///
/// PR2 (设计 2026-06-11 coverage 投影): `technique` / `asset` / `outcome` 写进
/// migration `20260611000001/2` 的三个 nullable 列 — 不进 `detail` JSON, 哈希链
/// 输入不变. 全 `None` = 行为与旧签名逐字节一致 (该行不参与 coverage 投影).
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
    technique: Option<&str>,
    asset: Option<&str>,
    outcome: Option<&str>,
) -> Result<AuditEntry> {
    let row = sqlx::query_as::<_, AuditEntry>(
        r#"INSERT INTO audit_log
               (action, category, details, project_path, source,
                target_id, session_id, tool_name, status, detail,
                run_id, audit_role, evidence_technique, evidence_asset, evidence_outcome)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'completed', $9, $10, 'evidence', $11, $12, $13)
           RETURNING *"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(audit_project_path(project_path))
    .bind(source)
    .bind(target_id)
    .bind(session_id)
    .bind(tool_name)
    .bind(detail)
    .bind(run_id)
    .bind(technique)
    .bind(asset)
    .bind(outcome)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Write an evidence row on an existing caller-owned transaction, persisting
/// the exact timestamp used by the evidence hash.
///
/// `EvidenceLedger::append` holds a transaction-scoped advisory lock for the
/// operation while it reads the predecessor hash and calls this function. The
/// explicit `created_at` is required because PostgreSQL's implicit `NOW()` is
/// not the same timestamp that the caller hashed and therefore cannot be used
/// to reconstruct the chain after a DB round trip.
#[allow(clippy::too_many_arguments)]
pub async fn log_evidence_in_transaction(
    connection: &mut PgConnection,
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
    technique: Option<&str>,
    asset: Option<&str>,
    outcome: Option<&str>,
    created_at: DateTime<Utc>,
) -> Result<AuditEntry> {
    let row = sqlx::query_as::<_, AuditEntry>(
        r#"INSERT INTO audit_log
               (action, category, details, project_path, source,
                target_id, session_id, tool_name, status, detail,
                run_id, audit_role, evidence_technique, evidence_asset,
                evidence_outcome, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'completed', $9, $10,
                   'evidence', $11, $12, $13, $14)
           RETURNING *"#,
    )
    .bind(action)
    .bind(category)
    .bind(details)
    .bind(audit_project_path(project_path))
    .bind(source)
    .bind(target_id)
    .bind(session_id)
    .bind(tool_name)
    .bind(detail)
    .bind(run_id)
    .bind(technique)
    .bind(asset)
    .bind(outcome)
    .bind(created_at)
    .fetch_one(connection)
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

/// 取某 chat session 下最近 `limit` 条**真实** evidence id (`audit_role='evidence'`),
/// 按 id 倒序 (最新优先).
///
/// 用途: harness gate 拦下「引用了不存在 evidence id」的交付物后, 拿这批真实可引用
/// 的 id 喂回 agent, 让它照填 `evidence_refs` 而不是抄模板占位 (1/2/3). 同步路径
/// (`golish-agent-runtime` 工具后置) 与后台 job 路径 (`bridge_config` 监听器) 写
/// evidence 行时都填同一个 chat session 字符串到 `session_id` 列, 故按该列即可覆盖
/// 两条来源. `limit <= 0` 直接返回空.
pub async fn recent_evidence_ids_for_session(
    pool: &PgPool,
    session_id: &str,
    limit: i64,
) -> Result<Vec<i64>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }
    let rows: Vec<(i64,)> = sqlx::query_as(
        r#"SELECT id FROM audit_log
           WHERE audit_role = 'evidence' AND session_id = $1
           ORDER BY id DESC
           LIMIT $2"#,
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// 一条「最近证据」明细行 (设计 2026-07-02-eas-worker-evidence): 给 `list_recent_evidence`
/// 工具用. 比 [`recent_evidence_ids_for_session`] 的裸 id 多带上下文
/// (tool / subject / technique / asset / outcome / kind / age), 让 worker 能把
/// **哪个真实 id** 对上 **哪条 claim**, 从而合法引用证据而不是瞎猜或拿 submit 当探针.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RecentEvidenceRow {
    pub id: i64,
    pub tool_name: Option<String>,
    pub subject: Option<String>,
    pub technique: Option<String>,
    pub asset: Option<String>,
    pub outcome: Option<String>,
    pub kind: Option<String>,
    pub age_seconds: Option<f64>,
}

/// 取某 chat session 下最近 `limit` 条**真实** evidence 行 (`audit_role='evidence'`),
/// 带 tool/subject/technique/asset/outcome/kind/age 上下文, 按 id 倒序 (最新优先).
///
/// 用途: `list_recent_evidence` 工具——EAS/enumeration/pentester worker 在
/// `submit_stage_deliverable` 前调它拿本 run 的真实可引用 id + 每个 id 的来源
/// (工具/资产/技术), 照填 claim 的 `evidence_ids`. `limit <= 0` 直接返回空.
pub async fn recent_evidence_detailed_for_session(
    pool: &PgPool,
    session_id: &str,
    limit: i64,
) -> Result<Vec<RecentEvidenceRow>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, RecentEvidenceRow>(
        r#"SELECT id,
                  tool_name,
                  NULLIF(details, '')            AS subject,
                  evidence_technique             AS technique,
                  evidence_asset                 AS asset,
                  evidence_outcome               AS outcome,
                  detail->>'kind'                AS kind,
                  EXTRACT(EPOCH FROM (NOW() - created_at))::double precision AS age_seconds
           FROM audit_log
           WHERE audit_role = 'evidence' AND session_id = $1
           ORDER BY id DESC
           LIMIT $2"#,
    )
    .bind(session_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// PR2 任务 2.5 (设计 2026-06-11 coverage 投影) · 按会话取证据事实四元组
/// `(asset, technique, outcome, id)` — coverage_complete 的 `derive_from_evidence`
/// (PR3) 的唯一数据源. 只返回三列齐全的行 (解析不出的行 NULL, 设计 §4 约束 3:
/// 歧义即不派生); 旧行三列全 NULL 自然排除. 升序 = 链上先后.
pub async fn evidence_facts_for_session(
    pool: &PgPool,
    session_id: &str,
) -> Result<Vec<(String, String, String, i64)>> {
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        r#"SELECT evidence_asset, evidence_technique, evidence_outcome, id
           FROM audit_log
           WHERE audit_role = 'evidence'
             AND session_id = $1
             AND evidence_technique IS NOT NULL
             AND evidence_asset IS NOT NULL
             AND evidence_outcome IS NOT NULL
           ORDER BY id ASC"#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TargetBoundEvidenceFactRow {
    pub evidence_asset: String,
    pub evidence_technique: String,
    pub evidence_outcome: String,
    pub evidence_id: i64,
    pub evidence_organization_id: String,
    pub tool_name: Option<String>,
    pub evidence_kind: Option<String>,
    pub target_id: Uuid,
    pub target_organization_id: Option<Uuid>,
    pub target_type: String,
    pub target_name: String,
    pub target_value: String,
    pub target_ports: Value,
}

const EVIDENCE_FACTS_FOR_SESSION_ORG_FRESH_SQL: &str = r#"SELECT al.evidence_asset,
              al.evidence_technique,
              al.evidence_outcome,
              al.id AS evidence_id,
              al.detail->>'organization_id' AS evidence_organization_id,
              al.tool_name,
              al.detail->>'kind' AS evidence_kind,
              t.id AS target_id,
              t.organization_id AS target_organization_id,
              t.target_type::text AS target_type,
              t.name AS target_name,
              t.value AS target_value,
              COALESCE(t.ports, '[]'::jsonb) AS target_ports
       FROM audit_log al
       JOIN targets t ON t.id = al.target_id
       WHERE al.audit_role = 'evidence'
         AND al.session_id = $1
         AND al.detail->>'organization_id' = $2::text
         AND t.organization_id = $2
         AND t.scope::text = 'in'
         AND t.project_path IS NOT NULL
         AND al.project_path = t.project_path
         AND al.created_at >= $3
         AND al.evidence_technique IS NOT NULL
         AND al.evidence_asset IS NOT NULL
         AND al.evidence_outcome IS NOT NULL
       ORDER BY al.id ASC"#;

/// Target-bound, organization-scoped and stage-fresh evidence facts for
/// Enumeration terminal outcome validation. Unlike the legacy session-wide
/// reader, this cannot reuse a prior stage attempt's evidence or a sibling
/// organization's identical `(asset, technique, outcome)` tuple.
pub async fn evidence_facts_for_session_org_fresh(
    pool: &PgPool,
    session_id: &str,
    organization_id: Uuid,
    since: DateTime<Utc>,
) -> Result<Vec<TargetBoundEvidenceFactRow>> {
    let rows =
        sqlx::query_as::<_, TargetBoundEvidenceFactRow>(EVIDENCE_FACTS_FOR_SESSION_ORG_FRESH_SQL)
            .bind(session_id)
            .bind(organization_id)
            .bind(since)
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// 给一组 evidence `audit_log.id`, 返回每条的 `detail->>'kind'` (可能 NULL).
///
/// P2 verification gate 用它做 evidence-kind 回查: stage 要求的 evidence 种类
/// (`required_evidence_kinds`) 是否真的在交付物引用的证据里出现.
pub async fn evidence_kinds_for(pool: &PgPool, ids: &[i64]) -> Result<Vec<(i64, Option<String>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
        r#"SELECT id, detail->>'kind'
           FROM audit_log
           WHERE audit_role = 'evidence' AND id = ANY($1)"#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// 给一组 evidence `audit_log.id`, 返回每条「已存在多久」(秒, = `NOW() - created_at`).
///
/// P0 Task 6 · freshness gate 用它把 evidence 真实 age 与 `evidence_kinds.json`
/// 的 max_age 比较, 拦截过期/陈旧证据. 不存在 / 非 evidence 的 id 不在结果里.
pub async fn evidence_ages_for(pool: &PgPool, ids: &[i64]) -> Result<Vec<(i64, Option<f64>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(i64, Option<f64>)> = sqlx::query_as(
        r#"SELECT id, EXTRACT(EPOCH FROM (NOW() - created_at))::double precision
           FROM audit_log
           WHERE audit_role = 'evidence' AND id = ANY($1)"#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests;
