//! Independently fenced worker lease/checkpoint schema contract.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Executor, PgConnection, PgPool, Postgres};
use uuid::Uuid;

use super::runtime_memory_tx::{RuntimeMemoryStoreError, RuntimeMemoryStoreResult};

pub const TABLE_NAME: &str = "stage_worker_runs";
pub const STATUS_CHECK_SQL: &str =
    "CHECK (status IN ('queued','running','waiting_background','gate_blocked','passed',\
     'failed','exhausted','superseded','recovery_required'))";
pub const LOGICAL_WORK_ITEM_UNIQUE_SQL: &str =
    "UNIQUE(stage_run_unit_id, work_item_kind, work_item_key, worker_generation)";
pub const UNIT_OWNER_FK_SQL: &str =
    "FOREIGN KEY(stage_run_unit_id, operation_id, stage_execution_id, organization_id) \
     REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id)";
pub const LEASE_SHAPE_CHECK_SQL: &str =
    "CHECK ((lease_token IS NULL AND lease_owner IS NULL AND lease_expires_at IS NULL) \
     OR (lease_token IS NOT NULL AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL))";
pub const ACTIVE_TOOL_SHAPE_CHECK_SQL: &str =
    "CHECK ((active_tool_call_id IS NULL AND active_tool_started_at IS NULL) \
     OR (active_tool_call_id IS NOT NULL AND active_tool_started_at IS NOT NULL))";
pub const CHAIN_OWNER_INDEX_SQL: &str = "CREATE UNIQUE INDEX stage_worker_runs_chain_owner \
     ON stage_worker_runs(message_chain_id) WHERE message_chain_id IS NOT NULL";

#[derive(Debug, Clone, sqlx::FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct StageWorkerRunRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub work_item_id: Option<Uuid>,
    pub organization_id: Uuid,
    pub worker_generation: i32,
    pub specialist: String,
    pub work_item_kind: String,
    pub work_item_key: String,
    pub agent_path: String,
    pub parent_request_id: Option<String>,
    pub message_chain_id: Option<Uuid>,
    pub status: String,
    pub gate_attempt: i32,
    pub checkpoint: Value,
    pub checkpoint_version: i64,
    pub lease_token: Option<Uuid>,
    pub lease_owner: Option<String>,
    pub lease_acquired_at: Option<DateTime<Utc>>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub heartbeat_at: Option<DateTime<Utc>>,
    pub attempt_epoch: i64,
    pub active_tool_call_id: Option<Uuid>,
    pub active_tool_started_at: Option<DateTime<Utc>>,
    pub evidence_watermark: Option<i64>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StageWorkerRunStatus {
    Queued,
    Running,
    WaitingBackground,
    GateBlocked,
    Passed,
    Failed,
    Exhausted,
    Superseded,
    RecoveryRequired,
}

impl StageWorkerRunStatus {
    pub const ALL: [Self; 9] = [
        Self::Queued,
        Self::Running,
        Self::WaitingBackground,
        Self::GateBlocked,
        Self::Passed,
        Self::Failed,
        Self::Exhausted,
        Self::Superseded,
        Self::RecoveryRequired,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingBackground => "waiting_background",
            Self::GateBlocked => "gate_blocked",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Exhausted => "exhausted",
            Self::Superseded => "superseded",
            Self::RecoveryRequired => "recovery_required",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Passed | Self::Failed | Self::Exhausted | Self::Superseded
        )
    }

    pub const fn blocks_automatic_reclaim(self) -> bool {
        matches!(self, Self::RecoveryRequired)
    }

    pub const fn may_finish_as(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Running,
                Self::WaitingBackground
                    | Self::GateBlocked
                    | Self::Passed
                    | Self::Failed
                    | Self::Exhausted
                    | Self::RecoveryRequired
                    | Self::Superseded
            ) | (
                Self::WaitingBackground,
                Self::RecoveryRequired | Self::Superseded
            ) | (Self::GateBlocked, Self::Exhausted | Self::Superseded)
                | (Self::Queued, Self::Superseded)
        )
    }
}

const COLUMNS: &str = r#"id, operation_id, stage_execution_id, stage_run_unit_id,
    work_item_id, organization_id, worker_generation, specialist, work_item_kind, work_item_key,
    agent_path, parent_request_id, message_chain_id, status, gate_attempt, checkpoint,
    checkpoint_version, lease_token, lease_owner, lease_acquired_at, lease_expires_at,
    heartbeat_at, attempt_epoch, active_tool_call_id, active_tool_started_at,
    evidence_watermark, started_at, updated_at, terminal_at"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStageWorkerRun {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub work_item_id: Option<Uuid>,
    pub organization_id: Uuid,
    pub worker_generation: i32,
    pub specialist: String,
    pub work_item_kind: String,
    pub work_item_key: String,
    pub agent_path: String,
    pub parent_request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpiredWorkerDisposition {
    Requeued,
    RecoveryRequired,
}

