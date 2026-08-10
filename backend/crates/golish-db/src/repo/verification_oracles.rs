//! Deterministic action-oracle assessments and objective-local exact census.

use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::verification_campaigns::{
    conflict, exact_set_hash_on, json_hash_on, require_sha256, CONTRACT_INVALID,
};
use crate::Result;

#[derive(Debug, Clone)]
pub struct RecordActionOracle {
    pub stable_request_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub action_execution_id: Uuid,
    pub campaign_coverage_member_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub oracle_revision_ordinal: i32,
    pub oracle_contract_version: String,
    pub oracle_contract_hash: String,
    pub observation_receipt_hash: String,
    pub precondition_validity: String,
    pub control_validity: String,
    pub verdict: String,
    pub assessment_body: Value,
    pub residual_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct ExistingActionOracle {
    oracle_assessment_id: Uuid,
    campaign_id: Uuid,
    prepared_action_id: Uuid,
    action_execution_id: Uuid,
    campaign_coverage_member_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    oracle_revision_ordinal: i32,
    oracle_contract_version: String,
    oracle_contract_hash: String,
    observation_receipt_hash: String,
    precondition_validity: String,
    control_validity: String,
    verdict: String,
    assessment_hash: String,
    residual_id: Option<Uuid>,
}

pub async fn record_action_oracle(pool: &PgPool, command: &RecordActionOracle) -> Result<Uuid> {
    let mut tx = pool.begin().await?;
    let result = record_action_oracle_in_transaction(&mut tx, command).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn record_action_oracle_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    command: &RecordActionOracle,
) -> Result<Uuid> {
    if !matches!(
        command.verdict.as_str(),
        "proof" | "refutation" | "inconclusive"
    ) || !matches!(
        command.precondition_validity.as_str(),
        "valid" | "invalid" | "unknown"
    ) || !matches!(
        command.control_validity.as_str(),
        "valid" | "invalid" | "not_assessed" | "not_required"
    ) || (command.verdict == "inconclusive") != command.residual_id.is_some()
        || command.oracle_revision_ordinal <= 0
        || command.oracle_contract_version.trim().is_empty()
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    require_sha256(&command.oracle_contract_hash)?;
    require_sha256(&command.observation_receipt_hash)?;
    let assessment_hash = json_hash_on(tx, &command.assessment_body).await?;
    let assessment_id = Uuid::new_v5(&command.stable_request_id, b"verification-action-oracle.v1");
    let existing = sqlx::query_as::<_, ExistingActionOracle>(
        r#"SELECT oracle_assessment_id,campaign_id,prepared_action_id,
                  action_execution_id,campaign_coverage_member_id,operation_id,
                  project_scope_id,organization_id,oracle_revision_ordinal,
                  oracle_contract_version,oracle_contract_hash,
                  observation_receipt_hash,precondition_validity,control_validity,
                  verdict,assessment_hash,residual_id
             FROM verification_oracle_assessments
            WHERE stable_request_id=$1 FOR SHARE"#,
    )
    .bind(command.stable_request_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = existing {
        if row.campaign_id == command.campaign_id
            && row.prepared_action_id == command.prepared_action_id
            && row.action_execution_id == command.action_execution_id
            && row.campaign_coverage_member_id == command.campaign_coverage_member_id
            && row.operation_id == command.operation_id
            && row.project_scope_id == command.project_scope_id
            && row.organization_id == command.organization_id
            && row.oracle_revision_ordinal == command.oracle_revision_ordinal
            && row.oracle_contract_version == command.oracle_contract_version
            && row.oracle_contract_hash == command.oracle_contract_hash
            && row.observation_receipt_hash == command.observation_receipt_hash
            && row.precondition_validity == command.precondition_validity
            && row.control_validity == command.control_validity
            && row.verdict == command.verdict
            && row.assessment_hash == assessment_hash
            && row.residual_id == command.residual_id
        {
            return Ok(row.oracle_assessment_id);
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    sqlx::query(
        r#"INSERT INTO verification_oracle_assessments(
               oracle_assessment_id,stable_request_id,campaign_id,prepared_action_id,
               action_execution_id,campaign_coverage_member_id,operation_id,project_scope_id,
               organization_id,oracle_revision_ordinal,oracle_contract_version,
               oracle_contract_hash,observation_receipt_hash,precondition_validity,
               control_validity,verdict,assessment_body,assessment_hash,residual_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)"#,
    )
    .bind(assessment_id)
    .bind(command.stable_request_id)
    .bind(command.campaign_id)
    .bind(command.prepared_action_id)
    .bind(command.action_execution_id)
    .bind(command.campaign_coverage_member_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.oracle_revision_ordinal)
    .bind(&command.oracle_contract_version)
    .bind(&command.oracle_contract_hash)
    .bind(&command.observation_receipt_hash)
    .bind(&command.precondition_validity)
    .bind(&command.control_validity)
    .bind(&command.verdict)
    .bind(&command.assessment_body)
    .bind(&assessment_hash)
    .bind(command.residual_id)
    .execute(&mut **tx)
    .await?;
    Ok(assessment_id)
}

#[derive(Debug, Clone)]
pub struct OracleCensusMember {
    pub campaign_coverage_member_id: Uuid,
    pub predicate_component_id: Uuid,
    pub control_binding_kind: String,
    pub required_control_id: Option<Uuid>,
    pub required_control_hash: Option<String>,
    pub no_control_marker_hash: Option<String>,
    pub disposition: String,
    pub oracle_assessment_id: Option<Uuid>,
    pub residual_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct SealOracleCensus {
    pub stable_request_id: Uuid,
    pub campaign_id: Uuid,
    pub campaign_denominator_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub verification_contract_hash: String,
    pub denominator_hash: String,
    pub result_set_hash: String,
    pub members: Vec<OracleCensusMember>,
}

pub async fn seal_oracle_census(pool: &PgPool, command: &SealOracleCensus) -> Result<Uuid> {
    if command.members.is_empty() {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let seal_id = Uuid::new_v5(&command.stable_request_id, b"verification-oracle-census.v1");
    let mut rows = Vec::with_capacity(command.members.len());
    for (ordinal, member) in command.members.iter().enumerate() {
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "campaign_coverage_member_id": member.campaign_coverage_member_id,
                "predicate_component_id": member.predicate_component_id,
                "control_binding_kind": member.control_binding_kind,
                "required_control_id": member.required_control_id,
                "required_control_hash": member.required_control_hash,
                "no_control_marker_hash": member.no_control_marker_hash,
                "disposition": member.disposition,
                "oracle_assessment_id": member.oracle_assessment_id,
                "residual_id": member.residual_id,
            }),
        )
        .await?;
        rows.push((member, member_hash));
    }
    let hashes = rows.iter().map(|row| row.1.clone()).collect::<Vec<_>>();
    let member_set_hash =
        exact_set_hash_on(&mut tx, "verification_oracle_census.v1", &hashes).await?;
    let census_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "campaign_id": command.campaign_id,
            "verification_contract_hash": command.verification_contract_hash,
            "denominator_hash": command.denominator_hash,
            "result_set_hash": command.result_set_hash,
            "member_set_hash": member_set_hash,
        }),
    )
    .await?;
    let existing: Option<(Uuid, String, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        r#"SELECT oracle_census_seal_id,census_hash,sealed_at
                 FROM verification_oracle_census_seals WHERE stable_request_id=$1 FOR SHARE"#,
    )
    .bind(command.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((existing_id, existing_hash, sealed_at)) = existing {
        if existing_hash == census_hash && sealed_at.is_some() {
            tx.commit().await?;
            return Ok(existing_id);
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    sqlx::query(
        r#"INSERT INTO verification_oracle_census_seals(
               oracle_census_seal_id,stable_request_id,campaign_id,campaign_denominator_id,
               operation_id,project_scope_id,organization_id,verification_contract_hash,
               denominator_hash,result_set_hash,member_count,member_set_hash,census_hash,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,NULL)"#,
    )
    .bind(seal_id)
    .bind(command.stable_request_id)
    .bind(command.campaign_id)
    .bind(command.campaign_denominator_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(&command.verification_contract_hash)
    .bind(&command.denominator_hash)
    .bind(&command.result_set_hash)
    .bind(rows.len() as i64)
    .bind(&member_set_hash)
    .bind(&census_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (member, member_hash)) in rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO verification_oracle_census_members(
                   oracle_census_member_id,oracle_census_seal_id,campaign_id,operation_id,
                   project_scope_id,organization_id,member_ordinal,campaign_coverage_member_id,
                   predicate_component_id,control_binding_kind,required_control_id,
                   required_control_hash,no_control_marker_hash,disposition,
                   oracle_assessment_id,residual_id,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
        )
        .bind(Uuid::new_v5(&seal_id, member_hash.as_bytes()))
        .bind(seal_id)
        .bind(command.campaign_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(ordinal as i32)
        .bind(member.campaign_coverage_member_id)
        .bind(member.predicate_component_id)
        .bind(&member.control_binding_kind)
        .bind(member.required_control_id)
        .bind(&member.required_control_hash)
        .bind(&member.no_control_marker_hash)
        .bind(&member.disposition)
        .bind(member.oracle_assessment_id)
        .bind(member.residual_id)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE verification_oracle_census_seals SET sealed_at=statement_timestamp() WHERE oracle_census_seal_id=$1 AND sealed_at IS NULL",
    )
    .bind(seal_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(seal_id)
}
