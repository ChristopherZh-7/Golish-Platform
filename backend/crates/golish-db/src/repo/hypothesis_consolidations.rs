//! FactDelta consumption, semantic quarantine and explicit correction lineage.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::verification_campaigns::{
    conflict, exact_set_hash_on, json_hash_on, require_sha256, AUTHORITY_STALE, CONTRACT_INVALID,
};
use crate::Result;

#[derive(Debug, Clone)]
pub struct RecordFactDeltaConsumption {
    pub stable_request_id: Uuid,
    pub fact_delta_bundle_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub generation_id: Uuid,
    pub disposition: String,
    pub residual_id: Option<Uuid>,
}

pub async fn record_fact_delta_consumption(
    pool: &PgPool,
    command: &RecordFactDeltaConsumption,
) -> Result<Uuid> {
    if !matches!(
        command.disposition.as_str(),
        "applied" | "no_semantic_change" | "quarantined_invalid_authority"
    ) || ((command.disposition == "quarantined_invalid_authority")
        != command.residual_id.is_some())
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let consumption_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "fact_delta_bundle_id": command.fact_delta_bundle_id,
            "generation_id": command.generation_id,
            "disposition": command.disposition,
            "residual_id": command.residual_id,
        }),
    )
    .await?;
    let consumption_id = Uuid::new_v5(&command.stable_request_id, b"fact-delta-consumption.v1");
    sqlx::query(
        r#"INSERT INTO fact_delta_consumptions(
               fact_delta_consumption_id,stable_request_id,fact_delta_bundle_id,
               operation_id,project_scope_id,organization_id,generation_id,
               disposition,consumption_hash,residual_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
    )
    .bind(consumption_id)
    .bind(command.stable_request_id)
    .bind(command.fact_delta_bundle_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.generation_id)
    .bind(&command.disposition)
    .bind(consumption_hash)
    .bind(command.residual_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(consumption_id)
}

#[derive(Debug, Clone)]
pub struct QuarantineAuthorityMember {
    pub authority_ref_kind: String,
    pub authority_ref_id: Uuid,
    pub authority_ref_hash: String,
}

