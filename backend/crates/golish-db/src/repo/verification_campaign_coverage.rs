//! Wave and Campaign coverage denominators frozen before execution.

use sqlx::PgPool;
use uuid::Uuid;

use super::verification_campaigns::{
    conflict, exact_set_hash_on, json_hash_on, require_sha256, CONTRACT_INVALID,
};
use crate::Result;

#[derive(Debug, Clone)]
pub struct WaveCoverageMember {
    pub semantic_key: String,
    pub input_ref_kind: String,
    pub input_ref_id: Uuid,
    pub input_identity_hash: String,
    pub hypothesis_revision_id: Uuid,
    pub claim_component_id: Uuid,
    pub claim_component_hash: String,
    pub verification_objective_id: Uuid,
    pub predicate_component_id: Uuid,
    pub control_binding_kind: String,
    pub required_control_id: Option<Uuid>,
    pub required_control_hash: Option<String>,
    pub no_control_marker_hash: Option<String>,
    pub capability_assessment_id: Uuid,
    pub expected_capability_kind: String,
    pub expected_action_kind: String,
    pub expected_oracle_kind: String,
}

#[derive(Debug, Clone)]
pub struct SealWaveCoverageDenominator {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub generation_seal_id: Uuid,
    pub contract_version: String,
    pub source_snapshot_hash: String,
    pub members: Vec<WaveCoverageMember>,
}

pub async fn seal_wave_coverage_denominator(
    pool: &PgPool,
    command: &SealWaveCoverageDenominator,
) -> Result<Uuid> {
    require_sha256(&command.source_snapshot_hash)?;
    if command.members.is_empty()
        || command.contract_version.trim().is_empty()
        || command
            .members
            .iter()
            .any(|member| !valid_wave_member(member))
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let denominator_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-wave-coverage-denominator.v1",
    );
    let mut member_rows = Vec::with_capacity(command.members.len());
    for (ordinal, member) in command.members.iter().enumerate() {
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "semantic_key": member.semantic_key,
                "input_ref_kind": member.input_ref_kind,
                "input_ref_id": member.input_ref_id,
                "input_identity_hash": member.input_identity_hash,
                "hypothesis_revision_id": member.hypothesis_revision_id,
                "claim_component_id": member.claim_component_id,
                "claim_component_hash": member.claim_component_hash,
                "verification_objective_id": member.verification_objective_id,
                "predicate_component_id": member.predicate_component_id,
                "control_binding_kind": member.control_binding_kind,
                "required_control_id": member.required_control_id,
                "required_control_hash": member.required_control_hash,
                "no_control_marker_hash": member.no_control_marker_hash,
                "capability_assessment_id": member.capability_assessment_id,
                "expected_capability_kind": member.expected_capability_kind,
                "expected_action_kind": member.expected_action_kind,
                "expected_oracle_kind": member.expected_oracle_kind,
            }),
        )
        .await?;
        member_rows.push((member, member_hash));
    }
    let hashes = member_rows
        .iter()
        .map(|row| row.1.clone())
        .collect::<Vec<_>>();
    let member_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_wave_coverage_denominator.v1",
        &hashes,
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO verification_wave_coverage_denominators(
               wave_denominator_id,stable_request_id,operation_id,project_scope_id,
               organization_id,generation_seal_id,contract_version,source_snapshot_hash,
               member_set_hash,member_count,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,NULL)"#,
    )
    .bind(denominator_id)
    .bind(command.stable_request_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.generation_seal_id)
    .bind(&command.contract_version)
    .bind(&command.source_snapshot_hash)
    .bind(&member_set_hash)
    .bind(member_rows.len() as i64)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (member, member_hash)) in member_rows.iter().enumerate() {
        let member_id = Uuid::new_v5(&denominator_id, member.semantic_key.as_bytes());
        sqlx::query(
            r#"INSERT INTO verification_wave_coverage_members(
                   wave_coverage_member_id,wave_denominator_id,operation_id,project_scope_id,
                   organization_id,member_ordinal,semantic_key,input_ref_kind,input_ref_id,
                   input_identity_hash,hypothesis_revision_id,claim_component_id,
                   claim_component_hash,verification_objective_id,predicate_component_id,
                   control_binding_kind,required_control_id,required_control_hash,
                   no_control_marker_hash,capability_assessment_id,expected_capability_kind,
                   expected_action_kind,expected_oracle_kind,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                        $17,$18,$19,$20,$21,$22,$23,$24)"#,
        )
        .bind(member_id)
        .bind(denominator_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(ordinal as i32)
        .bind(&member.semantic_key)
        .bind(&member.input_ref_kind)
        .bind(member.input_ref_id)
        .bind(&member.input_identity_hash)
        .bind(member.hypothesis_revision_id)
        .bind(member.claim_component_id)
        .bind(&member.claim_component_hash)
        .bind(member.verification_objective_id)
        .bind(member.predicate_component_id)
        .bind(&member.control_binding_kind)
        .bind(member.required_control_id)
        .bind(&member.required_control_hash)
        .bind(&member.no_control_marker_hash)
        .bind(member.capability_assessment_id)
        .bind(&member.expected_capability_kind)
        .bind(&member.expected_action_kind)
        .bind(&member.expected_oracle_kind)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE verification_wave_coverage_denominators SET sealed_at=statement_timestamp() WHERE wave_denominator_id=$1 AND sealed_at IS NULL",
    )
    .bind(denominator_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(denominator_id)
}

