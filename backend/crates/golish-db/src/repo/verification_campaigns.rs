//! Server-owned Campaign admission, round census and strategy compounds.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::capability_execution_receipts::{
    with_all_fresh_tool_truth_authority_bundle, AllFreshToolTruthAuthorityBundle,
    CheckToolTruthAuthorityBundle, ToolTruthAuthorityBundleConsumerV1,
};
use crate::{DbError, Result};

pub const CONTRACT_INVALID: &str = "VERIFICATION_CAMPAIGN_CONTRACT_INVALID";
pub const AUTHORITY_STALE: &str = "VERIFICATION_CAMPAIGN_AUTHORITY_STALE";
pub const REPLAY_DRIFT: &str = "VERIFICATION_CAMPAIGN_REPLAY_DRIFT";

pub(crate) fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

pub(crate) fn require_sha256(value: &str) -> Result<()> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(conflict(CONTRACT_INVALID))
    }
}

pub(crate) async fn json_hash_on(
    tx: &mut Transaction<'_, Postgres>,
    value: &Value,
) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(value)
            .fetch_one(&mut **tx)
            .await?,
    )
}

pub(crate) async fn exact_set_hash_on(
    tx: &mut Transaction<'_, Postgres>,
    domain: &str,
    member_hashes: &[String],
) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT investigation_exact_member_set_hash($1,$2::TEXT[])")
            .bind(domain)
            .bind(member_hashes)
            .fetch_one(&mut **tx)
            .await?,
    )
}

#[derive(Debug, Clone)]
pub struct AdmitCampaign {
    pub expected_campaign_id: Option<Uuid>,
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_plan_id: Uuid,
    pub verification_plan_hash: String,
    pub plan_objective_id: Uuid,
    pub verification_objective_id: Uuid,
    pub verification_contract_id: Uuid,
    pub verification_contract_hash: String,
    pub capability_assessment_set_seal_id: Uuid,
    pub wave_denominator_id: Uuid,
    pub campaign_version: i64,
    pub source_snapshot_hash: String,
}

