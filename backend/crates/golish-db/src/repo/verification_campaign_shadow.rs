//! Port-isolated Campaign shadow evaluation; no executable authority is stored.

use sqlx::PgPool;
use uuid::Uuid;

use super::verification_campaigns::{conflict, exact_set_hash_on, json_hash_on, CONTRACT_INVALID};
use crate::Result;

#[derive(Debug, Clone)]
pub struct ShadowEvaluationItem {
    pub plan_objective_id: Uuid,
    pub compiled_semantic_signature_hash: String,
    pub legacy_capability_execution_receipt_id: Uuid,
    pub deterministic_oracle_replay_ref: Uuid,
    pub comparison_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct RecordShadowEvaluation {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_plan_id: Uuid,
    pub source_snapshot_hash: String,
    pub items: Vec<ShadowEvaluationItem>,
}

pub async fn record_shadow_evaluation(
    pool: &PgPool,
    command: &RecordShadowEvaluation,
) -> Result<Uuid> {
    if command.items.is_empty() {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let evaluation_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-campaign-shadow-evaluation.v1",
    );
    let mut rows = Vec::with_capacity(command.items.len());
    for (ordinal, item) in command.items.iter().enumerate() {
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "plan_objective_id": item.plan_objective_id,
                "compiled_semantic_signature_hash": item.compiled_semantic_signature_hash,
                "legacy_capability_execution_receipt_id": item.legacy_capability_execution_receipt_id,
                "deterministic_oracle_replay_ref": item.deterministic_oracle_replay_ref,
                "comparison_id": item.comparison_id,
            }),
        )
        .await?;
        rows.push((item, member_hash));
    }
    let hashes = rows.iter().map(|row| row.1.clone()).collect::<Vec<_>>();
    let member_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_campaign_shadow_evaluation.v1",
        &hashes,
    )
    .await?;
    let evaluation_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "operation_id": command.operation_id,
            "hypothesis_revision_id": command.hypothesis_revision_id,
            "verification_plan_id": command.verification_plan_id,
            "source_snapshot_hash": command.source_snapshot_hash,
            "obligation_member_set_hash": member_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO verification_campaign_shadow_evaluations(
               shadow_evaluation_id,stable_request_id,operation_id,project_scope_id,
               organization_id,hypothesis_revision_id,verification_plan_id,
               frozen_snapshot_id,frozen_snapshot_hash,obligation_census_hash,as_of_change_seq,
               source_snapshot_hash,obligation_member_count,obligation_member_set_hash,
               evaluation_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,0,$10,$11,$12,$13)"#,
    )
    .bind(evaluation_id)
    .bind(command.stable_request_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.verification_plan_id)
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"legacy-shadow-frozen-snapshot.v1",
    ))
    .bind(&command.source_snapshot_hash)
    .bind(&command.source_snapshot_hash)
    .bind(rows.len() as i64)
    .bind(&member_set_hash)
    .bind(&evaluation_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (item, member_hash)) in rows.iter().enumerate() {
        let obligation_id =
            Uuid::new_v5(&evaluation_id, format!("obligation:{ordinal}").as_bytes());
        sqlx::query(
            r#"INSERT INTO verification_campaign_shadow_evaluation_obligations(
                   shadow_evaluation_obligation_id,shadow_evaluation_id,operation_id,
                   project_scope_id,organization_id,obligation_ordinal,plan_objective_id,
                   plan_objective_member_hash,frozen_target_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(obligation_id)
        .bind(evaluation_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(ordinal as i32)
        .bind(item.plan_objective_id)
        .bind(member_hash)
        .bind(&command.source_snapshot_hash)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO verification_campaign_shadow_evaluation_items(
                   shadow_evaluation_item_id,shadow_evaluation_id,operation_id,
                   project_scope_id,organization_id,item_ordinal,
                   shadow_evaluation_obligation_id,plan_objective_id,
                   compiled_semantic_signature_hash,legacy_capability_execution_receipt_id,
                   deterministic_oracle_replay_ref,comparison_id,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
        )
        .bind(Uuid::new_v5(&evaluation_id, member_hash.as_bytes()))
        .bind(evaluation_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(ordinal as i32)
        .bind(obligation_id)
        .bind(item.plan_objective_id)
        .bind(&item.compiled_semantic_signature_hash)
        .bind(item.legacy_capability_execution_receipt_id)
        .bind(item.deterministic_oracle_replay_ref)
        .bind(item.comparison_id)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    let comparison_ids = rows
        .iter()
        .map(|(item, _)| item.comparison_id.to_string())
        .collect::<Vec<_>>();
    let comparison_id_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_campaign_shadow_comparisons.v1",
        &comparison_ids,
    )
    .await?;
    let receipt_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "shadow_evaluation_id": evaluation_id,
            "comparison_id_set_hash": comparison_id_set_hash,
            "evaluation_hash": evaluation_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"UPDATE verification_campaign_shadow_evaluations
              SET state='closed',comparison_count=$1,comparison_id_set_hash=$2,
                  receipt_hash=$3,row_version=row_version+1,
                  closed_at=statement_timestamp()
            WHERE shadow_evaluation_id=$4 AND state='open'"#,
    )
    .bind(rows.len() as i64)
    .bind(comparison_id_set_hash)
    .bind(receipt_hash)
    .bind(evaluation_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(evaluation_id)
}
