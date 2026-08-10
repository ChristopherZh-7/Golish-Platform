//! Append-only capability availability and exact latest-set seals.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::verification_campaigns::{
    conflict, exact_set_hash_on, json_hash_on, require_sha256, AUTHORITY_STALE, CONTRACT_INVALID,
    REPLAY_DRIFT,
};
use crate::Result;

#[derive(Debug, Clone)]
pub struct RecordCapabilityAssessment {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_objective_id: Uuid,
    pub verification_contract_id: Uuid,
    pub verification_contract_hash: String,
    pub capability_key: String,
    pub capability_contract_version: String,
    pub capability_contract_hash: String,
    pub policy_snapshot_id: Uuid,
    pub policy_snapshot_hash: String,
    pub assessment_ordinal: i64,
    pub supersedes_assessment_id: Option<Uuid>,
    pub status: String,
    pub reason_code: Option<String>,
    pub residual_id: Option<Uuid>,
    pub adapter_contract_version: Option<String>,
    pub adapter_contract_digest: Option<String>,
    pub source_snapshot_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct CapabilityAssessmentRow {
    pub assessment_id: Uuid,
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_objective_id: Uuid,
    pub verification_contract_hash: String,
    pub capability_key: String,
    pub assessment_ordinal: i64,
    pub status: String,
    pub reason_code: Option<String>,
    pub residual_id: Option<Uuid>,
    pub assessment_hash: String,
    pub assessed_at: DateTime<Utc>,
}

const ASSESSMENT_COLUMNS: &str = r#"assessment_id,stable_request_id,operation_id,
    project_scope_id,organization_id,hypothesis_revision_id,verification_objective_id,
    verification_contract_hash,capability_key,assessment_ordinal,status,reason_code,
    residual_id,assessment_hash,assessed_at"#;

pub async fn record_capability_assessment(
    pool: &PgPool,
    command: &RecordCapabilityAssessment,
) -> Result<CapabilityAssessmentRow> {
    for hash in [
        &command.verification_contract_hash,
        &command.capability_contract_hash,
        &command.policy_snapshot_hash,
        &command.source_snapshot_hash,
    ] {
        require_sha256(hash)?;
    }
    if let Some(hash) = &command.adapter_contract_digest {
        require_sha256(hash)?;
    }
    let available = command.status == "available";
    if command.capability_key.trim().is_empty()
        || command.capability_contract_version.trim().is_empty()
        || command.assessment_ordinal < 0
        || !matches!(
            command.status.as_str(),
            "unassessed"
                | "available"
                | "adapter_missing"
                | "policy_denied"
                | "prerequisite_missing"
        )
        || (available
            != (command.adapter_contract_version.is_some()
                && command.adapter_contract_digest.is_some()
                && command.reason_code.is_none()
                && command.residual_id.is_none()))
        || (!available
            && (command
                .reason_code
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
                || command.residual_id.is_none()))
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let assessment_body = serde_json::json!({
        "operation_id": command.operation_id,
        "project_scope_id": command.project_scope_id,
        "organization_id": command.organization_id,
        "hypothesis_revision_id": command.hypothesis_revision_id,
        "verification_objective_id": command.verification_objective_id,
        "verification_contract_hash": command.verification_contract_hash,
        "capability_key": command.capability_key,
        "capability_contract_version": command.capability_contract_version,
        "capability_contract_hash": command.capability_contract_hash,
        "policy_snapshot_id": command.policy_snapshot_id,
        "policy_snapshot_hash": command.policy_snapshot_hash,
        "assessment_ordinal": command.assessment_ordinal,
        "supersedes_assessment_id": command.supersedes_assessment_id,
        "status": command.status,
        "reason_code": command.reason_code,
        "residual_id": command.residual_id,
        "adapter_contract_version": command.adapter_contract_version,
        "adapter_contract_digest": command.adapter_contract_digest,
        "source_snapshot_hash": command.source_snapshot_hash,
    });
    let assessment_hash = json_hash_on(&mut tx, &assessment_body).await?;
    if let Some(row) = sqlx::query_as::<_, CapabilityAssessmentRow>(&format!(
        "SELECT {ASSESSMENT_COLUMNS} FROM verification_capability_assessments WHERE stable_request_id=$1 FOR SHARE"
    ))
    .bind(command.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if row.assessment_hash == assessment_hash {
            tx.commit().await?;
            return Ok(row);
        }
        return Err(conflict(REPLAY_DRIFT));
    }
    if let Some(predecessor_id) = command.supersedes_assessment_id {
        let predecessor: (i64, String) = sqlx::query_as(
            r#"SELECT assessment_ordinal,capability_key
                 FROM verification_capability_assessments
                WHERE assessment_id=$1 AND hypothesis_revision_id=$2
                  AND verification_objective_id=$3 AND verification_contract_hash=$4
                FOR SHARE"#,
        )
        .bind(predecessor_id)
        .bind(command.hypothesis_revision_id)
        .bind(command.verification_objective_id)
        .bind(&command.verification_contract_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict(AUTHORITY_STALE))?;
        if predecessor.0 + 1 != command.assessment_ordinal
            || predecessor.1 != command.capability_key
        {
            return Err(conflict(AUTHORITY_STALE));
        }
    } else if command.assessment_ordinal != 0 {
        return Err(conflict(CONTRACT_INVALID));
    }
    let assessment_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-capability-assessment.v1",
    );
    let row = sqlx::query_as::<_, CapabilityAssessmentRow>(&format!(
        r#"INSERT INTO verification_capability_assessments(
               assessment_id,stable_request_id,operation_id,project_scope_id,organization_id,
               hypothesis_revision_id,verification_objective_id,verification_contract_id,
               verification_contract_hash,capability_key,capability_contract_version,
               capability_contract_hash,policy_snapshot_id,policy_snapshot_hash,
               assessment_ordinal,supersedes_assessment_id,status,reason_code,residual_id,
               adapter_contract_version,adapter_contract_digest,source_snapshot_hash,assessment_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23)
           RETURNING {ASSESSMENT_COLUMNS}"#
    ))
    .bind(assessment_id)
    .bind(command.stable_request_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.verification_objective_id)
    .bind(command.verification_contract_id)
    .bind(&command.verification_contract_hash)
    .bind(&command.capability_key)
    .bind(&command.capability_contract_version)
    .bind(&command.capability_contract_hash)
    .bind(command.policy_snapshot_id)
    .bind(&command.policy_snapshot_hash)
    .bind(command.assessment_ordinal)
    .bind(command.supersedes_assessment_id)
    .bind(&command.status)
    .bind(&command.reason_code)
    .bind(command.residual_id)
    .bind(&command.adapter_contract_version)
    .bind(&command.adapter_contract_digest)
    .bind(&command.source_snapshot_hash)
    .bind(&assessment_hash)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Debug, Clone)]
