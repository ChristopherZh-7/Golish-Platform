//! Transactional durable company/asset queue for Investigation.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{DbError, Result};

pub const CONTRACT_INVALID: &str = "INVESTIGATION_ASSET_QUEUE_CONTRACT_INVALID";
pub const AUTHORITY_MISMATCH: &str = "INVESTIGATION_ASSET_QUEUE_AUTHORITY_MISMATCH";
pub const CAS_CONFLICT: &str = "INVESTIGATION_ASSET_QUEUE_CAS_CONFLICT";
pub const REPLAY_DRIFT: &str = "INVESTIGATION_ASSET_QUEUE_REPLAY_DRIFT";
pub const EVOLUTION_FUEL_EXHAUSTED: &str = "INVESTIGATION_ASSET_QUEUE_EVOLUTION_FUEL_EXHAUSTED";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn validate_uuid(value: Uuid) -> Result<()> {
    if value.is_nil() {
        Err(conflict(CONTRACT_INVALID))
    } else {
        Ok(())
    }
}

async fn sha256_on(tx: &mut Transaction<'_, Postgres>, value: &str) -> Result<String> {
    Ok(sqlx::query_scalar("SELECT tool_truth_sha256($1)")
        .bind(value)
        .fetch_one(&mut **tx)
        .await?)
}

async fn set_sha256_on(
    tx: &mut Transaction<'_, Postgres>,
    domain: &str,
    values: &[String],
) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT investigation_exact_member_set_hash($1,$2::TEXT[])")
            .bind(domain)
            .bind(values)
            .fetch_one(&mut **tx)
            .await?,
    )
}

#[derive(Debug, sqlx::FromRow)]
struct QueueHeader {
    company_queue_id: Uuid,
    stable_freeze_request_id: Uuid,
    authority_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    owning_stage_run_request_id: String,
    scope_snapshot_id: Uuid,
    member_count: i64,
    member_set_sha256: String,
    max_evolution_epochs: i32,
    head_version: i64,
}

