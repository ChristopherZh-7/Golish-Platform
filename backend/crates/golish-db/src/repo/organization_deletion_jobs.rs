//! DB-global, lease-fenced two-phase organization deletion.
//!
//! `request` performs every mutable-DB precheck, freezes the organization and
//! target snapshots, closes each exact active Memory source and appends its
//! catalog deliveries in one transaction.  Artifact cleanup can only be
//! claimed from the committed job.  `hard_delete` is a third, independent
//! transaction so filesystem I/O never occurs while a DB transaction is held.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use golish_memory_domain::event_catalog::{
    routes_for, KnowledgeEventEnvelopeV1, KnowledgeEventNameV1, KnowledgeEventPayloadV1,
};
use golish_memory_domain::scope::ProjectScopeId;
use golish_memory_domain::source_ref::{CanonicalSourceKind, SourceRef, StoredCanonicalRowId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::Result;

pub const TABLE_NAME: &str = "organization_deletion_jobs";
pub const DEFAULT_JOB_LEASE_SECONDS: i64 = 120;
const ORGANIZATION_DELETION_STOPPED_TASK_RESULT: &str =
    "Stopped: organization deletion closed a quiescent stage task.";

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Serialize, Deserialize)]
pub struct OrganizationDeletionJobRow {
    pub id: Uuid,
    pub root_organization_id_at_time: Uuid,
    pub project_scope_id: Uuid,
    pub project_path_at_time: String,
    pub requested_by_principal_id: Uuid,
    pub state: String,
    pub organization_snapshot: Value,
    pub target_snapshot: Value,
    pub required_invalidation_count: i32,
    pub lease_owner: Option<String>,
    pub lease_token: Option<Uuid>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub attempt_count: i32,
    pub artifact_retry_not_before: DateTime<Utc>,
    pub hard_delete_attempt_count: i32,
    pub hard_delete_retry_not_before: DateTime<Utc>,
    pub row_version: i64,
    pub last_error_code: Option<String>,
    pub last_error: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub artifact_cleanup_started_at: Option<DateTime<Utc>>,
    pub artifact_cleanup_completed_at: Option<DateTime<Utc>>,
    pub hard_delete_committed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestOrganizationDeletion {
    pub job_id: Uuid,
    pub root_organization_id: Uuid,
    pub principal_id: Uuid,
    /// Active workspace witness supplied by the local command surface. The DB
    /// resolves it to the server-owned `project_scopes` row and then requires
    /// the complete organization subtree to match that canonical path.
    pub expected_project_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimOrganizationArtifactCleanup {
    pub worker_id: String,
    pub lease_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteOrganizationArtifactCleanup {
    pub job_id: Uuid,
    pub worker_id: String,
    pub lease_token: Uuid,
    pub expected_row_version: i64,
    pub result: std::result::Result<(), ArtifactCleanupFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactCleanupFailure {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCleanupTargetSnapshot {
    pub target_id_at_time: Uuid,
    pub organization_id_at_time: Uuid,
    pub project_path_at_time: String,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactCleanupPlan {
    pub job_id: Uuid,
    pub root_organization_id_at_time: Uuid,
    pub project_scope_id: Uuid,
    pub project_path_at_time: String,
    pub targets: Vec<ArtifactCleanupTargetSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupCloseoutGateRow {
    pub operation_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub missing_obligation_count: i64,
    pub nonterminal_obligation_count: i64,
    pub undisclosed_residual_count: i64,
    pub invalid_terminal_truth_count: i64,
    pub residual_obligation_ids: BTreeSet<Uuid>,
}

impl CleanupCloseoutGateRow {
    pub const fn allows_closeout(&self) -> bool {
        self.missing_obligation_count == 0
            && self.nonterminal_obligation_count == 0
            && self.undisclosed_residual_count == 0
            && self.invalid_terminal_truth_count == 0
    }
}

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
struct OrganizationSnapshotRow {
    id: Uuid,
    parent_id: Option<Uuid>,
    project_path: String,
    name: String,
    depth: i32,
    ordinal: i64,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct TargetSnapshotRow {
    id: Uuid,
    organization_id: Uuid,
    project_path: String,
    target_type: String,
    value: String,
    name: String,
    scope: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct ActiveStageForkBlockerRow {
    operation_id: Uuid,
    stage: String,
    status: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct LockedStageWorkerAuthorityRow {
    operation_id: Uuid,
    has_live_lease: bool,
    active_tool_call_id: Option<Uuid>,
}

#[derive(Clone, Debug, sqlx::FromRow)]
struct AssertionSourceRow {
    project_scope_id: Uuid,
    organization_id_at_time: Uuid,
    source_operation_id: Uuid,
    source_kind: String,
    source_id_kind: String,
    source_id_value: String,
    source_stream_key: String,
    source_version: i64,
}

fn conflict(message: impl Into<String>) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.into()))
}

fn parse_source_kind(value: &str) -> Result<CanonicalSourceKind> {
    match value {
        "stage_episode" => Ok(CanonicalSourceKind::StageEpisode),
        "finding" => Ok(CanonicalSourceKind::Finding),
        "candidate_attempt" => Ok(CanonicalSourceKind::CandidateAttempt),
        "technique_outcome" => Ok(CanonicalSourceKind::TechniqueOutcome),
        "fact_delta" => Ok(CanonicalSourceKind::FactDelta),
        "post_exploit_action" => Ok(CanonicalSourceKind::PostExploitAction),
        "foothold" => Ok(CanonicalSourceKind::Foothold),
        "objective_outcome" => Ok(CanonicalSourceKind::ObjectiveOutcome),
        "cleanup_obligation" => Ok(CanonicalSourceKind::CleanupObligation),
        "residual_risk" => Ok(CanonicalSourceKind::ResidualRisk),
        "report_revision" => Ok(CanonicalSourceKind::ReportRevision),
        _ => Err(conflict("organization_deletion_source_kind_invalid")),
    }
}

fn source_ref(row: &AssertionSourceRow) -> Result<SourceRef> {
    Ok(SourceRef {
        source_kind: parse_source_kind(&row.source_kind)?,
        row_id: StoredCanonicalRowId {
            kind: row.source_id_kind.clone(),
            value: row.source_id_value.clone(),
        }
        .into_domain()
        .map_err(|error| conflict(error.code()))?,
        source_stream_key: row.source_stream_key.clone(),
        version: row.source_version,
    })
}

fn invalidation_event(
    job: &OrganizationDeletionJobRow,
    row: &AssertionSourceRow,
    source: SourceRef,
) -> KnowledgeEventEnvelopeV1 {
    let event_id = Uuid::new_v5(
        &job.id,
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            row.project_scope_id,
            row.organization_id_at_time,
            row.source_operation_id,
            row.source_kind,
            row.source_id_value,
            row.source_stream_key,
            row.source_version
        )
        .as_bytes(),
    );
    KnowledgeEventEnvelopeV1 {
        event_id,
        project_scope_id: Some(ProjectScopeId(row.project_scope_id)),
        organization_id_at_time: Some(row.organization_id_at_time),
        source_operation_id: row.source_operation_id,
        event_name: KnowledgeEventNameV1::SourceScopeInvalidated,
        schema_version: 1,
        payload: KnowledgeEventPayloadV1 {
            source,
            source_stream_key: row.source_stream_key.clone(),
            source_version: row.source_version,
            structured_payload: json!({
                "reasonCode": "organization_deleted",
                "organizationDeletionJobId": job.id,
                "rootOrganizationIdAtTime": job.root_organization_id_at_time,
            }),
        },
        occurred_at: job.requested_at,
    }
}

fn required_invalidation_delivery_manifest() -> Result<Value> {
    let routes = routes_for(KnowledgeEventNameV1::SourceScopeInvalidated);
    if routes.is_empty() {
        return Err(conflict(
            "organization_delete_invalidation_delivery_manifest_empty",
        ));
    }
    Ok(Value::Array(
        routes
            .into_iter()
            .map(|route| {
                json!({
                    "projector_name": route.projector.name(),
                    "projector_schema_version": route.projector.schema_version(),
                })
            })
            .collect(),
    ))
}

async fn active_job_for_root_with_connection(
    connection: &mut PgConnection,
    root_organization_id: Uuid,
) -> Result<Option<OrganizationDeletionJobRow>> {
    Ok(sqlx::query_as::<_, OrganizationDeletionJobRow>(
        r#"SELECT * FROM organization_deletion_jobs
            WHERE root_organization_id_at_time=$1
              AND state<>'hard_delete_committed'
            FOR UPDATE"#,
    )
    .bind(root_organization_id)
    .fetch_optional(connection)
    .await?)
}

async fn append_state_history(
    connection: &mut PgConnection,
    job_id: Uuid,
    state: &str,
    detail: Value,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO organization_deletion_job_state_history(job_id,ordinal,state,detail)
           SELECT $1,COALESCE(MAX(ordinal),0)+1,$2,$3
             FROM organization_deletion_job_state_history WHERE job_id=$1"#,
    )
    .bind(job_id)
    .bind(state)
    .bind(detail)
    .execute(connection)
    .await?;
    Ok(())
}

async fn assert_deletion_preconditions(
    connection: &mut PgConnection,
    organization_ids: &[Uuid],
) -> Result<()> {
    let nonterminal: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM cleanup_obligations
            WHERE organization_id_at_time=ANY($1)
              AND status NOT IN ('verified_absent','blocked','waived_by_user')"#,
    )
    .bind(organization_ids)
    .fetch_one(&mut *connection)
    .await?;
    if nonterminal != 0 {
        return Err(conflict("organization_delete_cleanup_obligations_open"));
    }
    let invalid_terminal_truth: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM cleanup_obligations
            WHERE organization_id_at_time=ANY($1)
              AND status IN ('verified_absent','blocked','waived_by_user')
              AND NOT cleanup_obligation_state_truth_is_exact(id)"#,
    )
    .bind(organization_ids)
    .fetch_one(&mut *connection)
    .await?;
    if invalid_terminal_truth != 0 {
        return Err(conflict(
            "organization_delete_cleanup_terminal_truth_invalid",
        ));
    }
    let missing: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM post_exploit_actions AS action
            WHERE action.organization_id_at_time=ANY($1)
              AND action.side_effect_class<>'none'
              AND NOT EXISTS (
                  SELECT 1 FROM cleanup_obligations AS obligation
                   WHERE obligation.id=action.cleanup_obligation_id
                     AND obligation.source_action_id=action.id
              )"#,
    )
    .bind(organization_ids)
    .fetch_one(connection)
    .await?;
    if missing != 0 {
        return Err(conflict("organization_delete_cleanup_obligation_missing"));
    }
    Ok(())
}

pub async fn request(
    pool: &PgPool,
    input: &RequestOrganizationDeletion,
) -> Result<OrganizationDeletionJobRow> {
    let mut tx = pool.begin().await?;
    let principal_valid: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM operator_principals
                WHERE id=$1 AND principal_kind='local_operator' AND active
           )"#,
    )
    .bind(input.principal_id)
    .fetch_one(&mut *tx)
    .await?;
    if !principal_valid {
        return Err(conflict("organization_delete_operator_untrusted"));
    }
    if input.expected_project_path.trim().is_empty()
        || input.expected_project_path.chars().any(char::is_control)
    {
        return Err(conflict("organization_delete_project_scope_invalid"));
    }
    let project_scope = sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT project_scope_id,canonical_project_path
             FROM project_scopes
            WHERE canonical_project_path=$1 AND retired_at IS NULL
            FOR SHARE"#,
    )
    .bind(&input.expected_project_path)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("organization_delete_project_scope_not_authorized"))?;
    if let Some(existing) =
        active_job_for_root_with_connection(&mut tx, input.root_organization_id).await?
    {
        if existing.project_scope_id != project_scope.0
            || existing.project_path_at_time != project_scope.1
        {
            return Err(conflict("organization_delete_project_scope_not_authorized"));
        }
        tx.commit().await?;
        return Ok(existing);
    }

    let organizations = sqlx::query_as::<_, OrganizationSnapshotRow>(
        r#"WITH RECURSIVE subtree AS (
               SELECT id,0::INTEGER AS depth FROM organizations WHERE id=$1
               UNION ALL
               SELECT child.id,parent.depth+1
                 FROM organizations AS child
                 JOIN subtree AS parent ON child.parent_id=parent.id
           )
           SELECT organization.id,organization.parent_id,organization.project_path,
                  organization.name,subtree.depth,
                  ROW_NUMBER() OVER (
                      ORDER BY subtree.depth,organization.parent_id NULLS FIRST,
                               organization.sort_order,organization.name,organization.id
                  )-1 AS ordinal
             FROM organizations AS organization
             JOIN subtree ON subtree.id=organization.id
            ORDER BY ordinal"#,
    )
    .bind(input.root_organization_id)
    .fetch_all(&mut *tx)
    .await?;
    if organizations.is_empty() {
        return Err(crate::DbError::NotFound("organization".to_string()));
    }
    let organization_ids = organizations.iter().map(|row| row.id).collect::<Vec<_>>();
    let locked_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM organizations WHERE id=ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind(&organization_ids)
    .fetch_all(&mut *tx)
    .await?;
    if locked_ids.len() != organization_ids.len() {
        return Err(conflict("organization_delete_subtree_changed"));
    }
    // The first recursive read determines which rows to lock, but a concurrent
    // reparent can commit before those locks are acquired. Re-read only after
    // every original row is locked and require the complete ordered snapshot
    // to match. Once the root/parents are locked, FK checks also prevent a new
    // child attachment from committing until this transaction finishes.
    let locked_organizations = sqlx::query_as::<_, OrganizationSnapshotRow>(
        r#"WITH RECURSIVE subtree AS (
               SELECT id,0::INTEGER AS depth FROM organizations WHERE id=$1
               UNION ALL
               SELECT child.id,parent.depth+1
                 FROM organizations AS child
                 JOIN subtree AS parent ON child.parent_id=parent.id
           )
           SELECT organization.id,organization.parent_id,organization.project_path,
                  organization.name,subtree.depth,
                  ROW_NUMBER() OVER (
                      ORDER BY subtree.depth,organization.parent_id NULLS FIRST,
                               organization.sort_order,organization.name,organization.id
                  )-1 AS ordinal
             FROM organizations AS organization
             JOIN subtree ON subtree.id=organization.id
            ORDER BY ordinal"#,
    )
    .bind(input.root_organization_id)
    .fetch_all(&mut *tx)
    .await?;
    if locked_organizations != organizations {
        return Err(conflict("organization_delete_subtree_changed"));
    }
    if locked_organizations
        .iter()
        .any(|organization| organization.project_path != project_scope.1)
    {
        return Err(conflict("organization_delete_project_scope_not_authorized"));
    }
    if let Some(existing) =
        active_job_for_root_with_connection(&mut tx, input.root_organization_id).await?
    {
        if existing.project_scope_id != project_scope.0
            || existing.project_path_at_time != project_scope.1
        {
            return Err(conflict("organization_delete_project_scope_not_authorized"));
        }
        tx.commit().await?;
        return Ok(existing);
    }
    let overlaps_active_job: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM organization_deletion_job_units AS unit
                 JOIN organization_deletion_jobs AS job ON job.id=unit.job_id
                WHERE unit.organization_id_at_time=ANY($1)
                  AND job.state<>'hard_delete_committed'
           )"#,
    )
    .bind(&organization_ids)
    .fetch_one(&mut *tx)
    .await?;
    if overlaps_active_job {
        return Err(conflict("organization_delete_subtree_already_deleting"));
    }
    assert_deletion_preconditions(&mut tx, &organization_ids).await?;

    let targets = sqlx::query_as::<_, TargetSnapshotRow>(
        r#"SELECT id,organization_id,COALESCE(project_path,'') AS project_path,
                  target_type::text AS target_type,value,name,scope::text AS scope
             FROM targets
            WHERE organization_id=ANY($1)
            ORDER BY organization_id,id
            FOR UPDATE"#,
    )
    .bind(&organization_ids)
    .fetch_all(&mut *tx)
    .await?;
    let active_stage_fork_operation_ids = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT DISTINCT fork.operation_id
             FROM operation_stage_forks AS fork
             JOIN operation_org_scope_units AS unit
               ON unit.snapshot_id=fork.target_scope_snapshot_id
             JOIN tasks AS task
               ON task.id=fork.operation_id
              AND task.status IN ('created','running','waiting')
            WHERE unit.organization_id=ANY($1)
            ORDER BY fork.operation_id"#,
    )
    .bind(&organization_ids)
    .fetch_all(&mut *tx)
    .await?;
    let mut stopped_quiescent_stage_task_ids = Vec::new();
    if !active_stage_fork_operation_ids.is_empty() {
        // Exact resume locks operation_state before its Task CAS. Match that
        // order so deletion and resume cannot deadlock or both commit.
        let locked_operation_ids = sqlx::query_scalar::<_, Uuid>(
            r#"SELECT operation_id FROM operation_state
                WHERE operation_id=ANY($1)
                ORDER BY operation_id
                FOR UPDATE"#,
        )
        .bind(&active_stage_fork_operation_ids)
        .fetch_all(&mut *tx)
        .await?;
        if locked_operation_ids != active_stage_fork_operation_ids {
            return Err(conflict("organization_delete_stage_fork_authority_changed"));
        }
        let locked_tasks = sqlx::query_as::<_, ActiveStageForkBlockerRow>(
            r#"SELECT fork.operation_id,
                      fork.entry_stage AS stage,
                      task.status::TEXT AS status
                 FROM operation_stage_forks AS fork
                 JOIN tasks AS task ON task.id=fork.operation_id
                WHERE fork.operation_id=ANY($1)
                ORDER BY fork.operation_id
                FOR UPDATE OF task"#,
        )
        .bind(&active_stage_fork_operation_ids)
        .fetch_all(&mut *tx)
        .await?;
        if locked_tasks.len() != active_stage_fork_operation_ids.len() {
            return Err(conflict("organization_delete_stage_fork_authority_changed"));
        }
        let locked_worker_authority = sqlx::query_as::<_, LockedStageWorkerAuthorityRow>(
            r#"SELECT operation_id,
                          lease_token IS NOT NULL
                              AND lease_expires_at IS NOT NULL
                              AND lease_expires_at>NOW() AS has_live_lease,
                          active_tool_call_id
                     FROM stage_worker_runs
                    WHERE operation_id=ANY($1)
                    ORDER BY operation_id,id
                    FOR UPDATE"#,
        )
        .bind(&active_stage_fork_operation_ids)
        .fetch_all(&mut *tx)
        .await?;
        let mut worker_authority_by_operation = BTreeMap::<Uuid, (bool, bool)>::new();
        for worker in locked_worker_authority {
            let authority = worker_authority_by_operation
                .entry(worker.operation_id)
                .or_insert((false, false));
            authority.0 |= worker.has_live_lease;
            authority.1 |= worker.active_tool_call_id.is_some();
        }
        for task in locked_tasks {
            if matches!(task.status.as_str(), "finished" | "failed") {
                continue;
            }
            let (has_live_lease, has_active_tool) = worker_authority_by_operation
                .get(&task.operation_id)
                .copied()
                .unwrap_or((false, false));
            if task.status != "waiting" || has_live_lease || has_active_tool {
                return Err(crate::DbError::OrganizationDeletionActiveStageFork {
                    operation_id: task.operation_id,
                    stage: task.stage,
                    status: task.status,
                });
            }
            stopped_quiescent_stage_task_ids.push(task.operation_id);
        }
        if !stopped_quiescent_stage_task_ids.is_empty() {
            sqlx::query(
                r#"UPDATE tool_calls
                      SET status='failed',
                          result=COALESCE(result,$2),
                          updated_at=NOW()
                    WHERE task_id=ANY($1)
                      AND status IN ('received','running')"#,
            )
            .bind(&stopped_quiescent_stage_task_ids)
            .bind(ORGANIZATION_DELETION_STOPPED_TASK_RESULT)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                r#"UPDATE subtasks
                      SET status='failed',
                          result=COALESCE(result,$2),
                          updated_at=NOW()
                    WHERE task_id=ANY($1)
                      AND status IN ('created','running','waiting')"#,
            )
            .bind(&stopped_quiescent_stage_task_ids)
            .bind(ORGANIZATION_DELETION_STOPPED_TASK_RESULT)
            .execute(&mut *tx)
            .await?;
            let stopped_task_count = sqlx::query(
                r#"UPDATE tasks
                      SET status='failed',
                          result=COALESCE(result,$2),
                          updated_at=NOW()
                    WHERE id=ANY($1) AND status='waiting'"#,
            )
            .bind(&stopped_quiescent_stage_task_ids)
            .bind(ORGANIZATION_DELETION_STOPPED_TASK_RESULT)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if stopped_task_count
                != u64::try_from(stopped_quiescent_stage_task_ids.len())
                    .map_err(|_| conflict("organization_delete_stage_task_count"))?
            {
                return Err(conflict("organization_delete_stage_task_changed"));
            }
        }
    }
    let organization_snapshot = Value::Array(
        organizations
            .iter()
            .map(|row| {
                json!({
                    "organizationIdAtTime": row.id,
                    "parentOrganizationIdAtTime": row.parent_id,
                    "projectPathAtTime": row.project_path,
                    "organizationNameAtTime": row.name,
                    "depth": row.depth,
                    "ordinal": row.ordinal,
                })
            })
            .collect(),
    );
    let target_snapshot = Value::Array(
        targets
            .iter()
            .map(|row| {
                json!({
                    "targetIdAtTime": row.id,
                    "organizationIdAtTime": row.organization_id,
                    "projectPathAtTime": row.project_path,
                    "targetTypeAtTime": row.target_type,
                    "targetValueAtTime": row.value,
                    "targetNameAtTime": row.name,
                    "scopeAtTime": row.scope,
                })
            })
            .collect(),
    );
    let job = sqlx::query_as::<_, OrganizationDeletionJobRow>(
        r#"INSERT INTO organization_deletion_jobs(
               id,root_organization_id_at_time,project_scope_id,project_path_at_time,
               requested_by_principal_id,state,organization_snapshot,target_snapshot
           ) VALUES($1,$2,$3,$4,$5,'waiting_for_invalidation_delivery',$6,$7)
           RETURNING *"#,
    )
    .bind(input.job_id)
    .bind(input.root_organization_id)
    .bind(project_scope.0)
    .bind(&project_scope.1)
    .bind(input.principal_id)
    .bind(&organization_snapshot)
    .bind(&target_snapshot)
    .fetch_one(&mut *tx)
    .await?;
    append_state_history(
        &mut tx,
        job.id,
        "deleting_db_committed",
        json!({
            "preconditions": "passed",
            "stoppedQuiescentStageTaskIds": stopped_quiescent_stage_task_ids,
        }),
    )
    .await?;

    for row in &organizations {
        sqlx::query(
            r#"INSERT INTO organization_deletion_job_units(
                   job_id,organization_id_at_time,parent_organization_id_at_time,
                   organization_name_at_time,depth,ordinal
               ) VALUES($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(job.id)
        .bind(row.id)
        .bind(row.parent_id)
        .bind(&row.name)
        .bind(row.depth)
        .bind(i32::try_from(row.ordinal).map_err(|_| conflict("organization_delete_ordinal"))?)
        .execute(&mut *tx)
        .await?;
    }
    for row in &targets {
        let snapshot = json!({
            "targetIdAtTime": row.id,
            "organizationIdAtTime": row.organization_id,
            "projectPathAtTime": row.project_path,
            "targetTypeAtTime": row.target_type,
            "targetValueAtTime": row.value,
            "targetNameAtTime": row.name,
            "scopeAtTime": row.scope,
        });
        sqlx::query(
            r#"INSERT INTO organization_deletion_job_targets(
                   job_id,target_id_at_time,live_target_id,organization_id_at_time,
                   canonical_target_snapshot
               ) VALUES($1,$2,$2,$3,$4)"#,
        )
        .bind(job.id)
        .bind(row.id)
        .bind(row.organization_id)
        .bind(snapshot)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE operation_state SET engagement_org_id=NULL WHERE engagement_org_id=ANY($1)",
    )
    .bind(&organization_ids)
    .execute(&mut *tx)
    .await?;

    let sources = sqlx::query_as::<_, AssertionSourceRow>(
        r#"SELECT DISTINCT project_scope_id,organization_id_at_time,
                  source_operation_id,source_kind,source_id_kind,source_id_value,
                  source_stream_key,source_version
             FROM knowledge_assertions
            WHERE visibility='organization_long_term'
              AND status='active'
              AND organization_id_at_time=ANY($1)
            ORDER BY project_scope_id,organization_id_at_time,source_operation_id,
                     source_kind,source_id_kind,source_id_value,
                     source_stream_key,source_version"#,
    )
    .bind(&organization_ids)
    .fetch_all(&mut *tx)
    .await?;
    let required_delivery_manifest = required_invalidation_delivery_manifest()?;
    for row in &sources {
        let source = source_ref(row)?;
        let event = invalidation_event(&job, row, source.clone());
        let actual_event_id =
            super::knowledge_assertions::invalidate_projection_chain_with_event_with_connection(
                &mut tx,
                &source,
                job.requested_at,
                &event,
            )
            .await
            .map_err(|error| conflict(format!("{}: {error}", error.code())))?;
        sqlx::query(
            r#"INSERT INTO organization_deletion_job_invalidations(
                   job_id,event_id,source_stream_key,source_version,
                   required_delivery_manifest
               ) VALUES($1,$2,$3,$4,$5)"#,
        )
        .bind(job.id)
        .bind(actual_event_id)
        .bind(&row.source_stream_key)
        .bind(row.source_version)
        .bind(&required_delivery_manifest)
        .execute(&mut *tx)
        .await?;
    }
    let job = sqlx::query_as::<_, OrganizationDeletionJobRow>(
        r#"UPDATE organization_deletion_jobs
              SET required_invalidation_count=$2,row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 RETURNING *"#,
    )
    .bind(job.id)
    .bind(i32::try_from(sources.len()).map_err(|_| conflict("organization_delete_source_count"))?)
    .fetch_one(&mut *tx)
    .await?;
    append_state_history(
        &mut tx,
        job.id,
        "waiting_for_invalidation_delivery",
        json!({"requiredInvalidationCount": sources.len()}),
    )
    .await?;
    tx.commit().await?;
    Ok(job)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<OrganizationDeletionJobRow>> {
    Ok(sqlx::query_as::<_, OrganizationDeletionJobRow>(
        "SELECT * FROM organization_deletion_jobs WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_active(pool: &PgPool) -> Result<Vec<OrganizationDeletionJobRow>> {
    Ok(sqlx::query_as::<_, OrganizationDeletionJobRow>(
        r#"SELECT * FROM organization_deletion_jobs
            WHERE state<>'hard_delete_committed'
            ORDER BY requested_at,id"#,
    )
    .fetch_all(pool)
    .await?)
}

fn validate_worker(worker_id: &str, lease_seconds: i64) -> Result<()> {
    if worker_id.trim().is_empty()
        || worker_id.len() > 256
        || worker_id.chars().any(char::is_control)
        || !(15..=900).contains(&lease_seconds)
    {
        return Err(conflict("organization_deletion_worker_invalid"));
    }
    Ok(())
}

pub async fn claim_next_artifact_cleanup(
    pool: &PgPool,
    input: &ClaimOrganizationArtifactCleanup,
) -> Result<Option<(OrganizationDeletionJobRow, ArtifactCleanupPlan)>> {
    validate_worker(&input.worker_id, input.lease_seconds)?;
    let mut tx = pool.begin().await?;
    let candidate = sqlx::query_as::<_, OrganizationDeletionJobRow>(
        r#"SELECT job.* FROM organization_deletion_jobs AS job
            WHERE (
                job.state='pending_artifact_cleanup'
                AND job.artifact_retry_not_before<=NOW()
                AND (job.lease_token IS NULL OR job.lease_expires_at<=NOW())
            ) OR (
                job.state='waiting_for_invalidation_delivery'
                AND (
                    SELECT COUNT(*)
                      FROM organization_deletion_job_invalidations AS invalidation
                     WHERE invalidation.job_id=job.id
                )=job.required_invalidation_count
                AND NOT EXISTS (
                    SELECT 1
                      FROM organization_deletion_job_invalidations AS invalidation
                     WHERE invalidation.job_id=job.id
                       AND EXISTS (
                           SELECT 1
                             FROM jsonb_to_recordset(
                                 invalidation.required_delivery_manifest
                             ) AS required(
                                 projector_name TEXT,
                                 projector_schema_version INTEGER
                             )
                            WHERE NOT EXISTS (
                                SELECT 1
                                  FROM knowledge_projection_deliveries AS delivery
                                 WHERE delivery.event_id=invalidation.event_id
                                   AND delivery.projector_name=required.projector_name
                                   AND delivery.projector_schema_version=
                                       required.projector_schema_version
                                   AND delivery.status IN (
                                       'succeeded','succeeded_suppressed','stale'
                                   )
                            )
                       )
                )
            )
            ORDER BY job.requested_at,job.id
            FOR UPDATE OF job SKIP LOCKED
            LIMIT 1"#,
    )
    .fetch_optional(&mut *tx)
    .await?;
    let candidate = if let Some(candidate) = candidate {
        if candidate.state != "waiting_for_invalidation_delivery" {
            Some(candidate)
        } else {
            let ready = sqlx::query_as::<_, OrganizationDeletionJobRow>(
                r#"UPDATE organization_deletion_jobs
                  SET state='pending_artifact_cleanup',row_version=row_version+1,
                      artifact_retry_not_before=NOW(),updated_at=NOW(),
                      last_error_code=NULL,last_error=NULL
                WHERE id=$1 RETURNING *"#,
            )
            .bind(candidate.id)
            .fetch_one(&mut *tx)
            .await?;
            append_state_history(
                &mut tx,
                ready.id,
                "pending_artifact_cleanup",
                json!({"invalidationDeliveries": "succeeded"}),
            )
            .await?;
            Some(ready)
        }
    } else {
        None
    };
    let Some(candidate) = candidate else {
        tx.commit().await?;
        return Ok(None);
    };
    let lease_token = Uuid::new_v4();
    let lease_expires_at = Utc::now() + Duration::seconds(input.lease_seconds);
    let claimed = sqlx::query_as::<_, OrganizationDeletionJobRow>(
        r#"UPDATE organization_deletion_jobs
              SET lease_owner=$2,lease_token=$3,lease_expires_at=$4,
                  attempt_count=attempt_count+1,row_version=row_version+1,
                  artifact_cleanup_started_at=COALESCE(artifact_cleanup_started_at,NOW()),
                  updated_at=NOW()
            WHERE id=$1 AND state='pending_artifact_cleanup'
            RETURNING *"#,
    )
    .bind(candidate.id)
    .bind(input.worker_id.trim())
    .bind(lease_token)
    .bind(lease_expires_at)
    .fetch_one(&mut *tx)
    .await?;
    let targets = sqlx::query_as::<_, (Uuid, Uuid, Value)>(
        r#"SELECT target_id_at_time,organization_id_at_time,canonical_target_snapshot
             FROM organization_deletion_job_targets
            WHERE job_id=$1 ORDER BY organization_id_at_time,target_id_at_time"#,
    )
    .bind(claimed.id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(target_id_at_time, organization_id_at_time, snapshot)| {
        let string = |key: &str| {
            snapshot
                .get(key)
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| conflict("organization_delete_target_snapshot_corrupt"))
        };
        Ok(ArtifactCleanupTargetSnapshot {
            target_id_at_time,
            organization_id_at_time,
            // Artifact roots never come from target snapshots. Preserve the
            // field only for the internal plan compatibility shape, but fill
            // it from the server-owned project scope frozen on the job.
            project_path_at_time: claimed.project_path_at_time.clone(),
            target_type_at_time: string("targetTypeAtTime")?,
            target_value_at_time: string("targetValueAtTime")?,
        })
    })
    .collect::<Result<Vec<_>>>()?;
    let plan = ArtifactCleanupPlan {
        job_id: claimed.id,
        root_organization_id_at_time: claimed.root_organization_id_at_time,
        project_scope_id: claimed.project_scope_id,
        project_path_at_time: claimed.project_path_at_time.clone(),
        targets,
    };
    tx.commit().await?;
    Ok(Some((claimed, plan)))
}

pub async fn complete_artifact_cleanup(
    pool: &PgPool,
    input: &CompleteOrganizationArtifactCleanup,
) -> Result<OrganizationDeletionJobRow> {
    let (succeeded, error_code, error_message) = match &input.result {
        Ok(()) => (true, None, None),
        Err(error) => {
            if error.code.trim().is_empty()
                || error.code.len() > 128
                || error.message.trim().is_empty()
                || error.message.len() > 4096
            {
                return Err(conflict("organization_artifact_cleanup_result_invalid"));
            }
            (
                false,
                Some(error.code.as_str()),
                Some(error.message.as_str()),
            )
        }
    };
    let mut tx = pool.begin().await?;
    let row = sqlx::query_as::<_, OrganizationDeletionJobRow>(
        r#"UPDATE organization_deletion_jobs
              SET state=CASE WHEN $5 THEN 'artifact_cleanup_succeeded' ELSE state END,
                  lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,
                  artifact_cleanup_completed_at=CASE WHEN $5 THEN NOW() ELSE NULL END,
                  artifact_retry_not_before=CASE
                      WHEN $5 THEN artifact_retry_not_before
                      ELSE NOW()+make_interval(
                          secs => LEAST(300,(1 << LEAST(attempt_count,8)))
                      )
                  END,
                  hard_delete_attempt_count=CASE WHEN $5 THEN 0 ELSE hard_delete_attempt_count END,
                  hard_delete_retry_not_before=CASE WHEN $5 THEN NOW() ELSE hard_delete_retry_not_before END,
                  last_error_code=$6,last_error=$7,row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND state='pending_artifact_cleanup'
              AND lease_owner=$2 AND lease_token=$3 AND row_version=$4
              AND lease_expires_at>NOW()
            RETURNING *"#,
    )
    .bind(input.job_id)
    .bind(input.worker_id.trim())
    .bind(input.lease_token)
    .bind(input.expected_row_version)
    .bind(succeeded)
    .bind(error_code)
    .bind(error_message)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict("organization_artifact_cleanup_lease_fence_lost"))?;
    if succeeded {
        append_state_history(
            &mut tx,
            row.id,
            "artifact_cleanup_succeeded",
            json!({"attemptCount": row.attempt_count}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(row)
}

/// Return one DB-only hard-delete continuation. No external side effect is
/// performed here, so callers may race safely: `hard_delete` takes the row lock
/// and is idempotent after `hard_delete_committed`.
pub async fn next_hard_delete_ready(pool: &PgPool) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        r#"SELECT id FROM organization_deletion_jobs
            WHERE state='artifact_cleanup_succeeded'
              AND hard_delete_retry_not_before<=NOW()
            ORDER BY artifact_cleanup_completed_at NULLS FIRST,requested_at,id
            LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await?)
}

pub async fn hard_delete(pool: &PgPool, job_id: Uuid) -> Result<OrganizationDeletionJobRow> {
    let mut tx = pool.begin().await?;
    let current = sqlx::query_as::<_, OrganizationDeletionJobRow>(
        "SELECT * FROM organization_deletion_jobs WHERE id=$1 FOR UPDATE",
    )
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;
    if current.state == "hard_delete_committed" {
        tx.commit().await?;
        return Ok(current);
    }
    if current.state != "artifact_cleanup_succeeded" {
        return Err(conflict("organization_hard_delete_not_ready"));
    }
    let organization_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT organization_id_at_time FROM organization_deletion_job_units WHERE job_id=$1",
    )
    .bind(job_id)
    .fetch_all(&mut *tx)
    .await?;
    assert_deletion_preconditions(&mut tx, &organization_ids).await?;
    sqlx::query(
        r#"DELETE FROM targets AS target
           USING organization_deletion_job_targets AS frozen
           WHERE frozen.job_id=$1
             AND frozen.live_target_id=target.id"#,
    )
    .bind(job_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM organizations WHERE id=$1")
        .bind(current.root_organization_id_at_time)
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query_as::<_, OrganizationDeletionJobRow>(
        r#"UPDATE organization_deletion_jobs
              SET state='hard_delete_committed',hard_delete_committed_at=NOW(),
                  row_version=row_version+1,updated_at=NOW(),last_error_code=NULL,last_error=NULL
            WHERE id=$1 RETURNING *"#,
    )
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;
    append_state_history(
        &mut tx,
        row.id,
        "hard_delete_committed",
        json!({"retainedHistory": true}),
    )
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn record_hard_delete_error(
    pool: &PgPool,
    job_id: Uuid,
    error_code: &str,
    error: &str,
) -> Result<()> {
    if error_code.trim().is_empty()
        || error_code.len() > 128
        || error.trim().is_empty()
        || error.len() > 4096
    {
        return Err(conflict("organization_hard_delete_error_invalid"));
    }
    sqlx::query(
        r#"UPDATE organization_deletion_jobs
              SET hard_delete_attempt_count=hard_delete_attempt_count+1,
                  hard_delete_retry_not_before=NOW()+make_interval(
                      secs => LEAST(
                          300,
                          (1 << LEAST(hard_delete_attempt_count+1,8))
                      )
                  ),
                  last_error_code=$2,last_error=$3,row_version=row_version+1,updated_at=NOW()
            WHERE id=$1 AND state='artifact_cleanup_succeeded'"#,
    )
    .bind(job_id)
    .bind(error_code.trim())
    .bind(error.trim())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn cleanup_closeout_gate(
    pool: &PgPool,
    operation_id: Uuid,
    organization_id_at_time: Uuid,
) -> Result<CleanupCloseoutGateRow> {
    let mut connection = pool.acquire().await?;
    cleanup_closeout_gate_on(&mut connection, operation_id, organization_id_at_time).await
}

/// Transaction-bound Cleanup closeout truth for consumers such as Reporting
/// that must validate publication against one PostgreSQL snapshot. The SQL
/// remains Cleanup-owned; callers may not recreate terminal-state semantics.
pub async fn cleanup_closeout_gate_on(
    connection: &mut PgConnection,
    operation_id: Uuid,
    organization_id_at_time: Uuid,
) -> Result<CleanupCloseoutGateRow> {
    let row = sqlx::query_as::<_, (i64, i64, i64, i64, Vec<Uuid>)>(
        r#"SELECT
               (SELECT COUNT(*) FROM post_exploit_actions AS action
                 WHERE action.operation_id=$1 AND action.organization_id_at_time=$2
                   AND action.side_effect_class<>'none'
                   AND NOT EXISTS (
                       SELECT 1 FROM cleanup_obligations AS obligation
                        WHERE obligation.id=action.cleanup_obligation_id
                          AND obligation.source_action_id=action.id
                   )) AS missing_obligation_count,
               (SELECT COUNT(*) FROM cleanup_obligations AS obligation
                 WHERE obligation.operation_id=$1 AND obligation.organization_id_at_time=$2
                   AND obligation.status NOT IN (
                       'verified_absent','blocked','waived_by_user'
                   )) AS nonterminal_obligation_count,
               (SELECT COUNT(*) FROM cleanup_obligations AS obligation
                 WHERE obligation.operation_id=$1 AND obligation.organization_id_at_time=$2
                   AND obligation.status IN ('blocked','waived_by_user')
                   AND obligation.residual_risk IS NULL) AS undisclosed_residual_count,
               (SELECT COUNT(*) FROM cleanup_obligations AS obligation
                 WHERE obligation.operation_id=$1 AND obligation.organization_id_at_time=$2
                   AND obligation.status IN (
                       'verified_absent','blocked','waived_by_user'
                   )
                   AND NOT cleanup_obligation_state_truth_is_exact(obligation.id)
               ) AS invalid_terminal_truth_count,
               ARRAY(
                   SELECT obligation.id FROM cleanup_obligations AS obligation
                    WHERE obligation.operation_id=$1
                      AND obligation.organization_id_at_time=$2
                      AND obligation.status IN ('blocked','waived_by_user')
                      AND obligation.residual_risk IS NOT NULL
                    ORDER BY obligation.id
               ) AS residual_obligation_ids"#,
    )
    .bind(operation_id)
    .bind(organization_id_at_time)
    .fetch_one(&mut *connection)
    .await?;
    Ok(CleanupCloseoutGateRow {
        operation_id,
        organization_id_at_time,
        missing_obligation_count: row.0,
        nonterminal_obligation_count: row.1,
        undisclosed_residual_count: row.2,
        invalid_terminal_truth_count: row.3,
        residual_obligation_ids: row.4.into_iter().collect(),
    })
}

/// Reconcile expired generic cleanup attempts globally. `cleaned_pending_verification`
/// is never replayed as execution: it remains pending an independent absence
/// check. Claimed/executing attempts close as execution_failed and reopen the
/// obligation so a later attempt receives a new ordinal.
pub async fn reap_expired_cleanup_attempts(pool: &PgPool) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query_as::<_, (Uuid, Uuid)>(
        r#"UPDATE cleanup_attempts
              SET status='execution_failed',completed_at=NOW(),row_version=row_version+1,
                  terminal_note='cleanup_worker_lease_expired'
            WHERE status IN ('claimed','executing') AND lease_expires_at<=NOW()
            RETURNING id,obligation_id"#,
    )
    .fetch_all(&mut *tx)
    .await?;
    let obligation_ids = rows
        .iter()
        .map(|(_, obligation_id)| *obligation_id)
        .collect::<Vec<_>>();
    if !obligation_ids.is_empty() {
        sqlx::query(
            r#"UPDATE cleanup_obligations
                  SET status='open',row_version=row_version+1,updated_at=NOW()
                WHERE id=ANY($1) AND status='in_progress'"#,
        )
        .bind(&obligation_ids)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closeout_gate_is_db_counts_only() {
        let row = CleanupCloseoutGateRow {
            operation_id: Uuid::new_v4(),
            organization_id_at_time: Uuid::new_v4(),
            missing_obligation_count: 0,
            nonterminal_obligation_count: 0,
            undisclosed_residual_count: 0,
            invalid_terminal_truth_count: 0,
            residual_obligation_ids: BTreeSet::new(),
        };
        assert!(row.allows_closeout());
        assert!(!CleanupCloseoutGateRow {
            nonterminal_obligation_count: 1,
            ..row
        }
        .allows_closeout());
    }

    #[test]
    fn invalidation_ids_are_deterministic_per_job_and_source() {
        let job = OrganizationDeletionJobRow {
            id: Uuid::new_v4(),
            root_organization_id_at_time: Uuid::new_v4(),
            project_scope_id: Uuid::new_v4(),
            project_path_at_time: "/tmp/project".to_string(),
            requested_by_principal_id: Uuid::new_v4(),
            state: "waiting_for_invalidation_delivery".to_string(),
            organization_snapshot: json!([]),
            target_snapshot: json!([]),
            required_invalidation_count: 0,
            lease_owner: None,
            lease_token: None,
            lease_expires_at: None,
            attempt_count: 0,
            artifact_retry_not_before: Utc::now(),
            hard_delete_attempt_count: 0,
            hard_delete_retry_not_before: Utc::now(),
            row_version: 0,
            last_error_code: None,
            last_error: None,
            requested_at: Utc::now(),
            artifact_cleanup_started_at: None,
            artifact_cleanup_completed_at: None,
            hard_delete_committed_at: None,
            updated_at: Utc::now(),
        };
        let row = AssertionSourceRow {
            project_scope_id: Uuid::new_v4(),
            organization_id_at_time: Uuid::new_v4(),
            source_operation_id: Uuid::new_v4(),
            source_kind: "cleanup_obligation".to_string(),
            source_id_kind: "uuid".to_string(),
            source_id_value: Uuid::new_v4().to_string(),
            source_stream_key: "cleanup:test".to_string(),
            source_version: 1,
        };
        let first = invalidation_event(&job, &row, source_ref(&row).unwrap());
        let second = invalidation_event(&job, &row, source_ref(&row).unwrap());
        assert_eq!(first.event_id, second.event_id);
    }

    #[test]
    fn invalidation_delivery_manifest_tracks_the_event_catalog() {
        let manifest = required_invalidation_delivery_manifest().expect("invalidation routes");
        let entries = manifest.as_array().expect("manifest array");
        let routes = routes_for(KnowledgeEventNameV1::SourceScopeInvalidated);
        assert_eq!(entries.len(), routes.len());
        for (entry, route) in entries.iter().zip(routes) {
            assert_eq!(
                entry.get("projector_name").and_then(Value::as_str),
                Some(route.projector.name())
            );
            assert_eq!(
                entry
                    .get("projector_schema_version")
                    .and_then(Value::as_i64),
                Some(i64::from(route.projector.schema_version()))
            );
        }
    }
}