pub async fn insert_with_executor<'e, E>(
    executor: E,
    input: &NewStageWorkerRun,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if input.worker_generation < 0
        || input.specialist.trim().is_empty()
        || input.work_item_kind.trim().is_empty()
        || input.work_item_key.trim().is_empty()
        || input.agent_path.trim().is_empty()
    {
        return Err(RuntimeMemoryStoreError::IdentityMismatch {
            code: "invalid_stage_worker_identity",
        });
    }
    let sql = format!(
        "INSERT INTO stage_worker_runs (
            id, operation_id, stage_execution_id, stage_run_unit_id, work_item_id,
            organization_id, worker_generation, specialist, work_item_kind,
            work_item_key, agent_path, parent_request_id, status
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'queued')
         RETURNING {COLUMNS}"
    );
    Ok(sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(input.id)
        .bind(input.operation_id)
        .bind(input.stage_execution_id)
        .bind(input.stage_run_unit_id)
        .bind(input.work_item_id)
        .bind(input.organization_id)
        .bind(input.worker_generation)
        .bind(&input.specialist)
        .bind(&input.work_item_kind)
        .bind(&input.work_item_key)
        .bind(&input.agent_path)
        .bind(&input.parent_request_id)
        .fetch_one(executor)
        .await?)
}

pub async fn get(pool: &PgPool, id: Uuid) -> RuntimeMemoryStoreResult<Option<StageWorkerRunRow>> {
    get_with_executor(pool, id).await
}

pub async fn get_with_executor<'e, E>(
    executor: E,
    id: Uuid,
) -> RuntimeMemoryStoreResult<Option<StageWorkerRunRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!("SELECT {COLUMNS} FROM stage_worker_runs WHERE id = $1");
    Ok(sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(id)
        .fetch_optional(executor)
        .await?)
}

pub async fn list_for_execution(
    pool: &PgPool,
    operation_id: Uuid,
    stage_execution_id: Uuid,
) -> RuntimeMemoryStoreResult<Vec<StageWorkerRunRow>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM stage_worker_runs
         WHERE operation_id=$1 AND stage_execution_id=$2
         ORDER BY stage_run_unit_id, worker_generation, id"
    );
    Ok(sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(operation_id)
        .bind(stage_execution_id)
        .fetch_all(pool)
        .await?)
}

pub async fn get_logical_with_executor<'e, E>(
    executor: E,
    stage_run_unit_id: Uuid,
    work_item_kind: &str,
    work_item_key: &str,
    worker_generation: i32,
) -> RuntimeMemoryStoreResult<Option<StageWorkerRunRow>>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "SELECT {COLUMNS} FROM stage_worker_runs
         WHERE stage_run_unit_id = $1 AND work_item_kind = $2
           AND work_item_key = $3 AND worker_generation = $4"
    );
    Ok(sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(stage_run_unit_id)
        .bind(work_item_kind)
        .bind(work_item_key)
        .bind(worker_generation)
        .fetch_optional(executor)
        .await?)
}

#[allow(clippy::too_many_arguments)]
pub async fn claim_cas<'e, E>(
    executor: E,
    worker_run_id: Uuid,
    stage_run_unit_id: Uuid,
    expected_status: StageWorkerRunStatus,
    expected_attempt_epoch: i64,
    lease_token: Uuid,
    lease_owner: &str,
    lease_seconds: i32,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if !matches!(
        expected_status,
        StageWorkerRunStatus::Queued
            | StageWorkerRunStatus::GateBlocked
            | StageWorkerRunStatus::WaitingBackground
    ) || lease_owner.trim().is_empty()
        || lease_seconds <= 0
    {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_stage_worker_claim",
        });
    }
    let sql = format!(
        "UPDATE stage_worker_runs
         SET status = 'running', lease_token = $5, lease_owner = $6,
             lease_acquired_at = NOW(),
             lease_expires_at = NOW() + make_interval(secs => $7),
             heartbeat_at = NOW(), attempt_epoch = attempt_epoch + 1,
             started_at = COALESCE(started_at, NOW()), updated_at = NOW()
         WHERE id = $1 AND stage_run_unit_id = $2 AND status = $3
           AND attempt_epoch = $4 AND active_tool_call_id IS NULL
           AND (lease_token IS NULL OR lease_expires_at <= NOW())
         RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(worker_run_id)
        .bind(stage_run_unit_id)
        .bind(expected_status.as_str())
        .bind(expected_attempt_epoch)
        .bind(lease_token)
        .bind(lease_owner)
        .bind(lease_seconds)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id,
            attempt_epoch: expected_attempt_epoch,
        })
}

