//! Local-admin mutation seam for the Plan C-owned global rollout holds.
//!
//! This module is deliberately not exposed as a Tauri command.  It performs a
//! single-axis CAS and records the complete transition in the append-only
//! event ledger; promotion never calls it and therefore cannot clear a hold.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::operation_rollout::{OperationRolloutError, OperationRolloutResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationSafetyHoldScope {
    CampaignDispatch,
    OperationAdmission,
}

impl OperationSafetyHoldScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CampaignDispatch => "campaign_dispatch",
            Self::OperationAdmission => "operation_admission",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetOperationSafetyHold {
    pub scope: OperationSafetyHoldScope,
    pub next_held: bool,
    pub expected_generation: i64,
    pub expected_row_version: i64,
    pub reason_code: String,
    pub evidence_manifest_hash: String,
    pub principal_id: Uuid,
}

#[derive(Clone, Debug, sqlx::FromRow, Eq, PartialEq)]
pub struct OperationSafetyHoldEvent {
    pub event_id: Uuid,
    pub hold_scope: String,
    pub previous_held: bool,
    pub next_held: bool,
    pub previous_scope_generation: i64,
    pub next_scope_generation: i64,
    pub previous_row_version: i64,
    pub next_row_version: i64,
    pub reason_code: String,
    pub evidence_manifest_hash: String,
    pub principal_id: Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct LockedSafetyHold {
    campaign_dispatch_held: bool,
    operation_admission_held: bool,
    campaign_dispatch_generation: i64,
    operation_admission_generation: i64,
    row_version: i64,
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub async fn set_operation_safety_hold(
    tx: &mut Transaction<'_, Postgres>,
    request: SetOperationSafetyHold,
) -> OperationRolloutResult<OperationSafetyHoldEvent> {
    if request.reason_code.trim().is_empty()
        || request.reason_code.len() > 2048
        || !valid_hash(&request.evidence_manifest_hash)
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_SAFETY_HOLD_REQUEST_INVALID",
        });
    }
    let principal_is_active: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM operator_principals
                WHERE id=$1 AND active AND principal_kind='local_operator'
           )"#,
    )
    .bind(request.principal_id)
    .fetch_one(&mut **tx)
    .await?;
    if !principal_is_active {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_SAFETY_HOLD_PRINCIPAL_INVALID",
        });
    }

    let current = sqlx::query_as::<_, LockedSafetyHold>(
        r#"SELECT campaign_dispatch_held,operation_admission_held,
                  campaign_dispatch_generation,operation_admission_generation,row_version
             FROM verification_campaign_safety_holds
            WHERE singleton=TRUE FOR UPDATE"#,
    )
    .fetch_one(&mut **tx)
    .await?;
    let (previous_held, previous_generation) = match request.scope {
        OperationSafetyHoldScope::CampaignDispatch => (
            current.campaign_dispatch_held,
            current.campaign_dispatch_generation,
        ),
        OperationSafetyHoldScope::OperationAdmission => (
            current.operation_admission_held,
            current.operation_admission_generation,
        ),
    };
    if current.row_version != request.expected_row_version
        || previous_generation != request.expected_generation
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_SAFETY_HOLD_CAS_STALE",
        });
    }
    if previous_held == request.next_held {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_SAFETY_HOLD_NO_TRANSITION",
        });
    }

    let next_generation =
        previous_generation
            .checked_add(1)
            .ok_or(OperationRolloutError::Conflict {
                code: "OPERATION_SAFETY_HOLD_GENERATION_OVERFLOW",
            })?;
    let next_row_version =
        current
            .row_version
            .checked_add(1)
            .ok_or(OperationRolloutError::Conflict {
                code: "OPERATION_SAFETY_HOLD_VERSION_OVERFLOW",
            })?;
    let event_id = Uuid::new_v5(
        &request.principal_id,
        format!(
            "operation-safety-hold.v1:{}:{}:{}:{}:{}",
            request.scope.as_str(),
            previous_generation,
            next_generation,
            request.next_held,
            request.evidence_manifest_hash
        )
        .as_bytes(),
    );
    let event = sqlx::query_as::<_, OperationSafetyHoldEvent>(
        r#"INSERT INTO operation_rollout_safety_hold_events(
               event_id,hold_scope,previous_held,next_held,
               previous_scope_generation,next_scope_generation,
               previous_row_version,next_row_version,reason_code,
               evidence_manifest_hash,principal_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
           RETURNING *"#,
    )
    .bind(event_id)
    .bind(request.scope.as_str())
    .bind(previous_held)
    .bind(request.next_held)
    .bind(previous_generation)
    .bind(next_generation)
    .bind(current.row_version)
    .bind(next_row_version)
    .bind(request.reason_code.trim())
    .bind(&request.evidence_manifest_hash)
    .bind(request.principal_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query("SELECT set_config('golish.operation_safety_hold_event_id',$1,TRUE)")
        .bind(event_id.to_string())
        .execute(&mut **tx)
        .await?;
    let updated = match request.scope {
        OperationSafetyHoldScope::CampaignDispatch => {
            sqlx::query(
                r#"UPDATE verification_campaign_safety_holds
                  SET campaign_dispatch_held=$1,
                      campaign_dispatch_generation=$2,row_version=$3,reason_code=$4
                WHERE singleton=TRUE AND row_version=$5
                  AND campaign_dispatch_generation=$6"#,
            )
            .bind(request.next_held)
            .bind(next_generation)
            .bind(next_row_version)
            .bind(request.reason_code.trim())
            .bind(request.expected_row_version)
            .bind(request.expected_generation)
            .execute(&mut **tx)
            .await?
        }
        OperationSafetyHoldScope::OperationAdmission => {
            sqlx::query(
                r#"UPDATE verification_campaign_safety_holds
                  SET operation_admission_held=$1,
                      operation_admission_generation=$2,row_version=$3,reason_code=$4
                WHERE singleton=TRUE AND row_version=$5
                  AND operation_admission_generation=$6"#,
            )
            .bind(request.next_held)
            .bind(next_generation)
            .bind(next_row_version)
            .bind(request.reason_code.trim())
            .bind(request.expected_row_version)
            .bind(request.expected_generation)
            .execute(&mut **tx)
            .await?
        }
    };
    if updated.rows_affected() != 1 {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_SAFETY_HOLD_CAS_STALE",
        });
    }
    Ok(event)
}
