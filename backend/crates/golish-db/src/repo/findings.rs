use crate::Result;
use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, PgPool, Postgres};
use uuid::Uuid;

use crate::models::{Finding, FindingStatus, Severity};
use golish_pentest_domain::FindingWriteContext;

fn reject_write_context(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

/// Resolve the immutable operation contract before a legacy writer starts its
/// own transaction. V2-only harness operations cannot obtain legacy authority.
pub async fn authorize_legacy_write(
    pool: &PgPool,
    context: FindingWriteContext,
    operation_id: Option<Uuid>,
) -> Result<FindingWriteContext> {
    match (context, operation_id) {
        (FindingWriteContext::HarnessLegacy, Some(operation_id)) => {
            let contract: Option<String> = sqlx::query_scalar(
                "SELECT attack_execution_contract FROM operation_state WHERE operation_id=$1",
            )
            .bind(operation_id)
            .fetch_optional(pool)
            .await?;
            match contract.as_deref() {
                Some("legacy" | "dual_write_read_legacy" | "dual_write_read_v2_fallback") => {
                    Ok(context)
                }
                Some("v2_only") => Err(reject_write_context(
                    "V2-only harness Finding writes require exact Candidate terminalization",
                )),
                Some(_) => Err(reject_write_context(
                    "unknown attack execution contract cannot authorize Finding writes",
                )),
                None => Err(reject_write_context(
                    "attributed harness operation is missing",
                )),
            }
        }
        (FindingWriteContext::LegacyNonHarness, None) | (FindingWriteContext::UserCrud, None) => {
            Ok(context)
        }
        (FindingWriteContext::VerificationTerminalizer { .. }, _) => Err(reject_write_context(
            "terminalizer authority is internal to the compound Candidate transaction",
        )),
        _ => Err(reject_write_context(
            "Finding write context does not match operation attribution",
        )),
    }
}

/// Detailed projection of a finding row used by the Tauri command layer.
/// Carries `sev`/`status` as text and the `target_id` column that the canonical
/// [`Finding`] model does not expose.
#[derive(Debug, Clone, FromRow)]
pub struct FindingDetailRow {
    pub id: Uuid,
    pub title: String,
    pub sev: String,
    pub cvss: Option<f64>,
    pub url: String,
    pub target: String,
    pub target_id: Option<Uuid>,
    pub description: String,
    pub steps: String,
    pub remediation: String,
    pub tags: serde_json::Value,
    pub tool: String,
    pub template: String,
    pub refs: serde_json::Value,
    pub evidence: serde_json::Value,
    pub status: String,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

const DETAIL_COLS: &str = "id, title, sev::TEXT, cvss, url, target, target_id, description, steps, remediation, tags, tool, template, refs, evidence, status::TEXT, source, created_at, updated_at";

/// Owned upsert payload for [`upsert_full`]. `sev`/`status` are text values cast
/// to the `severity` / `finding_status` PG enums; `created_at`/`updated_at` are
/// epoch seconds.
pub struct FindingUpsert<'a> {
    pub id: Uuid,
    pub title: &'a str,
    pub sev: &'a str,
    pub cvss: Option<f64>,
    pub url: &'a str,
    pub target: &'a str,
    pub target_id: Option<Uuid>,
    pub description: &'a str,
    pub steps: &'a str,
    pub remediation: &'a str,
    pub tags: &'a serde_json::Value,
    pub tool: &'a str,
    pub template: &'a str,
    pub refs: &'a serde_json::Value,
    pub evidence: &'a serde_json::Value,
    pub status: &'a str,
    pub source: &'a str,
    pub project_path: Option<&'a str>,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone)]
pub(super) struct CandidateVerifiedFindingWrite {
    pub id: Uuid,
    pub title: String,
    pub severity: String,
    pub cvss: Option<f64>,
    pub target_live_id: Option<Uuid>,
    pub target_value_at_time: String,
    pub description: String,
    pub steps: String,
    pub remediation: String,
    pub evidence: serde_json::Value,
    pub project_path: String,
}

#[derive(Debug, Clone)]
pub struct LegacyFindingWrite {
    pub id: Uuid,
    pub title: String,
    pub severity: String,
    pub cvss: Option<f64>,
    pub url: String,
    pub target: String,
    pub target_id: Option<Uuid>,
    pub description: String,
    pub steps: String,
    pub remediation: String,
    pub evidence: serde_json::Value,
    pub tool: String,
    pub template: String,
    pub refs: serde_json::Value,
    pub source: String,
    pub project_path: Option<String>,
}

/// Central legacy/non-harness writer used while old persisted contracts remain
/// readable. Candidate V2 callers cannot select the terminalizer context here.
pub async fn insert_legacy_with_executor<'e, E>(
    executor: E,
    context: FindingWriteContext,
    finding: &LegacyFindingWrite,
) -> Result<Uuid>
where
    E: Executor<'e, Database = Postgres>,
{
    if matches!(
        context,
        FindingWriteContext::VerificationTerminalizer { .. }
    ) {
        return Err(reject_write_context(
            "terminalizer must use the candidate_v2 compound Finding API",
        ));
    }
    if finding.title.trim().is_empty()
        || finding.description.trim().is_empty()
        || !matches!(
            finding.severity.as_str(),
            "critical" | "high" | "medium" | "low" | "info"
        )
    {
        return Err(reject_write_context("invalid legacy Finding projection"));
    }
    sqlx::query(
        r#"INSERT INTO findings(
               id,title,sev,cvss,url,target,target_id,description,steps,remediation,
               evidence,tool,template,refs,source,project_path)
           VALUES($1,$2,$3::severity,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
           ON CONFLICT(id) DO UPDATE SET
             title=EXCLUDED.title,sev=EXCLUDED.sev,cvss=EXCLUDED.cvss,
             url=EXCLUDED.url,target=EXCLUDED.target,
             target_id=COALESCE(EXCLUDED.target_id,findings.target_id),
             description=EXCLUDED.description,steps=EXCLUDED.steps,
             remediation=EXCLUDED.remediation,evidence=EXCLUDED.evidence,
             tool=EXCLUDED.tool,template=EXCLUDED.template,refs=EXCLUDED.refs,
             source=EXCLUDED.source,updated_at=NOW()
           WHERE findings.project_path IS NOT DISTINCT FROM EXCLUDED.project_path"#,
    )
    .bind(finding.id)
    .bind(&finding.title)
    .bind(&finding.severity)
    .bind(finding.cvss)
    .bind(&finding.url)
    .bind(&finding.target)
    .bind(finding.target_id)
    .bind(&finding.description)
    .bind(&finding.steps)
    .bind(&finding.remediation)
    .bind(&finding.evidence)
    .bind(&finding.tool)
    .bind(&finding.template)
    .bind(&finding.refs)
    .bind(&finding.source)
    .bind(&finding.project_path)
    .execute(executor)
    .await?;
    Ok(finding.id)
}

/// Insert the Finding projection built from an immutable, validator-approved
/// CandidateAttempt result. The caller owns the surrounding terminalization
/// transaction and lineage write.
pub(super) async fn insert_verified_candidate_with_executor<'e, E>(
    executor: E,
    context: FindingWriteContext,
    finding: &CandidateVerifiedFindingWrite,
) -> Result<Uuid>
where
    E: Executor<'e, Database = Postgres>,
{
    let FindingWriteContext::VerificationTerminalizer { attempt_id } = context else {
        return Err(reject_write_context(
            "candidate_v2 Finding writes require VerificationTerminalizer context",
        ));
    };
    if attempt_id.is_nil() {
        return Err(reject_write_context(
            "candidate_v2 Finding terminalizer attempt_id is required",
        ));
    }
    if finding.title.trim().is_empty()
        || finding.target_value_at_time.trim().is_empty()
        || finding.description.trim().is_empty()
        || !matches!(
            finding.severity.as_str(),
            "critical" | "high" | "medium" | "low" | "info"
        )
        || finding
            .cvss
            .is_some_and(|score| !score.is_finite() || !(0.0..=10.0).contains(&score))
        || !finding.evidence.is_array()
    {
        return Err(crate::DbError::Other(anyhow::anyhow!(
            "invalid Candidate verified Finding projection"
        )));
    }
    Ok(sqlx::query_scalar(
        r#"INSERT INTO findings(
               id,title,sev,cvss,url,target,target_id,description,steps,remediation,
               evidence,status,source,project_path)
           VALUES($1,$2,$3::severity,$4,$5,$5,$6,$7,$8,$9,$10,'confirmed',
                  'candidate_v2',$11)
           RETURNING id"#,
    )
    .bind(finding.id)
    .bind(&finding.title)
    .bind(&finding.severity)
    .bind(finding.cvss)
    .bind(&finding.target_value_at_time)
    .bind(finding.target_live_id)
    .bind(&finding.description)
    .bind(&finding.steps)
    .bind(&finding.remediation)
    .bind(&finding.evidence)
    .bind(&finding.project_path)
    .fetch_one(executor)
    .await?)
}