#[derive(Debug, Clone, Copy)]
pub struct AdmitCampaignFromAuthority {
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub generation_seal_id: Uuid,
    pub verification_plan_id: Uuid,
    pub objective_id: Uuid,
    pub wave_coverage_seal_id: Uuid,
    pub capability_assessment_set_seal_id: Uuid,
    pub expected_campaign_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignAdmission {
    pub campaign: VerificationCampaignRow,
    pub campaign_dispatch_generation: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct VerificationCampaignRow {
    pub campaign_id: Uuid,
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_objective_id: Uuid,
    pub verification_contract_hash: String,
    pub campaign_version: i64,
    pub state: String,
    pub row_version: i64,
    pub admitted_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
    pub superseded_at: Option<DateTime<Utc>>,
}

const CAMPAIGN_COLUMNS: &str = r#"campaign_id,stable_request_id,operation_id,project_scope_id,
    organization_id,hypothesis_revision_id,verification_objective_id,
    verification_contract_hash,campaign_version,state,row_version,admitted_at,
    terminal_at,superseded_at"#;

async fn admit_campaign_on(
    tx: &mut Transaction<'_, Postgres>,
    authority: &AllFreshToolTruthAuthorityBundle<'_>,
    command: &AdmitCampaign,
) -> Result<VerificationCampaignRow> {
    for hash in [
        &command.verification_plan_hash,
        &command.verification_contract_hash,
        &command.source_snapshot_hash,
    ] {
        require_sha256(hash)?;
    }
    if command.stable_request_id.is_nil()
        || command.operation_id.is_nil()
        || command.project_scope_id.is_nil()
        || command.organization_id.is_nil()
        || command.hypothesis_revision_id.is_nil()
        || command.verification_plan_id.is_nil()
        || command.plan_objective_id.is_nil()
        || command.verification_objective_id.is_nil()
        || command.verification_contract_id.is_nil()
        || command.capability_assessment_set_seal_id.is_nil()
        || command.wave_denominator_id.is_nil()
        || command.campaign_version <= 0
        || command.expected_campaign_id.is_some_and(|id| id.is_nil())
    {
        return Err(conflict(CONTRACT_INVALID));
    }

    if authority.checked().operation_id() != command.operation_id
        || authority.checked().organization_id() != command.organization_id
    {
        return Err(conflict(AUTHORITY_STALE));
    }
    let existing = sqlx::query_as::<_, VerificationCampaignRow>(&format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM verification_campaigns WHERE stable_request_id=$1 FOR SHARE"
    ))
    .bind(command.stable_request_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = existing {
        let expected_campaign_id = command.expected_campaign_id.unwrap_or_else(|| {
            Uuid::new_v5(&command.stable_request_id, b"verification-campaign.v1")
        });
        if row.campaign_id == expected_campaign_id
            && row.operation_id == command.operation_id
            && row.project_scope_id == command.project_scope_id
            && row.organization_id == command.organization_id
            && row.hypothesis_revision_id == command.hypothesis_revision_id
            && row.verification_objective_id == command.verification_objective_id
            && row.verification_contract_hash == command.verification_contract_hash
            && row.campaign_version == command.campaign_version
        {
            return Ok(row);
        }
        return Err(conflict(REPLAY_DRIFT));
    }

    let campaign_id = command
        .expected_campaign_id
        .unwrap_or_else(|| Uuid::new_v5(&command.stable_request_id, b"verification-campaign.v1"));
    let row = sqlx::query_as::<_, VerificationCampaignRow>(&format!(
        r#"INSERT INTO verification_campaigns(
               campaign_id,stable_request_id,operation_id,project_scope_id,organization_id,
               hypothesis_revision_id,verification_plan_id,verification_plan_hash,
               plan_objective_id,verification_objective_id,verification_contract_id,
               verification_contract_hash,capability_assessment_set_seal_id,
               wave_denominator_id,tool_truth_authority_bundle_seal_id,
               relevant_root_set_hash,authority_member_set_hash,
               semantic_authority_bundle_hash,freshness_attestation_bundle_hash,
               temporal_validity_bundle_hash,effective_valid_until,
               campaign_version,state,source_snapshot_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                    $18,$19,$20,$21,$22,'admitted',$23)
           RETURNING {CAMPAIGN_COLUMNS}"#
    ))
    .bind(campaign_id)
    .bind(command.stable_request_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.hypothesis_revision_id)
    .bind(command.verification_plan_id)
    .bind(&command.verification_plan_hash)
    .bind(command.plan_objective_id)
    .bind(command.verification_objective_id)
    .bind(command.verification_contract_id)
    .bind(&command.verification_contract_hash)
    .bind(command.capability_assessment_set_seal_id)
    .bind(command.wave_denominator_id)
    .bind(authority.bundle_seal_id())
    .bind(authority.checked().relevant_root_set_hash())
    .bind(authority.checked().member_set_hash())
    .bind(authority.checked().semantic_authority_bundle_hash())
    .bind(authority.checked().freshness_attestation_bundle_hash())
    .bind(authority.checked().temporal_validity_bundle_hash())
    .bind(
        authority
            .checked()
            .effective_valid_until()
            .ok_or_else(|| conflict(AUTHORITY_STALE))?,
    )
    .bind(command.campaign_version)
    .bind(&command.source_snapshot_hash)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
}

/// Public admission takes only server selectors. Plan/root/tool-truth fields
/// are derived and frozen inside the all-fresh authority callback transaction.
pub async fn admit_campaign_with_fresh_tool_truth(
    pool: &PgPool,
    request: AdmitCampaignFromAuthority,
) -> Result<CampaignAdmission> {
    let authority_request = CheckToolTruthAuthorityBundle {
        stable_consumer_request_id: request.stable_consumer_request_id,
        operation_id: request.operation_id,
        organization_id: request.organization_id,
        consumer_kind: ToolTruthAuthorityBundleConsumerV1::VerificationCampaign,
    };
    with_all_fresh_tool_truth_authority_bundle(pool, &authority_request, move |tx, authority| {
        Box::pin(async move {
            let stable_request_id = Uuid::new_v5(
                &request.stable_consumer_request_id,
                b"verification-campaign-admission.v1",
            );
            #[derive(sqlx::FromRow)]
            struct AdmissionAuthority {
                project_scope_id: Uuid,
                hypothesis_revision_id: Uuid,
                verification_plan_hash: String,
                plan_objective_id: Uuid,
                verification_contract_id: Uuid,
                verification_contract_hash: String,
                wave_denominator_id: Uuid,
                source_snapshot_hash: String,
                campaign_version: i64,
            }
            let selected = sqlx::query_as::<_, AdmissionAuthority>(
                r#"SELECT operation.project_scope_id,plan.revision_id AS hypothesis_revision_id,
                              plan.plan_hash AS verification_plan_hash,
                              objective.plan_objective_id,objective.verification_contract_id,
                              objective.verification_contract_hash,
                              wave.wave_denominator_id,wave.source_snapshot_hash,
                              COALESCE(
                                  (SELECT replay.campaign_version
                                     FROM verification_campaigns replay
                                    WHERE replay.stable_request_id=$9),
                                  (SELECT COALESCE(MAX(existing.campaign_version)+1,1)
                                     FROM verification_campaigns existing
                                    WHERE existing.hypothesis_revision_id=plan.revision_id
                                      AND existing.verification_objective_id=objective.objective_id
                                      AND existing.verification_contract_hash=
                                          objective.verification_contract_hash)
                              ) AS campaign_version
                         FROM operation_state operation
                         JOIN operation_org_scope_snapshots scope
                           ON scope.id=$2 AND scope.operation_id=operation.operation_id
                          AND scope.project_scope_id=operation.project_scope_id
                          AND scope.sealed_at IS NOT NULL
                         JOIN operation_org_scope_units unit
                           ON unit.snapshot_id=scope.id AND unit.organization_id=$3
                         JOIN attack_hypothesis_verification_plans plan
                           ON plan.plan_id=$5 AND plan.sealed_at IS NOT NULL
                         JOIN attack_hypothesis_revisions revision
                           ON revision.revision_id=plan.revision_id
                          AND revision.operation_id=operation.operation_id
                          AND revision.organization_id=$3
                         JOIN attack_hypothesis_verification_plan_objectives objective
                           ON objective.plan_id=plan.plan_id AND objective.objective_id=$6
                         JOIN verification_capability_assessment_set_seals assessment_set
                           ON assessment_set.assessment_set_seal_id=$8
                          AND assessment_set.hypothesis_revision_id=plan.revision_id
                          AND assessment_set.verification_objective_id=objective.objective_id
                          AND assessment_set.verification_contract_hash=
                              objective.verification_contract_hash
                          AND assessment_set.operation_id=operation.operation_id
                          AND assessment_set.project_scope_id=operation.project_scope_id
                          AND assessment_set.organization_id=$3
                          AND assessment_set.sealed_at IS NOT NULL
                         JOIN verification_wave_coverage_denominators wave
                           ON wave.wave_denominator_id=$7 AND wave.generation_seal_id=$4
                          AND wave.operation_id=operation.operation_id
                          AND wave.project_scope_id=operation.project_scope_id
                          AND wave.organization_id=$3 AND wave.sealed_at IS NOT NULL
                         JOIN hypothesis_generation_seals generation_seal
                           ON generation_seal.seal_id=$4
                         JOIN hypothesis_generations generation
                           ON generation.generation_id=generation_seal.generation_id
                          AND generation.operation_id=operation.operation_id
                          AND generation.organization_id=$3
                        WHERE operation.operation_id=$1
                        FOR SHARE OF operation,scope,unit,plan,revision,objective,
                                     assessment_set,wave,generation_seal,generation"#,
            )
            .bind(request.operation_id)
            .bind(request.scope_snapshot_id)
            .bind(request.organization_id)
            .bind(request.generation_seal_id)
            .bind(request.verification_plan_id)
            .bind(request.objective_id)
            .bind(request.wave_coverage_seal_id)
            .bind(request.capability_assessment_set_seal_id)
            .bind(stable_request_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| conflict(AUTHORITY_STALE))?;
            let replayed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM verification_campaigns WHERE stable_request_id=$1)",
            )
            .bind(stable_request_id)
            .fetch_one(&mut **tx)
            .await?;
            let campaign = admit_campaign_on(
                tx,
                authority,
                &AdmitCampaign {
                    expected_campaign_id: request.expected_campaign_id,
                    stable_request_id,
                    operation_id: request.operation_id,
                    project_scope_id: selected.project_scope_id,
                    organization_id: request.organization_id,
                    hypothesis_revision_id: selected.hypothesis_revision_id,
                    verification_plan_id: request.verification_plan_id,
                    verification_plan_hash: selected.verification_plan_hash,
                    plan_objective_id: selected.plan_objective_id,
                    verification_objective_id: request.objective_id,
                    verification_contract_id: selected.verification_contract_id,
                    verification_contract_hash: selected.verification_contract_hash,
                    capability_assessment_set_seal_id: request.capability_assessment_set_seal_id,
                    wave_denominator_id: selected.wave_denominator_id,
                    campaign_version: selected.campaign_version,
                    source_snapshot_hash: selected.source_snapshot_hash,
                },
            )
            .await?;
            let campaign_dispatch_generation: i64 = sqlx::query_scalar(
                r#"SELECT campaign_dispatch_generation
                         FROM verification_campaign_safety_holds
                        WHERE singleton=TRUE AND campaign_dispatch_held=FALSE
                        FOR SHARE"#,
            )
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| conflict(AUTHORITY_STALE))?;
            Ok(CampaignAdmission {
                campaign,
                campaign_dispatch_generation,
                replayed,
            })
        })
    })
    .await
}

