//! Objective-local claim-component seals and atomic Campaign closeout.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::verification_campaigns::{
    conflict, exact_set_hash_on, json_hash_on, require_sha256, AUTHORITY_STALE, CONTRACT_INVALID,
};
use crate::Result;

#[derive(Debug, Clone)]
pub struct ClaimComponentOutcomeMember {
    pub claim_component_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub claim_component_hash: String,
    pub predicate_component_id: Uuid,
    pub oracle_census_member_id: Option<Uuid>,
    pub campaign_coverage_member_id: Option<Uuid>,
    pub component_outcome: String,
    pub residual_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct SealObjectiveClaimComponentOutcomes {
    pub stable_request_id: Uuid,
    pub verification_plan_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_objective_id: Uuid,
    pub campaign_id: Option<Uuid>,
    pub members: Vec<ClaimComponentOutcomeMember>,
}

#[derive(sqlx::FromRow)]
struct ExistingCampaignObjectiveCloseout {
    campaign_terminal_decision_id: Uuid,
    campaign_coverage_receipt_id: Uuid,
    fact_delta_bundle_id: Uuid,
    objective_outcome_receipt_id: Uuid,
    outcome: String,
    source_authority_hash: String,
    fact_delta_hash: String,
    result_membership_hash: String,
    residual_membership_hash: String,
    verification_plan_id: Uuid,
    verification_objective_id: Uuid,
}

pub async fn seal_objective_claim_component_outcomes(
    pool: &PgPool,
    command: &SealObjectiveClaimComponentOutcomes,
) -> Result<Uuid> {
    if command.members.is_empty() {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let seal_id = Uuid::new_v5(
        &command.stable_request_id,
        b"hypothesis-objective-claim-component-outcomes.v1",
    );
    let mut rows = Vec::with_capacity(command.members.len());
    for (ordinal, member) in command.members.iter().enumerate() {
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "claim_component_id": member.claim_component_id,
                "claim_component_hash": member.claim_component_hash,
                "predicate_component_id": member.predicate_component_id,
                "oracle_census_member_id": member.oracle_census_member_id,
                "campaign_coverage_member_id": member.campaign_coverage_member_id,
                "component_outcome": member.component_outcome,
                "residual_id": member.residual_id,
            }),
        )
        .await?;
        rows.push((member, member_hash));
    }
    let hashes = rows.iter().map(|row| row.1.clone()).collect::<Vec<_>>();
    let member_set_hash = exact_set_hash_on(
        &mut tx,
        "hypothesis_objective_claim_component_outcomes.v1",
        &hashes,
    )
    .await?;
    let seal_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "verification_plan_id": command.verification_plan_id,
            "verification_objective_id": command.verification_objective_id,
            "member_set_hash": member_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_objective_claim_component_outcome_seals(
               claim_component_outcome_seal_id,stable_request_id,verification_plan_id,
               hypothesis_revision_id,verification_objective_id,campaign_id,member_count,
               member_set_hash,seal_hash,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,NULL)"#,
    )
    .bind(seal_id)
    .bind(command.stable_request_id)
    .bind(command.verification_plan_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.verification_objective_id)
    .bind(command.campaign_id)
    .bind(rows.len() as i64)
    .bind(&member_set_hash)
    .bind(&seal_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (member, member_hash)) in rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO hypothesis_objective_claim_component_outcome_members(
                   claim_component_outcome_member_id,claim_component_outcome_seal_id,
                   verification_plan_id,verification_objective_id,member_ordinal,
                   claim_component_id,hypothesis_revision_id,claim_component_hash,
                   predicate_component_id,oracle_census_member_id,campaign_coverage_member_id,
                   component_outcome,residual_id,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(Uuid::new_v5(&seal_id, member_hash.as_bytes()))
        .bind(seal_id)
        .bind(command.verification_plan_id)
        .bind(command.verification_objective_id)
        .bind(ordinal as i32)
        .bind(member.claim_component_id)
        .bind(member.hypothesis_revision_id)
        .bind(&member.claim_component_hash)
        .bind(member.predicate_component_id)
        .bind(member.oracle_census_member_id)
        .bind(member.campaign_coverage_member_id)
        .bind(&member.component_outcome)
        .bind(member.residual_id)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE hypothesis_objective_claim_component_outcome_seals SET sealed_at=statement_timestamp() WHERE claim_component_outcome_seal_id=$1 AND sealed_at IS NULL",
    )
    .bind(seal_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(seal_id)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CampaignCoverageResult {
    pub campaign_coverage_member_id: Uuid,
    pub coverage_disposition: String,
    pub epistemic_outcome: String,
    pub control_binding_kind: String,
    pub control_validity: String,
    pub prepared_action_id: Option<Uuid>,
    pub capability_execution_receipt_id: Option<Uuid>,
    pub oracle_assessment_id: Option<Uuid>,
    pub residual_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct CloseCampaignObjective {
    pub stable_request_id: Uuid,
    pub campaign_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_plan_id: Uuid,
    pub verification_objective_id: Uuid,
    pub verification_contract_hash: String,
    pub expected_campaign_row_version: i64,
    pub oracle_census_seal_id: Uuid,
    pub campaign_denominator_id: Uuid,
    pub claim_component_outcome_seal_id: Uuid,
    pub outcome: String,
    pub unresolved_member_set_hash: Option<String>,
    pub residual_id: Option<Uuid>,
    pub coverage_results: Vec<CampaignCoverageResult>,
    pub fact_delta_kind: String,
    pub typed_fact_delta: Value,
    pub evidence_ref_set_hash: String,
    pub source_authority_hash: String,
    pub expected_predecessor_outcome_id: Option<Uuid>,
    pub expected_outcome_ordinal: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignObjectiveCloseout {
    pub campaign_terminal_decision_id: Uuid,
    pub campaign_coverage_receipt_id: Uuid,
    pub fact_delta_bundle_id: Uuid,
    pub objective_outcome_receipt_id: Uuid,
}

pub async fn close_campaign_objective_with_fact_delta(
    pool: &PgPool,
    command: &CloseCampaignObjective,
) -> Result<CampaignObjectiveCloseout> {
    if command.coverage_results.is_empty()
        || !matches!(
            command.outcome.as_str(),
            "proof" | "refutation" | "inconclusive" | "blocked" | "exhausted_with_residuals"
        )
        || !matches!(
            command.fact_delta_kind.as_str(),
            "support" | "contradiction" | "inconclusive" | "no_change" | "retraction"
        )
        || (matches!(command.outcome.as_str(), "proof" | "refutation"))
            != command.residual_id.is_none()
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    for hash in [
        &command.verification_contract_hash,
        &command.evidence_ref_set_hash,
        &command.source_authority_hash,
    ] {
        require_sha256(hash)?;
    }
    if let Some(hash) = &command.unresolved_member_set_hash {
        require_sha256(hash)?;
    }
    let mut tx = pool.begin().await?;
    let adjudication_id = Uuid::new_v5(&command.stable_request_id, b"campaign-adjudication.v1");
    let terminal_id = Uuid::new_v5(&command.stable_request_id, b"campaign-terminal.v1");
    let coverage_receipt_id = Uuid::new_v5(&command.stable_request_id, b"campaign-coverage.v1");
    let fact_delta_id = Uuid::new_v5(&command.stable_request_id, b"verification-fact-delta.v1");
    let objective_outcome_id = Uuid::new_v5(&command.stable_request_id, b"objective-outcome.v1");
    let fact_delta_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "campaign_id": command.campaign_id,
            "hypothesis_revision_id": command.hypothesis_revision_id,
            "verification_objective_id": command.verification_objective_id,
            "delta_kind": command.fact_delta_kind,
            "typed_delta": command.typed_fact_delta,
            "evidence_ref_set_hash": command.evidence_ref_set_hash,
            "source_authority_hash": command.source_authority_hash,
        }),
    )
    .await?;
    let mut result_rows = Vec::with_capacity(command.coverage_results.len());
    let mut residual_hashes = Vec::new();
    let mut counts = [0_i64; 4];
    for result in &command.coverage_results {
        let result_hash = json_hash_on(&mut tx, &serde_json::json!(result)).await?;
        match result.coverage_disposition.as_str() {
            "tested_complete" => counts[0] += 1,
            "tested_degraded" => counts[1] += 1,
            "untested" => counts[2] += 1,
            "blocked" => counts[3] += 1,
            _ => return Err(conflict(CONTRACT_INVALID)),
        }
        if let Some(residual_id) = result.residual_id {
            residual_hashes.push(json_hash_on(&mut tx, &serde_json::json!(residual_id)).await?);
        }
        result_rows.push((result, result_hash));
    }
    let result_hashes = result_rows
        .iter()
        .map(|row| row.1.clone())
        .collect::<Vec<_>>();
    let result_membership_hash = exact_set_hash_on(
        &mut tx,
        "verification_campaign_coverage_results.v1",
        &result_hashes,
    )
    .await?;
    let residual_membership_hash = exact_set_hash_on(
        &mut tx,
        "verification_campaign_coverage_residuals.v1",
        &residual_hashes,
    )
    .await?;
    let coverage_status = if counts[2] == 0 && counts[3] == 0 && counts[1] == 0 {
        "complete"
    } else {
        "partial"
    };
    let existing = sqlx::query_as::<_, ExistingCampaignObjectiveCloseout>(
        r#"SELECT terminal.campaign_terminal_decision_id,
                  coverage.campaign_coverage_receipt_id,fact.fact_delta_bundle_id,
                  outcome.objective_outcome_receipt_id,outcome.outcome,
                  outcome.source_authority_hash,fact.fact_delta_hash,
                  coverage.result_membership_hash,coverage.residual_membership_hash,
                  outcome.verification_plan_id,outcome.verification_objective_id
             FROM verification_campaign_terminal_decisions terminal
             JOIN verification_campaign_coverage_receipts coverage
               ON coverage.campaign_terminal_decision_id=terminal.campaign_terminal_decision_id
             JOIN verification_fact_delta_bundles fact
               ON fact.campaign_terminal_decision_id=terminal.campaign_terminal_decision_id
             JOIN hypothesis_objective_outcome_receipts outcome
               ON outcome.campaign_terminal_decision_id=terminal.campaign_terminal_decision_id
            WHERE terminal.campaign_terminal_decision_id=$1 AND terminal.campaign_id=$2
            FOR SHARE"#,
    )
    .bind(terminal_id)
    .bind(command.campaign_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = existing {
        if row.outcome == command.outcome
            && row.source_authority_hash == command.source_authority_hash
            && row.fact_delta_hash == fact_delta_hash
            && row.result_membership_hash == result_membership_hash
            && row.residual_membership_hash == residual_membership_hash
            && row.verification_plan_id == command.verification_plan_id
            && row.verification_objective_id == command.verification_objective_id
        {
            tx.commit().await?;
            return Ok(CampaignObjectiveCloseout {
                campaign_terminal_decision_id: row.campaign_terminal_decision_id,
                campaign_coverage_receipt_id: row.campaign_coverage_receipt_id,
                fact_delta_bundle_id: row.fact_delta_bundle_id,
                objective_outcome_receipt_id: row.objective_outcome_receipt_id,
            });
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    let campaign_state: (String, i64) = sqlx::query_as(
        r#"SELECT state,row_version FROM verification_campaigns
            WHERE campaign_id=$1 AND operation_id=$2 AND project_scope_id=$3
              AND organization_id=$4 AND hypothesis_revision_id=$5
              AND verification_objective_id=$6 AND verification_contract_hash=$7
            FOR UPDATE"#,
    )
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.verification_objective_id)
    .bind(&command.verification_contract_hash)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    if campaign_state.1 != command.expected_campaign_row_version
        || !matches!(campaign_state.0.as_str(), "running" | "draining")
    {
        return Err(conflict(AUTHORITY_STALE));
    }
    let (oracle_census_hash, sealed_at): (String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as(
            "SELECT census_hash,sealed_at FROM verification_oracle_census_seals WHERE oracle_census_seal_id=$1 AND campaign_id=$2 FOR SHARE",
        )
        .bind(command.oracle_census_seal_id)
        .bind(command.campaign_id)
        .fetch_one(&mut *tx)
        .await?;
    if sealed_at.is_none() {
        return Err(conflict(AUTHORITY_STALE));
    }
    let claim_seal_hash: String = sqlx::query_scalar(
        "SELECT seal_hash FROM hypothesis_objective_claim_component_outcome_seals WHERE claim_component_outcome_seal_id=$1 AND verification_plan_id=$2 AND verification_objective_id=$3 AND sealed_at IS NOT NULL FOR SHARE",
    )
    .bind(command.claim_component_outcome_seal_id)
    .bind(command.verification_plan_id)
    .bind(command.verification_objective_id)
    .fetch_one(&mut *tx)
    .await?;
    let adjudication_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "campaign_id": command.campaign_id,
            "oracle_census_hash": oracle_census_hash,
            "outcome": command.outcome,
            "unresolved_member_set_hash": command.unresolved_member_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO verification_campaign_adjudications(
               campaign_adjudication_id,stable_request_id,campaign_id,oracle_census_seal_id,
               operation_id,project_scope_id,organization_id,verification_contract_hash,
               oracle_census_hash,outcome,unresolved_member_set_hash,adjudication_hash,residual_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
    )
    .bind(adjudication_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"campaign-adjudication-request.v1",
    ))
    .bind(command.campaign_id)
    .bind(command.oracle_census_seal_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(&command.verification_contract_hash)
    .bind(&oracle_census_hash)
    .bind(&command.outcome)
    .bind(&command.unresolved_member_set_hash)
    .bind(&adjudication_hash)
    .bind(command.residual_id)
    .execute(&mut *tx)
    .await?;
    let terminal_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "campaign_id": command.campaign_id,
            "campaign_adjudication_id": adjudication_id,
            "outcome": command.outcome,
            "adjudication_hash": adjudication_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO verification_campaign_terminal_decisions(
               campaign_terminal_decision_id,stable_request_id,campaign_id,
               campaign_adjudication_id,operation_id,project_scope_id,organization_id,
               terminal_decision,terminal_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(terminal_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"campaign-terminal-request.v1",
    ))
    .bind(command.campaign_id)
    .bind(adjudication_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(&command.outcome)
    .bind(&terminal_hash)
    .execute(&mut *tx)
    .await?;
    let denominator_hash: String = sqlx::query_scalar(
        "SELECT member_set_hash FROM verification_campaign_coverage_denominators WHERE campaign_denominator_id=$1 AND campaign_id=$2 AND sealed_at IS NOT NULL FOR SHARE",
    )
    .bind(command.campaign_denominator_id)
    .bind(command.campaign_id)
    .fetch_one(&mut *tx)
    .await?;
    let coverage_receipt_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "denominator_hash": denominator_hash,
            "result_membership_hash": result_membership_hash,
            "residual_membership_hash": residual_membership_hash,
            "counts": counts,
            "coverage_status": coverage_status,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO verification_campaign_coverage_receipts(
               campaign_coverage_receipt_id,stable_request_id,campaign_id,
               campaign_terminal_decision_id,campaign_denominator_id,operation_id,
               project_scope_id,organization_id,denominator_hash,result_membership_hash,
               residual_membership_hash,tested_complete_count,tested_degraded_count,
               untested_count,blocked_count,coverage_status,receipt_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
    )
    .bind(coverage_receipt_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"campaign-coverage-request.v1",
    ))
    .bind(command.campaign_id)
    .bind(terminal_id)
    .bind(command.campaign_denominator_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(&denominator_hash)
    .bind(&result_membership_hash)
    .bind(&residual_membership_hash)
    .bind(counts[0])
    .bind(counts[1])
    .bind(counts[2])
    .bind(counts[3])
    .bind(coverage_status)
    .bind(&coverage_receipt_hash)
    .execute(&mut *tx)
    .await?;
    for (result, result_hash) in result_rows {
        sqlx::query(
            r#"INSERT INTO verification_campaign_coverage_results(
                   campaign_coverage_receipt_id,campaign_coverage_member_id,
                   coverage_disposition,epistemic_outcome,control_binding_kind,
                   control_validity,prepared_action_id,capability_execution_receipt_id,
                   oracle_assessment_id,residual_id,result_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(coverage_receipt_id)
        .bind(result.campaign_coverage_member_id)
        .bind(&result.coverage_disposition)
        .bind(&result.epistemic_outcome)
        .bind(&result.control_binding_kind)
        .bind(&result.control_validity)
        .bind(result.prepared_action_id)
        .bind(result.capability_execution_receipt_id)
        .bind(result.oracle_assessment_id)
        .bind(result.residual_id)
        .bind(result_hash)
        .execute(&mut *tx)
        .await?;
    }
    let fact_delta_hash = json_hash_on(&mut tx, &command.typed_fact_delta).await?;
    sqlx::query(
        r#"INSERT INTO verification_fact_delta_bundles(
               fact_delta_bundle_id,stable_request_id,campaign_id,campaign_terminal_decision_id,
               operation_id,project_scope_id,organization_id,hypothesis_revision_id,
               verification_objective_id,delta_kind,typed_delta,evidence_ref_set_hash,
               source_authority_hash,fact_delta_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(fact_delta_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"fact-delta-request.v1",
    ))
    .bind(command.campaign_id)
    .bind(terminal_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.verification_objective_id)
    .bind(&command.fact_delta_kind)
    .bind(&command.typed_fact_delta)
    .bind(&command.evidence_ref_set_hash)
    .bind(&command.source_authority_hash)
    .bind(&fact_delta_hash)
    .execute(&mut *tx)
    .await?;
    let objective_outcome_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "plan_id": command.verification_plan_id,
            "objective_id": command.verification_objective_id,
            "outcome_ordinal": command.expected_outcome_ordinal,
            "predecessor": command.expected_predecessor_outcome_id,
            "outcome": command.outcome,
            "terminal_hash": terminal_hash,
            "coverage_receipt_hash": coverage_receipt_hash,
            "claim_component_outcome_seal_hash": claim_seal_hash,
            "fact_delta_hash": fact_delta_hash,
            "source_authority_hash": command.source_authority_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_objective_outcome_receipts(
               objective_outcome_receipt_id,stable_request_id,verification_plan_id,
               hypothesis_revision_id,verification_objective_id,operation_id,project_scope_id,
               organization_id,outcome_ordinal,predecessor_outcome_id,outcome,
               campaign_terminal_decision_id,campaign_adjudication_id,
               campaign_coverage_receipt_id,oracle_census_seal_id,
               claim_component_outcome_seal_id,claim_component_outcome_seal_hash,
               fact_delta_bundle_id,residual_id,source_authority_hash,outcome_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)"#,
    )
    .bind(objective_outcome_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"objective-outcome-request.v1",
    ))
    .bind(command.verification_plan_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.verification_objective_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.expected_outcome_ordinal)
    .bind(command.expected_predecessor_outcome_id)
    .bind(&command.outcome)
    .bind(terminal_id)
    .bind(adjudication_id)
    .bind(coverage_receipt_id)
    .bind(command.oracle_census_seal_id)
    .bind(command.claim_component_outcome_seal_id)
    .bind(&claim_seal_hash)
    .bind(fact_delta_id)
    .bind(command.residual_id)
    .bind(&command.source_authority_hash)
    .bind(&objective_outcome_hash)
    .execute(&mut *tx)
    .await?;
    if command.expected_outcome_ordinal == 1 {
        sqlx::query(
            r#"INSERT INTO hypothesis_objective_outcome_heads(
                   verification_plan_id,verification_objective_id,current_outcome_id,
                   current_ordinal,row_version
               ) VALUES($1,$2,$3,1,0)"#,
        )
        .bind(command.verification_plan_id)
        .bind(command.verification_objective_id)
        .bind(objective_outcome_id)
        .execute(&mut *tx)
        .await?;
    } else {
        let affected = sqlx::query(
            r#"UPDATE hypothesis_objective_outcome_heads
                  SET current_outcome_id=$1,current_ordinal=$2,row_version=row_version+1
                WHERE verification_plan_id=$3 AND verification_objective_id=$4
                  AND current_outcome_id=$5 AND current_ordinal=$2-1"#,
        )
        .bind(objective_outcome_id)
        .bind(command.expected_outcome_ordinal)
        .bind(command.verification_plan_id)
        .bind(command.verification_objective_id)
        .bind(command.expected_predecessor_outcome_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if affected != 1 {
            return Err(conflict(AUTHORITY_STALE));
        }
    }
    let campaign_closed = sqlx::query(
        r#"UPDATE verification_campaigns
              SET state='terminal',terminal_at=statement_timestamp(),row_version=row_version+1
            WHERE campaign_id=$1 AND row_version=$2 AND state IN ('running','draining')"#,
    )
    .bind(command.campaign_id)
    .bind(command.expected_campaign_row_version)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if campaign_closed != 1 {
        return Err(conflict(AUTHORITY_STALE));
    }
    tx.commit().await?;
    Ok(CampaignObjectiveCloseout {
        campaign_terminal_decision_id: terminal_id,
        campaign_coverage_receipt_id: coverage_receipt_id,
        fact_delta_bundle_id: fact_delta_id,
        objective_outcome_receipt_id: objective_outcome_id,
    })
}