pub async fn bind_message_chain_cas<'e, E>(
    executor: E,
    worker_run_id: Uuid,
    stage_run_unit_id: Uuid,
    lease_token: Uuid,
    attempt_epoch: i64,
    message_chain_id: Uuid,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE stage_worker_runs SET message_chain_id = $5, updated_at = NOW()
         WHERE id = $1 AND stage_run_unit_id = $2 AND lease_token = $3
           AND attempt_epoch = $4 AND status = 'running'
           AND message_chain_id IS NULL
         RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(worker_run_id)
        .bind(stage_run_unit_id)
        .bind(lease_token)
        .bind(attempt_epoch)
        .bind(message_chain_id)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id,
            attempt_epoch,
        })
}

pub async fn checkpoint_cas<'e, E>(
    executor: E,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
    checkpoint: &Value,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE stage_worker_runs
         SET checkpoint = $8, checkpoint_version = checkpoint_version + 1,
             heartbeat_at = NOW(), updated_at = NOW()
         WHERE id = $1 AND operation_id = $2 AND stage_execution_id = $3
           AND stage_run_unit_id = $4 AND lease_token = $5
           AND attempt_epoch = $6 AND checkpoint_version = $7
           AND status IN ('running','waiting_background','gate_blocked')
           AND lease_expires_at > NOW()
         RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(fence.worker_run_id)
        .bind(fence.operation_id)
        .bind(fence.stage_execution_id)
        .bind(fence.stage_run_unit_id)
        .bind(fence.lease_token)
        .bind(fence.attempt_epoch)
        .bind(fence.expected_checkpoint_version)
        .bind(checkpoint)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: fence.worker_run_id,
            attempt_epoch: fence.attempt_epoch,
        })
}

pub async fn heartbeat_cas<'e, E>(
    executor: E,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
    extend_seconds: i32,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if extend_seconds <= 0 {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_worker_heartbeat_extension",
        });
    }
    let sql = format!(
        "UPDATE stage_worker_runs
         SET heartbeat_at = NOW(),
             lease_expires_at = NOW() + make_interval(secs => $8),
             updated_at = NOW()
         WHERE id = $1 AND operation_id = $2 AND stage_execution_id = $3
           AND stage_run_unit_id = $4 AND lease_token = $5
           AND attempt_epoch = $6 AND checkpoint_version = $7
           AND status IN ('running','waiting_background','gate_blocked')
           AND lease_expires_at > NOW()
         RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(fence.worker_run_id)
        .bind(fence.operation_id)
        .bind(fence.stage_execution_id)
        .bind(fence.stage_run_unit_id)
        .bind(fence.lease_token)
        .bind(fence.attempt_epoch)
        .bind(fence.expected_checkpoint_version)
        .bind(extend_seconds)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: fence.worker_run_id,
            attempt_epoch: fence.attempt_epoch,
        })
}

pub async fn begin_tool_cas<'e, E>(
    executor: E,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
    tool_call_id: Uuid,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE stage_worker_runs
         SET active_tool_call_id = $8, active_tool_started_at = NOW(), updated_at = NOW()
         WHERE id = $1 AND operation_id = $2 AND stage_execution_id = $3
           AND stage_run_unit_id = $4 AND lease_token = $5
           AND attempt_epoch = $6 AND checkpoint_version = $7
           AND status = 'running' AND lease_expires_at > NOW()
           AND active_tool_call_id IS NULL
         RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(fence.worker_run_id)
        .bind(fence.operation_id)
        .bind(fence.stage_execution_id)
        .bind(fence.stage_run_unit_id)
        .bind(fence.lease_token)
        .bind(fence.attempt_epoch)
        .bind(fence.expected_checkpoint_version)
        .bind(tool_call_id)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: fence.worker_run_id,
            attempt_epoch: fence.attempt_epoch,
        })
}