#[derive(Debug, Clone)]
pub struct QuarantineCampaignAuthority {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub campaign_terminal_decision_id: Uuid,
    pub objective_outcome_receipt_id: Uuid,
    pub campaign_coverage_receipt_id: Uuid,
    pub oracle_census_seal_id: Uuid,
    pub fact_delta_bundle_id: Uuid,
    pub invalid_semantic_reconciliation_id: Uuid,
    pub invalid_semantic_reconciliation_hash: String,
    pub residual_reason_code: String,
    pub members: Vec<QuarantineAuthorityMember>,
    pub typed_correction_delta: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineReceipt {
    pub quarantine_event_id: Uuid,
    pub invalidated_objective_outcome_id: Uuid,
    pub correction_bundle_id: Option<Uuid>,
    pub re_adjudication_obligation_id: Uuid,
}

pub async fn quarantine_campaign_authority(
    pool: &PgPool,
    command: &QuarantineCampaignAuthority,
) -> Result<QuarantineReceipt> {
    if command.members.is_empty() || command.residual_reason_code.trim().is_empty() {
        return Err(conflict(CONTRACT_INVALID));
    }
    require_sha256(&command.invalid_semantic_reconciliation_hash)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let reconciliation_invalid: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
              SELECT 1 FROM capability_execution_reconciliations reconciliation
               WHERE reconciliation.id=$1
                 AND reconciliation.semantic_reconciliation_hash=$2
                 AND reconciliation.sealed_at IS NOT NULL
                 AND reconciliation.reconciliation_state IN ('orphaned','superseded')
           )"#,
    )
    .bind(command.invalid_semantic_reconciliation_id)
    .bind(&command.invalid_semantic_reconciliation_hash)
    .fetch_one(&mut *tx)
    .await?;
    if !reconciliation_invalid {
        return Err(conflict(AUTHORITY_STALE));
    }
    #[derive(sqlx::FromRow)]
    struct OutcomeRow {
        verification_plan_id: Uuid,
        hypothesis_revision_id: Uuid,
        verification_objective_id: Uuid,
        outcome_ordinal: i64,
        claim_component_outcome_seal_id: Uuid,
        claim_component_outcome_seal_hash: String,
        source_authority_hash: String,
    }
    let outcome = sqlx::query_as::<_, OutcomeRow>(
        r#"SELECT verification_plan_id,hypothesis_revision_id,verification_objective_id,
                  outcome_ordinal,claim_component_outcome_seal_id,
                  claim_component_outcome_seal_hash,source_authority_hash
             FROM hypothesis_objective_outcome_receipts
            WHERE objective_outcome_receipt_id=$1 AND operation_id=$2
              AND project_scope_id=$3 AND organization_id=$4
              AND campaign_terminal_decision_id=$5 FOR UPDATE"#,
    )
    .bind(command.objective_outcome_receipt_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.campaign_terminal_decision_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let residual_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-authority-quarantine-residual.v1",
    );
    let residual_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "reason_code": command.residual_reason_code,
            "objective_outcome_receipt_id": command.objective_outcome_receipt_id,
            "invalid_semantic_reconciliation_id": command.invalid_semantic_reconciliation_id,
            "next_action": "re_adjudicate_hypothesis_revision",
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_residual_risks(
               residual_id,operation_id,organization_id,revision_id,reason_code,
               owner_kind,affected_inputs,next_action,residual_hash
           ) VALUES($1,$2,$3,$4,$5,'plan_c',$6,$7,$8)"#,
    )
    .bind(residual_id)
    .bind(command.operation_id)
    .bind(command.organization_id)
    .bind(outcome.hypothesis_revision_id)
    .bind(&command.residual_reason_code)
    .bind(serde_json::json!([{
        "kind": "objective_outcome",
        "id": command.objective_outcome_receipt_id,
    }, {
        "kind": "semantic_reconciliation",
        "id": command.invalid_semantic_reconciliation_id,
    }]))
    .bind(serde_json::json!({
        "kind": "re_adjudicate_hypothesis_revision",
        "revision_id": outcome.hypothesis_revision_id,
    }))
    .bind(residual_hash)
    .execute(&mut *tx)
    .await?;
    let mut member_rows = Vec::with_capacity(command.members.len());
    for (ordinal, member) in command.members.iter().enumerate() {
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "authority_ref_kind": member.authority_ref_kind,
                "authority_ref_id": member.authority_ref_id,
                "authority_ref_hash": member.authority_ref_hash,
            }),
        )
        .await?;
        member_rows.push((member, member_hash));
    }
    let member_hashes = member_rows
        .iter()
        .map(|row| row.1.clone())
        .collect::<Vec<_>>();
    let member_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_authority_quarantine.v1",
        &member_hashes,
    )
    .await?;
    let quarantine_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "campaign_terminal_decision_id": command.campaign_terminal_decision_id,
            "objective_outcome_receipt_id": command.objective_outcome_receipt_id,
            "invalid_semantic_reconciliation_id": command.invalid_semantic_reconciliation_id,
            "invalid_semantic_reconciliation_hash": command.invalid_semantic_reconciliation_hash,
            "member_set_hash": member_set_hash,
        }),
    )
    .await?;
    let quarantine_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-authority-quarantine.v1",
    );
    sqlx::query(
        r#"INSERT INTO verification_authority_quarantine_events(
               quarantine_event_id,stable_request_id,operation_id,project_scope_id,
               organization_id,campaign_terminal_decision_id,objective_outcome_receipt_id,
               campaign_coverage_receipt_id,oracle_census_seal_id,fact_delta_bundle_id,
               invalid_semantic_reconciliation_id,invalid_semantic_reconciliation_hash,
               member_count,member_set_hash,quarantine_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)"#,
    )
    .bind(quarantine_id)
    .bind(command.stable_request_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.campaign_terminal_decision_id)
    .bind(command.objective_outcome_receipt_id)
    .bind(command.campaign_coverage_receipt_id)
    .bind(command.oracle_census_seal_id)
    .bind(command.fact_delta_bundle_id)
    .bind(command.invalid_semantic_reconciliation_id)
    .bind(&command.invalid_semantic_reconciliation_hash)
    .bind(member_rows.len() as i64)
    .bind(&member_set_hash)
    .bind(&quarantine_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (member, member_hash)) in member_rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO verification_authority_quarantine_members(
                   quarantine_event_id,member_ordinal,authority_ref_kind,
                   authority_ref_id,authority_ref_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(quarantine_id)
        .bind(ordinal as i32)
        .bind(&member.authority_ref_kind)
        .bind(member.authority_ref_id)
        .bind(&member.authority_ref_hash)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    let invalidated_outcome_id = Uuid::new_v5(
        &command.stable_request_id,
        b"invalidated-objective-outcome.v1",
    );
    let invalidated_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "predecessor_outcome_id": command.objective_outcome_receipt_id,
            "outcome": "invalidated",
            "quarantine_hash": quarantine_hash,
            "residual_id": residual_id,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_objective_outcome_receipts(
               objective_outcome_receipt_id,stable_request_id,verification_plan_id,
               hypothesis_revision_id,verification_objective_id,operation_id,project_scope_id,
               organization_id,outcome_ordinal,predecessor_outcome_id,outcome,
               claim_component_outcome_seal_id,claim_component_outcome_seal_hash,
               residual_id,source_authority_hash,outcome_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'invalidated',$11,$12,$13,$14,$15)"#,
    )
    .bind(invalidated_outcome_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"invalidated-outcome-request.v1",
    ))
    .bind(outcome.verification_plan_id)
    .bind(outcome.hypothesis_revision_id)
    .bind(outcome.verification_objective_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(outcome.outcome_ordinal + 1)
    .bind(command.objective_outcome_receipt_id)
    .bind(outcome.claim_component_outcome_seal_id)
    .bind(&outcome.claim_component_outcome_seal_hash)
    .bind(residual_id)
    .bind(&outcome.source_authority_hash)
    .bind(&invalidated_hash)
    .execute(&mut *tx)
    .await?;
    let advanced = sqlx::query(
        r#"UPDATE hypothesis_objective_outcome_heads
              SET current_outcome_id=$1,current_ordinal=current_ordinal+1,row_version=row_version+1
            WHERE verification_plan_id=$2 AND verification_objective_id=$3
              AND current_outcome_id=$4 AND current_ordinal=$5"#,
    )
    .bind(invalidated_outcome_id)
    .bind(outcome.verification_plan_id)
    .bind(outcome.verification_objective_id)
    .bind(command.objective_outcome_receipt_id)
    .bind(outcome.outcome_ordinal)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if advanced != 1 {
        return Err(conflict(AUTHORITY_STALE));
    }
    let obligation_id = Uuid::new_v5(
        &command.stable_request_id,
        b"hypothesis-re-adjudication-obligation.v1",
    );
    let obligation_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "revision_id": outcome.hypothesis_revision_id,
            "quarantine_event_id": quarantine_id,
            "reason_code": "semantic_authority_invalid",
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_re_adjudication_obligations(
               re_adjudication_obligation_id,stable_request_id,operation_id,
               project_scope_id,organization_id,hypothesis_revision_id,
               invalidated_authority_ref_kind,invalidated_authority_ref_id,
               reason_code,status,obligation_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'objective_outcome',$7,
                    'semantic_authority_invalid','open',$8)"#,
    )
    .bind(obligation_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"readjudication-request.v1",
    ))
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(outcome.hypothesis_revision_id)
    .bind(command.objective_outcome_receipt_id)
    .bind(obligation_hash)
    .execute(&mut *tx)
    .await?;
    let was_applied: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM fact_delta_consumptions WHERE fact_delta_bundle_id=$1 AND disposition='applied')",
    )
    .bind(command.fact_delta_bundle_id)
    .fetch_one(&mut *tx)
    .await?;
    let correction_bundle_id = if was_applied {
        let correction_id = Uuid::new_v5(
            &command.stable_request_id,
            b"verification-authority-correction.v1",
        );
        let correction_hash = json_hash_on(&mut tx, &command.typed_correction_delta).await?;
        sqlx::query(
            r#"INSERT INTO verification_authority_correction_bundles(
                   correction_bundle_id,stable_request_id,quarantine_event_id,operation_id,
                   project_scope_id,organization_id,superseded_fact_delta_bundle_id,
                   correction_kind,typed_correction_delta,correction_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,'retraction',$8,$9)"#,
        )
        .bind(correction_id)
        .bind(Uuid::new_v5(
            &command.stable_request_id,
            b"correction-request.v1",
        ))
        .bind(quarantine_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(command.fact_delta_bundle_id)
        .bind(&command.typed_correction_delta)
        .bind(correction_hash)
        .execute(&mut *tx)
        .await?;
        Some(correction_id)
    } else {
        None
    };
    super::report_authority_invalidation::invalidate_reports_for_source_authority_on(
        &mut tx,
        super::report_authority_invalidation::ReportInvalidationSourceV1::VerificationAuthorityQuarantine {
            quarantine_event_id: quarantine_id,
            quarantine_hash: quarantine_hash.clone(),
        },
        Uuid::new_v5(
            &command.stable_request_id,
            b"verification-quarantine-report-invalidation.v1",
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(QuarantineReceipt {
        quarantine_event_id: quarantine_id,
        invalidated_objective_outcome_id: invalidated_outcome_id,
        correction_bundle_id,
        re_adjudication_obligation_id: obligation_id,
    })
}

#[derive(Debug, Clone)]
pub struct RecordAuthorityCorrectionConsumption {
    pub stable_request_id: Uuid,
    pub correction_bundle_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub generation_id: Uuid,
    pub disposition: String,
}

pub async fn record_authority_correction_consumption(
    pool: &PgPool,
    command: &RecordAuthorityCorrectionConsumption,
) -> Result<Uuid> {
    let mut tx = pool.begin().await?;
    let consumption_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "correction_bundle_id": command.correction_bundle_id,
            "generation_id": command.generation_id,
            "disposition": command.disposition,
        }),
    )
    .await?;
    let id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-authority-correction-consumption.v1",
    );
    sqlx::query(
        r#"INSERT INTO verification_authority_correction_consumptions(
               correction_consumption_id,stable_request_id,correction_bundle_id,
               operation_id,project_scope_id,organization_id,generation_id,
               disposition,consumption_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(id)
    .bind(command.stable_request_id)
    .bind(command.correction_bundle_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.generation_id)
    .bind(&command.disposition)
    .bind(consumption_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}