pub async fn create(
    pool: &PgPool,
    context: FindingWriteContext,
    title: &str,
    sev: Severity,
    project_path: Option<&str>,
    source: &str,
) -> Result<Finding> {
    if matches!(
        context,
        FindingWriteContext::VerificationTerminalizer { .. }
    ) {
        return Err(reject_write_context(
            "terminalizer must use the candidate_v2 compound Finding API",
        ));
    }
    let row = sqlx::query_as::<_, Finding>(
        r#"INSERT INTO findings (title, sev, project_path, source)
           VALUES ($1, $2, $3, $4)
           RETURNING *"#,
    )
    .bind(title)
    .bind(sev)
    .bind(project_path)
    .bind(source)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list(pool: &PgPool, project_path: Option<&str>) -> Result<Vec<Finding>> {
    super::scoped::list_by_project(pool, "findings", "created_at DESC", project_path).await
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Finding>> {
    super::scoped::get_by_id(pool, "findings", id).await
}

pub async fn update_full(pool: &PgPool, f: &Finding) -> Result<()> {
    sqlx::query(
        r#"UPDATE findings SET title=$1, sev=$2, cvss=$3, url=$4, target=$5,
           description=$6, steps=$7, remediation=$8, tags=$9, tool=$10,
           template=$11, refs=$12, evidence=$13, status=$14, updated_at=NOW()
           WHERE id=$15"#,
    )
    .bind(&f.title)
    .bind(f.sev)
    .bind(f.cvss)
    .bind(&f.url)
    .bind(&f.target)
    .bind(&f.description)
    .bind(&f.steps)
    .bind(&f.remediation)
    .bind(&f.tags)
    .bind(&f.tool)
    .bind(&f.template)
    .bind(&f.refs)
    .bind(&f.evidence)
    .bind(f.status)
    .bind(f.id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: FindingStatus) -> Result<()> {
    sqlx::query("UPDATE findings SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    super::scoped::delete_by_id(pool, "findings", id).await?;
    Ok(())
}

// ── Detailed / project-scoped helpers (AGENTS.md I2). Command layer must route
// through these instead of writing scoped SQL inline. ──────────────────────

/// List detailed finding rows for a project (exact `project_path = $1`).
pub async fn list_detail_by_project(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<FindingDetailRow>> {
    let sql = format!(
        "SELECT {DETAIL_COLS} FROM findings WHERE project_path = $1 ORDER BY created_at DESC"
    );
    let rows = sqlx::query_as::<_, FindingDetailRow>(&sql)
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// List detailed finding rows matching a host pattern, scoped to project_path (IDOR).
pub async fn list_detail_for_host(
    pool: &PgPool,
    pattern: &str,
    project_path: Option<&str>,
) -> Result<Vec<FindingDetailRow>> {
    let sql = format!(
        "SELECT {DETAIL_COLS} FROM findings \
         WHERE (LOWER(url) LIKE $1 OR LOWER(target) LIKE $1 OR LOWER(title) LIKE $1) \
         AND project_path IS NOT DISTINCT FROM $2"
    );
    let rows = sqlx::query_as::<_, FindingDetailRow>(&sql)
        .bind(pattern)
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// List detailed finding rows for de-duplication, scoped to project_path (IDOR),
/// oldest first so the earliest finding is treated as canonical.
pub async fn list_detail_for_dedup(
    pool: &PgPool,
    project_path: Option<&str>,
) -> Result<Vec<FindingDetailRow>> {
    let sql = format!(
        "SELECT {DETAIL_COLS} FROM findings WHERE project_path IS NOT DISTINCT FROM $1 ORDER BY created_at ASC"
    );
    let rows = sqlx::query_as::<_, FindingDetailRow>(&sql)
        .bind(project_path)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Upsert a finding (INSERT … ON CONFLICT (id) DO UPDATE), preserving the prior
/// command-layer SQL including the `target_id` COALESCE-on-conflict behaviour.
pub async fn upsert_full(
    pool: &PgPool,
    context: FindingWriteContext,
    f: &FindingUpsert<'_>,
) -> Result<()> {
    if matches!(
        context,
        FindingWriteContext::VerificationTerminalizer { .. }
    ) {
        return Err(reject_write_context(
            "terminalizer must use the candidate_v2 compound Finding API",
        ));
    }
    sqlx::query(
        r#"INSERT INTO findings (id, title, sev, cvss, url, target, target_id, description, steps, remediation, tags, tool, template, refs, evidence, status, source, project_path, created_at, updated_at)
           VALUES ($1, $2, $3::severity, $4, $5, $6, $20, $7, $8, $9, $10, $11, $12, $13, $14, $15::finding_status, $16, $17,
                   to_timestamp($18::DOUBLE PRECISION), to_timestamp($19::DOUBLE PRECISION))
           ON CONFLICT (id) DO UPDATE SET
             title=$2, sev=$3::severity, cvss=$4, url=$5, target=$6, target_id=COALESCE($20, findings.target_id), description=$7, steps=$8, remediation=$9,
             tags=$10, tool=$11, template=$12, refs=$13, evidence=$14, status=$15::finding_status, source=$16, updated_at=to_timestamp($19::DOUBLE PRECISION)"#,
    )
    .bind(f.id)
    .bind(f.title)
    .bind(f.sev)
    .bind(f.cvss)
    .bind(f.url)
    .bind(f.target)
    .bind(f.description)
    .bind(f.steps)
    .bind(f.remediation)
    .bind(f.tags)
    .bind(f.tool)
    .bind(f.template)
    .bind(f.refs)
    .bind(f.evidence)
    .bind(f.status)
    .bind(f.source)
    .bind(f.project_path)
    .bind(f.created_at)
    .bind(f.updated_at)
    .bind(f.target_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read a finding's `created_at`, scoped to project_path (IDOR).
pub async fn created_at_scoped(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<DateTime<Utc>>> {
    let row = sqlx::query_scalar::<_, DateTime<Utc>>(
        "SELECT created_at FROM findings WHERE id=$1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(id)
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Delete a finding scoped to project_path (IDOR). Returns rows affected.
pub async fn delete_scoped(pool: &PgPool, id: Uuid, project_path: Option<&str>) -> Result<u64> {
    super::scoped::delete_scoped(pool, "findings", id, project_path).await
}

/// Count findings matching an exact title + url (import de-dup heuristic).
pub async fn count_by_title_url(pool: &PgPool, title: &str, url: &str) -> Result<i64> {
    let count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM findings WHERE title=$1 AND url=$2")
            .bind(title)
            .bind(url)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

/// Read a finding's `evidence` JSON, scoped to project_path (IDOR).
pub async fn get_evidence_scoped(
    pool: &PgPool,
    id: Uuid,
    project_path: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    let row = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT evidence FROM findings WHERE id=$1 AND project_path IS NOT DISTINCT FROM $2",
    )
    .bind(id)
    .bind(project_path)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Overwrite a finding's `evidence` JSON (by id only; caller scope-guards first).
pub async fn set_evidence(pool: &PgPool, id: Uuid, evidence: &serde_json::Value) -> Result<()> {
    sqlx::query("UPDATE findings SET evidence=$1, updated_at=NOW() WHERE id=$2")
        .bind(evidence)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