pub async fn finish_tool_cas<'e, E>(
    executor: E,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
    tool_call_id: Uuid,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    let sql = format!(
        "UPDATE stage_worker_runs
         SET active_tool_call_id = NULL, active_tool_started_at = NULL,
             heartbeat_at = NOW(), updated_at = NOW()
         WHERE id = $1 AND operation_id = $2 AND stage_execution_id = $3
           AND stage_run_unit_id = $4 AND lease_token = $5
           AND attempt_epoch = $6 AND checkpoint_version = $7
           AND status = 'running' AND lease_expires_at > NOW()
           AND active_tool_call_id = $8
         RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(fence.worker_run_id)
        .bind(fence.operation_id)
        .bind(fence.stage_execution_id)
        .bind(fence.stage_run_unit_id)
        .bind(fence.lease_token)
        .bind(fence.attempt_epoch)
        .bind(fence.expected_checkpoint_version)
        .bind(tool_call_id)
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: fence.worker_run_id,
            attempt_epoch: fence.attempt_epoch,
        })
}

pub async fn finish_attempt_cas<'e, E>(
    executor: E,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
    expected_status: StageWorkerRunStatus,
    next_status: StageWorkerRunStatus,
    checkpoint: &Value,
    evidence_watermark: Option<i64>,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if next_status == StageWorkerRunStatus::Passed {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "worker_pass_requires_final_seal",
        });
    }
    finish_attempt_cas_inner(
        executor,
        fence,
        expected_status,
        next_status,
        checkpoint,
        evidence_watermark,
    )
    .await
}

pub(crate) async fn finish_passed_for_final_seal<'e, E>(
    executor: E,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
    checkpoint: &Value,
    evidence_watermark: Option<i64>,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    finish_attempt_cas_inner(
        executor,
        fence,
        StageWorkerRunStatus::Running,
        StageWorkerRunStatus::Passed,
        checkpoint,
        evidence_watermark,
    )
    .await
}

/// Team producers may finish their own independently fenced WorkerRun, but
/// they are deliberately not allowed to seal the owning StageRunUnit.  Keep
/// that narrow PASS authority separate from the public attempt finisher so a
/// caller cannot accidentally turn an ordinary worker completion into a Unit
/// final seal.
pub(crate) async fn finish_passed_for_stage_output<'e, E>(
    executor: E,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
    checkpoint: &Value,
    evidence_watermark: Option<i64>,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    finish_attempt_cas_inner(
        executor,
        fence,
        StageWorkerRunStatus::Running,
        StageWorkerRunStatus::Passed,
        checkpoint,
        evidence_watermark,
    )
    .await
}

async fn finish_attempt_cas_inner<'e, E>(
    executor: E,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
    expected_status: StageWorkerRunStatus,
    next_status: StageWorkerRunStatus,
    checkpoint: &Value,
    evidence_watermark: Option<i64>,
) -> RuntimeMemoryStoreResult<StageWorkerRunRow>
where
    E: Executor<'e, Database = Postgres>,
{
    if !expected_status.may_finish_as(next_status) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "invalid_stage_worker_transition",
        });
    }
    let sql = format!(
        "UPDATE stage_worker_runs
         SET status = $8, checkpoint = $9,
             checkpoint_version = checkpoint_version + 1,
             evidence_watermark = COALESCE($10, evidence_watermark),
             lease_token = NULL, lease_owner = NULL, lease_acquired_at = NULL,
             lease_expires_at = NULL, heartbeat_at = NULL, updated_at = NOW(),
             terminal_at = CASE WHEN $8 IN ('passed','failed','exhausted','superseded')
                                THEN NOW() ELSE NULL END
         WHERE id = $1 AND operation_id = $2 AND stage_execution_id = $3
           AND stage_run_unit_id = $4 AND lease_token = $5
           AND attempt_epoch = $6 AND checkpoint_version = $7
           AND status = $11 AND active_tool_call_id IS NULL
         RETURNING {COLUMNS}"
    );
    sqlx::query_as::<_, StageWorkerRunRow>(&sql)
        .bind(fence.worker_run_id)
        .bind(fence.operation_id)
        .bind(fence.stage_execution_id)
        .bind(fence.stage_run_unit_id)
        .bind(fence.lease_token)
        .bind(fence.attempt_epoch)
        .bind(fence.expected_checkpoint_version)
        .bind(next_status.as_str())
        .bind(checkpoint)
        .bind(evidence_watermark)
        .bind(expected_status.as_str())
        .fetch_optional(executor)
        .await?
        .ok_or(RuntimeMemoryStoreError::LeaseLost {
            worker_run_id: fence.worker_run_id,
            attempt_epoch: fence.attempt_epoch,
        })
}

/// Classify one expired worker under a row lock. Unknown in-flight external
/// work is parked for manual recovery; only a worker with no active tool may be
/// made claimable again.
pub async fn reap_expired(
    pool: &PgPool,
    worker_run_id: Uuid,
) -> RuntimeMemoryStoreResult<(ExpiredWorkerDisposition, StageWorkerRunRow)> {
    let mut tx = pool.begin().await?;
    let reaped = reap_expired_with_connection(&mut tx, worker_run_id).await?;
    tx.commit().await?;
    Ok(reaped)
}