pub struct SealCapabilityAssessmentSet {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_objective_id: Uuid,
    pub verification_contract_hash: String,
    pub policy_snapshot_hash: String,
    pub source_snapshot_hash: String,
    pub registry_contract_hash: String,
    pub assessment_ids: Vec<Uuid>,
}

pub async fn seal_capability_assessment_set(
    pool: &PgPool,
    command: &SealCapabilityAssessmentSet,
) -> Result<Uuid> {
    if command.assessment_ids.is_empty() {
        return Err(conflict(CONTRACT_INVALID));
    }
    require_sha256(&command.registry_contract_hash)?;
    let mut tx = pool.begin().await?;
    let mut member_rows = Vec::with_capacity(command.assessment_ids.len());
    for (ordinal, assessment_id) in command.assessment_ids.iter().enumerate() {
        let row: (String, i64, String, String) = sqlx::query_as(
            r#"SELECT capability_key,assessment_ordinal,assessment_hash,policy_snapshot_hash
                 FROM verification_capability_assessments
                WHERE assessment_id=$1 AND operation_id=$2 AND project_scope_id=$3
                  AND organization_id=$4 AND hypothesis_revision_id=$5
                  AND verification_objective_id=$6 AND verification_contract_hash=$7
                FOR SHARE"#,
        )
        .bind(assessment_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(command.hypothesis_revision_id)
        .bind(command.verification_objective_id)
        .bind(&command.verification_contract_hash)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict(AUTHORITY_STALE))?;
        if row.3 != command.policy_snapshot_hash {
            return Err(conflict(AUTHORITY_STALE));
        }
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "assessment_id": assessment_id,
                "capability_key": row.0,
                "assessment_ordinal": row.1,
                "assessment_hash": row.2,
            }),
        )
        .await?;
        member_rows.push((*assessment_id, row.0, row.1, row.2, member_hash));
    }
    let member_hashes = member_rows
        .iter()
        .map(|row| row.4.clone())
        .collect::<Vec<_>>();
    let member_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_capability_assessment_set.v1",
        &member_hashes,
    )
    .await?;
    let seal_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "revision_id": command.hypothesis_revision_id,
            "objective_id": command.verification_objective_id,
            "contract_hash": command.verification_contract_hash,
            "policy_snapshot_hash": command.policy_snapshot_hash,
            "source_snapshot_hash": command.source_snapshot_hash,
            "registry_contract_hash": command.registry_contract_hash,
            "member_set_hash": member_set_hash,
        }),
    )
    .await?;
    let seal_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-capability-assessment-set.v1",
    );
    sqlx::query(
        r#"INSERT INTO verification_capability_assessment_set_seals(
               assessment_set_seal_id,stable_request_id,operation_id,project_scope_id,
               organization_id,hypothesis_revision_id,verification_objective_id,
               verification_contract_hash,policy_snapshot_hash,source_snapshot_hash,
               registry_contract_hash,member_count,member_set_hash,seal_hash,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,NULL)"#,
    )
    .bind(seal_id)
    .bind(command.stable_request_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.verification_objective_id)
    .bind(&command.verification_contract_hash)
    .bind(&command.policy_snapshot_hash)
    .bind(&command.source_snapshot_hash)
    .bind(&command.registry_contract_hash)
    .bind(member_rows.len() as i64)
    .bind(&member_set_hash)
    .bind(&seal_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, row) in member_rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO verification_capability_assessment_set_members(
                   assessment_set_seal_id,assessment_id,operation_id,project_scope_id,
                   organization_id,member_ordinal,capability_key,assessment_ordinal,
                   assessment_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(seal_id)
        .bind(row.0)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(ordinal as i32)
        .bind(&row.1)
        .bind(row.2)
        .bind(&row.3)
        .bind(&row.4)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE verification_capability_assessment_set_seals SET sealed_at=statement_timestamp() WHERE assessment_set_seal_id=$1 AND sealed_at IS NULL",
    )
    .bind(seal_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(seal_id)
}