async fn load_queue_on(
    tx: &mut Transaction<'_, Postgres>,
    company_queue_id: Uuid,
    replayed: bool,
) -> Result<InvestigationCompanyAssetQueueRow> {
    let header = sqlx::query_as::<_, QueueHeader>(
        r#"SELECT company_queue_id,stable_freeze_request_id,authority_id,operation_id,
                  stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
                  member_count,member_set_sha256,max_evolution_epochs,head_version
             FROM investigation_company_queues WHERE company_queue_id=$1"#,
    )
    .bind(company_queue_id)
    .fetch_one(&mut **tx)
    .await?;
    let companies = sqlx::query_as::<_, InvestigationCompanyQueueMemberRow>(
        r#"SELECT member.company_member_id,member.company_queue_id,member.organization_id,
                  member.organization_name_at_freeze,member.depth,member.ordinal,member.state,
                  queue.head_version AS company_queue_head_version,member.row_version
             FROM investigation_company_queue_members member
             JOIN investigation_company_queues queue
               ON queue.company_queue_id=member.company_queue_id
            WHERE member.company_queue_id=$1
            ORDER BY member.depth,member.ordinal,member.organization_id"#,
    )
    .bind(company_queue_id)
    .fetch_all(&mut **tx)
    .await?;
    let assets = sqlx::query_as::<_, InvestigationAssetLaneRow>(
        r#"SELECT lane.asset_lane_id,lane.asset_queue_id,lane.company_queue_id,
                  lane.company_member_id,lane.operation_id,lane.scope_snapshot_id,
                  lane.organization_id,lane.target_id,lane.target_type_at_freeze,
                  lane.target_value_at_freeze,lane.target_source_at_freeze,
                  lane.target_created_at,lane.target_identity_sha256,lane.ordinal,
                  lane.state,lane.evolution_epoch,
                  lane.max_evolution_epochs,asset_queue.head_version AS asset_queue_head_version,
                  lane.row_version
             FROM investigation_asset_lanes lane
             JOIN investigation_asset_queues asset_queue
               ON asset_queue.asset_queue_id=lane.asset_queue_id
            WHERE lane.company_queue_id=$1
            ORDER BY (SELECT depth FROM investigation_company_queue_members m
                       WHERE m.company_member_id=lane.company_member_id),
                     (SELECT ordinal FROM investigation_company_queue_members m
                       WHERE m.company_member_id=lane.company_member_id),
                     lane.target_created_at,lane.target_value_at_freeze,lane.target_id"#,
    )
    .bind(company_queue_id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(InvestigationCompanyAssetQueueRow {
        company_queue_id: header.company_queue_id,
        authority_id: header.authority_id,
        operation_id: header.operation_id,
        stage_execution_id: header.stage_execution_id,
        owning_stage_run_request_id: header.owning_stage_run_request_id,
        scope_snapshot_id: header.scope_snapshot_id,
        company_member_count: header.member_count,
        company_member_set_sha256: header.member_set_sha256,
        company_head_version: header.head_version,
        companies,
        assets,
        replayed,
    })
}

async fn load_lane_on(
    tx: &mut Transaction<'_, Postgres>,
    asset_lane_id: Uuid,
) -> Result<InvestigationAssetLaneRow> {
    Ok(sqlx::query_as::<_, InvestigationAssetLaneRow>(
        r#"SELECT lane.asset_lane_id,lane.asset_queue_id,lane.company_queue_id,
                  lane.company_member_id,lane.operation_id,lane.scope_snapshot_id,
                  lane.organization_id,lane.target_id,lane.target_type_at_freeze,
                  lane.target_value_at_freeze,lane.target_source_at_freeze,
                  lane.target_created_at,lane.target_identity_sha256,lane.ordinal,
                  lane.state,lane.evolution_epoch,lane.max_evolution_epochs,
                  asset_queue.head_version AS asset_queue_head_version,lane.row_version
             FROM investigation_asset_lanes lane
             JOIN investigation_asset_queues asset_queue
               ON asset_queue.asset_queue_id=lane.asset_queue_id
            WHERE lane.asset_lane_id=$1"#,
    )
    .bind(asset_lane_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn load_company_on(
    tx: &mut Transaction<'_, Postgres>,
    company_member_id: Uuid,
) -> Result<InvestigationCompanyQueueMemberRow> {
    Ok(sqlx::query_as::<_, InvestigationCompanyQueueMemberRow>(
        r#"SELECT member.company_member_id,member.company_queue_id,member.organization_id,
                  member.organization_name_at_freeze,member.depth,member.ordinal,member.state,
                  queue.head_version AS company_queue_head_version,member.row_version
             FROM investigation_company_queue_members member
             JOIN investigation_company_queues queue
               ON queue.company_queue_id=member.company_queue_id
            WHERE member.company_member_id=$1"#,
    )
    .bind(company_member_id)
    .fetch_one(&mut **tx)
    .await?)
}

async fn load_claimed_lane_projection_on(
    tx: &mut Transaction<'_, Postgres>,
    asset_lane_id: Uuid,
    claim_event_id: Uuid,
) -> Result<InvestigationAssetLaneRow> {
    let mut lane = load_lane_on(tx, asset_lane_id).await?;
    let (event_lane_id, event_queue_id, expected_queue_head, expected_lane_row, evolution_epoch): (
        Uuid,
        Uuid,
        i64,
        i64,
        i32,
    ) = sqlx::query_as(
        r#"SELECT asset_lane_id,asset_queue_id,expected_queue_head_version,
                  expected_lane_row_version,evolution_epoch
             FROM investigation_asset_lane_events
            WHERE event_id=$1 AND event_kind='claim' AND from_state='queued'
              AND to_state='analyzing'"#,
    )
    .bind(claim_event_id)
    .fetch_one(&mut **tx)
    .await?;
    if event_lane_id != asset_lane_id || event_queue_id != lane.asset_queue_id {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    lane.state = "analyzing".to_string();
    lane.asset_queue_head_version = expected_queue_head + 1;
    lane.row_version = expected_lane_row + 1;
    lane.evolution_epoch = evolution_epoch;
    Ok(lane)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreezeInvestigationCompanyAssetQueueRow {
    pub stable_request_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub max_evolution_epochs: i32,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationCompanyQueueMemberRow {
    pub company_member_id: Uuid,
    pub company_queue_id: Uuid,
    pub organization_id: Uuid,
    pub organization_name_at_freeze: String,
    pub depth: i32,
    pub ordinal: i32,
    pub state: String,
    pub company_queue_head_version: i64,
    pub row_version: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationAssetLaneRow {
    pub asset_lane_id: Uuid,
    pub asset_queue_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub target_id: Uuid,
    pub target_type_at_freeze: String,
    pub target_value_at_freeze: String,
    pub target_source_at_freeze: String,
    pub target_created_at: DateTime<Utc>,
    pub target_identity_sha256: String,
    pub ordinal: i32,
    pub state: String,
    pub evolution_epoch: i32,
    pub max_evolution_epochs: i32,
    pub asset_queue_head_version: i64,
    pub row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadCurrentInvestigationAssetEvolutionAuthorityRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub expected_evolution_epoch: i32,
}

#[derive(Debug, Clone, Copy, sqlx::FromRow, PartialEq, Eq)]
pub struct InvestigationAssetEvolutionAuthorityRow {
    pub asset_lane_id: Uuid,
    pub evolution_epoch: i32,
    pub pending_evolution_authority_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCompanyAssetQueueRow {
    pub company_queue_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub owning_stage_run_request_id: String,
    pub scope_snapshot_id: Uuid,
    pub company_member_count: i64,
    pub company_member_set_sha256: String,
    pub company_head_version: i64,
    pub companies: Vec<InvestigationCompanyQueueMemberRow>,
    pub assets: Vec<InvestigationAssetLaneRow>,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimNextInvestigationCompanyRow {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub expected_company_member_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_member_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimNextInvestigationAssetRow {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_asset_lane_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_lane_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransitionInvestigationAssetLaneRow {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub asset_lane_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_lane_row_version: i64,
    pub from_state: &'static str,
    pub to_state: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompleteInvestigationCompanyRow {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_member_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealZeroHypothesisAssetFixedPointRow {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub asset_lane_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_queue_head_version: i64,
    pub expected_lane_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAssetFixedPointReceiptRow {
    pub fixed_point_receipt_id: Uuid,
    pub asset_lane: InvestigationAssetLaneRow,
    pub receipt_sha256: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadInvestigationAssetBacklogRow {
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub asset_lane_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAssetBacklogRow {
    pub asset_lane: InvestigationAssetLaneRow,
    pub latest_generation_id: Option<Uuid>,
    pub latest_generation_seal_id: Option<Uuid>,
    pub generation_count: i64,
    pub hypothesis_root_count: i64,
    pub dynamically_resolved_root_count: i64,
    pub revision_count: i64,
    pub verification_task_count: i64,
    pub open_verification_task_count: i64,
    pub campaign_count: i64,
    pub open_campaign_count: i64,
    pub prepared_action_count: i64,
    pub open_prepared_action_count: i64,
    pub action_execution_count: i64,
    pub open_action_execution_count: i64,
    pub oracle_count: i64,
    pub fact_delta_count: i64,
    pub wave_count: i64,
    pub advanced_wave_count: i64,
    pub fixed_point_wave_count: i64,
    pub pending_evolution_count: i64,
    pub pending_hypothesis_discovery_count: i64,
    pub backlog_member_count: i64,
    pub backlog_set_sha256: String,
    pub obligation_set_sha256: String,
    pub residual_set_sha256: String,
    pub zero_hypothesis_fixed_point_receipt_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseInvestigationAssetBacklogAndAdvanceRow {
    pub stable_request_id: Uuid,
    pub company_queue_id: Uuid,
    pub company_member_id: Uuid,
    pub asset_queue_id: Uuid,
    pub asset_lane_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub expected_company_queue_head_version: i64,
    pub expected_company_member_row_version: i64,
    pub expected_asset_queue_head_version: i64,
    pub expected_asset_lane_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvestigationAssetProgressionDispositionRow {
    NextAsset,
    NextCompany,
    InvestigationComplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationResolutionClosureMemberRow {
    pub organization_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub stage_team_plan_id: Uuid,
    pub passed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationResolutionClosurePublicationRow {
    pub publication_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub member_set_sha256: String,
    pub members: Vec<InvestigationResolutionClosureMemberRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAssetProgressionRow {
    pub progression_receipt_id: Uuid,
    pub fixed_asset_lane_id: Uuid,
    pub disposition: InvestigationAssetProgressionDispositionRow,
    pub next_company_member_id: Option<Uuid>,
    pub next_asset_lane: Option<InvestigationAssetLaneRow>,
    pub company_queue_head_version: i64,
    pub stage_closure: Option<InvestigationResolutionClosurePublicationRow>,
    pub replayed: bool,
}

fn progression_disposition_text(
    disposition: InvestigationAssetProgressionDispositionRow,
) -> &'static str {
    match disposition {
        InvestigationAssetProgressionDispositionRow::NextAsset => "next_asset",
        InvestigationAssetProgressionDispositionRow::NextCompany => "next_company",
        InvestigationAssetProgressionDispositionRow::InvestigationComplete => {
            "investigation_complete"
        }
    }
}

fn derived_stable_request(root: Uuid, domain: &str, member_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &root,
        format!("golish.investigation.asset_progression.v1:{domain}:{member_id}").as_bytes(),
    )
}

async fn load_resolution_closure_publication_on(
    tx: &mut Transaction<'_, Postgres>,
    publication_id: Uuid,
) -> Result<InvestigationResolutionClosurePublicationRow> {
    #[derive(sqlx::FromRow)]
    struct ClosureMember {
        organization_id: Uuid,
        company_member_id: Uuid,
        stage_run_unit_id: Uuid,
        stage_team_plan_id: Uuid,
        passed_at: DateTime<Utc>,
        member_sha256: String,
        member_hash_exact: bool,
        unit_status: String,
        unit_terminal_at: Option<DateTime<Utc>>,
        completion_passed_at: Option<DateTime<Utc>>,
        completion_stage_run_id: Option<String>,
        pass_watermark: serde_json::Value,
        plan_requests_closed_at: Option<DateTime<Utc>>,
    }
    let (
        member_count,
        member_set_sha256,
        company_queue_id,
        authority_id,
        operation_id,
        stage_execution_id,
        scope_snapshot_id,
        publication_sha256,
    ): (i64, String, Uuid, Uuid, Uuid, Uuid, Uuid, String) = sqlx::query_as(
        r#"SELECT member_count,member_set_sha256,company_queue_id,authority_id,operation_id,
                  stage_execution_id,scope_snapshot_id,publication_sha256
             FROM investigation_asset_queue_closure_publications
            WHERE publication_id=$1"#,
    )
    .bind(publication_id)
    .fetch_one(&mut **tx)
    .await?;
    let members = sqlx::query_as::<_, ClosureMember>(
        r#"SELECT member.organization_id,member.company_member_id,
                  member.stage_run_unit_id,member.stage_team_plan_id,
                  member.passed_at,member.member_sha256,
                  member.member_sha256=tool_truth_sha256(format(
                      'golish.investigation.asset_queue_closure_member.v1:%s:%s:%s:%s:%s',
                      member.publication_id,member.company_member_id,member.organization_id,
                      member.stage_run_unit_id,member.stage_team_plan_id
                  )) AS member_hash_exact,
                  unit.status AS unit_status,
                  unit.terminal_at AS unit_terminal_at,
                  completion.passed_at AS completion_passed_at,
                  completion.stage_run_id AS completion_stage_run_id,
                  unit.pass_watermark,plan.requests_closed_at AS plan_requests_closed_at
             FROM investigation_asset_queue_closure_publication_members member
             JOIN stage_run_units unit ON unit.id=member.stage_run_unit_id
             JOIN stage_team_plans plan ON plan.id=member.stage_team_plan_id
             LEFT JOIN org_stage_completions completion
               ON completion.organization_id=member.organization_id
              AND completion.stage_kind='investigation'
            WHERE member.publication_id=$1
            ORDER BY member.member_ordinal"#,
    )
    .bind(publication_id)
    .fetch_all(&mut **tx)
    .await?;
    if i64::try_from(members.len()).map_err(|_| conflict(CONTRACT_INVALID))? != member_count {
        return Err(conflict(REPLAY_DRIFT));
    }
    let operation_id_text = operation_id.to_string();
    if members.iter().any(|member| {
        member.unit_status != "passed"
            || !member.member_hash_exact
            || member.unit_terminal_at.as_ref() != Some(&member.passed_at)
            || member.completion_passed_at.as_ref() != Some(&member.passed_at)
            || member.completion_stage_run_id.as_deref() != Some(operation_id_text.as_str())
            || member.plan_requests_closed_at.is_none()
            || member.pass_watermark
                != serde_json::json!({
                    "schema": "investigation_asset_queue_closure_publication.v1",
                    "publication_id": publication_id,
                    "company_queue_id": company_queue_id,
                    "company_member_id": member.company_member_id,
                    "member_sha256": member.member_sha256.as_str(),
                })
    }) {
        return Err(conflict(REPLAY_DRIFT));
    }
    let member_hashes = members
        .iter()
        .map(|member| member.member_sha256.clone())
        .collect::<Vec<_>>();
    if set_sha256_on(
        tx,
        "golish.investigation.asset_queue_closure_members.v1",
        &member_hashes,
    )
    .await?
        != member_set_sha256
    {
        return Err(conflict(REPLAY_DRIFT));
    }
    let expected_publication_sha256 = sha256_on(
        tx,
        &format!(
            "golish.investigation.asset_queue_closure_publication.v1:{publication_id}:{company_queue_id}:{authority_id}:{operation_id}:{stage_execution_id}:{scope_snapshot_id}:{member_set_sha256}"
        ),
    )
    .await?;
    if publication_sha256 != expected_publication_sha256 {
        return Err(conflict(REPLAY_DRIFT));
    }
    Ok(InvestigationResolutionClosurePublicationRow {
        publication_id,
        operation_id,
        stage_execution_id,
        scope_snapshot_id,
        member_set_sha256,
        members: members
            .into_iter()
            .map(|member| InvestigationResolutionClosureMemberRow {
                organization_id: member.organization_id,
                stage_run_unit_id: member.stage_run_unit_id,
                stage_team_plan_id: member.stage_team_plan_id,
                passed_at: member.passed_at,
            })
            .collect(),
    })
}

pub async fn load_resolution_closure_publication(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<InvestigationResolutionClosurePublicationRow>> {
    let mut tx = pool.begin().await?;
    let publication_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT publication_id FROM investigation_asset_queue_closure_publications WHERE operation_id=$1",
    )
    .bind(operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    let row = match publication_id {
        Some(publication_id) => {
            Some(load_resolution_closure_publication_on(&mut tx, publication_id).await?)
        }
        None => None,
    };
    tx.commit().await?;
    Ok(row)
}

async fn load_progression_replay_on(
    tx: &mut Transaction<'_, Postgres>,
    stable_request_id: Uuid,
    request_fingerprint: &str,
    request: &CloseInvestigationAssetBacklogAndAdvanceRow,
) -> Result<Option<InvestigationAssetProgressionRow>> {
    #[derive(sqlx::FromRow)]
    struct Existing {
        progression_receipt_id: Uuid,
        source_fixed_point_receipt_id: Option<Uuid>,
        source_zero_fixed_point_receipt_id: Option<Uuid>,
        fixed_asset_lane_id: Uuid,
        company_queue_id: Uuid,
        company_member_id: Uuid,
        asset_queue_id: Uuid,
        operation_id: Uuid,
        scope_snapshot_id: Uuid,
        organization_id: Uuid,
        expected_company_queue_head_version: i64,
        expected_company_member_row_version: i64,
        expected_asset_queue_head_version: i64,
        expected_asset_lane_row_version: i64,
        disposition: String,
        next_company_member_id: Option<Uuid>,
        next_asset_lane_id: Option<Uuid>,
        next_asset_claim_event_id: Option<Uuid>,
        stage_closure_publication_id: Option<Uuid>,
        result_company_queue_head_version: i64,
        request_fingerprint_sha256: String,
    }
    let existing = sqlx::query_as::<_, Existing>(
        r#"SELECT progression_receipt_id,source_fixed_point_receipt_id,
                  source_zero_fixed_point_receipt_id,fixed_asset_lane_id,company_queue_id,
                  company_member_id,asset_queue_id,operation_id,scope_snapshot_id,
                  organization_id,expected_company_queue_head_version,
                  expected_company_member_row_version,expected_asset_queue_head_version,
                  expected_asset_lane_row_version,disposition,next_company_member_id,
                  next_asset_lane_id,next_asset_claim_event_id,stage_closure_publication_id,
                  result_company_queue_head_version,
                  request_fingerprint_sha256
             FROM investigation_asset_progression_receipts
            WHERE stable_request_id=$1"#,
    )
    .bind(stable_request_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(existing) = existing else {
        return Ok(None);
    };
    if existing.request_fingerprint_sha256 != request_fingerprint
        || existing.fixed_asset_lane_id != request.asset_lane_id
        || existing.company_queue_id != request.company_queue_id
        || existing.company_member_id != request.company_member_id
        || existing.asset_queue_id != request.asset_queue_id
        || existing.operation_id != request.operation_id
        || existing.scope_snapshot_id != request.scope_snapshot_id
        || existing.organization_id != request.organization_id
        || existing.expected_company_queue_head_version
            != request.expected_company_queue_head_version
        || existing.expected_company_member_row_version
            != request.expected_company_member_row_version
        || existing.expected_asset_queue_head_version != request.expected_asset_queue_head_version
        || existing.expected_asset_lane_row_version != request.expected_asset_lane_row_version
    {
        return Err(conflict(REPLAY_DRIFT));
    }
    if let Some(fixed_point_receipt_id) = existing.source_fixed_point_receipt_id {
        let provenance_is_current: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                 SELECT 1 FROM investigation_asset_backlog_fixed_point_receipts receipt
                  WHERE receipt.fixed_point_receipt_id=$1
                    AND receipt.dynamic_resolution_member_count=receipt.hypothesis_root_count
                    AND receipt.dynamic_resolution_member_count>0
                    AND receipt.dynamic_resolution_member_set_sha256 IS NOT NULL
                    AND (SELECT count(*)
                           FROM investigation_asset_backlog_dynamic_resolution_members member
                          WHERE member.fixed_point_receipt_id=receipt.fixed_point_receipt_id)=
                        receipt.dynamic_resolution_member_count)"#,
        )
        .bind(fixed_point_receipt_id)
        .fetch_one(&mut **tx)
        .await?;
        if !provenance_is_current || existing.source_zero_fixed_point_receipt_id.is_some() {
            return Err(conflict(REPLAY_DRIFT));
        }
    } else if existing.source_zero_fixed_point_receipt_id.is_none() {
        return Err(conflict(REPLAY_DRIFT));
    }
    let disposition = match existing.disposition.as_str() {
        "next_asset" => InvestigationAssetProgressionDispositionRow::NextAsset,
        "next_company" => InvestigationAssetProgressionDispositionRow::NextCompany,
        "investigation_complete" => {
            InvestigationAssetProgressionDispositionRow::InvestigationComplete
        }
        _ => return Err(conflict(CONTRACT_INVALID)),
    };
    let next_asset_lane = match (
        existing.next_asset_lane_id,
        existing.next_asset_claim_event_id,
    ) {
        (Some(lane_id), Some(event_id)) => {
            Some(load_claimed_lane_projection_on(tx, lane_id, event_id).await?)
        }
        (None, None) => None,
        _ => return Err(conflict(CONTRACT_INVALID)),
    };
    let stage_closure = match (disposition, existing.stage_closure_publication_id) {
        (InvestigationAssetProgressionDispositionRow::InvestigationComplete, Some(id)) => {
            Some(load_resolution_closure_publication_on(tx, id).await?)
        }
        (InvestigationAssetProgressionDispositionRow::InvestigationComplete, None) => {
            return Err(conflict(REPLAY_DRIFT));
        }
        (_, None) => None,
        (_, Some(_)) => return Err(conflict(REPLAY_DRIFT)),
    };
    Ok(Some(InvestigationAssetProgressionRow {
        progression_receipt_id: existing.progression_receipt_id,
        fixed_asset_lane_id: existing.fixed_asset_lane_id,
        disposition,
        next_company_member_id: existing.next_company_member_id,
        next_asset_lane,
        company_queue_head_version: existing.result_company_queue_head_version,
        stage_closure,
        replayed: true,
    }))
}

async fn load_asset_backlog_on(
    tx: &mut Transaction<'_, Postgres>,
    request: &LoadInvestigationAssetBacklogRow,
) -> Result<InvestigationAssetBacklogRow> {
    let asset_lane = load_lane_on(tx, request.asset_lane_id).await?;
    if (
        asset_lane.company_queue_id,
        asset_lane.company_member_id,
        asset_lane.asset_queue_id,
        asset_lane.operation_id,
        asset_lane.scope_snapshot_id,
        asset_lane.organization_id,
    ) != (
        request.company_queue_id,
        request.company_member_id,
        request.asset_queue_id,
        request.operation_id,
        request.scope_snapshot_id,
        request.organization_id,
    ) {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    #[derive(sqlx::FromRow)]
    struct Census {
        generation_count: i64,
        hypothesis_root_count: i64,
        dynamically_resolved_root_count: i64,
        revision_count: i64,
        verification_task_count: i64,
        open_verification_task_count: i64,
        campaign_count: i64,
        open_campaign_count: i64,
        prepared_action_count: i64,
        open_prepared_action_count: i64,
        action_execution_count: i64,
        open_action_execution_count: i64,
        oracle_count: i64,
        fact_delta_count: i64,
        wave_count: i64,
        advanced_wave_count: i64,
        fixed_point_wave_count: i64,
        pending_evolution_count: i64,
        pending_hypothesis_discovery_count: i64,
    }
    let census = sqlx::query_as::<_, Census>(
        r#"SELECT
             (SELECT count(*) FROM hypothesis_generations generation
               WHERE generation.asset_lane_id=$1) AS generation_count,
             (SELECT count(*) FROM attack_hypotheses root
               WHERE root.asset_lane_id=$1) AS hypothesis_root_count,
             (SELECT count(*) FROM attack_hypotheses root
                JOIN attack_hypothesis_heads head
                  ON head.root_id=root.root_id
                 AND head.operation_id=root.operation_id
                 AND head.organization_id=root.organization_id
                JOIN attack_hypothesis_revisions terminal
                  ON terminal.revision_id=head.head_revision_id
                 AND terminal.root_id=root.root_id
                 AND terminal.operation_id=root.operation_id
                 AND terminal.organization_id=root.organization_id
               WHERE root.asset_lane_id=$1
                 AND head.head_lifecycle_state='closed'
                 AND head.head_epistemic_state IN('verified','refuted','invalid')
                 AND terminal.lifecycle_state='closed'
                 AND terminal.epistemic_state=head.head_epistemic_state
                 AND EXISTS(
                   SELECT 1
                     FROM investigation_dynamic_hypothesis_terminal_transitions transition
                     JOIN investigation_dynamic_hypothesis_resolutions resolution
                       ON resolution.resolution_authority_id=transition.resolution_authority_id
                      AND resolution.hypothesis_revision_id=transition.source_revision_id
                      AND resolution.asset_lane_id=root.asset_lane_id
                      AND resolution.disposition=transition.disposition
                     JOIN investigation_dynamic_verification_rounds dynamic_round
                       ON dynamic_round.session_id=resolution.session_id
                      AND dynamic_round.operation_id=root.operation_id
                      AND dynamic_round.organization_id=root.organization_id
                      AND dynamic_round.asset_lane_id=root.asset_lane_id
                      AND dynamic_round.hypothesis_revision_id=transition.source_revision_id
                      AND dynamic_round.state='resolved'
                      AND dynamic_round.resolution_authority_id=resolution.resolution_authority_id
                     JOIN attack_hypothesis_state_events event
                       ON event.event_id=transition.state_event_id
                      AND event.predecessor_revision_id=transition.source_revision_id
                      AND event.successor_revision_id=transition.terminal_revision_id
                      AND event.origin_authority='dynamic_verification_resolution'
                      AND event.authority_receipt_kind='dynamic_resolution'
                      AND event.authority_receipt_id=resolution.resolution_authority_id
                      AND event.authority_receipt_hash=resolution.resolution_sha256
                    WHERE transition.asset_lane_id=root.asset_lane_id
                      AND transition.terminal_revision_id=head.head_revision_id
                      AND transition.disposition=head.head_epistemic_state
                      AND EXISTS(SELECT 1 FROM attack_hypothesis_revisions source_revision
                           WHERE source_revision.revision_id=transition.source_revision_id
                             AND source_revision.root_id=root.root_id
                             AND source_revision.operation_id=root.operation_id
                             AND source_revision.organization_id=root.organization_id)))
                AS dynamically_resolved_root_count,
             (SELECT count(*) FROM attack_hypothesis_revisions revision
               WHERE revision.asset_lane_id=$1) AS revision_count,
             (SELECT count(*) FROM hypothesis_verification_tasks task
               WHERE task.asset_lane_id=$1) AS verification_task_count,
             (SELECT count(*) FROM hypothesis_verification_tasks task
                LEFT JOIN hypothesis_verification_task_state_heads head
                  ON head.task_id=task.task_id
               WHERE task.asset_lane_id=$1
                 AND (head.task_id IS NULL OR head.current_state<>'terminal'))
                AS open_verification_task_count,
             (SELECT count(*) FROM verification_campaigns campaign
               WHERE campaign.asset_lane_id=$1) AS campaign_count,
             (SELECT count(*) FROM verification_campaigns campaign
               WHERE campaign.asset_lane_id=$1 AND campaign.state<>'terminal')
                AS open_campaign_count,
             (SELECT count(*) FROM verification_prepared_actions action
                JOIN verification_campaigns campaign ON campaign.campaign_id=action.campaign_id
               WHERE campaign.asset_lane_id=$1) AS prepared_action_count,
             (SELECT count(*) FROM verification_prepared_actions action
                JOIN verification_campaigns campaign ON campaign.campaign_id=action.campaign_id
               WHERE campaign.asset_lane_id=$1
                 AND action.state IN('pending_authorization','authorized','started','outcome_unknown'))
                AS open_prepared_action_count,
             (SELECT count(*) FROM verification_action_executions execution
                JOIN verification_prepared_actions action
                  ON action.prepared_action_id=execution.prepared_action_id
                JOIN verification_campaigns campaign ON campaign.campaign_id=action.campaign_id
               WHERE campaign.asset_lane_id=$1) AS action_execution_count,
             (SELECT count(*) FROM verification_action_executions execution
                JOIN verification_prepared_actions action
                  ON action.prepared_action_id=execution.prepared_action_id
                JOIN verification_campaigns campaign ON campaign.campaign_id=action.campaign_id
               WHERE campaign.asset_lane_id=$1 AND execution.state='started')
                AS open_action_execution_count,
             (SELECT count(*) FROM verification_oracle_assessments oracle
                JOIN verification_campaigns campaign ON campaign.campaign_id=oracle.campaign_id
               WHERE campaign.asset_lane_id=$1) AS oracle_count,
             (SELECT count(*) FROM verification_fact_delta_bundles delta
                JOIN verification_campaigns campaign ON campaign.campaign_id=delta.campaign_id
               WHERE campaign.asset_lane_id=$1) AS fact_delta_count,
             (SELECT count(*) FROM verification_wave_coverage_denominators wave
               WHERE wave.asset_lane_id=$1 AND wave.sealed_at IS NOT NULL) AS wave_count,
             (SELECT count(*) FROM verification_wave_coverage_denominators wave
                JOIN verification_wave_coverage_receipts coverage
                  ON coverage.wave_denominator_id=wave.wave_denominator_id
                JOIN hypothesis_consolidation_batches batch
                  ON batch.wave_coverage_receipt_id=coverage.wave_coverage_receipt_id
                JOIN hypothesis_consolidation_receipts receipt
                  ON receipt.consolidation_batch_id=batch.consolidation_batch_id
                 AND receipt.disposition='advanced'
               WHERE wave.asset_lane_id=$1) AS advanced_wave_count,
             (SELECT count(*) FROM hypothesis_fixed_point_receipts fixed
               WHERE fixed.asset_lane_id=$1) AS fixed_point_wave_count,
             (SELECT count(*) FROM hypothesis_pending_evolution_authorities pending
                LEFT JOIN hypothesis_consolidation_receipts receipt
                  ON receipt.consolidation_batch_id=pending.consolidation_batch_id
               WHERE pending.asset_lane_id=$1 AND receipt.consolidation_receipt_id IS NULL)
                AS pending_evolution_count,
             (SELECT count(*) FROM investigation_pending_hypothesis_discovery_backlog discovery
               WHERE discovery.asset_lane_id=$1) AS pending_hypothesis_discovery_count"#,
    )
    .bind(request.asset_lane_id)
    .fetch_one(&mut **tx)
    .await?;
    let latest_generation: Option<(Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT generation.generation_id,seal.seal_id
             FROM hypothesis_generations generation
             JOIN hypothesis_generation_seals seal ON seal.generation_id=generation.generation_id
            WHERE generation.asset_lane_id=$1
            ORDER BY generation.generation_ordinal DESC,generation.generation_id DESC LIMIT 1"#,
    )
    .bind(request.asset_lane_id)
    .fetch_optional(&mut **tx)
    .await?;
    let zero_hypothesis_fixed_point_receipt_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT fixed.fixed_point_receipt_id
             FROM investigation_asset_zero_hypothesis_fixed_point_receipts fixed
            WHERE fixed.asset_lane_id=$1"#,
    )
    .bind(request.asset_lane_id)
    .fetch_optional(&mut **tx)
    .await?;
    let backlog_members: Vec<String> = sqlx::query_scalar(
        r#"SELECT member FROM (
             SELECT 'hypothesis_resolution:'||root.root_id::TEXT AS member
               FROM attack_hypotheses root
               LEFT JOIN attack_hypothesis_heads head
                 ON head.root_id=root.root_id
                AND head.operation_id=root.operation_id
                AND head.organization_id=root.organization_id
               LEFT JOIN attack_hypothesis_revisions revision
                 ON revision.revision_id=head.head_revision_id
                AND revision.root_id=head.root_id
                AND revision.operation_id=head.operation_id
                AND revision.organization_id=head.organization_id
              WHERE root.asset_lane_id=$1 AND (
                    head.root_id IS NULL OR revision.revision_id IS NULL OR NOT (
                    head.head_lifecycle_state='closed'
                AND head.head_epistemic_state IN('verified','refuted','invalid')
                AND revision.lifecycle_state='closed'
                AND revision.epistemic_state=head.head_epistemic_state
                AND EXISTS(
                    SELECT 1
                      FROM investigation_dynamic_hypothesis_terminal_transitions transition
                      JOIN investigation_dynamic_hypothesis_resolutions resolution
                        ON resolution.resolution_authority_id=transition.resolution_authority_id
                       AND resolution.hypothesis_revision_id=transition.source_revision_id
                       AND resolution.asset_lane_id=root.asset_lane_id
                       AND resolution.disposition=transition.disposition
                      JOIN investigation_dynamic_verification_rounds dynamic_round
                        ON dynamic_round.session_id=resolution.session_id
                       AND dynamic_round.operation_id=root.operation_id
                       AND dynamic_round.organization_id=root.organization_id
                       AND dynamic_round.asset_lane_id=root.asset_lane_id
                       AND dynamic_round.hypothesis_revision_id=transition.source_revision_id
                       AND dynamic_round.state='resolved'
                       AND dynamic_round.resolution_authority_id=resolution.resolution_authority_id
                      JOIN attack_hypothesis_state_events event
                        ON event.event_id=transition.state_event_id
                       AND event.predecessor_revision_id=transition.source_revision_id
                       AND event.successor_revision_id=transition.terminal_revision_id
                       AND event.origin_authority='dynamic_verification_resolution'
                       AND event.authority_receipt_kind='dynamic_resolution'
                       AND event.authority_receipt_id=resolution.resolution_authority_id
                       AND event.authority_receipt_hash=resolution.resolution_sha256
                     WHERE transition.asset_lane_id=root.asset_lane_id
                       AND transition.terminal_revision_id=head.head_revision_id
                       AND transition.disposition=head.head_epistemic_state
                       AND EXISTS(SELECT 1 FROM attack_hypothesis_revisions source_revision
                            WHERE source_revision.revision_id=transition.source_revision_id
                              AND source_revision.root_id=root.root_id
                              AND source_revision.operation_id=root.operation_id
                              AND source_revision.organization_id=root.organization_id))
              ))
             UNION ALL SELECT 'pending_hypothesis_discovery:'||discovery.discovery_authority_id::TEXT
               FROM investigation_pending_hypothesis_discovery_backlog discovery
              WHERE discovery.asset_lane_id=$1
           ) members ORDER BY member"#,
    )
    .bind(request.asset_lane_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut backlog_hashes = Vec::with_capacity(backlog_members.len());
    for member in &backlog_members {
        backlog_hashes.push(sha256_on(tx, member).await?);
    }
    let backlog_set_sha256 =
        set_sha256_on(tx, "golish.investigation.asset_backlog.v1", &backlog_hashes).await?;
    let obligation_set_sha256 =
        set_sha256_on(tx, "golish.investigation.asset_obligations.v1", &[]).await?;
    let residual_set_sha256 =
        set_sha256_on(tx, "golish.investigation.asset_residuals.v1", &[]).await?;
    let row = InvestigationAssetBacklogRow {
        asset_lane,
        latest_generation_id: latest_generation.map(|row| row.0),
        latest_generation_seal_id: latest_generation.map(|row| row.1),
        generation_count: census.generation_count,
        hypothesis_root_count: census.hypothesis_root_count,
        dynamically_resolved_root_count: census.dynamically_resolved_root_count,
        revision_count: census.revision_count,
        verification_task_count: census.verification_task_count,
        open_verification_task_count: census.open_verification_task_count,
        campaign_count: census.campaign_count,
        open_campaign_count: census.open_campaign_count,
        prepared_action_count: census.prepared_action_count,
        open_prepared_action_count: census.open_prepared_action_count,
        action_execution_count: census.action_execution_count,
        open_action_execution_count: census.open_action_execution_count,
        oracle_count: census.oracle_count,
        fact_delta_count: census.fact_delta_count,
        wave_count: census.wave_count,
        advanced_wave_count: census.advanced_wave_count,
        fixed_point_wave_count: census.fixed_point_wave_count,
        pending_evolution_count: census.pending_evolution_count,
        pending_hypothesis_discovery_count: census.pending_hypothesis_discovery_count,
        backlog_member_count: i64::try_from(backlog_hashes.len())
            .map_err(|_| conflict(CONTRACT_INVALID))?,
        backlog_set_sha256,
        obligation_set_sha256,
        residual_set_sha256,
        zero_hypothesis_fixed_point_receipt_id,
    };
    Ok(row)
}

pub async fn load_asset_backlog(
    pool: &PgPool,
    request: &LoadInvestigationAssetBacklogRow,
) -> Result<InvestigationAssetBacklogRow> {
    let mut tx = pool.begin().await?;
    let row = load_asset_backlog_on(&mut tx, request).await?;
    tx.commit().await?;
    Ok(row)
}

/// Load the single open FactDelta evolution authority for the exact current
/// asset Analysis epoch. The selector derives the authority id from durable
/// lane/generation truth; callers cannot nominate one.
pub async fn load_current_evolution_authority(
    pool: &PgPool,
    request: &LoadCurrentInvestigationAssetEvolutionAuthorityRow,
) -> Result<InvestigationAssetEvolutionAuthorityRow> {
    for value in [
        request.operation_id,
        request.stage_execution_id,
        request.scope_snapshot_id,
        request.organization_id,
        request.asset_lane_id,
    ] {
        validate_uuid(value)?;
    }
    if request.expected_evolution_epoch <= 0 {
        return Err(conflict(CONTRACT_INVALID));
    }
    let rows = sqlx::query_as::<_, InvestigationAssetEvolutionAuthorityRow>(
        r#"SELECT lane.asset_lane_id,lane.evolution_epoch,
                  pending.pending_evolution_authority_id
             FROM investigation_asset_lanes lane
             JOIN operation_state operation
               ON operation.operation_id=lane.operation_id
              AND operation.superseded_by IS NULL
              AND operation.current_stage='investigation'
             JOIN operation_org_scope_snapshots snapshot
               ON snapshot.id=lane.scope_snapshot_id
              AND snapshot.operation_id=lane.operation_id
              AND snapshot.project_scope_id=operation.project_scope_id
              AND snapshot.sealed_at IS NOT NULL
             JOIN operation_org_scope_units scope_unit
               ON scope_unit.snapshot_id=snapshot.id
              AND scope_unit.organization_id=lane.organization_id
             JOIN hypothesis_pending_evolution_authorities pending
               ON pending.asset_lane_id=lane.asset_lane_id
              AND pending.operation_id=lane.operation_id
              AND pending.project_scope_id=operation.project_scope_id
              AND pending.organization_id=lane.organization_id
             JOIN hypothesis_generations generation
               ON generation.generation_id=pending.source_generation_id
              AND generation.operation_id=pending.operation_id
              AND generation.organization_id=pending.organization_id
              AND generation.asset_lane_id=lane.asset_lane_id
              AND generation.generation_ordinal+1=lane.evolution_epoch
            WHERE lane.asset_lane_id=$1
              AND lane.operation_id=$2
              AND lane.stage_execution_id=$3
              AND lane.scope_snapshot_id=$4
              AND lane.organization_id=$5
              AND lane.evolution_epoch=$6
              AND lane.state='analyzing'
              AND EXISTS(
                    SELECT 1 FROM hypothesis_generation_seals generation_seal
                     WHERE generation_seal.generation_id=generation.generation_id)
              AND EXISTS(
                    SELECT 1 FROM investigation_asset_primary_schedules schedule
                     WHERE schedule.asset_lane_id=lane.asset_lane_id
                       AND schedule.evolution_epoch=lane.evolution_epoch
                       AND schedule.schedule_contract='primary_dynamic_v2'
                       AND schedule.operation_id=lane.operation_id
                       AND schedule.stage_execution_id=lane.stage_execution_id
                       AND schedule.scope_snapshot_id=lane.scope_snapshot_id
                       AND schedule.organization_id=lane.organization_id
                       AND schedule.status='applied')
              AND NOT EXISTS(
                    SELECT 1 FROM hypothesis_consolidation_receipts terminal
                     WHERE terminal.consolidation_batch_id=pending.consolidation_batch_id)
            ORDER BY pending.pending_evolution_authority_id"#,
    )
    .bind(request.asset_lane_id)
    .bind(request.operation_id)
    .bind(request.stage_execution_id)
    .bind(request.scope_snapshot_id)
    .bind(request.organization_id)
    .bind(request.expected_evolution_epoch)
    .fetch_all(pool)
    .await?;
    let [row] = rows.as_slice() else {
        return Err(conflict(AUTHORITY_MISMATCH));
    };
    Ok(*row)
}

#[derive(Debug, sqlx::FromRow)]
struct DynamicResolutionBacklogMember {
    hypothesis_root_id: Uuid,
    source_revision_id: Uuid,
    terminal_revision_id: Uuid,
    dynamic_session_id: Uuid,
    resolution_authority_id: Uuid,
    terminal_transition_id: Uuid,
    state_event_id: Uuid,
    disposition: String,
}

async fn load_dynamic_resolution_backlog_members_on(
    tx: &mut Transaction<'_, Postgres>,
    asset_lane_id: Uuid,
) -> Result<Vec<DynamicResolutionBacklogMember>> {
    Ok(sqlx::query_as(
        r#"SELECT root.root_id AS hypothesis_root_id,
                  transition.source_revision_id,transition.terminal_revision_id,
                  dynamic_round.session_id AS dynamic_session_id,
                  resolution.resolution_authority_id,
                  transition.terminal_transition_id,transition.state_event_id,
                  transition.disposition
             FROM attack_hypotheses root
             JOIN attack_hypothesis_heads head
               ON head.root_id=root.root_id
              AND head.operation_id=root.operation_id
              AND head.organization_id=root.organization_id
             JOIN attack_hypothesis_revisions terminal
               ON terminal.revision_id=head.head_revision_id
              AND terminal.root_id=root.root_id
              AND terminal.operation_id=root.operation_id
              AND terminal.organization_id=root.organization_id
             JOIN investigation_dynamic_hypothesis_terminal_transitions transition
               ON transition.asset_lane_id=root.asset_lane_id
              AND transition.terminal_revision_id=terminal.revision_id
              AND transition.disposition=head.head_epistemic_state
             JOIN attack_hypothesis_revisions source_revision
               ON source_revision.revision_id=transition.source_revision_id
              AND source_revision.root_id=root.root_id
              AND source_revision.operation_id=root.operation_id
              AND source_revision.organization_id=root.organization_id
             JOIN investigation_dynamic_hypothesis_resolutions resolution
               ON resolution.resolution_authority_id=transition.resolution_authority_id
              AND resolution.hypothesis_revision_id=transition.source_revision_id
              AND resolution.asset_lane_id=root.asset_lane_id
              AND resolution.disposition=transition.disposition
             JOIN investigation_dynamic_verification_rounds dynamic_round
               ON dynamic_round.session_id=resolution.session_id
              AND dynamic_round.operation_id=root.operation_id
              AND dynamic_round.organization_id=root.organization_id
              AND dynamic_round.asset_lane_id=root.asset_lane_id
              AND dynamic_round.hypothesis_revision_id=source_revision.revision_id
              AND dynamic_round.state='resolved'
              AND dynamic_round.resolution_authority_id=resolution.resolution_authority_id
             JOIN attack_hypothesis_state_events event
               ON event.event_id=transition.state_event_id
              AND event.predecessor_revision_id=transition.source_revision_id
              AND event.successor_revision_id=transition.terminal_revision_id
              AND event.origin_authority='dynamic_verification_resolution'
              AND event.authority_receipt_kind='dynamic_resolution'
              AND event.authority_receipt_id=resolution.resolution_authority_id
              AND event.authority_receipt_hash=resolution.resolution_sha256
            WHERE root.asset_lane_id=$1
              AND head.head_lifecycle_state='closed'
              AND head.head_epistemic_state IN('verified','refuted','invalid')
              AND terminal.lifecycle_state='closed'
              AND terminal.epistemic_state=head.head_epistemic_state
            ORDER BY root.root_id
            FOR SHARE OF root,head,terminal,transition,resolution,dynamic_round,event"#,
    )
    .bind(asset_lane_id)
    .fetch_all(&mut **tx)
    .await?)
}

pub async fn close_asset_backlog_and_advance(
    pool: &PgPool,
    request: &CloseInvestigationAssetBacklogAndAdvanceRow,
) -> Result<InvestigationAssetProgressionRow> {
    for value in [
        request.stable_request_id,
        request.company_queue_id,
        request.company_member_id,
        request.asset_queue_id,
        request.asset_lane_id,
        request.operation_id,
        request.scope_snapshot_id,
        request.organization_id,
    ] {
        validate_uuid(value)?;
    }
    if request.expected_company_queue_head_version < 0
        || request.expected_company_member_row_version < 0
        || request.expected_asset_queue_head_version < 0
        || request.expected_asset_lane_row_version < 0
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let request_fingerprint = sha256_on(&mut tx, &format!("{request:?}")).await?;
    if let Some(existing) = load_progression_replay_on(
        &mut tx,
        request.stable_request_id,
        &request_fingerprint,
        request,
    )
    .await?
    {
        tx.commit().await?;
        return Ok(existing);
    }

    #[derive(sqlx::FromRow)]
    struct CompanyQueueLock {
        authority_id: Uuid,
        stage_execution_id: Uuid,
        owning_stage_run_request_id: String,
        member_count: i64,
        head_version: i64,
        state: String,
    }
    let company_queue = sqlx::query_as::<_, CompanyQueueLock>(
        r#"SELECT authority_id,stage_execution_id,owning_stage_run_request_id,
                  member_count,head_version,state FROM investigation_company_queues
            WHERE company_queue_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
            FOR UPDATE"#,
    )
    .bind(request.company_queue_id)
    .bind(request.operation_id)
    .bind(request.scope_snapshot_id)
    .fetch_one(&mut *tx)
    .await?;
    let company_member = load_company_on(&mut tx, request.company_member_id).await?;
    let asset_lane = load_lane_on(&mut tx, request.asset_lane_id).await?;
    let asset_queue: (i64, String) = sqlx::query_as(
        r#"SELECT head_version,state FROM investigation_asset_queues
            WHERE asset_queue_id=$1 AND company_queue_id=$2 AND company_member_id=$3
              AND operation_id=$4 AND scope_snapshot_id=$5 AND organization_id=$6
            FOR UPDATE"#,
    )
    .bind(request.asset_queue_id)
    .bind(request.company_queue_id)
    .bind(request.company_member_id)
    .bind(request.operation_id)
    .bind(request.scope_snapshot_id)
    .bind(request.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    if company_queue.state != "open"
        || company_queue.head_version != request.expected_company_queue_head_version
        || company_member.company_queue_id != request.company_queue_id
        || company_member.organization_id != request.organization_id
        || company_member.state != "active"
        || company_member.row_version != request.expected_company_member_row_version
        || asset_queue.0 != request.expected_asset_queue_head_version
        || asset_lane.company_queue_id != request.company_queue_id
        || asset_lane.company_member_id != request.company_member_id
        || asset_lane.asset_queue_id != request.asset_queue_id
        || asset_lane.operation_id != request.operation_id
        || asset_lane.scope_snapshot_id != request.scope_snapshot_id
        || asset_lane.organization_id != request.organization_id
        || asset_lane.row_version != request.expected_asset_lane_row_version
    {
        return Err(conflict(CAS_CONFLICT));
    }

    let backlog_request = LoadInvestigationAssetBacklogRow {
        company_queue_id: request.company_queue_id,
        company_member_id: request.company_member_id,
        asset_queue_id: request.asset_queue_id,
        asset_lane_id: request.asset_lane_id,
        operation_id: request.operation_id,
        scope_snapshot_id: request.scope_snapshot_id,
        organization_id: request.organization_id,
    };
    let backlog = load_asset_backlog_on(&mut tx, &backlog_request).await?;
    let (source_fixed_point_receipt_id, source_zero_fixed_point_receipt_id, fixed_event_id) =
        if asset_lane.state == "consolidating" {
            if backlog.hypothesis_root_count == 0
                || backlog.revision_count == 0
                || backlog.backlog_member_count != 0
                || backlog.pending_hypothesis_discovery_count != 0
            {
                return Err(conflict("INVESTIGATION_ASSET_BACKLOG_NOT_DRAINED"));
            }
            let receipt_id = Uuid::new_v5(
                &request.stable_request_id,
                b"investigation-asset-backlog-fixed-point.v1",
            );
            let dynamic_members =
                load_dynamic_resolution_backlog_members_on(&mut tx, request.asset_lane_id).await?;
            if i64::try_from(dynamic_members.len()).map_err(|_| conflict(CONTRACT_INVALID))?
                != backlog.hypothesis_root_count
                || backlog.dynamically_resolved_root_count != backlog.hypothesis_root_count
            {
                return Err(conflict("INVESTIGATION_ASSET_BACKLOG_NOT_DRAINED"));
            }
            let mut dynamic_member_hashes = Vec::with_capacity(dynamic_members.len());
            for member in &dynamic_members {
                dynamic_member_hashes.push(
                    sha256_on(
                        &mut tx,
                        &format!(
                            "golish.investigation.asset_backlog.dynamic_resolution_member.v1:{receipt_id}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                            request.asset_lane_id,
                            member.hypothesis_root_id,
                            member.source_revision_id,
                            member.terminal_revision_id,
                            member.dynamic_session_id,
                            member.resolution_authority_id,
                            member.terminal_transition_id,
                            member.state_event_id,
                            member.disposition,
                            request.operation_id,
                        ),
                    )
                    .await?,
                );
            }
            let dynamic_member_set_sha256 = set_sha256_on(
                &mut tx,
                "golish.investigation.asset_backlog.dynamic_resolution_members.v1",
                &dynamic_member_hashes,
            )
            .await?;
            let receipt_sha256 = sha256_on(
                &mut tx,
                &format!(
                    "{}:{}:{}:{}:{}:{}:{}:{}",
                    request_fingerprint,
                    backlog.hypothesis_root_count,
                    backlog.revision_count,
                    backlog.backlog_set_sha256,
                    backlog.obligation_set_sha256,
                    backlog.residual_set_sha256,
                    dynamic_members.len(),
                    dynamic_member_set_sha256,
                ),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO investigation_asset_backlog_fixed_point_receipts(
                       fixed_point_receipt_id,stable_request_id,asset_lane_id,asset_queue_id,
                       company_queue_id,company_member_id,operation_id,scope_snapshot_id,
                       organization_id,generation_count,hypothesis_root_count,revision_count,
                       verification_task_count,campaign_count,prepared_action_count,
                       action_execution_count,oracle_count,fact_delta_count,wave_count,
                       advanced_wave_count,fixed_point_wave_count,backlog_member_count,
                       backlog_set_sha256,obligation_set_sha256,residual_set_sha256,
                       request_fingerprint_sha256,receipt_sha256,
                       dynamic_resolution_member_count,dynamic_resolution_member_set_sha256)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                          $18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29)"#,
            )
            .bind(receipt_id)
            .bind(request.stable_request_id)
            .bind(request.asset_lane_id)
            .bind(request.asset_queue_id)
            .bind(request.company_queue_id)
            .bind(request.company_member_id)
            .bind(request.operation_id)
            .bind(request.scope_snapshot_id)
            .bind(request.organization_id)
            .bind(backlog.generation_count)
            .bind(backlog.hypothesis_root_count)
            .bind(backlog.revision_count)
            .bind(backlog.verification_task_count)
            .bind(backlog.campaign_count)
            .bind(backlog.prepared_action_count)
            .bind(backlog.action_execution_count)
            .bind(backlog.oracle_count)
            .bind(backlog.fact_delta_count)
            .bind(backlog.wave_count)
            .bind(backlog.advanced_wave_count)
            .bind(backlog.fixed_point_wave_count)
            .bind(backlog.backlog_member_count)
            .bind(&backlog.backlog_set_sha256)
            .bind(&backlog.obligation_set_sha256)
            .bind(&backlog.residual_set_sha256)
            .bind(&request_fingerprint)
            .bind(receipt_sha256)
            .bind(i64::try_from(dynamic_members.len()).map_err(|_| conflict(CONTRACT_INVALID))?)
            .bind(&dynamic_member_set_sha256)
            .execute(&mut *tx)
            .await?;
            for (member, member_sha256) in dynamic_members.iter().zip(dynamic_member_hashes) {
                sqlx::query(
                    r#"INSERT INTO investigation_asset_backlog_dynamic_resolution_members(
                           member_id,fixed_point_receipt_id,asset_lane_id,hypothesis_root_id,
                           source_revision_id,terminal_revision_id,dynamic_session_id,
                           resolution_authority_id,terminal_transition_id,state_event_id,
                           disposition,member_sha256)
                       VALUES(uuid_generate_v5($1,$2::TEXT),$1,$3,$2,$4,$5,$6,$7,$8,$9,$10,$11)"#,
                )
                .bind(receipt_id)
                .bind(member.hypothesis_root_id)
                .bind(request.asset_lane_id)
                .bind(member.source_revision_id)
                .bind(member.terminal_revision_id)
                .bind(member.dynamic_session_id)
                .bind(member.resolution_authority_id)
                .bind(member.terminal_transition_id)
                .bind(member.state_event_id)
                .bind(&member.disposition)
                .bind(member_sha256)
                .execute(&mut *tx)
                .await?;
            }
            insert_asset_event_on(
                &mut tx,
                request.stable_request_id,
                request.company_queue_id,
                request.company_member_id,
                request.asset_queue_id,
                request.asset_lane_id,
                request.operation_id,
                request.scope_snapshot_id,
                request.organization_id,
                request.expected_asset_queue_head_version,
                request.expected_asset_lane_row_version,
                "consolidating",
                "fixed_point",
                "fixed_point",
            )
            .await?;
            (
                Some(receipt_id),
                None,
                Uuid::new_v5(&request.stable_request_id, b"asset-lane-event.v1"),
            )
        } else if asset_lane.state == "fixed_point" {
            let zero_receipt_id = backlog
                .zero_hypothesis_fixed_point_receipt_id
                .ok_or_else(|| conflict("INVESTIGATION_ASSET_BACKLOG_FIXED_AUTHORITY_MISMATCH"))?;
            let latest_event_id: Uuid = sqlx::query_scalar(
                "SELECT latest_event_id FROM investigation_asset_lanes WHERE asset_lane_id=$1",
            )
            .bind(request.asset_lane_id)
            .fetch_one(&mut *tx)
            .await?;
            (None, Some(zero_receipt_id), latest_event_id)
        } else {
            return Err(conflict("INVESTIGATION_ASSET_BACKLOG_NOT_DRAINED"));
        };

    let next_asset_same_company: Option<(Uuid, i64)> = sqlx::query_as(
        r#"SELECT asset_lane_id,row_version FROM investigation_asset_lanes
            WHERE asset_queue_id=$1 AND state='queued'
            ORDER BY target_created_at,target_value_at_freeze,target_id LIMIT 1"#,
    )
    .bind(request.asset_queue_id)
    .fetch_optional(&mut *tx)
    .await?;
    let mut next_company_member_id = None;
    let mut next_asset_lane_id = None;
    let mut next_company_claim_event_id = None;
    let mut next_asset_claim_event_id = None;
    let mut auto_completed_company_count = 0_i64;
    let disposition;
    if let Some((lane_id, lane_row_version)) = next_asset_same_company {
        let queue_head: i64 = sqlx::query_scalar(
            "SELECT head_version FROM investigation_asset_queues WHERE asset_queue_id=$1",
        )
        .bind(request.asset_queue_id)
        .fetch_one(&mut *tx)
        .await?;
        let stable = derived_stable_request(request.stable_request_id, "claim-next-asset", lane_id);
        insert_asset_event_on(
            &mut tx,
            stable,
            request.company_queue_id,
            request.company_member_id,
            request.asset_queue_id,
            lane_id,
            request.operation_id,
            request.scope_snapshot_id,
            request.organization_id,
            queue_head,
            lane_row_version,
            "queued",
            "analyzing",
            "claim",
        )
        .await?;
        next_company_member_id = Some(request.company_member_id);
        next_asset_lane_id = Some(lane_id);
        next_asset_claim_event_id = Some(Uuid::new_v5(&stable, b"asset-lane-event.v1"));
        disposition = InvestigationAssetProgressionDispositionRow::NextAsset;
    } else {
        let complete_stable = derived_stable_request(
            request.stable_request_id,
            "complete-company",
            request.company_member_id,
        );
        insert_company_event_on(
            &mut tx,
            complete_stable,
            request.company_queue_id,
            request.company_member_id,
            request.operation_id,
            request.scope_snapshot_id,
            request.organization_id,
            request.expected_company_queue_head_version,
            request.expected_company_member_row_version,
            "active",
            "completed",
        )
        .await?;
        let mut chosen = None;
        for _ in 0..company_queue.member_count {
            let next_company: Option<(Uuid, Uuid, i64)> = sqlx::query_as(
                r#"SELECT company_member_id,organization_id,row_version
                    FROM investigation_company_queue_members
                    WHERE company_queue_id=$1 AND state='queued'
                    ORDER BY depth,ordinal,organization_id LIMIT 1"#,
            )
            .bind(request.company_queue_id)
            .fetch_optional(&mut *tx)
            .await?;
            let Some((member_id, organization_id, member_row_version)) = next_company else {
                break;
            };
            let company_head: i64 = sqlx::query_scalar(
                "SELECT head_version FROM investigation_company_queues WHERE company_queue_id=$1",
            )
            .bind(request.company_queue_id)
            .fetch_one(&mut *tx)
            .await?;
            let company_claim_stable =
                derived_stable_request(request.stable_request_id, "claim-next-company", member_id);
            let company_claim_event_id = insert_company_event_on(
                &mut tx,
                company_claim_stable,
                request.company_queue_id,
                member_id,
                request.operation_id,
                request.scope_snapshot_id,
                organization_id,
                company_head,
                member_row_version,
                "queued",
                "active",
            )
            .await?;
            let asset_queue_for_company: (Uuid, i64) = sqlx::query_as(
                r#"SELECT asset_queue_id,head_version FROM investigation_asset_queues
                    WHERE company_queue_id=$1 AND company_member_id=$2"#,
            )
            .bind(request.company_queue_id)
            .bind(member_id)
            .fetch_one(&mut *tx)
            .await?;
            let first_asset: Option<(Uuid, i64)> = sqlx::query_as(
                r#"SELECT asset_lane_id,row_version FROM investigation_asset_lanes
                    WHERE asset_queue_id=$1 AND state='queued'
                    ORDER BY target_created_at,target_value_at_freeze,target_id LIMIT 1"#,
            )
            .bind(asset_queue_for_company.0)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some((lane_id, lane_row_version)) = first_asset {
                let asset_claim_stable = derived_stable_request(
                    request.stable_request_id,
                    "claim-next-company-asset",
                    lane_id,
                );
                insert_asset_event_on(
                    &mut tx,
                    asset_claim_stable,
                    request.company_queue_id,
                    member_id,
                    asset_queue_for_company.0,
                    lane_id,
                    request.operation_id,
                    request.scope_snapshot_id,
                    organization_id,
                    asset_queue_for_company.1,
                    lane_row_version,
                    "queued",
                    "analyzing",
                    "claim",
                )
                .await?;
                chosen = Some((
                    member_id,
                    lane_id,
                    company_claim_event_id,
                    Uuid::new_v5(&asset_claim_stable, b"asset-lane-event.v1"),
                ));
                break;
            }
            let active_member = load_company_on(&mut tx, member_id).await?;
            let empty_complete_stable = derived_stable_request(
                request.stable_request_id,
                "complete-empty-company",
                member_id,
            );
            insert_company_event_on(
                &mut tx,
                empty_complete_stable,
                request.company_queue_id,
                member_id,
                request.operation_id,
                request.scope_snapshot_id,
                organization_id,
                active_member.company_queue_head_version,
                active_member.row_version,
                "active",
                "completed",
            )
            .await?;
            auto_completed_company_count += 1;
        }
        if let Some((member_id, lane_id, company_event_id, asset_event_id)) = chosen {
            next_company_member_id = Some(member_id);
            next_asset_lane_id = Some(lane_id);
            next_company_claim_event_id = Some(company_event_id);
            next_asset_claim_event_id = Some(asset_event_id);
            disposition = InvestigationAssetProgressionDispositionRow::NextCompany;
        } else {
            disposition = InvestigationAssetProgressionDispositionRow::InvestigationComplete;
        }
    }

    let result_company_queue_head_version: i64 = sqlx::query_scalar(
        "SELECT head_version FROM investigation_company_queues WHERE company_queue_id=$1",
    )
    .bind(request.company_queue_id)
    .fetch_one(&mut *tx)
    .await?;
    let stage_closure_publication_id = if disposition
        == InvestigationAssetProgressionDispositionRow::InvestigationComplete
    {
        #[derive(sqlx::FromRow)]
        struct RuntimeClosureMember {
            company_member_id: Uuid,
            organization_id: Uuid,
            stage_run_unit_id: Uuid,
            unit_row_version: i64,
            stage_team_plan_id: Uuid,
            plan_row_version: i64,
            plan_requests_closed_at: Option<DateTime<Utc>>,
            final_submitter_worker_run_id: Option<Uuid>,
        }
        let publication_id = Uuid::new_v5(
            &request.stable_request_id,
            b"investigation-asset-queue-closure-publication.v1",
        );
        let published_at: DateTime<Utc> = sqlx::query_scalar("SELECT statement_timestamp()")
            .fetch_one(&mut *tx)
            .await?;
        let closure_members = sqlx::query_as::<_, RuntimeClosureMember>(
            r#"SELECT company.company_member_id,company.organization_id,
                      unit.id AS stage_run_unit_id,unit.row_version AS unit_row_version,
                      plan.id AS stage_team_plan_id,plan.row_version AS plan_row_version,
                      plan.requests_closed_at AS plan_requests_closed_at,
                      plan.final_submitter_worker_run_id
                 FROM investigation_company_queue_members company
                 JOIN stage_run_units unit
                   ON unit.operation_id=company.operation_id
                  AND unit.stage_execution_id=$2
                  AND unit.scope_snapshot_id=company.scope_snapshot_id
                  AND unit.organization_id=company.organization_id
                  AND unit.stage_kind='investigation'
                  AND unit.status='running'
                 JOIN stage_team_plans plan
                   ON plan.stage_run_unit_id=unit.id
                  AND plan.operation_id=unit.operation_id
                  AND plan.stage_execution_id=unit.stage_execution_id
                  AND plan.scope_snapshot_id=unit.scope_snapshot_id
                  AND plan.organization_id=unit.organization_id
                  AND plan.stage_kind='investigation'
                WHERE company.company_queue_id=$1 AND company.state='completed'
                ORDER BY company.depth,company.ordinal,company.organization_id
                FOR UPDATE OF company,unit,plan"#,
        )
        .bind(request.company_queue_id)
        .bind(company_queue.stage_execution_id)
        .fetch_all(&mut *tx)
        .await?;
        if i64::try_from(closure_members.len()).map_err(|_| conflict(CONTRACT_INVALID))?
            != company_queue.member_count
        {
            return Err(conflict("INVESTIGATION_ASSET_QUEUE_CLOSURE_MEMBER_DRIFT"));
        }
        let mut member_hashes = Vec::with_capacity(closure_members.len());
        for member in &closure_members {
            member_hashes.push(
                    sha256_on(
                        &mut tx,
                        &format!(
                            "golish.investigation.asset_queue_closure_member.v1:{publication_id}:{}:{}:{}:{}",
                            member.company_member_id,
                            member.organization_id,
                            member.stage_run_unit_id,
                            member.stage_team_plan_id
                        ),
                    )
                    .await?,
                );
        }
        let member_set_sha256 = set_sha256_on(
            &mut tx,
            "golish.investigation.asset_queue_closure_members.v1",
            &member_hashes,
        )
        .await?;
        let publication_sha256 = sha256_on(
                &mut tx,
                &format!(
                    "golish.investigation.asset_queue_closure_publication.v1:{publication_id}:{}:{}:{}:{}:{}:{}",
                    request.company_queue_id,
                    company_queue.authority_id,
                    request.operation_id,
                    company_queue.stage_execution_id,
                    request.scope_snapshot_id,
                    member_set_sha256
                ),
            )
            .await?;
        sqlx::query(
                r#"INSERT INTO investigation_asset_queue_closure_publications(
                       publication_id,stable_request_id,company_queue_id,authority_id,
                       operation_id,stage_execution_id,owning_stage_run_request_id,
                       scope_snapshot_id,member_count,member_set_sha256,publication_sha256,published_at)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
            )
            .bind(publication_id)
            .bind(request.stable_request_id)
            .bind(request.company_queue_id)
            .bind(company_queue.authority_id)
            .bind(request.operation_id)
            .bind(company_queue.stage_execution_id)
            .bind(&company_queue.owning_stage_run_request_id)
            .bind(request.scope_snapshot_id)
            .bind(company_queue.member_count)
            .bind(&member_set_sha256)
            .bind(publication_sha256)
            .bind(published_at)
            .execute(&mut *tx)
            .await?;
        for (ordinal, (member, member_sha256)) in
            closure_members.iter().zip(member_hashes.iter()).enumerate()
        {
            if member.plan_requests_closed_at.is_none() {
                if member.final_submitter_worker_run_id.is_some() {
                    return Err(conflict(
                        "INVESTIGATION_ASSET_QUEUE_PLAN_CLOSE_AUTHORITY_MISMATCH",
                    ));
                }
                let closed_plan_id: Option<Uuid> = sqlx::query_scalar(
                    r#"UPDATE stage_team_plans
                          SET requests_closed_at=$2,row_version=row_version+1,updated_at=$2
                        WHERE id=$1 AND row_version=$3 AND requests_closed_at IS NULL
                          AND final_submitter_worker_run_id IS NULL
                        RETURNING id"#,
                )
                .bind(member.stage_team_plan_id)
                .bind(published_at)
                .bind(member.plan_row_version)
                .fetch_optional(&mut *tx)
                .await?;
                if closed_plan_id != Some(member.stage_team_plan_id) {
                    return Err(conflict(CAS_CONFLICT));
                }
            }
            let pass_watermark = serde_json::json!({
                "schema": "investigation_asset_queue_closure_publication.v1",
                "publication_id": publication_id,
                "company_queue_id": request.company_queue_id,
                "company_member_id": member.company_member_id,
                "member_sha256": member_sha256,
            });
            let passed_unit_id: Option<Uuid> = sqlx::query_scalar(
                r#"UPDATE stage_run_units
                      SET status='passed',pass_watermark=$2,row_version=row_version+1,
                          terminal_at=$3,updated_at=$3
                    WHERE id=$1 AND row_version=$4 AND status='running'
                    RETURNING id"#,
            )
            .bind(member.stage_run_unit_id)
            .bind(&pass_watermark)
            .bind(published_at)
            .bind(member.unit_row_version)
            .fetch_optional(&mut *tx)
            .await?;
            if passed_unit_id != Some(member.stage_run_unit_id) {
                return Err(conflict(CAS_CONFLICT));
            }
            let member_id = Uuid::new_v5(
                &publication_id,
                format!("asset-queue-closure-member:{}", member.company_member_id).as_bytes(),
            );
            sqlx::query(
                r#"INSERT INTO investigation_asset_queue_closure_publication_members(
                           publication_member_id,publication_id,member_ordinal,company_queue_id,
                           company_member_id,operation_id,stage_execution_id,scope_snapshot_id,
                           organization_id,stage_run_unit_id,stage_team_plan_id,
                           member_sha256,passed_at)
                       VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
            )
            .bind(member_id)
            .bind(publication_id)
            .bind(i32::try_from(ordinal).map_err(|_| conflict(CONTRACT_INVALID))?)
            .bind(request.company_queue_id)
            .bind(member.company_member_id)
            .bind(request.operation_id)
            .bind(company_queue.stage_execution_id)
            .bind(request.scope_snapshot_id)
            .bind(member.organization_id)
            .bind(member.stage_run_unit_id)
            .bind(member.stage_team_plan_id)
            .bind(member_sha256)
            .bind(published_at)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                r#"INSERT INTO org_stage_completions(
                           organization_id,stage_kind,passed_at,stage_run_id,updated_at)
                       VALUES($1,'investigation',$2,$3,$2)
                       ON CONFLICT(organization_id,stage_kind) DO UPDATE
                         SET passed_at=EXCLUDED.passed_at,stage_run_id=EXCLUDED.stage_run_id,
                             updated_at=EXCLUDED.updated_at"#,
            )
            .bind(member.organization_id)
            .bind(published_at)
            .bind(request.operation_id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        Some(publication_id)
    } else {
        None
    };
    let progression_receipt_id = Uuid::new_v5(
        &request.stable_request_id,
        b"investigation-asset-progression.v1",
    );
    let disposition_text = progression_disposition_text(disposition);
    let receipt_sha256 = sha256_on(
        &mut tx,
        &format!(
            "{}:{}:{}:{:?}:{:?}:{:?}:{}",
            request_fingerprint,
            fixed_event_id,
            disposition_text,
            next_company_member_id,
            next_asset_lane_id,
            stage_closure_publication_id,
            result_company_queue_head_version
        ),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_asset_progression_receipts(
               progression_receipt_id,stable_request_id,source_fixed_point_receipt_id,
               source_zero_fixed_point_receipt_id,fixed_asset_lane_id,fixed_asset_event_id,
               company_queue_id,company_member_id,asset_queue_id,operation_id,scope_snapshot_id,
               organization_id,expected_company_queue_head_version,
               expected_company_member_row_version,expected_asset_queue_head_version,
               expected_asset_lane_row_version,disposition,next_company_member_id,
               next_asset_lane_id,next_company_claim_event_id,next_asset_claim_event_id,
               auto_completed_company_count,result_company_queue_head_version,
               stage_closure_publication_id,
               request_fingerprint_sha256,receipt_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
                  $19,$20,$21,$22,$23,$24,$25,$26)"#,
    )
    .bind(progression_receipt_id)
    .bind(request.stable_request_id)
    .bind(source_fixed_point_receipt_id)
    .bind(source_zero_fixed_point_receipt_id)
    .bind(request.asset_lane_id)
    .bind(fixed_event_id)
    .bind(request.company_queue_id)
    .bind(request.company_member_id)
    .bind(request.asset_queue_id)
    .bind(request.operation_id)
    .bind(request.scope_snapshot_id)
    .bind(request.organization_id)
    .bind(request.expected_company_queue_head_version)
    .bind(request.expected_company_member_row_version)
    .bind(request.expected_asset_queue_head_version)
    .bind(request.expected_asset_lane_row_version)
    .bind(disposition_text)
    .bind(next_company_member_id)
    .bind(next_asset_lane_id)
    .bind(next_company_claim_event_id)
    .bind(next_asset_claim_event_id)
    .bind(auto_completed_company_count)
    .bind(result_company_queue_head_version)
    .bind(stage_closure_publication_id)
    .bind(&request_fingerprint)
    .bind(receipt_sha256)
    .execute(&mut *tx)
    .await?;
    let next_asset_lane = match (next_asset_lane_id, next_asset_claim_event_id) {
        (Some(lane_id), Some(event_id)) => {
            Some(load_claimed_lane_projection_on(&mut tx, lane_id, event_id).await?)
        }
        (None, None) => None,
        _ => return Err(conflict(CONTRACT_INVALID)),
    };
    let stage_closure = match stage_closure_publication_id {
        Some(publication_id) => {
            Some(load_resolution_closure_publication_on(&mut tx, publication_id).await?)
        }
        None => None,
    };
    tx.commit().await?;
    Ok(InvestigationAssetProgressionRow {
        progression_receipt_id,
        fixed_asset_lane_id: request.asset_lane_id,
        disposition,
        next_company_member_id,
        next_asset_lane,
        company_queue_head_version: result_company_queue_head_version,
        stage_closure,
        replayed: false,
    })
}

pub async fn freeze_company_asset_queue(
    pool: &PgPool,
    request: &FreezeInvestigationCompanyAssetQueueRow,
) -> Result<InvestigationCompanyAssetQueueRow> {
    for value in [
        request.stable_request_id,
        request.authority_id,
        request.operation_id,
        request.stage_execution_id,
        request.scope_snapshot_id,
    ] {
        validate_uuid(value)?;
    }
    if request.owning_stage_run_request_id.trim().is_empty() || request.max_evolution_epochs < 0 {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let existing = sqlx::query_as::<_, QueueHeader>(
        r#"SELECT company_queue_id,stable_freeze_request_id,authority_id,operation_id,
                  stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
                  member_count,member_set_sha256,max_evolution_epochs,head_version
             FROM investigation_company_queues
            WHERE stable_freeze_request_id=$1
               OR (authority_id=$2 AND operation_id=$3 AND stage_execution_id=$4
                   AND scope_snapshot_id=$5)
            FOR UPDATE"#,
    )
    .bind(request.stable_request_id)
    .bind(request.authority_id)
    .bind(request.operation_id)
    .bind(request.stage_execution_id)
    .bind(request.scope_snapshot_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        if existing.stable_freeze_request_id != request.stable_request_id
            || existing.authority_id != request.authority_id
            || existing.operation_id != request.operation_id
            || existing.stage_execution_id != request.stage_execution_id
            || existing.owning_stage_run_request_id != request.owning_stage_run_request_id
            || existing.scope_snapshot_id != request.scope_snapshot_id
            || existing.max_evolution_epochs != request.max_evolution_epochs
        {
            return Err(conflict(REPLAY_DRIFT));
        }
        let row = load_queue_on(&mut tx, existing.company_queue_id, true).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let authority_valid: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM investigation_run_heads head
               JOIN operation_org_scope_snapshots snapshot
                 ON snapshot.id=head.scope_snapshot_id
                AND snapshot.operation_id=head.operation_id
              WHERE head.authority_id=$1 AND head.operation_id=$2
                AND head.stage_execution_id=$3 AND head.owning_stage_run_request_id=$4
                AND head.scope_snapshot_id=$5 AND head.run_state='running'
                AND head.admission_open AND snapshot.sealed_at IS NOT NULL
           )"#,
    )
    .bind(request.authority_id)
    .bind(request.operation_id)
    .bind(request.stage_execution_id)
    .bind(&request.owning_stage_run_request_id)
    .bind(request.scope_snapshot_id)
    .fetch_one(&mut *tx)
    .await?;
    if !authority_valid {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    #[derive(sqlx::FromRow)]
    struct CompanySource {
        organization_id: Uuid,
        organization_name_at_freeze: String,
        depth: i32,
        ordinal: i32,
    }
    let companies = sqlx::query_as::<_, CompanySource>(
        r#"SELECT organization_id,organization_name_at_freeze,depth,ordinal
             FROM operation_org_scope_units WHERE snapshot_id=$1
            ORDER BY depth,ordinal,organization_id"#,
    )
    .bind(request.scope_snapshot_id)
    .fetch_all(&mut *tx)
    .await?;
    if companies.is_empty() {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let mut company_hashes = Vec::with_capacity(companies.len());
    for company in &companies {
        company_hashes.push(
            sha256_on(
                &mut tx,
                &format!(
                    "golish.investigation.company_queue_member.v1:{}:{}:{}:{}:{}",
                    request.operation_id,
                    request.scope_snapshot_id,
                    company.organization_id,
                    company.depth,
                    company.ordinal
                ),
            )
            .await?,
        );
    }
    let member_set_sha256 = set_sha256_on(
        &mut tx,
        "golish.investigation.company_queue.v1",
        &company_hashes,
    )
    .await?;
    let company_queue_id = Uuid::new_v5(
        &request.stable_request_id,
        b"investigation-company-queue.v1",
    );
    sqlx::query(
        r#"INSERT INTO investigation_company_queues(
               company_queue_id,stable_freeze_request_id,authority_id,operation_id,
               stage_execution_id,owning_stage_run_request_id,scope_snapshot_id,
               member_count,member_set_sha256,max_evolution_epochs)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
    )
    .bind(company_queue_id)
    .bind(request.stable_request_id)
    .bind(request.authority_id)
    .bind(request.operation_id)
    .bind(request.stage_execution_id)
    .bind(&request.owning_stage_run_request_id)
    .bind(request.scope_snapshot_id)
    .bind(companies.len() as i64)
    .bind(&member_set_sha256)
    .bind(request.max_evolution_epochs)
    .execute(&mut *tx)
    .await?;
    for company in companies {
        let company_member_id = Uuid::new_v5(
            &company_queue_id,
            format!("company:{}", company.organization_id).as_bytes(),
        );
        sqlx::query(
            r#"INSERT INTO investigation_company_queue_members(
                   company_member_id,company_queue_id,authority_id,operation_id,
                   stage_execution_id,scope_snapshot_id,organization_id,
                   organization_name_at_freeze,depth,ordinal)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(company_member_id)
        .bind(company_queue_id)
        .bind(request.authority_id)
        .bind(request.operation_id)
        .bind(request.stage_execution_id)
        .bind(request.scope_snapshot_id)
        .bind(company.organization_id)
        .bind(&company.organization_name_at_freeze)
        .bind(company.depth)
        .bind(company.ordinal)
        .execute(&mut *tx)
        .await?;
        #[derive(sqlx::FromRow)]
        struct TargetSource {
            id: Uuid,
            target_type: String,
            value: String,
            source: String,
            created_at: DateTime<Utc>,
        }
        let targets = sqlx::query_as::<_, TargetSource>(
            r#"SELECT target.id,target.target_type::TEXT AS target_type,target.value,
                      target.source,target.created_at
                 FROM targets target
                 JOIN operation_org_scope_snapshots snapshot ON snapshot.id=$1
                WHERE target.organization_id=$2 AND target.scope='in'
                  AND target.project_path=snapshot.project_path_at_freeze
                ORDER BY target.created_at,target.value,target.id"#,
        )
        .bind(request.scope_snapshot_id)
        .bind(company.organization_id)
        .fetch_all(&mut *tx)
        .await?;
        let asset_queue_id = Uuid::new_v5(&company_member_id, b"investigation-asset-queue.v1");
        let mut asset_hashes = Vec::with_capacity(targets.len());
        for target in &targets {
            asset_hashes.push(
                sha256_on(
                    &mut tx,
                    &format!(
                        "golish.investigation.asset_queue_member.v1:{}:{}:{}:{}:{}:{}",
                        request.operation_id,
                        company.organization_id,
                        target.id,
                        target.target_type,
                        target.value,
                        target.created_at.to_rfc3339()
                    ),
                )
                .await?,
            );
        }
        let asset_set_sha256 = set_sha256_on(
            &mut tx,
            "golish.investigation.asset_queue.v1",
            &asset_hashes,
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO investigation_asset_queues(
                   asset_queue_id,company_queue_id,company_member_id,authority_id,
                   operation_id,stage_execution_id,scope_snapshot_id,organization_id,
                   member_count,member_set_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(asset_queue_id)
        .bind(company_queue_id)
        .bind(company_member_id)
        .bind(request.authority_id)
        .bind(request.operation_id)
        .bind(request.stage_execution_id)
        .bind(request.scope_snapshot_id)
        .bind(company.organization_id)
        .bind(targets.len() as i64)
        .bind(asset_set_sha256)
        .execute(&mut *tx)
        .await?;
        for (ordinal, target) in targets.into_iter().enumerate() {
            let asset_lane_id =
                Uuid::new_v5(&asset_queue_id, format!("asset:{}", target.id).as_bytes());
            let target_identity_sha256 = sha256_on(
                &mut tx,
                &format!(
                    "golish.investigation.asset_lane.v1:{}:{}:{}:{}:{}:{}",
                    request.operation_id,
                    company.organization_id,
                    target.id,
                    target.target_type,
                    target.value,
                    target.created_at.to_rfc3339()
                ),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO investigation_asset_lanes(
                       asset_lane_id,asset_queue_id,company_queue_id,company_member_id,
                       authority_id,operation_id,stage_execution_id,scope_snapshot_id,
                       organization_id,target_id,target_type_at_freeze,target_value_at_freeze,
                       target_source_at_freeze,target_created_at,target_identity_sha256,ordinal,
                       max_evolution_epochs)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
            )
            .bind(asset_lane_id)
            .bind(asset_queue_id)
            .bind(company_queue_id)
            .bind(company_member_id)
            .bind(request.authority_id)
            .bind(request.operation_id)
            .bind(request.stage_execution_id)
            .bind(request.scope_snapshot_id)
            .bind(company.organization_id)
            .bind(target.id)
            .bind(target.target_type)
            .bind(target.value)
            .bind(target.source)
            .bind(target.created_at)
            .bind(target_identity_sha256)
            .bind(ordinal as i32)
            .bind(request.max_evolution_epochs)
            .execute(&mut *tx)
            .await?;
        }
    }
    let row = load_queue_on(&mut tx, company_queue_id, false).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn claim_next_company(
    pool: &PgPool,
    request: &ClaimNextInvestigationCompanyRow,
) -> Result<InvestigationCompanyQueueMemberRow> {
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Uuid, i64, i64, String, String)>(
        r#"SELECT company_queue_id,company_member_id,operation_id,scope_snapshot_id,
                  expected_queue_head_version,expected_member_row_version,from_state,to_state
             FROM investigation_company_queue_events WHERE stable_request_id=$1"#,
    )
    .bind(request.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing
            != (
                request.company_queue_id,
                request.expected_company_member_id,
                request.operation_id,
                request.scope_snapshot_id,
                request.expected_queue_head_version,
                request.expected_member_row_version,
                "queued".to_string(),
                "active".to_string(),
            )
        {
            return Err(conflict(REPLAY_DRIFT));
        }
        let row = load_company_on(&mut tx, request.expected_company_member_id).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let event_id = Uuid::new_v5(&request.stable_request_id, b"company-claim.v1");
    let event_sha256 = sha256_on(&mut tx, &format!("{request:?}")).await?;
    sqlx::query(
        r#"INSERT INTO investigation_company_queue_events(
               event_id,stable_request_id,company_queue_id,company_member_id,
               operation_id,scope_snapshot_id,organization_id,event_ordinal,
               expected_queue_head_version,expected_member_row_version,
               from_state,to_state,event_sha256)
           SELECT $1,$2,$3,$4,$5,$6,member.organization_id,$7,$8,$9,
                  'queued','active',$10
             FROM investigation_company_queue_members member
            WHERE member.company_member_id=$4 AND member.company_queue_id=$3
              AND member.operation_id=$5 AND member.scope_snapshot_id=$6"#,
    )
    .bind(event_id)
    .bind(request.stable_request_id)
    .bind(request.company_queue_id)
    .bind(request.expected_company_member_id)
    .bind(request.operation_id)
    .bind(request.scope_snapshot_id)
    .bind(request.expected_queue_head_version + 1)
    .bind(request.expected_queue_head_version)
    .bind(request.expected_member_row_version)
    .bind(event_sha256)
    .execute(&mut *tx)
    .await?;
    let row = load_company_on(&mut tx, request.expected_company_member_id).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn claim_next_asset(
    pool: &PgPool,
    request: &ClaimNextInvestigationAssetRow,
) -> Result<InvestigationAssetLaneRow> {
    transition_asset_event(
        pool,
        request.stable_request_id,
        request.company_queue_id,
        request.company_member_id,
        request.asset_queue_id,
        request.expected_asset_lane_id,
        request.operation_id,
        request.scope_snapshot_id,
        request.organization_id,
        request.expected_queue_head_version,
        request.expected_lane_row_version,
        "queued",
        "analyzing",
        "claim",
    )
    .await
}

pub async fn transition_asset_lane(
    pool: &PgPool,
    request: &TransitionInvestigationAssetLaneRow,
) -> Result<InvestigationAssetLaneRow> {
    let event_kind = match (request.from_state, request.to_state) {
        ("analyzing", "verifying") => "verification_started",
        ("verifying", "consolidating") => "consolidation_started",
        ("consolidating", "evolving") => "evolution_requested",
        ("evolving", "analyzing") => "analysis_resumed",
        ("consolidating", "blocked") => "blocked",
        ("consolidating", "residual") => "residual",
        _ => return Err(conflict(CONTRACT_INVALID)),
    };
    transition_asset_event(
        pool,
        request.stable_request_id,
        request.company_queue_id,
        request.company_member_id,
        request.asset_queue_id,
        request.asset_lane_id,
        request.operation_id,
        request.scope_snapshot_id,
        request.organization_id,
        request.expected_queue_head_version,
        request.expected_lane_row_version,
        request.from_state,
        request.to_state,
        event_kind,
    )
    .await
}

pub async fn seal_zero_hypothesis_fixed_point(
    pool: &PgPool,
    request: &SealZeroHypothesisAssetFixedPointRow,
) -> Result<InvestigationAssetFixedPointReceiptRow> {
    let mut tx = pool.begin().await?;
    if let Some((
        receipt_id,
        lane_id,
        decision_id,
        generation_id,
        generation_seal_id,
        apply_receipt_id,
        backlog_hash,
        obligation_hash,
        residual_hash,
        receipt_sha256,
    )) = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            String,
            String,
            String,
            String,
        ),
    >(
        r#"SELECT fixed_point_receipt_id,asset_lane_id,compilation_decision_id,generation_id,
                  generation_seal_id,canonical_apply_receipt_id,backlog_set_sha256,
                  obligation_set_sha256,residual_set_sha256,receipt_sha256
                 FROM investigation_asset_zero_hypothesis_fixed_point_receipts
                WHERE stable_request_id=$1"#,
    )
    .bind(request.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let expected_receipt_sha256 = sha256_on(
            &mut tx,
            &format!(
                "{request:?}:{decision_id}:{generation_id}:{generation_seal_id}:{apply_receipt_id}:{backlog_hash}:{obligation_hash}:{residual_hash}"
            ),
        )
        .await?;
        if lane_id != request.asset_lane_id || receipt_sha256 != expected_receipt_sha256 {
            return Err(conflict(REPLAY_DRIFT));
        }
        let lane = load_lane_on(&mut tx, lane_id).await?;
        tx.commit().await?;
        return Ok(InvestigationAssetFixedPointReceiptRow {
            fixed_point_receipt_id: receipt_id,
            asset_lane: lane,
            receipt_sha256,
            replayed: true,
        });
    }
    let (compilation_decision_id, generation_id, generation_seal_id, canonical_apply_receipt_id): (
        Uuid,
        Uuid,
        Uuid,
        Uuid,
    ) = sqlx::query_as(
        r#"WITH latest_generation AS (
               SELECT generation_id,investigation_compilation_decision_id
                 FROM hypothesis_generations
                WHERE asset_lane_id=$1 AND operation_id=$2 AND organization_id=$3
                ORDER BY generation_ordinal DESC,generation_id DESC LIMIT 1
           )
           SELECT decision.decision_id,generation.generation_id,generation_seal.seal_id,
                  apply_receipt.apply_receipt_id
             FROM latest_generation latest
             JOIN hypothesis_generations generation
               ON generation.generation_id=latest.generation_id
             JOIN investigation_hypothesis_compilation_decisions decision
               ON decision.decision_id=latest.investigation_compilation_decision_id
              AND decision.operation_id=generation.operation_id
              AND decision.organization_id=generation.organization_id
              AND decision.proposal_count=0
             JOIN hypothesis_generation_seals generation_seal
               ON generation_seal.generation_id=generation.generation_id
              AND generation_seal.member_count=0
             JOIN investigation_hypothesis_canonical_apply_receipts apply_receipt
               ON apply_receipt.decision_id=decision.decision_id
              AND apply_receipt.generation_id=generation.generation_id
              AND apply_receipt.generation_seal_id=generation_seal.seal_id
              AND apply_receipt.revision_count=0
            WHERE NOT EXISTS(
                SELECT 1 FROM attack_hypotheses root WHERE root.asset_lane_id=$1
            )
            FOR SHARE OF generation,decision,generation_seal,apply_receipt"#,
    )
    .bind(request.asset_lane_id)
    .bind(request.operation_id)
    .bind(request.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    let backlog_set_sha256 =
        set_sha256_on(&mut tx, "golish.investigation.asset_backlog.v1", &[]).await?;
    let obligation_set_sha256 =
        set_sha256_on(&mut tx, "golish.investigation.asset_obligations.v1", &[]).await?;
    let residual_set_sha256 =
        set_sha256_on(&mut tx, "golish.investigation.asset_residuals.v1", &[]).await?;
    let receipt_id = Uuid::new_v5(&request.stable_request_id, b"asset-zero-fixed.v1");
    let receipt_sha256 = sha256_on(
        &mut tx,
        &format!(
            "{request:?}:{compilation_decision_id}:{generation_id}:{generation_seal_id}:{canonical_apply_receipt_id}:{backlog_set_sha256}:{obligation_set_sha256}:{residual_set_sha256}"
        ),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_asset_zero_hypothesis_fixed_point_receipts(
               fixed_point_receipt_id,stable_request_id,asset_lane_id,asset_queue_id,
               company_queue_id,company_member_id,operation_id,scope_snapshot_id,
               organization_id,compilation_decision_id,generation_id,generation_seal_id,
               canonical_apply_receipt_id,backlog_set_sha256,obligation_set_sha256,
               residual_set_sha256,receipt_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
    )
    .bind(receipt_id)
    .bind(request.stable_request_id)
    .bind(request.asset_lane_id)
    .bind(request.asset_queue_id)
    .bind(request.company_queue_id)
    .bind(request.company_member_id)
    .bind(request.operation_id)
    .bind(request.scope_snapshot_id)
    .bind(request.organization_id)
    .bind(compilation_decision_id)
    .bind(generation_id)
    .bind(generation_seal_id)
    .bind(canonical_apply_receipt_id)
    .bind(&backlog_set_sha256)
    .bind(&obligation_set_sha256)
    .bind(&residual_set_sha256)
    .bind(&receipt_sha256)
    .execute(&mut *tx)
    .await?;
    insert_asset_event_on(
        &mut tx,
        request.stable_request_id,
        request.company_queue_id,
        request.company_member_id,
        request.asset_queue_id,
        request.asset_lane_id,
        request.operation_id,
        request.scope_snapshot_id,
        request.organization_id,
        request.expected_queue_head_version,
        request.expected_lane_row_version,
        "analyzing",
        "fixed_point",
        "zero_hypothesis_fixed_point",
    )
    .await?;
    let lane = load_lane_on(&mut tx, request.asset_lane_id).await?;
    tx.commit().await?;
    Ok(InvestigationAssetFixedPointReceiptRow {
        fixed_point_receipt_id: receipt_id,
        asset_lane: lane,
        receipt_sha256,
        replayed: false,
    })
}

pub async fn complete_company(
    pool: &PgPool,
    request: &CompleteInvestigationCompanyRow,
) -> Result<InvestigationCompanyQueueMemberRow> {
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as::<
        _,
        (Uuid, Uuid, Uuid, Uuid, Uuid, i64, i64, String, String),
    >(
        r#"SELECT company_queue_id,company_member_id,operation_id,scope_snapshot_id,organization_id,
                  expected_queue_head_version,expected_member_row_version,from_state,to_state
             FROM investigation_company_queue_events WHERE stable_request_id=$1"#,
    )
    .bind(request.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing
            != (
                request.company_queue_id,
                request.company_member_id,
                request.operation_id,
                request.scope_snapshot_id,
                request.organization_id,
                request.expected_queue_head_version,
                request.expected_member_row_version,
                "active".to_string(),
                "completed".to_string(),
            )
        {
            return Err(conflict(REPLAY_DRIFT));
        }
        let row = load_company_on(&mut tx, request.company_member_id).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let event_id = Uuid::new_v5(&request.stable_request_id, b"company-complete.v1");
    let event_sha256 = sha256_on(&mut tx, &format!("{request:?}")).await?;
    sqlx::query(
        r#"INSERT INTO investigation_company_queue_events(
               event_id,stable_request_id,company_queue_id,company_member_id,
               operation_id,scope_snapshot_id,organization_id,event_ordinal,
               expected_queue_head_version,expected_member_row_version,
               from_state,to_state,event_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'active','completed',$11)"#,
    )
    .bind(event_id)
    .bind(request.stable_request_id)
    .bind(request.company_queue_id)
    .bind(request.company_member_id)
    .bind(request.operation_id)
    .bind(request.scope_snapshot_id)
    .bind(request.organization_id)
    .bind(request.expected_queue_head_version + 1)
    .bind(request.expected_queue_head_version)
    .bind(request.expected_member_row_version)
    .bind(event_sha256)
    .execute(&mut *tx)
    .await?;
    let row = load_company_on(&mut tx, request.company_member_id).await?;
    tx.commit().await?;
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
async fn transition_asset_event(
    pool: &PgPool,
    stable_request_id: Uuid,
    company_queue_id: Uuid,
    company_member_id: Uuid,
    asset_queue_id: Uuid,
    asset_lane_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    expected_queue_head_version: i64,
    expected_lane_row_version: i64,
    from_state: &'static str,
    to_state: &'static str,
    event_kind: &'static str,
) -> Result<InvestigationAssetLaneRow> {
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            String,
            String,
            String,
            i64,
            i64,
        ),
    >(
        r#"SELECT company_queue_id,company_member_id,asset_queue_id,asset_lane_id,
                  operation_id,scope_snapshot_id,organization_id,from_state,to_state,event_kind,
                  expected_queue_head_version,expected_lane_row_version
             FROM investigation_asset_lane_events WHERE stable_request_id=$1"#,
    )
    .bind(stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing
            != (
                company_queue_id,
                company_member_id,
                asset_queue_id,
                asset_lane_id,
                operation_id,
                scope_snapshot_id,
                organization_id,
                from_state.to_string(),
                to_state.to_string(),
                event_kind.to_string(),
                expected_queue_head_version,
                expected_lane_row_version,
            )
        {
            return Err(conflict(REPLAY_DRIFT));
        }
        let row = load_lane_on(&mut tx, asset_lane_id).await?;
        tx.commit().await?;
        return Ok(row);
    }
    insert_asset_event_on(
        &mut tx,
        stable_request_id,
        company_queue_id,
        company_member_id,
        asset_queue_id,
        asset_lane_id,
        operation_id,
        scope_snapshot_id,
        organization_id,
        expected_queue_head_version,
        expected_lane_row_version,
        from_state,
        to_state,
        event_kind,
    )
    .await?;
    let row = load_lane_on(&mut tx, asset_lane_id).await?;
    tx.commit().await?;
    Ok(row)
}

#[allow(clippy::too_many_arguments)]
async fn insert_company_event_on(
    tx: &mut Transaction<'_, Postgres>,
    stable_request_id: Uuid,
    company_queue_id: Uuid,
    company_member_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    expected_queue_head_version: i64,
    expected_member_row_version: i64,
    from_state: &'static str,
    to_state: &'static str,
) -> Result<Uuid> {
    let event_id = Uuid::new_v5(&stable_request_id, b"company-queue-event.v1");
    let event_sha256 = sha256_on(
        tx,
        &format!(
            "{stable_request_id}:{company_member_id}:{from_state}:{to_state}:{expected_queue_head_version}:{expected_member_row_version}"
        ),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_company_queue_events(
               event_id,stable_request_id,company_queue_id,company_member_id,
               operation_id,scope_snapshot_id,organization_id,event_ordinal,
               expected_queue_head_version,expected_member_row_version,
               from_state,to_state,event_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
    )
    .bind(event_id)
    .bind(stable_request_id)
    .bind(company_queue_id)
    .bind(company_member_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(expected_queue_head_version + 1)
    .bind(expected_queue_head_version)
    .bind(expected_member_row_version)
    .bind(from_state)
    .bind(to_state)
    .bind(event_sha256)
    .execute(&mut **tx)
    .await?;
    Ok(event_id)
}

#[allow(clippy::too_many_arguments)]
async fn insert_asset_event_on(
    tx: &mut Transaction<'_, Postgres>,
    stable_request_id: Uuid,
    company_queue_id: Uuid,
    company_member_id: Uuid,
    asset_queue_id: Uuid,
    asset_lane_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    expected_queue_head_version: i64,
    expected_lane_row_version: i64,
    from_state: &'static str,
    to_state: &'static str,
    event_kind: &'static str,
) -> Result<()> {
    let event_id = Uuid::new_v5(&stable_request_id, b"asset-lane-event.v1");
    let evolution_epoch: i32 = sqlx::query_scalar(
        "SELECT evolution_epoch FROM investigation_asset_lanes WHERE asset_lane_id=$1",
    )
    .bind(asset_lane_id)
    .fetch_one(&mut **tx)
    .await?;
    let evolution_epoch = if from_state == "consolidating" && to_state == "evolving" {
        evolution_epoch + 1
    } else {
        evolution_epoch
    };
    let event_sha256 = sha256_on(
        tx,
        &format!(
            "{stable_request_id}:{asset_lane_id}:{from_state}:{to_state}:{event_kind}:{evolution_epoch}"
        ),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_asset_lane_events(
               event_id,stable_request_id,asset_queue_id,asset_lane_id,
               company_queue_id,company_member_id,operation_id,scope_snapshot_id,
               organization_id,event_ordinal,expected_queue_head_version,
               expected_lane_row_version,from_state,to_state,event_kind,
               evolution_epoch,event_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
    )
    .bind(event_id)
    .bind(stable_request_id)
    .bind(asset_queue_id)
    .bind(asset_lane_id)
    .bind(company_queue_id)
    .bind(company_member_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(expected_queue_head_version + 1)
    .bind(expected_queue_head_version)
    .bind(expected_lane_row_version)
    .bind(from_state)
    .bind(to_state)
    .bind(event_kind)
    .bind(evolution_epoch)
    .bind(event_sha256)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