pub async fn reap_expired_with_connection(
    connection: &mut PgConnection,
    worker_run_id: Uuid,
) -> RuntimeMemoryStoreResult<(ExpiredWorkerDisposition, StageWorkerRunRow)> {
    let select_sql = format!("SELECT {COLUMNS} FROM stage_worker_runs WHERE id = $1 FOR UPDATE");
    let locked = sqlx::query_as::<_, StageWorkerRunRow>(&select_sql)
        .bind(worker_run_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(RuntimeMemoryStoreError::Missing { entity: TABLE_NAME })?;
    if locked.status == StageWorkerRunStatus::RecoveryRequired.as_str() {
        return Ok((ExpiredWorkerDisposition::RecoveryRequired, locked));
    }
    if matches!(
        locked.status.as_str(),
        "passed" | "failed" | "exhausted" | "superseded"
    ) {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "terminal_worker_cannot_be_reaped",
        });
    }
    let Some(expires_at) = locked.lease_expires_at else {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "worker_has_no_lease_to_reap",
        });
    };
    if expires_at > Utc::now() {
        return Err(RuntimeMemoryStoreError::Conflict {
            code: "worker_lease_not_expired",
        });
    }

    let (disposition, update_sql) = if locked.active_tool_call_id.is_some() {
        (
            ExpiredWorkerDisposition::RecoveryRequired,
            format!(
                "UPDATE stage_worker_runs
                 SET status = 'recovery_required', updated_at = NOW()
                 WHERE id = $1 AND attempt_epoch = $2
                 RETURNING {COLUMNS}"
            ),
        )
    } else {
        (
            ExpiredWorkerDisposition::Requeued,
            format!(
                "UPDATE stage_worker_runs
                 SET status = 'queued', lease_token = NULL, lease_owner = NULL,
                     lease_acquired_at = NULL, lease_expires_at = NULL,
                     heartbeat_at = NULL, updated_at = NOW(), terminal_at = NULL
                 WHERE id = $1 AND attempt_epoch = $2
                 RETURNING {COLUMNS}"
            ),
        )
    };
    let updated = sqlx::query_as::<_, StageWorkerRunRow>(&update_sql)
        .bind(worker_run_id)
        .bind(locked.attempt_epoch)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(RuntimeMemoryStoreError::Conflict {
            code: "worker_changed_during_reap",
        })?;
    Ok((disposition, updated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_memory_repo_contract_worker_has_fencing_and_logical_identity() {
        assert_eq!(TABLE_NAME, "stage_worker_runs");
        assert!(LOGICAL_WORK_ITEM_UNIQUE_SQL
            .contains("stage_run_unit_id, work_item_kind, work_item_key, worker_generation"));
        assert!(LEASE_SHAPE_CHECK_SQL.contains("lease_token IS NULL"));
        assert!(LEASE_SHAPE_CHECK_SQL.contains("lease_expires_at IS NOT NULL"));
        assert!(ACTIVE_TOOL_SHAPE_CHECK_SQL.contains("active_tool_call_id"));
        assert!(StageWorkerRunStatus::RecoveryRequired.blocks_automatic_reclaim());
        assert!(!StageWorkerRunStatus::RecoveryRequired.is_terminal());
        assert!(StageWorkerRunStatus::ALL
            .iter()
            .all(|status| STATUS_CHECK_SQL.contains(status.as_str())));
    }

    #[test]
    fn worker_transition_graph_never_reopens_terminal_or_recovery_rows() {
        assert!(
            StageWorkerRunStatus::Running.may_finish_as(StageWorkerRunStatus::WaitingBackground)
        );
        assert!(StageWorkerRunStatus::Running.may_finish_as(StageWorkerRunStatus::Passed));
        assert!(!StageWorkerRunStatus::GateBlocked.may_finish_as(StageWorkerRunStatus::Running));
        assert!(StageWorkerRunStatus::Queued.may_finish_as(StageWorkerRunStatus::Superseded));
        assert!(!StageWorkerRunStatus::Passed.may_finish_as(StageWorkerRunStatus::Running));
        assert!(
            !StageWorkerRunStatus::RecoveryRequired.may_finish_as(StageWorkerRunStatus::Running)
        );
        assert_ne!(
            ExpiredWorkerDisposition::Requeued,
            ExpiredWorkerDisposition::RecoveryRequired
        );
    }
}
