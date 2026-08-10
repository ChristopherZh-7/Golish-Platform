//! Server-selected latest objective outcomes and immutable adjudication census.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::verification_campaigns::{
    conflict, exact_set_hash_on, json_hash_on, AUTHORITY_STALE, CONTRACT_INVALID,
};
use crate::Result;

#[derive(Debug, Clone)]
pub struct SealObjectiveOutcomeSet {
    pub stable_request_id: Uuid,
    pub verification_plan_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub cutoff_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct CurrentOutcome {
    objective_id: Uuid,
    objective_ordinal: i32,
    outcome_id: Uuid,
    outcome_ordinal: i64,
    outcome_hash: String,
}

pub async fn seal_hypothesis_objective_outcome_set(
    pool: &PgPool,
    command: &SealObjectiveOutcomeSet,
) -> Result<Uuid> {
    if command.stable_request_id.is_nil() || command.verification_plan_id.is_nil() {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let expected_count: i64 = sqlx::query_scalar(
        "SELECT objective_count FROM attack_hypothesis_verification_plans WHERE plan_id=$1 AND revision_id=$2 AND sealed_at IS NOT NULL FOR SHARE",
    )
    .bind(command.verification_plan_id)
    .bind(command.hypothesis_revision_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let outcomes = sqlx::query_as::<_, CurrentOutcome>(
        r#"SELECT objective.objective_id,objective.ordinal AS objective_ordinal,
                  receipt.objective_outcome_receipt_id AS outcome_id,
                  receipt.outcome_ordinal,receipt.outcome_hash
             FROM attack_hypothesis_verification_plan_objectives objective
             JOIN hypothesis_objective_outcome_heads head
               ON head.verification_plan_id=objective.plan_id
              AND head.verification_objective_id=objective.objective_id
             JOIN hypothesis_objective_outcome_receipts receipt
               ON receipt.objective_outcome_receipt_id=head.current_outcome_id
            WHERE objective.plan_id=$1
              AND receipt.operation_id=$2 AND receipt.project_scope_id=$3
              AND receipt.organization_id=$4 AND receipt.created_at<=$5
              AND NOT EXISTS(
                  SELECT 1 FROM verification_authority_quarantine_events quarantine
                   WHERE quarantine.objective_outcome_receipt_id=receipt.objective_outcome_receipt_id
              )
            ORDER BY objective.ordinal
            FOR SHARE OF head,receipt"#,
    )
    .bind(command.verification_plan_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.cutoff_at)
    .fetch_all(&mut *tx)
    .await?;
    if outcomes.len() as i64 != expected_count {
        return Err(conflict(AUTHORITY_STALE));
    }
    let mut rows = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "objective_id": outcome.objective_id,
                "objective_ordinal": outcome.objective_ordinal,
                "outcome_id": outcome.outcome_id,
                "outcome_ordinal": outcome.outcome_ordinal,
                "outcome_hash": outcome.outcome_hash,
            }),
        )
        .await?;
        rows.push((outcome, member_hash));
    }
    let hashes = rows.iter().map(|row| row.1.clone()).collect::<Vec<_>>();
    let member_set_hash =
        exact_set_hash_on(&mut tx, "hypothesis_objective_outcome_set.v1", &hashes).await?;
    let head_set_hash = exact_set_hash_on(
        &mut tx,
        "hypothesis_objective_outcome_heads.v1",
        &rows
            .iter()
            .map(|row| row.0.outcome_hash.clone())
            .collect::<Vec<_>>(),
    )
    .await?;
    let seal_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "plan_id": command.verification_plan_id,
            "cutoff_at": command.cutoff_at,
            "head_set_hash": head_set_hash,
            "member_set_hash": member_set_hash,
        }),
    )
    .await?;
    let seal_id = Uuid::new_v5(
        &command.stable_request_id,
        b"hypothesis-objective-outcome-set.v1",
    );
    let existing: Option<(Uuid, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        r#"SELECT objective_outcome_set_seal_id,seal_hash,sealed_at
             FROM hypothesis_objective_outcome_set_seals
            WHERE stable_request_id=$1 FOR SHARE"#,
    )
    .bind(command.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((existing_id, existing_hash, sealed_at)) = existing {
        if existing_hash == seal_hash && sealed_at.is_some() {
            tx.commit().await?;
            return Ok(existing_id);
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    sqlx::query(
        r#"INSERT INTO hypothesis_objective_outcome_set_seals(
               objective_outcome_set_seal_id,stable_request_id,verification_plan_id,
               hypothesis_revision_id,operation_id,project_scope_id,organization_id,
               cutoff_at,head_set_hash,member_count,member_set_hash,seal_hash,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,NULL)"#,
    )
    .bind(seal_id)
    .bind(command.stable_request_id)
    .bind(command.verification_plan_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.cutoff_at)
    .bind(&head_set_hash)
    .bind(rows.len() as i64)
    .bind(&member_set_hash)
    .bind(&seal_hash)
    .execute(&mut *tx)
    .await?;
    for (outcome, member_hash) in rows {
        sqlx::query(
            r#"INSERT INTO hypothesis_objective_outcome_set_members(
                   objective_outcome_set_seal_id,verification_plan_id,operation_id,
                   project_scope_id,organization_id,member_ordinal,verification_objective_id,
                   selected_current_outcome_id,selected_current_ordinal,
                   selected_current_outcome_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(seal_id)
        .bind(command.verification_plan_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(outcome.objective_ordinal)
        .bind(outcome.objective_id)
        .bind(outcome.outcome_id)
        .bind(outcome.outcome_ordinal)
        .bind(outcome.outcome_hash)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE hypothesis_objective_outcome_set_seals SET sealed_at=statement_timestamp() WHERE objective_outcome_set_seal_id=$1 AND sealed_at IS NULL",
    )
    .bind(seal_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(seal_id)
}