#[derive(Debug, Clone)]
pub struct ConsultCensusMember {
    pub role_kind: String,
    pub request_packet: Value,
    pub response_artifact: Option<Value>,
    pub residual_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct OpenRoundWithConsultCensus {
    pub stable_request_id: Uuid,
    pub campaign_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub expected_campaign_row_version: i64,
    pub round_input: Value,
    pub consults: Vec<ConsultCensusMember>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct VerificationRoundRow {
    pub round_id: Uuid,
    pub campaign_id: Uuid,
    pub round_ordinal: i32,
    pub expected_campaign_row_version: i64,
    pub round_input_hash: String,
    pub consult_member_count: i64,
    pub consult_member_set_hash: String,
}

pub async fn open_round_with_consult_census(
    pool: &PgPool,
    command: &OpenRoundWithConsultCensus,
) -> Result<VerificationRoundRow> {
    if command.stable_request_id.is_nil()
        || command.campaign_id.is_nil()
        || command
            .consults
            .iter()
            .any(|consult| consult.role_kind.trim().is_empty())
        || command
            .consults
            .iter()
            .map(|consult| consult.role_kind.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != command.consults.len()
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let campaign: (String, i64) = sqlx::query_as(
        r#"SELECT state,row_version FROM verification_campaigns
            WHERE campaign_id=$1 AND operation_id=$2 AND project_scope_id=$3
              AND organization_id=$4 FOR UPDATE"#,
    )
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    if !matches!(campaign.0.as_str(), "admitted" | "running")
        || campaign.1 != command.expected_campaign_row_version
    {
        return Err(conflict(AUTHORITY_STALE));
    }
    let round_ordinal: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(round_ordinal)+1,0)::INT FROM verification_campaign_rounds WHERE campaign_id=$1",
    )
    .bind(command.campaign_id)
    .fetch_one(&mut *tx)
    .await?;
    let round_input_hash = json_hash_on(&mut tx, &command.round_input).await?;
    let mut consult_hashes = Vec::with_capacity(command.consults.len());
    for (ordinal, consult) in command.consults.iter().enumerate() {
        consult_hashes.push(
            json_hash_on(
                &mut tx,
                &serde_json::json!({
                    "ordinal": ordinal,
                    "role_kind": consult.role_kind,
                    "request_packet": consult.request_packet,
                }),
            )
            .await?,
        );
    }
    let consult_member_set_hash =
        exact_set_hash_on(&mut tx, "verification_consult_census.v1", &consult_hashes).await?;
    let round_id = Uuid::new_v5(&command.stable_request_id, b"verification-round.v1");
    sqlx::query(
        r#"INSERT INTO verification_campaign_rounds(
               round_id,stable_request_id,campaign_id,operation_id,project_scope_id,
               organization_id,round_ordinal,expected_campaign_row_version,
               round_input,round_input_hash,consult_member_count,consult_member_set_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(round_id)
    .bind(command.stable_request_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(round_ordinal)
    .bind(command.expected_campaign_row_version)
    .bind(&command.round_input)
    .bind(&round_input_hash)
    .bind(command.consults.len() as i64)
    .bind(&consult_member_set_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (consult, member_hash)) in command
        .consults
        .iter()
        .zip(consult_hashes.iter())
        .enumerate()
    {
        let response_hash = if let Some(response) = &consult.response_artifact {
            Some(json_hash_on(&mut tx, response).await?)
        } else {
            None
        };
        sqlx::query(
            r#"INSERT INTO verification_consults(
                   consult_id,round_id,campaign_id,operation_id,project_scope_id,
                   organization_id,consult_ordinal,role_kind,request_packet,
                   request_packet_hash,response_artifact,response_artifact_hash,
                   disposition,residual_id
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(Uuid::new_v5(&round_id, member_hash.as_bytes()))
        .bind(round_id)
        .bind(command.campaign_id)
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(ordinal as i32)
        .bind(&consult.role_kind)
        .bind(&consult.request_packet)
        .bind(member_hash)
        .bind(&consult.response_artifact)
        .bind(response_hash)
        .bind(if consult.response_artifact.is_some() {
            "completed"
        } else {
            "pending"
        })
        .bind(consult.residual_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE verification_campaigns SET state=CASE WHEN state='admitted' THEN 'running' ELSE state END,row_version=row_version+1 WHERE campaign_id=$1 AND row_version=$2",
    )
    .bind(command.campaign_id)
    .bind(command.expected_campaign_row_version)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(VerificationRoundRow {
        round_id,
        campaign_id: command.campaign_id,
        round_ordinal,
        expected_campaign_row_version: command.expected_campaign_row_version,
        round_input_hash,
        consult_member_count: command.consults.len() as i64,
        consult_member_set_hash,
    })
}

#[derive(Debug, Clone)]
pub struct StrategyObligation {
    pub obligation_kind: String,
    pub semantic_key: String,
    pub disposition: String,
    pub residual_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct RecordStrategyDecision {
    pub stable_request_id: Uuid,
    pub round_id: Uuid,
    pub campaign_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub decision_kind: String,
    pub typed_strategy: Value,
    pub reason_code: String,
    pub residual_id: Option<Uuid>,
    pub obligations: Vec<StrategyObligation>,
}

pub async fn record_strategy_decision(
    pool: &PgPool,
    command: &RecordStrategyDecision,
) -> Result<Uuid> {
    if command.reason_code.trim().is_empty()
        || !matches!(
            command.decision_kind.as_str(),
            "compile_action" | "no_action_compilable" | "stop" | "refine"
        )
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let strategy_hash = json_hash_on(&mut tx, &command.typed_strategy).await?;
    let mut member_hashes = Vec::with_capacity(command.obligations.len());
    for (ordinal, obligation) in command.obligations.iter().enumerate() {
        member_hashes.push(
            json_hash_on(
                &mut tx,
                &serde_json::json!({
                    "ordinal": ordinal,
                    "kind": obligation.obligation_kind,
                    "semantic_key": obligation.semantic_key,
                    "disposition": obligation.disposition,
                    "residual_id": obligation.residual_id,
                }),
            )
            .await?,
        );
    }
    let member_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_strategy_obligations.v1",
        &member_hashes,
    )
    .await?;
    let artifact_id = Uuid::new_v5(&command.stable_request_id, b"verification-strategy.v1");
    sqlx::query(
        r#"INSERT INTO verification_strategy_artifacts(
               strategy_artifact_id,stable_request_id,round_id,campaign_id,operation_id,
               project_scope_id,organization_id,decision_kind,typed_strategy,strategy_hash,
               obligation_member_count,obligation_member_set_hash,reason_code,residual_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(artifact_id)
    .bind(command.stable_request_id)
    .bind(command.round_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(&command.decision_kind)
    .bind(&command.typed_strategy)
    .bind(strategy_hash)
    .bind(command.obligations.len() as i64)
    .bind(member_set_hash)
    .bind(&command.reason_code)
    .bind(command.residual_id)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (obligation, member_hash)) in command
        .obligations
        .iter()
        .zip(member_hashes.iter())
        .enumerate()
    {
        sqlx::query(
            r#"INSERT INTO verification_strategy_obligations(
                   strategy_artifact_id,obligation_id,obligation_ordinal,obligation_kind,
                   semantic_key,disposition,residual_id,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(artifact_id)
        .bind(Uuid::new_v5(&artifact_id, member_hash.as_bytes()))
        .bind(ordinal as i32)
        .bind(&obligation.obligation_kind)
        .bind(&obligation.semantic_key)
        .bind(&obligation.disposition)
        .bind(obligation.residual_id)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(artifact_id)
}