fn valid_wave_member(member: &WaveCoverageMember) -> bool {
    let control_shape = match member.control_binding_kind.as_str() {
        "required" => {
            member.required_control_id.is_some()
                && member.required_control_hash.is_some()
                && member.no_control_marker_hash.is_none()
        }
        "explicit_no_control" => {
            member.required_control_id.is_none()
                && member.required_control_hash.is_none()
                && member.no_control_marker_hash.is_some()
        }
        _ => false,
    };
    control_shape
        && !member.semantic_key.trim().is_empty()
        && !member.expected_capability_kind.trim().is_empty()
        && !member.expected_action_kind.trim().is_empty()
        && !member.expected_oracle_kind.trim().is_empty()
        && require_sha256(&member.input_identity_hash).is_ok()
        && require_sha256(&member.claim_component_hash).is_ok()
        && member
            .required_control_hash
            .as_deref()
            .is_none_or(|hash| require_sha256(hash).is_ok())
        && member
            .no_control_marker_hash
            .as_deref()
            .is_none_or(|hash| require_sha256(hash).is_ok())
}

#[derive(Debug, Clone)]
pub struct CampaignCoverageMember {
    pub wave_coverage_member_id: Uuid,
    pub semantic_key: String,
    pub claim_component_id: Uuid,
    pub claim_component_hash: String,
    pub obligation_kind: String,
    pub control_binding_kind: String,
    pub capability_assessment_id: Uuid,
    pub expected_capability_kind: String,
    pub expected_action_kind: String,
    pub expected_oracle_kind: String,
}

#[derive(Debug, Clone)]
pub struct SealCampaignCoverageDenominator {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub campaign_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub wave_denominator_id: Uuid,
    pub contract_version: String,
    pub source_snapshot_hash: String,
    pub members: Vec<CampaignCoverageMember>,
}

pub async fn seal_campaign_coverage_denominator(
    pool: &PgPool,
    command: &SealCampaignCoverageDenominator,
) -> Result<Uuid> {
    if command.members.is_empty() {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let denominator_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-campaign-coverage-denominator.v1",
    );
    let mut rows = Vec::with_capacity(command.members.len());
    for (ordinal, member) in command.members.iter().enumerate() {
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "wave_coverage_member_id": member.wave_coverage_member_id,
                "semantic_key": member.semantic_key,
                "claim_component_id": member.claim_component_id,
                "claim_component_hash": member.claim_component_hash,
                "obligation_kind": member.obligation_kind,
                "control_binding_kind": member.control_binding_kind,
                "capability_assessment_id": member.capability_assessment_id,
                "expected_capability_kind": member.expected_capability_kind,
                "expected_action_kind": member.expected_action_kind,
                "expected_oracle_kind": member.expected_oracle_kind,
            }),
        )
        .await?;
        rows.push((member, member_hash));
    }
    let member_hashes = rows.iter().map(|row| row.1.clone()).collect::<Vec<_>>();
    let member_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_campaign_coverage_denominator.v1",
        &member_hashes,
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO verification_campaign_coverage_denominators(
               campaign_denominator_id,stable_request_id,operation_id,project_scope_id,
               organization_id,campaign_id,hypothesis_revision_id,wave_denominator_id,
               contract_version,source_snapshot_hash,member_set_hash,member_count,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,NULL)"#,
    )
    .bind(denominator_id)
    .bind(command.stable_request_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.campaign_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.wave_denominator_id)
    .bind(&command.contract_version)
    .bind(&command.source_snapshot_hash)
    .bind(&member_set_hash)
    .bind(rows.len() as i64)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (member, member_hash)) in rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO verification_campaign_coverage_members(
                   campaign_coverage_member_id,campaign_denominator_id,wave_coverage_member_id,
                   wave_denominator_id,operation_id,project_scope_id,organization_id,
                   member_ordinal,semantic_key,claim_component_id,claim_component_hash,
                   obligation_kind,control_binding_kind,capability_assessment_id,
                   expected_capability_kind,expected_action_kind,expected_oracle_kind,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
        )
        .bind(Uuid::new_v5(
            &denominator_id,
            member.semantic_key.as_bytes(),
        ))
        .bind(denominator_id)
        .bind(member.wave_coverage_member_id)
        .bind(command.wave_denominator_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(ordinal as i32)
        .bind(&member.semantic_key)
        .bind(member.claim_component_id)
        .bind(&member.claim_component_hash)
        .bind(&member.obligation_kind)
        .bind(&member.control_binding_kind)
        .bind(member.capability_assessment_id)
        .bind(&member.expected_capability_kind)
        .bind(&member.expected_action_kind)
        .bind(&member.expected_oracle_kind)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE verification_campaign_coverage_denominators SET sealed_at=statement_timestamp() WHERE campaign_denominator_id=$1 AND sealed_at IS NULL",
    )
    .bind(denominator_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(denominator_id)
}
