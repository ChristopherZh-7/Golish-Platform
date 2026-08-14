//! Transactional authority for asset-bound dynamic Tool Manager verification.

use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::{
    AtTimeSubjectIdentity, ClaimPolarity, HypothesisSemanticKeyV1, PredicateIdentity,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::models::AgentType;
use crate::repo::runtime_memory_tx::{ClaimedStageWorkItemRow, RuntimeMemoryTxFence};
use crate::repo::{message_chains, stage_teams, stage_worker_runs};
use crate::{DbError, Result};

const PLAN_COLUMNS: &str = r#"id,operation_id,stage_execution_id,stage_run_unit_id,
    scope_snapshot_id,organization_id,stage_kind,unit_generation,schema_version,plan_version,
    plan_hash,leader_role,aggregator_kind,aggregator_role,allowed_worker_roles,
    max_workers_total,max_workers_active,dynamic_requests_allowed,dynamic_request_policy,
    dispatch_epoch,requests_closed_at,final_submitter_kind,final_submitter_worker_run_id,
    created_from_stage_spec_hash,row_version,created_at,updated_at"#;
const ITEM_COLUMNS: &str = r#"id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
    scope_snapshot_id,organization_id,dispatch_epoch,kind,stable_key,role,input_manifest_hash,
    input_refs,required_for_barrier,conflict_key,priority,status,attempt_policy,budget,
    output_schema,created_by,row_version,created_at,updated_at,started_at,terminal_at"#;
const OUTPUT_COLUMNS: &str = r#"id,team_plan_id,work_item_id,worker_run_id,operation_id,
    stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,output_schema,
    output_version,business_disposition,canonical_output,canonical_fact_refs,evidence_ids,
    checked_empty_cells,blocker_codes,output_hash,created_at"#;

pub const CONTRACT_INVALID: &str = "INVESTIGATION_ASSET_VERIFICATION_CONTRACT_INVALID";
pub const AUTHORITY_MISMATCH: &str = "INVESTIGATION_ASSET_VERIFICATION_AUTHORITY_MISMATCH";
pub const REPLAY_DRIFT: &str = "INVESTIGATION_ASSET_VERIFICATION_REPLAY_DRIFT";
pub const CAS_CONFLICT: &str = "INVESTIGATION_ASSET_VERIFICATION_CAS_CONFLICT";

fn fail(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_uuid(value: Uuid) -> Result<()> {
    if value.is_nil() {
        Err(fail(CONTRACT_INVALID))
    } else {
        Ok(())
    }
}

async fn sha256_on(tx: &mut Transaction<'_, Postgres>, value: &Value) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256($1::JSONB::TEXT)")
            .bind(value)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn string_set_sha256_on(
    tx: &mut Transaction<'_, Postgres>,
    values: &[String],
) -> Result<String> {
    Ok(sqlx::query_scalar(
        "SELECT tool_truth_sha256(COALESCE(jsonb_agg(value ORDER BY value)::TEXT,'[]')) \
         FROM unnest($1::TEXT[]) value",
    )
    .bind(values)
    .fetch_one(&mut **tx)
    .await?)
}

#[derive(Debug, Clone)]
pub struct AuthorizeAssetVerificationSessionInput {
    pub stable_request_id: Uuid,
    pub session_authorization_id: Uuid,
    pub session_budget_envelope_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_task_id: Uuid,
    pub allowed_effect_classes: Vec<String>,
    pub maximum_risk_tier: String,
    pub allowed_credential_binding_sha256s: Vec<String>,
    pub credential_binding_set_sha256: String,
    pub maximum_invocations: i64,
    pub maximum_network_requests: i64,
    pub maximum_wall_time_ms: i64,
    pub maximum_output_bytes: i64,
    pub maximum_parallel_invocations: i32,
}

#[derive(Debug, Clone, FromRow)]
pub struct AssetVerificationSessionAuthorizationRow {
    pub session_authorization_id: Uuid,
    pub session_budget_envelope_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_task_id: Uuid,
    pub allowed_effect_classes: Value,
    pub maximum_risk_tier: String,
    pub allowed_credential_binding_sha256s: Value,
    pub credential_binding_set_sha256: String,
    pub authorization_sha256: String,
    pub envelope_sha256: String,
    pub expires_at: DateTime<Utc>,
    pub maximum_invocations: i64,
    pub remaining_invocations: i64,
    pub maximum_network_requests: i64,
    pub remaining_network_requests: i64,
    pub maximum_wall_time_ms: i64,
    pub remaining_wall_time_ms: i64,
    pub maximum_output_bytes: i64,
    pub remaining_output_bytes: i64,
    pub maximum_parallel_invocations: i32,
    pub replayed: bool,
}

const LOAD_AUTHORIZATION_SQL: &str = r#"
SELECT authz.session_authorization_id,budget.session_budget_envelope_id,
       authz.operation_id,authz.project_scope_id,
       authz.stage_execution_id,authz.stage_run_unit_id,
       authz.scope_snapshot_id,authz.organization_id,
       authz.asset_lane_id,authz.target_live_id,
       authz.hypothesis_revision_id,authz.verification_task_id,
       authz.allowed_effect_classes,authz.maximum_risk_tier,
       authz.allowed_credential_binding_sha256s,
       authz.credential_binding_set_sha256,authz.authorization_sha256,
       authz.expires_at,budget.envelope_sha256,
       budget.maximum_invocations,budget.remaining_invocations,
       budget.maximum_network_requests,budget.remaining_network_requests,
       budget.maximum_wall_time_ms,budget.remaining_wall_time_ms,
       budget.maximum_output_bytes,budget.remaining_output_bytes,
       budget.maximum_parallel_invocations,$2::BOOLEAN AS replayed
  FROM investigation_asset_verification_authorizations authz
  JOIN investigation_asset_verification_budget_envelopes budget
    ON budget.session_authorization_id=authz.session_authorization_id
 WHERE authz.session_authorization_id=$1"#;

async fn load_authorization_on(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    replayed: bool,
) -> Result<AssetVerificationSessionAuthorizationRow> {
    Ok(sqlx::query_as(LOAD_AUTHORIZATION_SQL)
        .bind(id)
        .bind(replayed)
        .fetch_one(&mut **tx)
        .await?)
}

pub async fn authorize_session(
    pool: &PgPool,
    input: &AuthorizeAssetVerificationSessionInput,
) -> Result<AssetVerificationSessionAuthorizationRow> {
    for id in [
        input.stable_request_id,
        input.session_authorization_id,
        input.session_budget_envelope_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
        input.asset_lane_id,
        input.target_live_id,
        input.hypothesis_revision_id,
        input.verification_task_id,
    ] {
        validate_uuid(id)?;
    }
    if input.allowed_effect_classes.is_empty()
        || !valid_sha256(&input.credential_binding_set_sha256)
        || input.maximum_invocations <= 0
        || input.maximum_network_requests < 0
        || input.maximum_wall_time_ms <= 0
        || input.maximum_output_bytes <= 0
        || input.maximum_parallel_invocations <= 0
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut effects = input.allowed_effect_classes.clone();
    effects.sort();
    effects.dedup();
    let mut credentials = input.allowed_credential_binding_sha256s.clone();
    credentials.sort();
    credentials.dedup();
    if credentials.iter().any(|hash| !valid_sha256(hash)) {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    if let Some(existing_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT session_authorization_id FROM investigation_asset_verification_authorizations \
         WHERE stable_request_id=$1",
    )
    .bind(input.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing_id != input.session_authorization_id {
            return Err(fail(REPLAY_DRIFT));
        }
        let row = load_authorization_on(&mut tx, existing_id, true).await?;
        if row.session_budget_envelope_id != input.session_budget_envelope_id
            || row.operation_id != input.operation_id
            || row.stage_execution_id != input.stage_execution_id
            || row.stage_run_unit_id != input.stage_run_unit_id
            || row.scope_snapshot_id != input.scope_snapshot_id
            || row.organization_id != input.organization_id
            || row.asset_lane_id != input.asset_lane_id
            || row.target_live_id != input.target_live_id
            || row.hypothesis_revision_id != input.hypothesis_revision_id
            || row.verification_task_id != input.verification_task_id
            || row.allowed_effect_classes != json!(effects)
            || row.maximum_risk_tier != input.maximum_risk_tier
            || row.allowed_credential_binding_sha256s != json!(credentials)
            || row.credential_binding_set_sha256 != input.credential_binding_set_sha256
            || row.maximum_invocations != input.maximum_invocations
            || row.maximum_network_requests != input.maximum_network_requests
            || row.maximum_wall_time_ms != input.maximum_wall_time_ms
            || row.maximum_output_bytes != input.maximum_output_bytes
            || row.maximum_parallel_invocations != input.maximum_parallel_invocations
        {
            return Err(fail(REPLAY_DRIFT));
        }
        tx.commit().await?;
        return Ok(row);
    }
    let project_scope_id: Uuid =
        sqlx::query_scalar("SELECT project_scope_id FROM operation_state WHERE operation_id=$1")
            .bind(input.operation_id)
            .fetch_one(&mut *tx)
            .await?;
    let authorized_by: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals \
         WHERE principal_kind='local_operator' AND active FOR SHARE",
    )
    .fetch_one(&mut *tx)
    .await?;
    let expected_credentials = string_set_sha256_on(&mut tx, &credentials).await?;
    if expected_credentials != input.credential_binding_set_sha256 {
        return Err(fail(CONTRACT_INVALID));
    }
    // Authorization lifetime is server-owned. A replay returns the stored
    // expiry instead of comparing a newly computed wall-clock timestamp.
    let expires_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT statement_timestamp()+INTERVAL '4 hours'")
            .fetch_one(&mut *tx)
            .await?;
    let authorization_sha256 = sha256_on(
        &mut tx,
        &json!({
            "domain":"investigation_asset_verification_session_authorization.v1",
            "session_authorization_id":input.session_authorization_id,
            "operation_id":input.operation_id,
            "project_scope_id":project_scope_id,
            "stage_execution_id":input.stage_execution_id,
            "stage_run_unit_id":input.stage_run_unit_id,
            "scope_snapshot_id":input.scope_snapshot_id,
            "organization_id":input.organization_id,
            "asset_lane_id":input.asset_lane_id,
            "target_live_id":input.target_live_id,
            "hypothesis_revision_id":input.hypothesis_revision_id,
            "verification_task_id":input.verification_task_id,
            "allowed_effect_classes":effects,
            "maximum_risk_tier":input.maximum_risk_tier,
            "credential_binding_set_sha256":input.credential_binding_set_sha256,
            "authorized_by":authorized_by,
            "operator_channel":"local_cli",
            "expires_at":expires_at,
        }),
    )
    .await?;
    let envelope_sha256 = sha256_on(
        &mut tx,
        &json!({
            "domain":"investigation_asset_verification_budget_envelope.v1",
            "session_authorization_id":input.session_authorization_id,
            "maximum_invocations":input.maximum_invocations,
            "maximum_network_requests":input.maximum_network_requests,
            "maximum_wall_time_ms":input.maximum_wall_time_ms,
            "maximum_output_bytes":input.maximum_output_bytes,
            "maximum_parallel_invocations":input.maximum_parallel_invocations,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_asset_verification_authorizations(
              session_authorization_id,stable_request_id,operation_id,project_scope_id,
              stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
              asset_lane_id,target_live_id,hypothesis_revision_id,verification_task_id,
              allowed_effect_classes,maximum_risk_tier,allowed_credential_binding_sha256s,
              credential_binding_set_sha256,decision,authorized_by,operator_channel,
              authorization_sha256,expires_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                  'authorized',$17,$18,$19,$20)"#,
    )
    .bind(input.session_authorization_id)
    .bind(input.stable_request_id)
    .bind(input.operation_id)
    .bind(project_scope_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(input.asset_lane_id)
    .bind(input.target_live_id)
    .bind(input.hypothesis_revision_id)
    .bind(input.verification_task_id)
    .bind(json!(effects))
    .bind(&input.maximum_risk_tier)
    .bind(json!(credentials))
    .bind(&input.credential_binding_set_sha256)
    .bind(authorized_by)
    .bind("local_cli")
    .bind(&authorization_sha256)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_asset_verification_budget_envelopes(
              session_budget_envelope_id,session_authorization_id,maximum_invocations,
              remaining_invocations,maximum_network_requests,remaining_network_requests,
              maximum_wall_time_ms,remaining_wall_time_ms,maximum_output_bytes,
              remaining_output_bytes,maximum_parallel_invocations,envelope_sha256)
           VALUES($1,$2,$3,$3,$4,$4,$5,$5,$6,$6,$7,$8)"#,
    )
    .bind(input.session_budget_envelope_id)
    .bind(input.session_authorization_id)
    .bind(input.maximum_invocations)
    .bind(input.maximum_network_requests)
    .bind(input.maximum_wall_time_ms)
    .bind(input.maximum_output_bytes)
    .bind(input.maximum_parallel_invocations)
    .bind(envelope_sha256)
    .execute(&mut *tx)
    .await?;
    let row = load_authorization_on(&mut tx, input.session_authorization_id, false).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Debug, Clone)]
pub struct OpenDynamicVerificationRoundInput {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_task_id: Uuid,
    pub session_authorization_id: Uuid,
    pub session_budget_envelope_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct AssetVerificationActorRow {
    pub role: String,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub message_chain_id: Uuid,
}

#[derive(Debug, Clone, FromRow)]
struct DynamicRoundFlatRow {
    session_id: Uuid,
    stable_request_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    asset_lane_id: Uuid,
    target_live_id: Uuid,
    hypothesis_revision_id: Uuid,
    verification_task_id: Uuid,
    asset_primary_schedule_receipt_id: Uuid,
    evolution_epoch: i32,
    stage_team_plan_id: Uuid,
    dispatch_epoch: i64,
    session_authorization_id: Uuid,
    authorization_expires_at: DateTime<Utc>,
    session_budget_envelope_id: Uuid,
    source_primary_work_item_id: Uuid,
    source_primary_worker_run_id: Uuid,
    primary_work_item_id: Uuid,
    primary_worker_run_id: Uuid,
    primary_message_chain_id: Uuid,
    maximum_primary_turns: i64,
    consumed_primary_turns: i64,
    maximum_actor_calls: i64,
    consumed_actor_calls: i64,
    state: String,
    head_version: i64,
    resolution_authority_id: Option<Uuid>,
    opened_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    replayed: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct DynamicVerificationActorCallRow {
    pub actor_call_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub actor_ordinal: i64,
    pub subtask_id: Uuid,
    pub specialist_role: String,
    pub objective_redacted: Value,
    pub objective_sha256: String,
    pub work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub message_chain_id: Uuid,
    pub primary_turn_id: Uuid,
    pub turn_actor_ordinal: i32,
    pub actor_call_sha256: String,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct DynamicVerificationRoundRow {
    pub session_id: Uuid,
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub verification_task_id: Uuid,
    pub asset_primary_schedule_receipt_id: Uuid,
    pub evolution_epoch: i32,
    pub stage_team_plan_id: Uuid,
    pub dispatch_epoch: i64,
    pub session_authorization_id: Uuid,
    pub authorization_expires_at: DateTime<Utc>,
    pub session_budget_envelope_id: Uuid,
    pub source_primary_work_item_id: Uuid,
    pub source_primary_worker_run_id: Uuid,
    pub primary: AssetVerificationActorRow,
    pub actor_calls: Vec<DynamicVerificationActorCallRow>,
    pub maximum_primary_turns: i64,
    pub consumed_primary_turns: i64,
    pub maximum_actor_calls: i64,
    pub consumed_actor_calls: i64,
    pub state: String,
    pub head_version: i64,
    pub resolution_authority_id: Option<Uuid>,
    pub opened_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub replayed: bool,
}

#[derive(Debug, Clone, FromRow)]
struct DynamicVerificationPrimaryTurnFlatRow {
    primary_turn_id: Uuid,
    stable_request_id: Uuid,
    session_id: Uuid,
    turn_ordinal: i64,
    decision_kind: String,
    expected_session_head_version: i64,
    source_primary_worker_run_id: Uuid,
    consumer_primary_lease_token: Uuid,
    consumer_primary_attempt_epoch: i64,
    consumer_primary_checkpoint_version: i64,
    consumer_primary_checkpoint_sha256: String,
    source_tool_call_record_id: Uuid,
    source_provider_call_id: String,
    canonical_turn_sha256: String,
    actor_call_set_sha256: String,
    replayed: bool,
}

#[derive(Debug, Clone)]
pub struct DynamicVerificationPrimaryTurnRow {
    pub primary_turn_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub turn_ordinal: i64,
    pub decision_kind: String,
    pub expected_session_head_version: i64,
    pub source_primary_worker_run_id: Uuid,
    pub consumer_primary_lease_token: Uuid,
    pub consumer_primary_attempt_epoch: i64,
    pub source_primary_checkpoint_version: i64,
    pub source_primary_checkpoint_sha256: String,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub canonical_turn_sha256: String,
    pub actor_call_set_sha256: String,
    pub actors: Vec<DynamicVerificationActorCallRow>,
    pub replayed: bool,
}

async fn load_dynamic_primary_turn_on(
    tx: &mut Transaction<'_, Postgres>,
    primary_turn_id: Uuid,
    replayed: bool,
) -> Result<DynamicVerificationPrimaryTurnRow> {
    let row = sqlx::query_as::<_, DynamicVerificationPrimaryTurnFlatRow>(
        "SELECT primary_turn_id,stable_request_id,session_id,turn_ordinal,decision_kind,\
         expected_session_head_version,source_primary_worker_run_id,\
         consumer_primary_lease_token,consumer_primary_attempt_epoch,\
         consumer_primary_checkpoint_version,consumer_primary_checkpoint_sha256,\
         source_tool_call_record_id,source_provider_call_id,canonical_turn_sha256,\
         actor_call_set_sha256,\
         $2::BOOLEAN AS replayed FROM investigation_dynamic_verification_primary_turns \
         WHERE primary_turn_id=$1",
    )
    .bind(primary_turn_id)
    .bind(replayed)
    .fetch_one(&mut **tx)
    .await?;
    let actors = sqlx::query_as::<_, DynamicVerificationActorCallRow>(
        "SELECT actor_call.*,$2::BOOLEAN AS replayed FROM \
         investigation_dynamic_verification_actor_calls actor_call \
         WHERE primary_turn_id=$1 ORDER BY turn_actor_ordinal",
    )
    .bind(primary_turn_id)
    .bind(replayed)
    .fetch_all(&mut **tx)
    .await?;
    Ok(DynamicVerificationPrimaryTurnRow {
        primary_turn_id: row.primary_turn_id,
        stable_request_id: row.stable_request_id,
        session_id: row.session_id,
        turn_ordinal: row.turn_ordinal,
        decision_kind: row.decision_kind,
        expected_session_head_version: row.expected_session_head_version,
        source_primary_worker_run_id: row.source_primary_worker_run_id,
        consumer_primary_lease_token: row.consumer_primary_lease_token,
        consumer_primary_attempt_epoch: row.consumer_primary_attempt_epoch,
        source_primary_checkpoint_version: row.consumer_primary_checkpoint_version,
        source_primary_checkpoint_sha256: row.consumer_primary_checkpoint_sha256,
        source_tool_call_record_id: row.source_tool_call_record_id,
        source_provider_call_id: row.source_provider_call_id,
        canonical_turn_sha256: row.canonical_turn_sha256,
        actor_call_set_sha256: row.actor_call_set_sha256,
        actors,
        replayed: row.replayed,
    })
}

async fn load_dynamic_round_on(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    replayed: bool,
) -> Result<DynamicVerificationRoundRow> {
    let row = sqlx::query_as::<_, DynamicRoundFlatRow>(
        "SELECT dynamic_round.*,$2::BOOLEAN AS replayed FROM \
         investigation_dynamic_verification_rounds dynamic_round \
         WHERE dynamic_round.session_id=$1",
    )
    .bind(session_id)
    .bind(replayed)
    .fetch_one(&mut **tx)
    .await?;
    let actor_calls = sqlx::query_as::<_, DynamicVerificationActorCallRow>(
        "SELECT actor_call.*,$2::BOOLEAN AS replayed FROM \
         investigation_dynamic_verification_actor_calls actor_call \
         WHERE actor_call.session_id=$1 ORDER BY actor_call.actor_ordinal",
    )
    .bind(session_id)
    .bind(replayed)
    .fetch_all(&mut **tx)
    .await?;
    Ok(DynamicVerificationRoundRow {
        session_id: row.session_id,
        stable_request_id: row.stable_request_id,
        operation_id: row.operation_id,
        project_scope_id: row.project_scope_id,
        stage_execution_id: row.stage_execution_id,
        stage_run_unit_id: row.stage_run_unit_id,
        scope_snapshot_id: row.scope_snapshot_id,
        organization_id: row.organization_id,
        asset_lane_id: row.asset_lane_id,
        target_live_id: row.target_live_id,
        hypothesis_revision_id: row.hypothesis_revision_id,
        verification_task_id: row.verification_task_id,
        asset_primary_schedule_receipt_id: row.asset_primary_schedule_receipt_id,
        evolution_epoch: row.evolution_epoch,
        stage_team_plan_id: row.stage_team_plan_id,
        dispatch_epoch: row.dispatch_epoch,
        session_authorization_id: row.session_authorization_id,
        authorization_expires_at: row.authorization_expires_at,
        session_budget_envelope_id: row.session_budget_envelope_id,
        source_primary_work_item_id: row.source_primary_work_item_id,
        source_primary_worker_run_id: row.source_primary_worker_run_id,
        primary: AssetVerificationActorRow {
            role: "primary".into(),
            work_item_id: row.primary_work_item_id,
            worker_run_id: row.primary_worker_run_id,
            message_chain_id: row.primary_message_chain_id,
        },
        actor_calls,
        maximum_primary_turns: row.maximum_primary_turns,
        consumed_primary_turns: row.consumed_primary_turns,
        maximum_actor_calls: row.maximum_actor_calls,
        consumed_actor_calls: row.consumed_actor_calls,
        state: row.state,
        head_version: row.head_version,
        resolution_authority_id: row.resolution_authority_id,
        opened_at: row.opened_at,
        resolved_at: row.resolved_at,
        replayed: row.replayed,
    })
}

pub async fn load_dynamic_round(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<DynamicVerificationRoundRow>> {
    let mut tx = pool.begin().await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM investigation_dynamic_verification_rounds WHERE session_id=$1)",
    )
    .bind(session_id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        tx.commit().await?;
        return Ok(None);
    }
    let row = load_dynamic_round_on(&mut tx, session_id, false).await?;
    tx.commit().await?;
    Ok(Some(row))
}

pub async fn open_dynamic_round(
    pool: &PgPool,
    input: &OpenDynamicVerificationRoundInput,
) -> Result<DynamicVerificationRoundRow> {
    let session_id = Uuid::new_v5(
        &input.hypothesis_revision_id,
        b"investigation-dynamic-verification-round-v2",
    );
    let mut tx = pool.begin().await?;
    if let Some(existing_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT session_id FROM investigation_dynamic_verification_rounds \
         WHERE stable_request_id=$1",
    )
    .bind(input.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing_id != session_id {
            return Err(fail(REPLAY_DRIFT));
        }
        let row = load_dynamic_round_on(&mut tx, session_id, true).await?;
        if row.operation_id != input.operation_id
            || row.stage_execution_id != input.stage_execution_id
            || row.stage_run_unit_id != input.stage_run_unit_id
            || row.scope_snapshot_id != input.scope_snapshot_id
            || row.organization_id != input.organization_id
            || row.asset_lane_id != input.asset_lane_id
            || row.target_live_id != input.target_live_id
            || row.hypothesis_revision_id != input.hypothesis_revision_id
            || row.verification_task_id != input.verification_task_id
            || row.session_authorization_id != input.session_authorization_id
            || row.session_budget_envelope_id != input.session_budget_envelope_id
        {
            return Err(fail(REPLAY_DRIFT));
        }
        tx.commit().await?;
        return Ok(row);
    }
    let authorization =
        load_authorization_on(&mut tx, input.session_authorization_id, false).await?;
    if authorization.session_budget_envelope_id != input.session_budget_envelope_id
        || authorization.operation_id != input.operation_id
        || authorization.stage_execution_id != input.stage_execution_id
        || authorization.stage_run_unit_id != input.stage_run_unit_id
        || authorization.scope_snapshot_id != input.scope_snapshot_id
        || authorization.organization_id != input.organization_id
        || authorization.asset_lane_id != input.asset_lane_id
        || authorization.target_live_id != input.target_live_id
        || authorization.hypothesis_revision_id != input.hypothesis_revision_id
        || authorization.verification_task_id != input.verification_task_id
    {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let schedule: (Uuid, i32, Uuid, i64, Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT current_primary.source_schedule_receipt_id,
                  current_primary.evolution_epoch,current_primary.stage_team_plan_id,
                  current_primary.resume_dispatch_epoch,current_primary.primary_work_item_id,
                  current_primary.primary_worker_run_id,current_primary.primary_message_chain_id
             FROM investigation_asset_primary_current_authorities current_primary
             JOIN investigation_asset_lanes lane
               ON lane.asset_lane_id=current_primary.asset_lane_id
              AND lane.operation_id=current_primary.operation_id
              AND lane.stage_execution_id=current_primary.stage_execution_id
              AND lane.scope_snapshot_id=current_primary.scope_snapshot_id
              AND lane.organization_id=current_primary.organization_id
              AND lane.target_id=current_primary.target_id
              AND lane.evolution_epoch=current_primary.evolution_epoch
            WHERE current_primary.operation_id=$1
              AND current_primary.stage_execution_id=$2
              AND current_primary.stage_run_unit_id=$3
              AND current_primary.scope_snapshot_id=$4
              AND current_primary.organization_id=$5
              AND current_primary.asset_lane_id=$6
              AND current_primary.target_id=$7
              AND lane.state='verifying'"#,
    )
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(input.asset_lane_id)
    .bind(input.target_live_id)
    .fetch_one(&mut *tx)
    .await?;
    let mut plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1 FOR UPDATE"
    ))
    .bind(schedule.2)
    .fetch_one(&mut *tx)
    .await?;
    let predecessor: (Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT item.id,worker.id,worker.message_chain_id
             FROM stage_work_items item
             JOIN stage_worker_runs worker ON worker.work_item_id=item.id
            WHERE item.team_plan_id=$1 AND item.id=COALESCE(
              (SELECT previous.primary_work_item_id
                 FROM investigation_dynamic_verification_rounds previous
                WHERE previous.asset_lane_id=$2 AND previous.evolution_epoch=$3
                  AND previous.state='resolved'
                ORDER BY previous.resolved_at DESC,previous.session_id DESC LIMIT 1),$4)
              AND item.status='completed' AND worker.status='passed'
              AND worker.id=COALESCE(
              (SELECT previous.primary_worker_run_id
                 FROM investigation_dynamic_verification_rounds previous
                WHERE previous.asset_lane_id=$2 AND previous.evolution_epoch=$3
                  AND previous.state='resolved'
                ORDER BY previous.resolved_at DESC,previous.session_id DESC LIMIT 1),$5)
              AND worker.message_chain_id=$6 FOR SHARE OF item,worker"#,
    )
    .bind(schedule.2)
    .bind(input.asset_lane_id)
    .bind(schedule.1)
    .bind(schedule.4)
    .bind(schedule.5)
    .bind(schedule.6)
    .fetch_one(&mut *tx)
    .await?;
    if predecessor.2 != schedule.6
        || plan.requests_closed_at.is_none()
        || plan.final_submitter_worker_run_id.is_some()
    {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let round_rearm_id = Uuid::new_v5(&session_id, b"dynamic-verification-round-rearm-v2");
    let resume_dispatch_epoch = plan.dispatch_epoch.saturating_add(1);
    let rearm_sha256 = sha256_on(
        &mut tx,
        &json!({
            "domain":"investigation_dynamic_verification_round_rearm.v2",
            "session_id":session_id,"asset_lane_id":input.asset_lane_id,
            "target_live_id":input.target_live_id,
            "hypothesis_revision_id":input.hypothesis_revision_id,
            "verification_task_id":input.verification_task_id,
            "stage_team_plan_id":plan.id,"source_dispatch_epoch":plan.dispatch_epoch,
            "resume_dispatch_epoch":resume_dispatch_epoch,
            "source_plan_row_version":plan.row_version,
            "source_primary_work_item_id":predecessor.0,
            "source_primary_worker_run_id":predecessor.1,
            "primary_message_chain_id":predecessor.2,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_asset_verification_round_rearms(
           round_rearm_id,stable_request_id,session_id,operation_id,stage_execution_id,
           stage_run_unit_id,scope_snapshot_id,organization_id,asset_lane_id,target_live_id,
           hypothesis_revision_id,verification_task_id,asset_primary_schedule_receipt_id,
           stage_team_plan_id,source_dispatch_epoch,resume_dispatch_epoch,
           source_plan_row_version,rearm_sha256,status,round_contract,
           source_primary_work_item_id,source_primary_worker_run_id,primary_message_chain_id)
           VALUES($1,uuid_generate_v5($1,'investigation-dynamic-verification-round-rearm-request-v2'),
             $2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,'building',
             'primary_dynamic_v2',$18,$19,$20)"#,
    )
    .bind(round_rearm_id)
    .bind(session_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(input.asset_lane_id)
    .bind(input.target_live_id)
    .bind(input.hypothesis_revision_id)
    .bind(input.verification_task_id)
    .bind(schedule.0)
    .bind(plan.id)
    .bind(plan.dispatch_epoch)
    .bind(resume_dispatch_epoch)
    .bind(plan.row_version)
    .bind(rearm_sha256)
    .bind(predecessor.0)
    .bind(predecessor.1)
    .bind(predecessor.2)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "SELECT set_config('golish.investigation_asset_verification_round_rearm_id',$1,TRUE)",
    )
    .bind(round_rearm_id.to_string())
    .execute(&mut *tx)
    .await?;
    plan = sqlx::query_as(
        r#"UPDATE stage_team_plans SET dispatch_epoch=$2,requests_closed_at=NULL,
             final_submitter_worker_run_id=NULL,row_version=row_version+1,updated_at=NOW()
           WHERE id=$1 AND dispatch_epoch=$3 AND row_version=$4
             AND requests_closed_at IS NOT NULL AND final_submitter_worker_run_id IS NULL
           RETURNING *"#,
    )
    .bind(plan.id)
    .bind(resume_dispatch_epoch)
    .bind(plan.dispatch_epoch)
    .bind(plan.row_version)
    .fetch_one(&mut *tx)
    .await?;
    let primary_work_item_id =
        Uuid::new_v5(&session_id, b"dynamic-verification-primary-work-item-v2");
    let primary_worker_run_id =
        Uuid::new_v5(&session_id, b"dynamic-verification-primary-worker-v2");
    let primary_input_refs = json!([{
        "kind":"investigation_dynamic_verification_round",
        "session_id":session_id,"asset_lane_id":input.asset_lane_id,
        "target_id":input.target_live_id,"hypothesis_revision_id":input.hypothesis_revision_id,
        "verification_task_id":input.verification_task_id,
    }]);
    let primary_input_manifest_hash = sha256_on(&mut tx, &primary_input_refs).await?;
    let primary_item = stage_teams::insert_work_item_with_executor(
        &mut *tx,
        &stage_teams::NewStageWorkItem {
            id: primary_work_item_id,
            team_plan_id: plan.id,
            operation_id: plan.operation_id,
            stage_execution_id: plan.stage_execution_id,
            stage_run_unit_id: plan.stage_run_unit_id,
            scope_snapshot_id: plan.scope_snapshot_id,
            organization_id: plan.organization_id,
            dispatch_epoch: plan.dispatch_epoch,
            kind: "investigation_dynamic_verification_primary".into(),
            stable_key: format!(
                "asset:{}:verification:{}:primary",
                input.asset_lane_id, input.hypothesis_revision_id
            ),
            role: plan.leader_role.clone(),
            input_manifest_hash: primary_input_manifest_hash,
            input_refs: primary_input_refs,
            required_for_barrier: false,
            conflict_key: None,
            priority: 0,
            attempt_policy: json!({"max_attempts":3}),
            budget: json!({}),
            output_schema: "investigation_asset_verification_primary_resolution.v2".into(),
            created_by: "server_seed".into(),
        },
    )
    .await
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    stage_worker_runs::insert_with_executor(
        &mut *tx,
        &stage_worker_runs::NewStageWorkerRun {
            id: primary_worker_run_id,
            operation_id: plan.operation_id,
            stage_execution_id: plan.stage_execution_id,
            stage_run_unit_id: plan.stage_run_unit_id,
            work_item_id: Some(primary_item.id),
            organization_id: plan.organization_id,
            worker_generation: schedule.1,
            specialist: plan.leader_role.clone(),
            work_item_kind: primary_item.kind.clone(),
            work_item_key: primary_item.stable_key.clone(),
            agent_path: format!(
                "main>stage_run:investigation>org:{}>asset:{}>verification:{}>primary",
                plan.organization_id, input.asset_lane_id, input.hypothesis_revision_id
            ),
            parent_request_id: Some(format!(
                "investigation-asset-primary:{}",
                input.asset_lane_id
            )),
        },
    )
    .await
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    sqlx::query(
        "UPDATE stage_worker_runs SET message_chain_id=$2,updated_at=NOW() \
         WHERE id=$1 AND status='queued' AND message_chain_id IS NULL",
    )
    .bind(primary_worker_run_id)
    .bind(schedule.6)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_dynamic_verification_primary_continuities(
           continuity_id,session_id,asset_lane_id,hypothesis_revision_id,
           predecessor_work_item_id,predecessor_worker_run_id,verification_work_item_id,
           verification_worker_run_id,durable_primary_message_chain_id)
           VALUES(uuid_generate_v5($1,'investigation-dynamic-verification-primary-continuity-v2'),
             $1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(session_id)
    .bind(input.asset_lane_id)
    .bind(input.hypothesis_revision_id)
    .bind(predecessor.0)
    .bind(predecessor.1)
    .bind(primary_work_item_id)
    .bind(primary_worker_run_id)
    .bind(schedule.6)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_dynamic_verification_rounds(
           session_id,stable_request_id,operation_id,project_scope_id,stage_execution_id,
           stage_run_unit_id,scope_snapshot_id,organization_id,asset_lane_id,target_live_id,
           hypothesis_revision_id,verification_task_id,asset_primary_schedule_receipt_id,
           evolution_epoch,round_rearm_id,stage_team_plan_id,dispatch_epoch,
           session_authorization_id,session_budget_envelope_id,authorization_expires_at,
           source_primary_work_item_id,source_primary_worker_run_id,primary_work_item_id,
           primary_worker_run_id,primary_message_chain_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25)"#,
    )
    .bind(session_id)
    .bind(input.stable_request_id)
    .bind(input.operation_id)
    .bind(authorization.project_scope_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(input.asset_lane_id)
    .bind(input.target_live_id)
    .bind(input.hypothesis_revision_id)
    .bind(input.verification_task_id)
    .bind(schedule.0)
    .bind(schedule.1)
    .bind(round_rearm_id)
    .bind(schedule.2)
    .bind(plan.dispatch_epoch)
    .bind(input.session_authorization_id)
    .bind(input.session_budget_envelope_id)
    .bind(authorization.expires_at)
    .bind(predecessor.0)
    .bind(predecessor.1)
    .bind(primary_work_item_id)
    .bind(primary_worker_run_id)
    .bind(schedule.6)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE investigation_asset_verification_round_rearms SET status='applied',applied_at=NOW() \
         WHERE round_rearm_id=$1 AND status='building'",
    )
    .bind(round_rearm_id)
    .execute(&mut *tx)
    .await?;
    let row = load_dynamic_round_on(&mut tx, session_id, false).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Debug, Clone)]
pub struct DynamicVerificationActorRequestInput {
    pub actor_call_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct DispatchDynamicVerificationActorBatchInput {
    pub stable_request_id: Uuid,
    pub primary_turn_id: Uuid,
    pub session_id: Uuid,
    pub expected_session_head_version: i64,
    pub primary_worker_fence: VerificationWorkerFenceInput,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub actors: Vec<DynamicVerificationActorRequestInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDynamicPrimarySubjectRef {
    kind: String,
    id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDynamicPrimarySubtask {
    stable_key: String,
    role: String,
    objective: String,
    rationale: String,
    subject_refs: Vec<StoredDynamicPrimarySubjectRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDynamicPrimaryProposal {
    predicate_schema: String,
    predicate_version: u32,
    predicate_arguments: Vec<(String, String)>,
    trust_boundary: String,
    polarity: String,
    structured_claim: String,
    preconditions: Vec<String>,
    impact: String,
    rationale: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
enum StoredDynamicPrimaryTurn {
    Delegate {
        schema_version: u32,
        session_id: Uuid,
        hypothesis_revision_id: Uuid,
        subtasks: Vec<StoredDynamicPrimarySubtask>,
    },
    Resolve {
        schema_version: u32,
        session_id: Uuid,
        hypothesis_revision_id: Uuid,
        subtasks: Vec<StoredDynamicPrimarySubtask>,
        disposition: String,
        conclusion: String,
        cited_evidence_ids: Vec<i64>,
        new_hypothesis_proposals: Vec<StoredDynamicPrimaryProposal>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDynamicActorObservation {
    schema_version: u32,
    session_id: Uuid,
    hypothesis_revision_id: Uuid,
    actor_call_id: Uuid,
    actor_ordinal: i64,
    subtask_id: Uuid,
    specialist_role: String,
    summary: String,
    cited_evidence_ids: Vec<i64>,
    new_hypothesis_proposals: Vec<StoredDynamicPrimaryProposal>,
}

fn stored_dynamic_subtask_is_valid(subtask: &StoredDynamicPrimarySubtask) -> bool {
    !subtask.stable_key.trim().is_empty()
        && subtask.stable_key.chars().count() <= 128
        && subtask
            .stable_key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_:".contains(character))
        && !subtask.objective.trim().is_empty()
        && subtask.objective.chars().count() <= 4_096
        && !subtask.rationale.trim().is_empty()
        && subtask.rationale.chars().count() <= 2_048
        && subtask.subject_refs.len() == 2
        && subtask
            .subject_refs
            .iter()
            .map(|subject| subject.kind.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
            == subtask.subject_refs.len()
}

fn stored_dynamic_proposal_is_valid(proposal: &StoredDynamicPrimaryProposal) -> bool {
    proposal.predicate_version > 0
        && !proposal.predicate_schema.trim().is_empty()
        && proposal.predicate_schema.chars().count() <= 128
        && !proposal.trust_boundary.trim().is_empty()
        && proposal.trust_boundary.chars().count() <= 256
        && matches!(proposal.polarity.as_str(), "positive" | "negative")
        && !proposal.structured_claim.trim().is_empty()
        && proposal.structured_claim.chars().count() <= 8_192
        && proposal.preconditions.len() <= 64
        && proposal
            .preconditions
            .iter()
            .all(|condition| !condition.trim().is_empty() && condition.chars().count() <= 1_024)
        && !proposal.impact.trim().is_empty()
        && proposal.impact.chars().count() <= 4_096
        && !proposal.rationale.trim().is_empty()
        && proposal.rationale.chars().count() <= 4_096
        && proposal.predicate_arguments.len() <= 64
        && proposal.predicate_arguments.iter().all(|(key, value)| {
            !key.trim().is_empty() && key.chars().count() <= 128 && value.chars().count() <= 4_096
        })
        && proposal
            .predicate_arguments
            .iter()
            .map(|(key, _)| key)
            .collect::<std::collections::HashSet<_>>()
            .len()
            == proposal.predicate_arguments.len()
}

#[derive(Debug, Clone, FromRow)]
pub struct PendingDynamicPrimarySubmissionRow {
    pub session_id: Uuid,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub canonical_turn: Value,
    pub canonical_turn_sha256: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct DynamicVerificationAuthorizationRenewalRow {
    pub renewal_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub previous_expires_at: DateTime<Utc>,
    pub renewed_expires_at: DateTime<Utc>,
    pub renewal_sha256: String,
    pub replayed: bool,
}

enum InternalSubmitResultOwner<'a> {
    Primary(&'a DynamicVerificationRoundRow),
    Actor(
        &'a DynamicVerificationRoundRow,
        &'a DynamicVerificationActorCallRow,
    ),
}

/// Reconcile only Golish's internal `submit_result` tool.  Its complete typed
/// payload is already durable in `tool_calls.args`; it performs no external
/// I/O.  This closes both split windows in WorkerToolLifecycle::finish: the
/// expired worker may still point at the call, or that pointer may already be
/// cleared while telemetry remains `running`.  No other tool is eligible.
async fn reconcile_internal_submit_result_on(
    tx: &mut Transaction<'_, Postgres>,
    owner: InternalSubmitResultOwner<'_>,
) -> Result<()> {
    let (round, actor) = match owner {
        InternalSubmitResultOwner::Primary(round) => (round, None),
        InternalSubmitResultOwner::Actor(round, actor) => (round, Some(actor)),
    };
    let worker_id = actor.map_or(round.primary.worker_run_id, |actor| actor.worker_run_id);
    let worker: (String, Option<Uuid>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT status,active_tool_call_id,lease_expires_at FROM stage_worker_runs \
         WHERE id=$1 FOR UPDATE",
    )
    .bind(worker_id)
    .fetch_one(&mut **tx)
    .await?;
    if worker.2.is_some_and(|expires_at| expires_at > Utc::now()) {
        return Ok(());
    }
    let candidates: Vec<(Uuid, String, Value)> = sqlx::query_as(
        r#"SELECT call.id,call.call_id,call.args->'result'
             FROM tool_calls call
            WHERE call.worker_run_id=$1 AND call.operation_id=$2
              AND call.stage_execution_id=$3 AND call.stage_run_unit_id=$4
              AND call.organization_id=$5 AND call.name='submit_result'
              AND call.status IN('received','running') AND call.args ? 'result'
            ORDER BY call.created_at,call.id FOR UPDATE"#,
    )
    .bind(worker_id)
    .bind(round.operation_id)
    .bind(round.stage_execution_id)
    .bind(round.stage_run_unit_id)
    .bind(round.organization_id)
    .fetch_all(&mut **tx)
    .await?;
    if candidates.len() > 1 {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let Some((record_id, _provider_call_id, raw_result)) = candidates.into_iter().next() else {
        return Ok(());
    };
    if worker.1.is_some_and(|active| active != record_id) {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    if let Some(actor) = actor {
        let observation: StoredDynamicActorObservation =
            serde_json::from_value(raw_result).map_err(|_| fail(CONTRACT_INVALID))?;
        if observation.schema_version != 1
            || observation.session_id != round.session_id
            || observation.hypothesis_revision_id != round.hypothesis_revision_id
            || observation.actor_call_id != actor.actor_call_id
            || observation.actor_ordinal != actor.actor_ordinal
            || observation.subtask_id != actor.subtask_id
            || observation.specialist_role != actor.specialist_role
            || observation.summary.trim().is_empty()
            || observation.summary.chars().count() > 4_096
            || observation.cited_evidence_ids.len() > 256
            || observation.cited_evidence_ids.iter().any(|id| *id <= 0)
            || observation
                .cited_evidence_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != observation.cited_evidence_ids.len()
            || observation.new_hypothesis_proposals.len() > 64
            || observation
                .new_hypothesis_proposals
                .iter()
                .any(|proposal| !stored_dynamic_proposal_is_valid(proposal))
        {
            return Err(fail(CONTRACT_INVALID));
        }
    } else {
        let turn: StoredDynamicPrimaryTurn =
            serde_json::from_value(raw_result).map_err(|_| fail(CONTRACT_INVALID))?;
        let (schema_version, session_id, hypothesis_revision_id) = match turn {
            StoredDynamicPrimaryTurn::Delegate {
                schema_version,
                session_id,
                hypothesis_revision_id,
                subtasks,
            } if !subtasks.is_empty()
                && subtasks.len() <= 8
                && subtasks.iter().all(stored_dynamic_subtask_is_valid) =>
            {
                (schema_version, session_id, hypothesis_revision_id)
            }
            StoredDynamicPrimaryTurn::Resolve {
                schema_version,
                session_id,
                hypothesis_revision_id,
                subtasks,
                disposition,
                conclusion,
                cited_evidence_ids,
                new_hypothesis_proposals,
            } if subtasks.is_empty()
                && matches!(disposition.as_str(), "verified" | "refuted" | "invalid")
                && !conclusion.trim().is_empty()
                && conclusion.chars().count() <= 8_192
                && cited_evidence_ids.len() <= 256
                && cited_evidence_ids.iter().all(|id| *id > 0)
                && cited_evidence_ids
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    == cited_evidence_ids.len()
                && new_hypothesis_proposals.len() <= 64
                && new_hypothesis_proposals
                    .iter()
                    .all(stored_dynamic_proposal_is_valid) =>
            {
                (schema_version, session_id, hypothesis_revision_id)
            }
            _ => return Err(fail(CONTRACT_INVALID)),
        };
        if schema_version != 1
            || session_id != round.session_id
            || hypothesis_revision_id != round.hypothesis_revision_id
        {
            return Err(fail(CONTRACT_INVALID));
        }
    }
    sqlx::query(
        r#"UPDATE stage_worker_runs
              SET active_tool_call_id=NULL,active_tool_started_at=NULL,updated_at=NOW()
            WHERE id=$1 AND (active_tool_call_id=$2 OR active_tool_call_id IS NULL)"#,
    )
    .bind(worker_id)
    .bind(record_id)
    .execute(&mut **tx)
    .await?;
    let finished = sqlx::query(
        r#"UPDATE tool_calls
              SET status='finished',result=$2,duration_ms=COALESCE(duration_ms,0),updated_at=NOW()
            WHERE id=$1 AND status IN('received','running') AND name='submit_result'"#,
    )
    .bind(record_id)
    .bind(r#"{"status":"result submitted"}"#)
    .execute(&mut **tx)
    .await?;
    if finished.rows_affected() != 1 {
        return Err(fail(CAS_CONFLICT));
    }
    if worker.0 == "recovery_required" {
        let recovered_worker = sqlx::query(
            r#"UPDATE stage_worker_runs
                  SET status='waiting_background',lease_token=NULL,lease_owner=NULL,
                      lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
                      attempt_epoch=attempt_epoch+1,updated_at=NOW()
                WHERE id=$1 AND status='recovery_required'
                  AND active_tool_call_id IS NULL"#,
        )
        .bind(worker_id)
        .execute(&mut **tx)
        .await?;
        if recovered_worker.rows_affected() != 1 {
            return Err(fail(CAS_CONFLICT));
        }
        let item_id = actor.map_or(round.primary.work_item_id, |actor| actor.work_item_id);
        let recovered_item = sqlx::query(
            r#"UPDATE stage_work_items
                  SET status='queued',row_version=row_version+1,updated_at=NOW()
                WHERE id=$1 AND status='recovery_required'"#,
        )
        .bind(item_id)
        .execute(&mut **tx)
        .await?;
        if recovered_item.rows_affected() != 1 {
            return Err(fail(CAS_CONFLICT));
        }
        // The generic WorkItem state machine intentionally has no direct
        // recovery_required -> waiting_dependency edge. Re-enter its legal
        // lifecycle before parking the reconciled internal submit_result.
        let running_item = sqlx::query(
            r#"UPDATE stage_work_items
                  SET status='running',row_version=row_version+1,updated_at=NOW()
                WHERE id=$1 AND status='queued'"#,
        )
        .bind(item_id)
        .execute(&mut **tx)
        .await?;
        if running_item.rows_affected() != 1 {
            return Err(fail(CAS_CONFLICT));
        }
        let parked_item = sqlx::query(
            r#"UPDATE stage_work_items
                  SET status='waiting_dependency',row_version=row_version+1,updated_at=NOW()
                WHERE id=$1 AND status='running'"#,
        )
        .bind(item_id)
        .execute(&mut **tx)
        .await?;
        if parked_item.rows_affected() != 1 {
            return Err(fail(CAS_CONFLICT));
        }
        if let Some(actor) = actor {
            let parked = sqlx::query(
                "UPDATE investigation_dynamic_verification_actor_calls SET state='parked' \
                 WHERE actor_call_id=$1 AND state='running'",
            )
            .bind(actor.actor_call_id)
            .execute(&mut **tx)
            .await?;
            if parked.rows_affected() != 1 {
                return Err(fail(CAS_CONFLICT));
            }
        }
    }
    Ok(())
}

pub async fn renew_dynamic_authorization(
    pool: &PgPool,
    stable_request_id: Uuid,
    renewal_id: Uuid,
    session_id: Uuid,
) -> Result<DynamicVerificationAuthorizationRenewalRow> {
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as::<_, DynamicVerificationAuthorizationRenewalRow>(
        "SELECT renewal.*,$2::BOOLEAN AS replayed FROM \
         investigation_dynamic_verification_authorization_renewals renewal \
         WHERE stable_request_id=$1",
    )
    .bind(stable_request_id)
    .bind(true)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing.renewal_id != renewal_id || existing.session_id != session_id {
            return Err(fail(REPLAY_DRIFT));
        }
        tx.commit().await?;
        return Ok(existing);
    }
    let previous_expires_at: DateTime<Utc> = sqlx::query_scalar(
        "SELECT authorization_expires_at FROM investigation_dynamic_verification_rounds \
         WHERE session_id=$1 AND state='open' FOR UPDATE",
    )
    .bind(session_id)
    .fetch_one(&mut *tx)
    .await?;
    let renewed_expires_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT statement_timestamp()+INTERVAL '4 hours'")
            .fetch_one(&mut *tx)
            .await?;
    // Keep timestamptz canonicalization inside Postgres. Serializing these
    // instants through serde_json first may use a different textual offset
    // shape than jsonb_build_object, while the migration guard deliberately
    // hashes the latter as its single authority representation.
    let renewal_sha256: String = sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(jsonb_build_object(
             'domain','investigation_dynamic_verification_authorization_renewal.v1',
             'renewal_id',$1::UUID,'session_id',$2::UUID,
             'previous_expires_at',$3::TIMESTAMPTZ,
             'renewed_expires_at',$4::TIMESTAMPTZ)::TEXT)"#,
    )
    .bind(renewal_id)
    .bind(session_id)
    .bind(previous_expires_at)
    .bind(renewed_expires_at)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO investigation_dynamic_verification_authorization_renewals(\
         renewal_id,stable_request_id,session_id,previous_expires_at,renewed_expires_at,\
         renewal_sha256) VALUES($1,$2,$3,$4,$5,$6)",
    )
    .bind(renewal_id)
    .bind(stable_request_id)
    .bind(session_id)
    .bind(previous_expires_at)
    .bind(renewed_expires_at)
    .bind(&renewal_sha256)
    .execute(&mut *tx)
    .await?;
    let updated = sqlx::query(
        "UPDATE investigation_dynamic_verification_rounds \
         SET authorization_expires_at=$2 WHERE session_id=$1 AND state='open' \
         AND authorization_expires_at=$3",
    )
    .bind(session_id)
    .bind(renewed_expires_at)
    .bind(previous_expires_at)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(fail(CAS_CONFLICT));
    }
    let row = sqlx::query_as(
        "SELECT renewal.*,$2::BOOLEAN AS replayed FROM \
         investigation_dynamic_verification_authorization_renewals renewal WHERE renewal_id=$1",
    )
    .bind(renewal_id)
    .bind(false)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn load_pending_dynamic_primary_submission(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<PendingDynamicPrimarySubmissionRow>> {
    validate_uuid(session_id)?;
    let mut tx = pool.begin().await?;
    let round = load_dynamic_round_on(&mut tx, session_id, false).await?;
    reconcile_internal_submit_result_on(&mut tx, InternalSubmitResultOwner::Primary(&round))
        .await?;
    let pending = sqlx::query_as(
        r#"SELECT dynamic_round.session_id,call.id AS source_tool_call_record_id,
                  call.call_id AS source_provider_call_id,
                  call.args->'result' AS canonical_turn,
                  tool_truth_sha256((call.args->'result')::TEXT) AS canonical_turn_sha256
             FROM investigation_dynamic_verification_rounds dynamic_round
             JOIN stage_worker_runs worker ON worker.id=dynamic_round.primary_worker_run_id
             JOIN tool_calls call ON call.worker_run_id=worker.id
              AND call.operation_id=dynamic_round.operation_id
              AND call.stage_execution_id=dynamic_round.stage_execution_id
              AND call.stage_run_unit_id=dynamic_round.stage_run_unit_id
              AND call.organization_id=dynamic_round.organization_id
              AND call.name='submit_result' AND call.status='finished'
              AND call.result IS NOT NULL
              AND call.result::JSONB->>'status'='result submitted'
              AND call.args ? 'result'
            WHERE dynamic_round.session_id=$1 AND dynamic_round.state='open'
              AND worker.active_tool_call_id IS NULL
              AND NOT EXISTS(SELECT 1 FROM investigation_dynamic_verification_primary_turns turn_row
                              WHERE turn_row.source_tool_call_record_id=call.id)
            ORDER BY call.updated_at,call.id LIMIT 1"#,
    )
    .bind(session_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(pending)
}

fn dynamic_actor_agent(role: &str) -> Result<AgentType> {
    match role {
        "browser" | "pentester" => Ok(AgentType::Pentester),
        "researcher" => Ok(AgentType::Searcher),
        "adviser" => Ok(AgentType::Adviser),
        "coder" => Ok(AgentType::Coder),
        "installer" => Ok(AgentType::Installer),
        "enricher" => Ok(AgentType::Enricher),
        "memorist" => Ok(AgentType::Memorist),
        _ => Err(fail(CONTRACT_INVALID)),
    }
}

pub async fn dispatch_dynamic_actor_batch(
    pool: &PgPool,
    input: &DispatchDynamicVerificationActorBatchInput,
) -> Result<DynamicVerificationPrimaryTurnRow> {
    if input.actors.is_empty()
        || input.actors.len() > 8
        || input.source_tool_call_record_id.is_nil()
        || input.source_provider_call_id.trim().is_empty()
        || input
            .actors
            .iter()
            .any(|actor| actor.actor_call_id.is_nil())
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    if let Some(existing_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT primary_turn_id FROM investigation_dynamic_verification_primary_turns \
         WHERE stable_request_id=$1",
    )
    .bind(input.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if existing_id != input.primary_turn_id {
            return Err(fail(REPLAY_DRIFT));
        }
        let row = load_dynamic_primary_turn_on(&mut tx, existing_id, true).await?;
        if row.session_id != input.session_id
            || row.decision_kind != "delegate"
            || row.expected_session_head_version != input.expected_session_head_version
            || row.source_primary_worker_run_id != input.primary_worker_fence.worker_run_id
            || row.consumer_primary_lease_token != input.primary_worker_fence.lease_token
            || row.consumer_primary_attempt_epoch != input.primary_worker_fence.attempt_epoch
            || row.source_primary_checkpoint_version
                != input.primary_worker_fence.checkpoint_version
            || row.source_tool_call_record_id != input.source_tool_call_record_id
            || row.source_provider_call_id != input.source_provider_call_id
            || row
                .actors
                .iter()
                .map(|actor| actor.actor_call_id)
                .collect::<Vec<_>>()
                != input
                    .actors
                    .iter()
                    .map(|actor| actor.actor_call_id)
                    .collect::<Vec<_>>()
        {
            return Err(fail(REPLAY_DRIFT));
        }
        tx.commit().await?;
        return Ok(row);
    }
    let round = load_dynamic_round_on(&mut tx, input.session_id, false).await?;
    sqlx::query(
        "SELECT session_id FROM investigation_dynamic_verification_rounds \
         WHERE session_id=$1 FOR UPDATE",
    )
    .bind(input.session_id)
    .execute(&mut *tx)
    .await?;
    if round.state != "open"
        || round.head_version != input.expected_session_head_version
        || round.authorization_expires_at <= Utc::now()
        || round.primary.worker_run_id != input.primary_worker_fence.worker_run_id
        || round.consumed_primary_turns >= round.maximum_primary_turns
        || round.consumed_actor_calls
            + i64::try_from(input.actors.len()).map_err(|_| fail(CONTRACT_INVALID))?
            > round.maximum_actor_calls
    {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let (primary_checkpoint, source_attempt_epoch, source_lease_token, raw_turn): (
        Value,
        i64,
        Uuid,
        Value,
    ) = sqlx::query_as(
        r#"SELECT worker.checkpoint,call.attempt_epoch,call.lease_token,call.args->'result'
              FROM stage_worker_runs worker
              JOIN tool_calls call ON call.id=$7 AND call.call_id=$8
               AND call.worker_run_id=worker.id AND call.name='submit_result'
               AND call.status='finished' AND call.result IS NOT NULL
               AND call.result::JSONB->>'status'='result submitted' AND call.args ? 'result'
               AND call.operation_id=$9 AND call.stage_execution_id=$10
               AND call.stage_run_unit_id=$11 AND call.organization_id=$12
            WHERE worker.id=$1 AND worker.work_item_id=$2
              AND worker.message_chain_id=$3 AND worker.status='running'
              AND worker.lease_token=$4 AND worker.attempt_epoch=$5
              AND worker.checkpoint_version=$6
              AND worker.lease_expires_at>statement_timestamp()
              AND worker.active_tool_call_id IS NULL FOR SHARE"#,
    )
    .bind(input.primary_worker_fence.worker_run_id)
    .bind(round.primary.work_item_id)
    .bind(round.primary.message_chain_id)
    .bind(input.primary_worker_fence.lease_token)
    .bind(input.primary_worker_fence.attempt_epoch)
    .bind(input.primary_worker_fence.checkpoint_version)
    .bind(input.source_tool_call_record_id)
    .bind(&input.source_provider_call_id)
    .bind(round.operation_id)
    .bind(round.stage_execution_id)
    .bind(round.stage_run_unit_id)
    .bind(round.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    let source_primary_checkpoint_sha256 = sha256_on(&mut tx, &primary_checkpoint).await?;
    let canonical_turn_sha256 = sha256_on(&mut tx, &raw_turn).await?;
    let stored_turn: StoredDynamicPrimaryTurn =
        serde_json::from_value(raw_turn).map_err(|_| fail(CONTRACT_INVALID))?;
    let StoredDynamicPrimaryTurn::Delegate {
        schema_version,
        session_id,
        hypothesis_revision_id,
        subtasks,
    } = stored_turn
    else {
        return Err(fail(AUTHORITY_MISMATCH));
    };
    if schema_version != 1
        || session_id != round.session_id
        || hypothesis_revision_id != round.hypothesis_revision_id
        || subtasks.len() != input.actors.len()
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let turn_ordinal = round.consumed_primary_turns + 1;
    let first_actor_ordinal = round.consumed_actor_calls + 1;
    let mut actor_hashes = Vec::with_capacity(input.actors.len());
    let mut derived_actors = Vec::with_capacity(subtasks.len());
    for (actor, subtask) in input.actors.iter().zip(subtasks) {
        if !stored_dynamic_subtask_is_valid(&subtask)
            || !subtask
                .subject_refs
                .iter()
                .any(|subject| subject.kind == "target" && subject.id == round.target_live_id)
            || !subtask.subject_refs.iter().any(|subject| {
                subject.kind == "hypothesis_revision" && subject.id == round.hypothesis_revision_id
            })
            || dynamic_actor_agent(&subtask.role).is_err()
        {
            return Err(fail(CONTRACT_INVALID));
        }
        let objective_redacted = json!({"objective":subtask.objective,"rationale":subtask.rationale,
            "stable_key":subtask.stable_key,"subject_refs":subtask.subject_refs.iter()
                .map(|subject| json!({"kind":subject.kind,"id":subject.id})).collect::<Vec<_>>()});
        let objective_sha256 = sha256_on(&mut tx, &objective_redacted).await?;
        let role_allowed: bool = sqlx::query_scalar(
            "SELECT allowed_worker_roles ? $2 FROM stage_team_plans WHERE id=$1 FOR SHARE",
        )
        .bind(round.stage_team_plan_id)
        .bind(&subtask.role)
        .fetch_one(&mut *tx)
        .await?;
        if !role_allowed {
            return Err(fail(AUTHORITY_MISMATCH));
        }
        let turn_actor_ordinal = actor_hashes.len();
        actor_hashes.push(
            sha256_on(
                &mut tx,
                &json!({
                    "actor_call_id":actor.actor_call_id,
                    "turn_actor_ordinal":turn_actor_ordinal,
                    "specialist_role":subtask.role,
                    "objective_sha256":objective_sha256,
                }),
            )
            .await?,
        );
        derived_actors.push((
            actor.actor_call_id,
            subtask.role,
            objective_redacted,
            objective_sha256,
        ));
    }
    let actor_call_set_sha256 = sha256_on(&mut tx, &json!(actor_hashes)).await?;
    sqlx::query(
        r#"INSERT INTO investigation_dynamic_verification_primary_turns(
             primary_turn_id,stable_request_id,session_id,turn_ordinal,decision_kind,
             expected_session_head_version,
             source_primary_work_item_id,source_primary_worker_run_id,
             source_primary_lease_token,source_primary_attempt_epoch,
             consumer_primary_lease_token,consumer_primary_attempt_epoch,
             consumer_primary_checkpoint_version,consumer_primary_checkpoint_sha256,
             source_tool_call_record_id,source_provider_call_id,canonical_turn_sha256,
             actor_call_count,actor_call_set_sha256)
           VALUES($1,$2,$3,$4,'delegate',$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
    )
    .bind(input.primary_turn_id)
    .bind(input.stable_request_id)
    .bind(round.session_id)
    .bind(turn_ordinal)
    .bind(input.expected_session_head_version)
    .bind(round.primary.work_item_id)
    .bind(round.primary.worker_run_id)
    .bind(source_lease_token)
    .bind(source_attempt_epoch)
    .bind(input.primary_worker_fence.lease_token)
    .bind(input.primary_worker_fence.attempt_epoch)
    .bind(input.primary_worker_fence.checkpoint_version)
    .bind(&source_primary_checkpoint_sha256)
    .bind(input.source_tool_call_record_id)
    .bind(&input.source_provider_call_id)
    .bind(&canonical_turn_sha256)
    .bind(i64::try_from(input.actors.len()).map_err(|_| fail(CONTRACT_INVALID))?)
    .bind(&actor_call_set_sha256)
    .execute(&mut *tx)
    .await?;
    let task_session_id: Uuid = sqlx::query_scalar("SELECT session_id FROM tasks WHERE id=$1")
        .bind(round.operation_id)
        .fetch_one(&mut *tx)
        .await?;
    for (offset, (actor, actor_call_sha256)) in
        derived_actors.into_iter().zip(actor_hashes).enumerate()
    {
        let actor_ordinal =
            first_actor_ordinal + i64::try_from(offset).map_err(|_| fail(CONTRACT_INVALID))?;
        let (actor_call_id, specialist_role, objective_redacted, objective_sha256) = actor;
        let agent = dynamic_actor_agent(&specialist_role)?;
        let subtask_id = Uuid::new_v5(&actor_call_id, b"dynamic-verification-subtask-v2");
        let work_item_id = Uuid::new_v5(&actor_call_id, b"dynamic-verification-work-item-v2");
        let worker_run_id = Uuid::new_v5(&actor_call_id, b"dynamic-verification-worker-v2");
        let message_chain_id =
            Uuid::new_v5(&actor_call_id, b"dynamic-verification-message-chain-v2");
        sqlx::query(
            r#"INSERT INTO subtasks(id,task_id,session_id,title,description,agent,status)
               VALUES($1,$2,$3,$4,$5,$6,'created')"#,
        )
        .bind(subtask_id)
        .bind(round.operation_id)
        .bind(task_session_id)
        .bind(format!(
            "asset verification {specialist_role} #{actor_ordinal}"
        ))
        .bind(objective_redacted.to_string())
        .bind(agent)
        .execute(&mut *tx)
        .await?;
        message_chains::create_bound_with_executor(
            &mut *tx,
            message_chain_id,
            task_session_id,
            round.operation_id,
            Some(subtask_id),
            agent,
            None,
            None,
            &json!([]),
        )
        .await?;
        let input_refs = json!([{
            "kind":"investigation_dynamic_verification_actor",
            "session_id":round.session_id,
            "asset_lane_id":round.asset_lane_id,
            "target_id":round.target_live_id,
            "hypothesis_revision_id":round.hypothesis_revision_id,
            "primary_turn_id":input.primary_turn_id,
            "actor_call_id":actor_call_id,
            "actor_ordinal":actor_ordinal,
            "objective_sha256":objective_sha256,
        }]);
        let input_manifest_hash = sha256_on(&mut tx, &input_refs).await?;
        let work_item = stage_teams::insert_work_item_with_executor(
            &mut *tx,
            &stage_teams::NewStageWorkItem {
                id: work_item_id,
                team_plan_id: round.stage_team_plan_id,
                operation_id: round.operation_id,
                stage_execution_id: round.stage_execution_id,
                stage_run_unit_id: round.stage_run_unit_id,
                scope_snapshot_id: round.scope_snapshot_id,
                organization_id: round.organization_id,
                dispatch_epoch: round.dispatch_epoch,
                kind: "investigation_dynamic_verification_actor".into(),
                stable_key: format!(
                    "asset:{}:verification:{}:actor:{}",
                    round.asset_lane_id, round.hypothesis_revision_id, actor_ordinal
                ),
                role: specialist_role.clone(),
                input_manifest_hash,
                input_refs,
                required_for_barrier: false,
                conflict_key: None,
                priority: i32::try_from(actor_ordinal).map_err(|_| fail(CONTRACT_INVALID))?,
                attempt_policy: json!({"max_attempts":3}),
                budget: json!({}),
                output_schema: "investigation_dynamic_verification_actor_observation.v2".into(),
                created_by: "accepted_worker_request".into(),
            },
        )
        .await
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        stage_worker_runs::insert_with_executor(
            &mut *tx,
            &stage_worker_runs::NewStageWorkerRun {
                id: worker_run_id,
                operation_id: round.operation_id,
                stage_execution_id: round.stage_execution_id,
                stage_run_unit_id: round.stage_run_unit_id,
                work_item_id: Some(work_item.id),
                organization_id: round.organization_id,
                worker_generation: i32::try_from(actor_ordinal)
                    .map_err(|_| fail(CONTRACT_INVALID))?,
                specialist: specialist_role.clone(),
                work_item_kind: work_item.kind.clone(),
                work_item_key: work_item.stable_key.clone(),
                agent_path: format!(
                    "main>stage_run:investigation>org:{}>asset:{}>verification:{}>actor:{}:{}",
                    round.organization_id,
                    round.asset_lane_id,
                    round.hypothesis_revision_id,
                    actor_ordinal,
                    specialist_role
                ),
                parent_request_id: Some(format!(
                    "investigation-dynamic-verification-primary:{}",
                    round.primary.worker_run_id
                )),
            },
        )
        .await
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        sqlx::query(
            "UPDATE stage_worker_runs SET message_chain_id=$2,updated_at=NOW() \
             WHERE id=$1 AND status='queued' AND message_chain_id IS NULL",
        )
        .bind(worker_run_id)
        .bind(message_chain_id)
        .execute(&mut *tx)
        .await?;
        let request_id = Uuid::new_v5(&actor_call_id, b"dynamic-verification-worker-request-v2");
        let bounded_subject_refs = json!([
            {"kind":"target","id":round.target_live_id},
            {"kind":"hypothesis_revision","id":round.hypothesis_revision_id}
        ]);
        let request_reason = "investigation_dynamic_verification_primary_delegate";
        let request_dedupe_key = format!("dynamic-verification-actor:{actor_call_id}");
        let request_payload_hash = sha256_on(
            &mut tx,
            &json!({
                "domain":"investigation_dynamic_verification_actor_request.v2",
                "request_id":request_id,
                "session_id":round.session_id,
                "actor_call_id":actor_call_id,
                "parent_work_item_id":round.primary.work_item_id,
                "parent_worker_run_id":round.primary.worker_run_id,
                "dispatch_epoch":round.dispatch_epoch,
                "requested_role":specialist_role,
                "request_kind":"investigation_dynamic_verification_actor",
                "bounded_subject_refs":bounded_subject_refs,
                "reason_code":request_reason,
                "expected_output_schema":"investigation_dynamic_verification_actor_observation.v2",
                "budget_hint":{},
                "dedupe_key":request_dedupe_key,
                "accepted_work_item_id":work_item_id,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO stage_worker_requests(
                 id,team_plan_id,operation_id,stage_execution_id,stage_run_unit_id,
                 scope_snapshot_id,organization_id,parent_work_item_id,parent_worker_run_id,
                 dispatch_epoch,requested_role,request_kind,bounded_subject_refs,reason_code,
                 expected_output_schema,budget_hint,dedupe_key,request_payload_hash,status,
                 decision_reason_code,accepted_work_item_id)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,
                      'investigation_dynamic_verification_actor',$12,$13,
                      'investigation_dynamic_verification_actor_observation.v2','{}'::JSONB,
                      $14,$15,'accepted',NULL,$16)"#,
        )
        .bind(request_id)
        .bind(round.stage_team_plan_id)
        .bind(round.operation_id)
        .bind(round.stage_execution_id)
        .bind(round.stage_run_unit_id)
        .bind(round.scope_snapshot_id)
        .bind(round.organization_id)
        .bind(round.primary.work_item_id)
        .bind(round.primary.worker_run_id)
        .bind(round.dispatch_epoch)
        .bind(&specialist_role)
        .bind(&bounded_subject_refs)
        .bind(request_reason)
        .bind(&request_dedupe_key)
        .bind(&request_payload_hash)
        .bind(work_item_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO investigation_dynamic_verification_actor_calls(
               actor_call_id,stable_request_id,session_id,actor_ordinal,subtask_id,
               specialist_role,objective_redacted,objective_sha256,work_item_id,
               worker_run_id,message_chain_id,primary_turn_id,turn_actor_ordinal,
               actor_call_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(actor_call_id)
        .bind(Uuid::new_v5(
            &input.stable_request_id,
            format!("dynamic-actor:{offset}").as_bytes(),
        ))
        .bind(input.session_id)
        .bind(actor_ordinal)
        .bind(subtask_id)
        .bind(&specialist_role)
        .bind(&objective_redacted)
        .bind(&objective_sha256)
        .bind(work_item_id)
        .bind(worker_run_id)
        .bind(message_chain_id)
        .bind(input.primary_turn_id)
        .bind(i32::try_from(offset).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(actor_call_sha256)
        .execute(&mut *tx)
        .await?;
    }
    let advanced = sqlx::query(
        "UPDATE investigation_dynamic_verification_rounds \
         SET consumed_primary_turns=consumed_primary_turns+1, \
             consumed_actor_calls=consumed_actor_calls+$2 \
         WHERE session_id=$1 AND state='open' \
           AND consumed_primary_turns=$3 AND consumed_actor_calls=$4",
    )
    .bind(round.session_id)
    .bind(i64::try_from(input.actors.len()).map_err(|_| fail(CONTRACT_INVALID))?)
    .bind(round.consumed_primary_turns)
    .bind(round.consumed_actor_calls)
    .execute(&mut *tx)
    .await?;
    if advanced.rows_affected() != 1 {
        return Err(fail(CAS_CONFLICT));
    }
    let row = load_dynamic_primary_turn_on(&mut tx, input.primary_turn_id, false).await?;
    tx.commit().await?;
    Ok(row)
}
#[derive(Debug, Clone)]
pub struct ClaimDynamicVerificationActorInput {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub lease_owner: String,
    pub lease_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct ClaimDynamicVerificationPrimaryInput {
    pub session_id: Uuid,
    pub lease_owner: String,
    pub lease_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct ParkDynamicVerificationPrimaryInput {
    pub session_id: Uuid,
    pub worker_fence: VerificationWorkerFenceInput,
    pub checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ParkDynamicVerificationActorInput {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub worker_fence: VerificationWorkerFenceInput,
    pub checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CompleteDynamicVerificationActorInput {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub worker_fence: VerificationWorkerFenceInput,
    pub expected_work_item_row_version: i64,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub terminal_checkpoint: Value,
    pub evidence_watermark: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct PendingDynamicActorSubmissionRow {
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
    pub canonical_observation: Value,
    pub canonical_observation_sha256: String,
}

pub async fn load_pending_dynamic_actor_submission(
    pool: &PgPool,
    session_id: Uuid,
    actor_call_id: Uuid,
) -> Result<Option<PendingDynamicActorSubmissionRow>> {
    let mut tx = pool.begin().await?;
    let (round, actor) = load_dynamic_actor_on(&mut tx, session_id, actor_call_id, false).await?;
    reconcile_internal_submit_result_on(&mut tx, InternalSubmitResultOwner::Actor(&round, &actor))
        .await?;
    let pending = sqlx::query_as(
        r#"SELECT dynamic_round.session_id,actor.actor_call_id,
                  call.id AS source_tool_call_record_id,call.call_id AS source_provider_call_id,
                  call.args->'result' AS canonical_observation,
                  tool_truth_sha256((call.args->'result')::TEXT)
                    AS canonical_observation_sha256
             FROM investigation_dynamic_verification_rounds dynamic_round
             JOIN investigation_dynamic_verification_actor_calls actor
               ON actor.session_id=dynamic_round.session_id
             JOIN tool_calls call ON call.worker_run_id=actor.worker_run_id
              AND call.operation_id=dynamic_round.operation_id
              AND call.stage_execution_id=dynamic_round.stage_execution_id
              AND call.stage_run_unit_id=dynamic_round.stage_run_unit_id
              AND call.organization_id=dynamic_round.organization_id
              AND call.name='submit_result' AND call.status='finished'
              AND call.result IS NOT NULL
              AND call.result::JSONB->>'status'='result submitted' AND call.args ? 'result'
            WHERE dynamic_round.session_id=$1 AND actor.actor_call_id=$2
              AND actor.state<>'completed'
              AND NOT EXISTS(SELECT 1 FROM stage_worker_outputs output
                              WHERE output.work_item_id=actor.work_item_id
                                AND output.worker_run_id=actor.worker_run_id)
            ORDER BY call.updated_at,call.id LIMIT 1"#,
    )
    .bind(session_id)
    .bind(actor_call_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(pending)
}

async fn load_dynamic_actor_on(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    actor_call_id: Uuid,
    replayed: bool,
) -> Result<(DynamicVerificationRoundRow, DynamicVerificationActorCallRow)> {
    let round = load_dynamic_round_on(tx, session_id, replayed).await?;
    let actor = sqlx::query_as::<_, DynamicVerificationActorCallRow>(
        "SELECT actor_call.*,$3::BOOLEAN AS replayed FROM \
         investigation_dynamic_verification_actor_calls actor_call \
         WHERE actor_call.session_id=$1 AND actor_call.actor_call_id=$2",
    )
    .bind(session_id)
    .bind(actor_call_id)
    .bind(replayed)
    .fetch_one(&mut **tx)
    .await?;
    Ok((round, actor))
}

pub async fn claim_dynamic_primary(
    pool: &PgPool,
    input: &ClaimDynamicVerificationPrimaryInput,
) -> Result<ClaimedStageWorkItemRow> {
    let mut tx = pool.begin().await?;
    let round = load_dynamic_round_on(&mut tx, input.session_id, false).await?;
    let pending_terminalization = round.state == "resolved"
        && round.resolution_authority_id.is_some()
        && sqlx::query_scalar::<_, bool>(
            "SELECT NOT EXISTS(SELECT 1 FROM stage_worker_outputs output \
             WHERE output.work_item_id=$1 AND output.worker_run_id=$2) \
             AND EXISTS(SELECT 1 FROM stage_team_plans plan WHERE plan.id=$3 \
                        AND plan.requests_closed_at IS NULL)",
        )
        .bind(round.primary.work_item_id)
        .bind(round.primary.worker_run_id)
        .bind(round.stage_team_plan_id)
        .fetch_one(&mut *tx)
        .await?;
    if !((round.state == "open" && round.authorization_expires_at > Utc::now())
        || pending_terminalization)
        || input.lease_owner.trim().is_empty()
        || input.lease_seconds <= 0
    {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE"
    ))
    .bind(round.primary.work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    let mut current_worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(round.primary.worker_run_id)
    .fetch_one(&mut *tx)
    .await?;
    let expired_open_recovery = round.state == "open"
        && current_worker.status == "running"
        && current_worker
            .lease_expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        && current_worker.active_tool_call_id.is_none();
    if (pending_terminalization || expired_open_recovery)
        && current_worker.status == "running"
        && current_worker
            .lease_expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        && current_worker.active_tool_call_id.is_none()
        && !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM investigation_asset_verification_invocations \
             WHERE dynamic_session_id=$1 AND state='running')",
        )
        .bind(round.session_id)
        .fetch_one(&mut *tx)
        .await?
    {
        current_worker = sqlx::query_as(
            r#"UPDATE stage_worker_runs SET status='waiting_background',lease_token=NULL,
               lease_owner=NULL,lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
               attempt_epoch=attempt_epoch+1,updated_at=NOW()
               WHERE id=$1 AND status='running' AND lease_expires_at<=NOW()
                 AND active_tool_call_id IS NULL RETURNING *"#,
        )
        .bind(current_worker.id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE stage_work_items SET status='waiting_dependency',row_version=row_version+1,\
             updated_at=NOW() WHERE id=$1 AND status='running'",
        )
        .bind(item.id)
        .execute(&mut *tx)
        .await?;
    }
    let expected_status = match current_worker.status.as_str() {
        "queued" => stage_worker_runs::StageWorkerRunStatus::Queued,
        "waiting_background" => stage_worker_runs::StageWorkerRunStatus::WaitingBackground,
        _ => return Err(fail(CAS_CONFLICT)),
    };
    let worker = stage_worker_runs::claim_cas(
        &mut *tx,
        current_worker.id,
        round.stage_run_unit_id,
        expected_status,
        current_worker.attempt_epoch,
        Uuid::new_v4(),
        &input.lease_owner,
        input.lease_seconds,
    )
    .await
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let current_item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE"
    ))
    .bind(item.id)
    .fetch_one(&mut *tx)
    .await?;
    let expected_item_status = if current_item.status == "waiting_dependency" {
        "waiting_dependency"
    } else {
        "queued"
    };
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "UPDATE stage_work_items SET status='running',started_at=COALESCE(started_at,NOW()),\
         row_version=row_version+1,updated_at=NOW() WHERE id=$1 AND status=$2 \
         AND row_version=$3 RETURNING {ITEM_COLUMNS}"
    ))
    .bind(item.id)
    .bind(expected_item_status)
    .bind(current_item.row_version)
    .fetch_one(&mut *tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1"
    ))
    .bind(round.stage_team_plan_id)
    .fetch_one(&mut *tx)
    .await?;
    let unit = sqlx::query_as("SELECT * FROM stage_run_units WHERE id=$1")
        .bind(round.stage_run_unit_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ClaimedStageWorkItemRow {
        unit,
        plan,
        work_item: item,
        worker,
        message_chain_id: round.primary.message_chain_id,
    })
}

pub async fn park_dynamic_primary(
    pool: &PgPool,
    input: &ParkDynamicVerificationPrimaryInput,
) -> Result<ClaimedStageWorkItemRow> {
    let mut tx = pool.begin().await?;
    let round = load_dynamic_round_on(&mut tx, input.session_id, false).await?;
    if round.state != "open" || round.primary.worker_run_id != input.worker_fence.worker_run_id {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        r#"UPDATE stage_worker_runs SET status='waiting_background',checkpoint=$2,
           checkpoint_version=checkpoint_version+1,evidence_watermark=$3,
           lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,lease_expires_at=NULL,
           heartbeat_at=NULL,updated_at=NOW()
           WHERE id=$1 AND status='running' AND lease_token=$4 AND attempt_epoch=$5
             AND checkpoint_version=$6 AND active_tool_call_id IS NULL RETURNING *"#,
    )
    .bind(round.primary.worker_run_id)
    .bind(&input.checkpoint)
    .bind(input.evidence_watermark)
    .bind(input.worker_fence.lease_token)
    .bind(input.worker_fence.attempt_epoch)
    .bind(input.worker_fence.checkpoint_version)
    .fetch_one(&mut *tx)
    .await?;
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "UPDATE stage_work_items SET status='waiting_dependency',row_version=row_version+1,\
         updated_at=NOW() WHERE id=$1 AND status='running' RETURNING {ITEM_COLUMNS}"
    ))
    .bind(round.primary.work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1"
    ))
    .bind(round.stage_team_plan_id)
    .fetch_one(&mut *tx)
    .await?;
    let unit = sqlx::query_as("SELECT * FROM stage_run_units WHERE id=$1")
        .bind(round.stage_run_unit_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ClaimedStageWorkItemRow {
        unit,
        plan,
        work_item: item,
        worker,
        message_chain_id: round.primary.message_chain_id,
    })
}

pub async fn claim_dynamic_actor(
    pool: &PgPool,
    input: &ClaimDynamicVerificationActorInput,
) -> Result<ClaimedStageWorkItemRow> {
    let mut tx = pool.begin().await?;
    let (round, actor) =
        load_dynamic_actor_on(&mut tx, input.session_id, input.actor_call_id, false).await?;
    if round.state != "open"
        || round.authorization_expires_at <= Utc::now()
        || !matches!(actor.state.as_str(), "queued" | "parked" | "running")
        || input.lease_owner.trim().is_empty()
        || input.lease_seconds <= 0
    {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE"
    ))
    .bind(actor.work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    let mut current_worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(actor.worker_run_id)
    .fetch_one(&mut *tx)
    .await?;
    let expired_recovery = actor.state == "running"
        && current_worker
            .lease_expires_at
            .is_some_and(|expires_at| expires_at <= Utc::now())
        && current_worker.active_tool_call_id.is_none()
        && !sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM investigation_asset_verification_invocations \
             WHERE dynamic_session_id=$1 AND actor_call_id=$2 \
               AND state='running')",
        )
        .bind(round.session_id)
        .bind(actor.actor_call_id)
        .fetch_one(&mut *tx)
        .await?;
    if actor.state == "running" && !expired_recovery {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    if expired_recovery {
        current_worker = sqlx::query_as(
            r#"UPDATE stage_worker_runs SET status='waiting_background',lease_token=NULL,
               lease_owner=NULL,lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
               attempt_epoch=attempt_epoch+1,updated_at=NOW()
               WHERE id=$1 AND status='running' AND lease_expires_at<=NOW()
                 AND active_tool_call_id IS NULL RETURNING *"#,
        )
        .bind(current_worker.id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE stage_work_items SET status='waiting_dependency',row_version=row_version+1,\
             updated_at=NOW() WHERE id=$1 AND status='running'",
        )
        .bind(item.id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE investigation_dynamic_verification_actor_calls SET state='parked' \
             WHERE actor_call_id=$1 AND state='running'",
        )
        .bind(actor.actor_call_id)
        .execute(&mut *tx)
        .await?;
    }
    let effective_actor_state = if expired_recovery {
        "parked"
    } else {
        actor.state.as_str()
    };
    let expected_status = match current_worker.status.as_str() {
        "waiting_background" => stage_worker_runs::StageWorkerRunStatus::WaitingBackground,
        "queued" => stage_worker_runs::StageWorkerRunStatus::Queued,
        _ if effective_actor_state == "parked" => return Err(fail(CAS_CONFLICT)),
        _ => return Err(fail(CAS_CONFLICT)),
    };
    let worker = stage_worker_runs::claim_cas(
        &mut *tx,
        actor.worker_run_id,
        round.stage_run_unit_id,
        expected_status,
        current_worker.attempt_epoch,
        Uuid::new_v4(),
        &input.lease_owner,
        input.lease_seconds,
    )
    .await
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE"
    ))
    .bind(actor.work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    let next_item_status = if effective_actor_state == "parked" {
        "waiting_dependency"
    } else {
        "queued"
    };
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "UPDATE stage_work_items SET status='running',started_at=COALESCE(started_at,NOW()),\
         row_version=row_version+1,updated_at=NOW() WHERE id=$1 AND status=$2 \
         AND row_version=$3 RETURNING {ITEM_COLUMNS}"
    ))
    .bind(item.id)
    .bind(next_item_status)
    .bind(item.row_version)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE investigation_dynamic_verification_actor_calls SET state='running' \
         WHERE actor_call_id=$1 AND state=$2",
    )
    .bind(actor.actor_call_id)
    .bind(effective_actor_state)
    .execute(&mut *tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1"
    ))
    .bind(round.stage_team_plan_id)
    .fetch_one(&mut *tx)
    .await?;
    let unit = sqlx::query_as("SELECT * FROM stage_run_units WHERE id=$1")
        .bind(round.stage_run_unit_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ClaimedStageWorkItemRow {
        unit,
        plan,
        work_item: item,
        worker,
        message_chain_id: actor.message_chain_id,
    })
}

pub async fn load_dynamic_actor_completion(
    pool: &PgPool,
    session_id: Uuid,
    actor_call_id: Uuid,
) -> Result<Option<stage_teams::CompletedStageWorkerRow>> {
    let mut tx = pool.begin().await?;
    let (round, actor) = load_dynamic_actor_on(&mut tx, session_id, actor_call_id, true).await?;
    if actor.state != "completed" {
        tx.commit().await?;
        return Ok(None);
    }
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR SHARE"
    ))
    .bind(actor.work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR SHARE",
    )
    .bind(actor.worker_run_id)
    .fetch_one(&mut *tx)
    .await?;
    let output = sqlx::query_as::<_, stage_teams::StageWorkerOutputRow>(&format!(
        "SELECT {OUTPUT_COLUMNS} FROM stage_worker_outputs \
         WHERE work_item_id=$1 AND worker_run_id=$2"
    ))
    .bind(actor.work_item_id)
    .bind(actor.worker_run_id)
    .fetch_one(&mut *tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1"
    ))
    .bind(round.stage_team_plan_id)
    .fetch_one(&mut *tx)
    .await?;
    let unit = sqlx::query_as("SELECT * FROM stage_run_units WHERE id=$1")
        .bind(round.stage_run_unit_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(stage_teams::CompletedStageWorkerRow {
        unit,
        plan,
        work_item: item,
        worker,
        output,
        replayed: true,
    }))
}

pub async fn park_dynamic_actor(
    pool: &PgPool,
    input: &ParkDynamicVerificationActorInput,
) -> Result<ClaimedStageWorkItemRow> {
    let mut tx = pool.begin().await?;
    let (round, actor) =
        load_dynamic_actor_on(&mut tx, input.session_id, input.actor_call_id, false).await?;
    if actor.state != "running" || actor.worker_run_id != input.worker_fence.worker_run_id {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        r#"UPDATE stage_worker_runs SET status='waiting_background',checkpoint=$2,
           checkpoint_version=checkpoint_version+1,evidence_watermark=$3,
           lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,lease_expires_at=NULL,
           heartbeat_at=NULL,updated_at=NOW()
           WHERE id=$1 AND status='running' AND lease_token=$4 AND attempt_epoch=$5
             AND checkpoint_version=$6 AND active_tool_call_id IS NULL RETURNING *"#,
    )
    .bind(actor.worker_run_id)
    .bind(&input.checkpoint)
    .bind(input.evidence_watermark)
    .bind(input.worker_fence.lease_token)
    .bind(input.worker_fence.attempt_epoch)
    .bind(input.worker_fence.checkpoint_version)
    .fetch_one(&mut *tx)
    .await?;
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "UPDATE stage_work_items SET status='waiting_dependency',row_version=row_version+1,\
         updated_at=NOW() WHERE id=$1 AND status='running' RETURNING {ITEM_COLUMNS}"
    ))
    .bind(actor.work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE investigation_dynamic_verification_actor_calls SET state='parked' \
         WHERE actor_call_id=$1 AND state='running'",
    )
    .bind(actor.actor_call_id)
    .execute(&mut *tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1"
    ))
    .bind(round.stage_team_plan_id)
    .fetch_one(&mut *tx)
    .await?;
    let unit = sqlx::query_as("SELECT * FROM stage_run_units WHERE id=$1")
        .bind(round.stage_run_unit_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ClaimedStageWorkItemRow {
        unit,
        plan,
        work_item: item,
        worker,
        message_chain_id: actor.message_chain_id,
    })
}

pub async fn complete_dynamic_actor(
    pool: &PgPool,
    input: &CompleteDynamicVerificationActorInput,
) -> Result<stage_teams::CompletedStageWorkerRow> {
    let mut tx = pool.begin().await?;
    let (round, actor) =
        load_dynamic_actor_on(&mut tx, input.session_id, input.actor_call_id, false).await?;
    let canonical_output: Value = sqlx::query_scalar(
        r#"SELECT call.args->'result' FROM tool_calls call
            WHERE call.id=$1 AND call.call_id=$2 AND call.worker_run_id=$3
              AND call.operation_id=$4 AND call.stage_execution_id=$5
              AND call.stage_run_unit_id=$6 AND call.organization_id=$7
              AND call.name='submit_result' AND call.status='finished'
              AND call.result IS NOT NULL AND call.result::JSONB->>'status'='result submitted'
              AND call.args ? 'result' FOR SHARE"#,
    )
    .bind(input.source_tool_call_record_id)
    .bind(&input.source_provider_call_id)
    .bind(actor.worker_run_id)
    .bind(round.operation_id)
    .bind(round.stage_execution_id)
    .bind(round.stage_run_unit_id)
    .bind(round.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    let output_uuid = |field: &str| {
        canonical_output
            .get(field)
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
    };
    let cited_evidence_ids = canonical_output
        .get("cited_evidence_ids")
        .and_then(Value::as_array)
        .and_then(|values| values.iter().map(Value::as_i64).collect::<Option<Vec<_>>>())
        .ok_or_else(|| fail(CONTRACT_INVALID))?;
    let typed_output: StoredDynamicActorObservation =
        serde_json::from_value(canonical_output.clone()).map_err(|_| fail(CONTRACT_INVALID))?;
    let live_invocation_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM investigation_asset_verification_invocations \
         WHERE dynamic_session_id=$1 AND actor_call_id=$2 \
           AND state='running')",
    )
    .bind(round.session_id)
    .bind(actor.actor_call_id)
    .fetch_one(&mut *tx)
    .await?;
    if actor.state != "running"
        || actor.worker_run_id != input.worker_fence.worker_run_id
        || typed_output.schema_version != 1
        || output_uuid("session_id") != Some(round.session_id)
        || output_uuid("hypothesis_revision_id") != Some(round.hypothesis_revision_id)
        || output_uuid("actor_call_id") != Some(actor.actor_call_id)
        || canonical_output
            .get("actor_ordinal")
            .and_then(Value::as_i64)
            != Some(actor.actor_ordinal)
        || output_uuid("subtask_id") != Some(actor.subtask_id)
        || canonical_output
            .get("specialist_role")
            .and_then(Value::as_str)
            != Some(actor.specialist_role.as_str())
        || typed_output.summary.trim().is_empty()
        || typed_output.summary.chars().count() > 4_096
        || typed_output.new_hypothesis_proposals.len() > 64
        || typed_output
            .new_hypothesis_proposals
            .iter()
            .any(|proposal| !stored_dynamic_proposal_is_valid(proposal))
        || cited_evidence_ids.len() > 256
        || cited_evidence_ids.iter().any(|id| *id <= 0)
        || cited_evidence_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != cited_evidence_ids.len()
        || live_invocation_exists
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let exact_evidence_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM unnest($3::BIGINT[]) cited(evidence_id)
           WHERE EXISTS(
             SELECT 1 FROM investigation_asset_verification_invocations invocation
              WHERE invocation.dynamic_session_id=$1 AND invocation.actor_call_id=$2
                AND invocation.state='succeeded'
                AND cited.evidence_id=ANY(invocation.audit_evidence_ids))"#,
    )
    .bind(round.session_id)
    .bind(actor.actor_call_id)
    .bind(&cited_evidence_ids)
    .fetch_one(&mut *tx)
    .await?;
    if usize::try_from(exact_evidence_count).ok() != Some(cited_evidence_ids.len()) {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let generic = stage_teams::CompleteStageWorkerRow {
        fence: RuntimeMemoryTxFence {
            operation_id: round.operation_id,
            stage_execution_id: round.stage_execution_id,
            stage_run_unit_id: round.stage_run_unit_id,
            worker_run_id: input.worker_fence.worker_run_id,
            lease_token: input.worker_fence.lease_token,
            attempt_epoch: input.worker_fence.attempt_epoch,
            expected_checkpoint_version: input.worker_fence.checkpoint_version,
        },
        team_plan_id: round.stage_team_plan_id,
        work_item_id: actor.work_item_id,
        expected_work_item_row_version: input.expected_work_item_row_version,
        output_schema: "investigation_dynamic_verification_actor_observation.v2".into(),
        business_disposition: "artifact_recorded".into(),
        canonical_output: canonical_output.clone(),
        canonical_fact_refs: json!([]),
        evidence_ids: cited_evidence_ids.clone(),
        checked_empty_cells: json!([]),
        blocker_codes: vec![],
        output_hash: String::new(),
        terminal_checkpoint: input.terminal_checkpoint.clone(),
        evidence_watermark: input.evidence_watermark,
    };
    let output_hash = stage_teams::canonical_stage_worker_output_hash(&generic);
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1 FOR UPDATE"
    ))
    .bind(round.stage_team_plan_id)
    .fetch_one(&mut *tx)
    .await?;
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE"
    ))
    .bind(actor.work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    let unit = sqlx::query_as("SELECT * FROM stage_run_units WHERE id=$1 FOR SHARE")
        .bind(round.stage_run_unit_id)
        .fetch_one(&mut *tx)
        .await?;
    if item.status != "running"
        || item.row_version != input.expected_work_item_row_version
        || plan.requests_closed_at.is_some()
    {
        return Err(fail(CAS_CONFLICT));
    }
    let worker = stage_worker_runs::finish_passed_for_stage_output(
        &mut *tx,
        &generic.fence,
        &input.terminal_checkpoint,
        input.evidence_watermark,
    )
    .await
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "UPDATE stage_work_items SET status='completed',row_version=row_version+1,\
         terminal_at=NOW(),updated_at=NOW() WHERE id=$1 AND status='running' AND row_version=$2 \
         RETURNING {ITEM_COLUMNS}"
    ))
    .bind(actor.work_item_id)
    .bind(input.expected_work_item_row_version)
    .fetch_one(&mut *tx)
    .await?;
    let output = sqlx::query_as::<_, stage_teams::StageWorkerOutputRow>(&format!(
        r#"INSERT INTO stage_worker_outputs(id,team_plan_id,work_item_id,worker_run_id,
           operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
           output_schema,output_version,business_disposition,canonical_output,
           canonical_fact_refs,evidence_ids,checked_empty_cells,blocker_codes,output_hash)
           VALUES(uuid_generate_v5($1,'stage-worker-output-v1'),$2,$1,$3,$4,$5,$6,$7,$8,
             $9,1,$10,$11,$12,$13,$14,$15,$16) RETURNING {OUTPUT_COLUMNS}"#,
    ))
    .bind(item.id)
    .bind(plan.id)
    .bind(worker.id)
    .bind(round.operation_id)
    .bind(round.stage_execution_id)
    .bind(round.stage_run_unit_id)
    .bind(round.scope_snapshot_id)
    .bind(round.organization_id)
    .bind("investigation_dynamic_verification_actor_observation.v2")
    .bind("artifact_recorded")
    .bind(&canonical_output)
    .bind(json!([]))
    .bind(&cited_evidence_ids)
    .bind(json!([]))
    .bind(Vec::<String>::new())
    .bind(&output_hash)
    .fetch_one(&mut *tx)
    .await?;
    let canonical_observation_sha256 = sha256_on(&mut tx, &canonical_output).await?;
    sqlx::query(
        "UPDATE investigation_dynamic_verification_actor_calls SET state='completed',\
         completed_at=NOW(),source_tool_call_record_id=$2,source_provider_call_id=$3,\
         canonical_observation_sha256=$4 WHERE actor_call_id=$1 AND state='running'",
    )
    .bind(actor.actor_call_id)
    .bind(input.source_tool_call_record_id)
    .bind(&input.source_provider_call_id)
    .bind(canonical_observation_sha256)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(stage_teams::CompletedStageWorkerRow {
        unit,
        plan,
        work_item: item,
        worker,
        output,
        replayed: false,
    })
}

#[derive(Debug, Clone, FromRow)]
pub struct AssetVerificationCandidateRow {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_root_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub hypothesis_revision_sha256: String,
    pub hypothesis_claim: Value,
    pub hypothesis_claim_sha256: String,
    pub falsification_conditions: Value,
    pub falsification_conditions_sha256: String,
    pub verification_objectives: Value,
    pub verification_objectives_sha256: String,
    pub hypothesis_head_version: i64,
    pub verification_task_id: Uuid,
    pub verification_plan_id: Uuid,
    pub verification_plan_sha256: String,
    pub priority: i32,
    pub existing_open_round_id: Option<Uuid>,
}

/// Select the next unresolved canonical head inside one server-frozen asset
/// lane. Revision/task ids are intentionally absent from the input boundary.
pub async fn load_next_unresolved_current_hypothesis(
    pool: &PgPool,
    operation_id: Uuid,
    asset_lane_id: Uuid,
) -> Result<Option<AssetVerificationCandidateRow>> {
    validate_uuid(operation_id)?;
    validate_uuid(asset_lane_id)?;
    Ok(sqlx::query_as::<_, AssetVerificationCandidateRow>(
        r#"SELECT task.operation_id,task.stage_execution_id,task.stage_run_unit_id,
                  task.scope_snapshot_id,task.organization_id,lane.asset_lane_id,
                  lane.target_id AS target_live_id,root.root_id AS hypothesis_root_id,
                  revision.revision_id AS hypothesis_revision_id,
                  revision.revision_hash AS hypothesis_revision_sha256,
                  revision.structured_claim AS hypothesis_claim,
                  tool_truth_sha256(revision.structured_claim::TEXT) AS hypothesis_claim_sha256,
                  COALESCE(revision.missing_facts,'[]'::JSONB) AS falsification_conditions,
                  tool_truth_sha256(COALESCE(revision.missing_facts,'[]'::JSONB)::TEXT)
                    AS falsification_conditions_sha256,
                  COALESCE((SELECT jsonb_agg(jsonb_build_object(
                       'objective_id',objective.objective_id,
                       'objective_ordinal',objective.objective_ordinal,
                       'objective_intent',objective.objective_intent,
                       'stopping_criteria',objective.stopping_criteria,
                       'objective_hash',objective.objective_hash)
                       ORDER BY objective.objective_ordinal,objective.objective_id)
                     FROM attack_hypothesis_verification_objectives objective
                    WHERE objective.revision_id=revision.revision_id),'[]'::JSONB)
                    AS verification_objectives,
                  tool_truth_sha256(COALESCE((SELECT jsonb_agg(objective.objective_hash
                       ORDER BY objective.objective_ordinal,objective.objective_id)::TEXT
                     FROM attack_hypothesis_verification_objectives objective
                    WHERE objective.revision_id=revision.revision_id),'[]'))
                    AS verification_objectives_sha256,
                  head.head_version AS hypothesis_head_version,
                  task.task_id AS verification_task_id,task.verification_plan_id,
                  task.verification_plan_sha256,revision.priority,
                  dynamic_round.session_id AS existing_open_round_id
             FROM investigation_asset_lanes lane
             JOIN attack_hypotheses root ON root.asset_lane_id=lane.asset_lane_id
             JOIN attack_hypothesis_heads head ON head.root_id=root.root_id
             JOIN attack_hypothesis_revisions revision
               ON revision.revision_id=head.head_revision_id
              AND revision.asset_lane_id=lane.asset_lane_id
              AND revision.target_live_id=lane.target_id
             JOIN hypothesis_verification_tasks task
               ON task.hypothesis_revision_id=revision.revision_id
              AND task.asset_lane_id=lane.asset_lane_id
             LEFT JOIN investigation_dynamic_verification_rounds dynamic_round
               ON dynamic_round.hypothesis_revision_id=revision.revision_id
              AND dynamic_round.asset_lane_id=lane.asset_lane_id
              AND dynamic_round.state='open'
            WHERE lane.operation_id=$1 AND lane.asset_lane_id=$2
              AND lane.state='verifying'
              AND root.operation_id=$1
              AND head.operation_id=$1
              AND head.head_lifecycle_state='current'
              AND revision.lifecycle_state='current'
              AND revision.epistemic_state NOT IN('verified','refuted','invalid')
              AND task.operation_id=$1
              AND NOT EXISTS(
                    SELECT 1 FROM investigation_dynamic_hypothesis_resolutions resolution
                     WHERE resolution.hypothesis_revision_id=revision.revision_id)
            ORDER BY revision.priority DESC,revision.created_at,revision.revision_id,
                     task.created_at,task.task_id
            LIMIT 1"#,
    )
    .bind(operation_id)
    .bind(asset_lane_id)
    .fetch_optional(pool)
    .await?)
}

#[derive(Debug, Clone)]
pub struct DynamicToolInventoryMemberInput {
    pub tool_id: String,
    pub tool_name: String,
    pub config_sha256: String,
    pub executable_identity_sha256: String,
    pub runtime: String,
    pub runtime_version: String,
    pub launch_mode: String,
    pub parameter_schema: Value,
    pub output_schema: Value,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FreezeDynamicToolInventoryInput {
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub inventory_source_sha256: String,
    pub members: Vec<DynamicToolInventoryMemberInput>,
}

#[derive(Debug, Clone, FromRow)]
pub struct DynamicToolInventoryMemberRow {
    pub inventory_member_id: Uuid,
    pub member_ordinal: i32,
    pub tool_id: String,
    pub tool_name: String,
    pub config_sha256: String,
    pub executable_identity_sha256: String,
    pub runtime: String,
    pub runtime_version: String,
    pub launch_mode: String,
    pub parameter_schema: Value,
    pub output_schema: Value,
    pub tags: Value,
    pub member_sha256: String,
}

#[derive(Debug, Clone)]
pub struct DynamicToolInventoryRow {
    pub inventory_snapshot_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub inventory_source_sha256: String,
    pub member_count: i64,
    pub member_set_sha256: String,
    pub members: Vec<DynamicToolInventoryMemberRow>,
    pub sealed_at: DateTime<Utc>,
    pub replayed: bool,
}

#[derive(FromRow)]
struct InventoryHeader {
    inventory_snapshot_id: Uuid,
    stable_request_id: Uuid,
    session_id: Option<Uuid>,
    dynamic_session_id: Option<Uuid>,
    inventory_source_sha256: String,
    member_count: i64,
    member_set_sha256: String,
    sealed_at: DateTime<Utc>,
}

async fn load_inventory_on(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    replayed: bool,
) -> Result<DynamicToolInventoryRow> {
    let header = sqlx::query_as::<_, InventoryHeader>("SELECT * FROM investigation_dynamic_tool_inventory_snapshots WHERE inventory_snapshot_id=$1")
        .bind(id).fetch_one(&mut **tx).await?;
    let members = sqlx::query_as::<_, DynamicToolInventoryMemberRow>(
        r#"SELECT inventory_member_id,
        member_ordinal,tool_id,tool_name,config_sha256,executable_identity_sha256,runtime,
        runtime_version,launch_mode,parameter_schema,output_schema,tags,member_sha256
        FROM investigation_dynamic_tool_inventory_members WHERE inventory_snapshot_id=$1
        ORDER BY member_ordinal"#,
    )
    .bind(id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(DynamicToolInventoryRow {
        inventory_snapshot_id: header.inventory_snapshot_id,
        stable_request_id: header.stable_request_id,
        session_id: header
            .dynamic_session_id
            .or(header.session_id)
            .ok_or_else(|| fail(AUTHORITY_MISMATCH))?,
        inventory_source_sha256: header.inventory_source_sha256,
        member_count: header.member_count,
        member_set_sha256: header.member_set_sha256,
        members,
        sealed_at: header.sealed_at,
        replayed,
    })
}

pub async fn freeze_dynamic_inventory(
    pool: &PgPool,
    input: &FreezeDynamicToolInventoryInput,
) -> Result<DynamicToolInventoryRow> {
    if !valid_sha256(&input.inventory_source_sha256) {
        return Err(fail(CONTRACT_INVALID));
    }
    let inventory_snapshot_id = Uuid::new_v5(
        &input.session_id,
        format!(
            "investigation-dynamic-tool-inventory-v1:{}",
            input.inventory_source_sha256
        )
        .as_bytes(),
    );
    let mut members = input.members.clone();
    members.sort_by(|left, right| {
        left.tool_name
            .cmp(&right.tool_name)
            .then(left.tool_id.cmp(&right.tool_id))
    });
    if members
        .windows(2)
        .any(|pair| pair[0].tool_name == pair[1].tool_name)
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    if let Some(id) = sqlx::query_scalar::<_, Uuid>("SELECT inventory_snapshot_id FROM investigation_dynamic_tool_inventory_snapshots WHERE stable_request_id=$1")
        .bind(input.stable_request_id).fetch_optional(&mut *tx).await? {
        if id != inventory_snapshot_id { return Err(fail(REPLAY_DRIFT)); }
        let row = load_inventory_on(&mut tx, id, true).await?;
        let mut requested_hashes = Vec::with_capacity(members.len());
        for member in &members {
            let mut tags = member.tags.clone();
            tags.sort();
            tags.dedup();
            requested_hashes.push(
                sha256_on(
                    &mut tx,
                    &json!({"tool_id":member.tool_id,"tool_name":member.tool_name,
                    "config_sha256":member.config_sha256,
                    "executable_identity_sha256":member.executable_identity_sha256,
                    "runtime":member.runtime,"runtime_version":member.runtime_version,
                    "launch_mode":member.launch_mode,"parameter_schema":member.parameter_schema,
                    "output_schema":member.output_schema,"tags":tags}),
                )
                .await?,
            );
        }
        let stored_hashes = row
            .members
            .iter()
            .map(|member| member.member_sha256.clone())
            .collect::<Vec<_>>();
        if row.session_id != input.session_id
            || row.inventory_source_sha256 != input.inventory_source_sha256
            || requested_hashes != stored_hashes
        {
            return Err(fail(REPLAY_DRIFT));
        }
        tx.commit().await?; return Ok(row);
    }
    let mut hashed = Vec::with_capacity(members.len());
    for member in members {
        if !valid_sha256(&member.config_sha256)
            || !valid_sha256(&member.executable_identity_sha256)
            || member.tool_id.trim().is_empty()
            || member.tool_name.trim().is_empty()
        {
            return Err(fail(CONTRACT_INVALID));
        }
        let mut tags = member.tags.clone();
        tags.sort();
        tags.dedup();
        let member_sha256 = sha256_on(
            &mut tx,
            &json!({"tool_id":member.tool_id,
            "tool_name":member.tool_name,"config_sha256":member.config_sha256,
            "executable_identity_sha256":member.executable_identity_sha256,
            "runtime":member.runtime,"runtime_version":member.runtime_version,
            "launch_mode":member.launch_mode,"parameter_schema":member.parameter_schema,
            "output_schema":member.output_schema,"tags":tags}),
        )
        .await?;
        hashed.push((member, tags, member_sha256));
    }
    let member_hashes = hashed
        .iter()
        .map(|(_, _, hash)| hash.clone())
        .collect::<Vec<_>>();
    let member_set_sha256: String = sqlx::query_scalar("SELECT tool_truth_sha256($1::JSONB::TEXT)")
        .bind(json!(member_hashes))
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO investigation_dynamic_tool_inventory_snapshots(inventory_snapshot_id,stable_request_id,dynamic_session_id,inventory_source_sha256,member_count,member_set_sha256) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(inventory_snapshot_id).bind(input.stable_request_id).bind(input.session_id)
        .bind(&input.inventory_source_sha256).bind(i64::try_from(hashed.len()).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(&member_set_sha256).execute(&mut *tx).await?;
    for (ordinal, (member, tags, member_sha256)) in hashed.into_iter().enumerate() {
        let member_id = Uuid::new_v5(&inventory_snapshot_id, member_sha256.as_bytes());
        sqlx::query(
            r#"INSERT INTO investigation_dynamic_tool_inventory_members(
          inventory_member_id,inventory_snapshot_id,member_ordinal,tool_id,tool_name,installed,
          environment_ready,config_sha256,executable_identity_sha256,runtime,runtime_version,
          launch_mode,parameter_schema,output_schema,tags,member_sha256)
          VALUES($1,$2,$3,$4,$5,TRUE,TRUE,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(member_id)
        .bind(inventory_snapshot_id)
        .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(member.tool_id)
        .bind(member.tool_name)
        .bind(member.config_sha256)
        .bind(member.executable_identity_sha256)
        .bind(member.runtime)
        .bind(member.runtime_version)
        .bind(member.launch_mode)
        .bind(member.parameter_schema)
        .bind(member.output_schema)
        .bind(json!(tags))
        .bind(member_sha256)
        .execute(&mut *tx)
        .await?;
    }
    let row = load_inventory_on(&mut tx, inventory_snapshot_id, false).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Debug, Clone)]
pub struct VerificationWorkerFenceInput {
    pub worker_run_id: Uuid,
    pub lease_token: Uuid,
    pub attempt_epoch: i64,
    pub checkpoint_version: i64,
}

#[derive(Debug, Clone)]
pub struct BeginAssetVerificationInvocationInput {
    pub stable_request_id: Uuid,
    pub invocation_id: Uuid,
    pub session_id: Uuid,
    pub actor_call_id: Uuid,
    pub worker_fence: VerificationWorkerFenceInput,
    pub wrapper_name: String,
    pub selected_tool_name: Option<String>,
    pub credential_binding_sha256: Option<String>,
    pub model_args_redacted: Value,
    pub model_args_sha256: String,
}

#[derive(Debug, Clone)]
pub struct CompleteAssetVerificationInvocationInput {
    pub stable_request_id: Uuid,
    pub invocation_id: Uuid,
    pub expected_row_version: i64,
    pub worker_fence: VerificationWorkerFenceInput,
    pub disposition: String,
    pub capability_execution_receipt_id: Option<Uuid>,
    pub oracle_receipt_id: Option<Uuid>,
    pub audit_evidence_ids: Vec<i64>,
    pub evidence_set_sha256: String,
    pub redacted_result: Value,
    pub result_sha256: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct AssetVerificationInvocationRow {
    pub invocation_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Option<Uuid>,
    pub dynamic_session_id: Option<Uuid>,
    pub invocation_ordinal: i64,
    pub actor_call_id: Option<Uuid>,
    pub actor_ordinal: Option<i64>,
    pub actor_subtask_id: Option<Uuid>,
    pub actor_role: String,
    pub actor_work_item_id: Uuid,
    pub actor_worker_run_id: Uuid,
    pub actor_message_chain_id: Uuid,
    pub inventory_snapshot_id: Uuid,
    pub inventory_member_id: Option<Uuid>,
    pub wrapper_name: String,
    pub selected_tool_name: Option<String>,
    pub selected_tool_config_sha256: Option<String>,
    pub invocation_authorization_id: Uuid,
    pub invocation_authorization_sha256: String,
    pub invocation_authorization_expires_at: DateTime<Utc>,
    pub effect_class: String,
    pub risk_tier: String,
    pub credential_binding_sha256: Option<String>,
    pub network_request_limit: i64,
    pub wall_time_limit_ms: i64,
    pub output_byte_limit: i64,
    pub model_args_redacted: Value,
    pub model_args_sha256: String,
    pub request_manifest_sha256: String,
    pub started_lease_token: Uuid,
    pub started_attempt_epoch: i64,
    pub started_checkpoint_version: i64,
    pub state: String,
    pub row_version: i64,
    pub capability_execution_receipt_id: Option<Uuid>,
    pub oracle_receipt_id: Option<Uuid>,
    pub audit_evidence_ids: Vec<i64>,
    pub evidence_set_sha256: Option<String>,
    pub redacted_result: Option<Value>,
    pub result_sha256: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub replayed: bool,
}

const LOAD_INVOCATION_SQL: &str = r#"SELECT invocation.*,actor.actor_ordinal,
 actor.subtask_id AS actor_subtask_id,$2::BOOLEAN AS replayed
  FROM investigation_asset_verification_invocations invocation
  LEFT JOIN investigation_dynamic_verification_actor_calls actor
    ON actor.actor_call_id=invocation.actor_call_id
 WHERE invocation.invocation_id=$1"#;

async fn load_invocation_on(
    tx: &mut Transaction<'_, Postgres>,
    invocation_id: Uuid,
    replayed: bool,
) -> Result<AssetVerificationInvocationRow> {
    Ok(sqlx::query_as(LOAD_INVOCATION_SQL)
        .bind(invocation_id)
        .bind(replayed)
        .fetch_one(&mut **tx)
        .await?)
}

pub async fn begin_invocation(
    pool: &PgPool,
    input: &BeginAssetVerificationInvocationInput,
) -> Result<AssetVerificationInvocationRow> {
    if !valid_sha256(&input.model_args_sha256)
        || input
            .credential_binding_sha256
            .as_deref()
            .is_some_and(|hash| !valid_sha256(hash))
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    if let Some(existing_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT invocation_id FROM investigation_asset_verification_invocations WHERE stable_request_id=$1")
        .bind(input.stable_request_id).fetch_optional(&mut *tx).await? {
        if existing_id != input.invocation_id { return Err(fail(REPLAY_DRIFT)); }
        let row = load_invocation_on(&mut tx, existing_id, true).await?;
        if row.dynamic_session_id != Some(input.session_id)
            || row.actor_call_id != Some(input.actor_call_id)
            || row.wrapper_name != input.wrapper_name
            || row.selected_tool_name != input.selected_tool_name
            || row.credential_binding_sha256 != input.credential_binding_sha256
            || row.model_args_redacted != input.model_args_redacted
            || row.model_args_sha256 != input.model_args_sha256
            || row.actor_worker_run_id != input.worker_fence.worker_run_id
            || row.started_lease_token != input.worker_fence.lease_token
            || row.started_attempt_epoch != input.worker_fence.attempt_epoch
            || row.started_checkpoint_version != input.worker_fence.checkpoint_version
        { return Err(fail(REPLAY_DRIFT)); }
        tx.commit().await?; return Ok(row);
    }
    let actor: (Uuid, i64, Uuid, String, Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT actor.actor_call_id,actor.actor_ordinal,actor.subtask_id,
                  actor.specialist_role,actor.work_item_id,actor.worker_run_id,
                  actor.message_chain_id
             FROM investigation_dynamic_verification_actor_calls actor
             JOIN investigation_dynamic_verification_rounds dynamic_round
               ON dynamic_round.session_id=actor.session_id
            WHERE actor.session_id=$1 AND actor.actor_call_id=$2
              AND actor.state='running' AND dynamic_round.state='open' FOR SHARE"#,
    )
    .bind(input.session_id)
    .bind(input.actor_call_id)
    .fetch_one(&mut *tx)
    .await?;
    if actor.5 != input.worker_fence.worker_run_id {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let inventory_snapshot_id: Uuid = sqlx::query_scalar(
        r#"SELECT snapshot.inventory_snapshot_id
             FROM investigation_dynamic_tool_inventory_snapshots snapshot
            WHERE snapshot.dynamic_session_id=$1
            ORDER BY snapshot.sealed_at DESC,snapshot.inventory_snapshot_id DESC LIMIT 1
            FOR SHARE"#,
    )
    .bind(input.session_id)
    .fetch_one(&mut *tx)
    .await?;
    let (inventory_member_id, selected_tool_config_sha256): (Option<Uuid>, Option<String>) =
        if let Some(tool_name) = &input.selected_tool_name {
            let member: (Uuid, String) = sqlx::query_as(
            "SELECT inventory_member_id,config_sha256 FROM investigation_dynamic_tool_inventory_members \
             WHERE inventory_snapshot_id=$1 AND tool_name=$2 AND installed AND environment_ready",
        )
        .bind(inventory_snapshot_id)
        .bind(tool_name)
        .fetch_one(&mut *tx)
        .await?;
            (Some(member.0), Some(member.1))
        } else {
            (None, None)
        };
    let derived: (String, String, i64, i64, i64, DateTime<Utc>) = sqlx::query_as(
        r#"SELECT CASE
              WHEN $2 IN('pentest_list_tools','pentest_read_skill') THEN 'read_only'
              WHEN allowed_effect_classes ? 'active_network' THEN 'active_network'
              WHEN allowed_effect_classes ? 'passive_network' THEN 'passive_network'
              ELSE 'read_only' END,
            maximum_risk_tier,
            CASE WHEN $2 IN('pentest_list_tools','pentest_read_skill') THEN 0
                 ELSE LEAST(remaining_network_requests,64) END,
            LEAST(remaining_wall_time_ms,120000),
            LEAST(remaining_output_bytes,1048576),
            LEAST(session_row.authorization_expires_at,
                  statement_timestamp()+INTERVAL '15 minutes')
           FROM investigation_dynamic_verification_rounds session_row
           JOIN investigation_asset_verification_authorizations authz
             ON authz.session_authorization_id=session_row.session_authorization_id
           JOIN investigation_asset_verification_budget_envelopes budget
             ON budget.session_budget_envelope_id=session_row.session_budget_envelope_id
          WHERE session_row.session_id=$1 AND session_row.state='open'
            AND session_row.authorization_expires_at>statement_timestamp()
            AND budget.remaining_invocations>0
          FOR UPDATE OF session_row,budget"#,
    )
    .bind(input.session_id)
    .bind(&input.wrapper_name)
    .fetch_one(&mut *tx)
    .await?;
    if derived.2 < 0 || derived.3 <= 0 || derived.4 <= 0 {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let request_manifest_sha256 = sha256_on(
        &mut tx,
        &json!({
            "domain":"investigation_asset_verification_invocation_request.v1",
            "invocation_id":input.invocation_id,"session_id":input.session_id,
            "inventory_snapshot_id":inventory_snapshot_id,"inventory_member_id":inventory_member_id,
            "wrapper_name":input.wrapper_name,"selected_tool_name":input.selected_tool_name,
            "selected_tool_config_sha256":selected_tool_config_sha256,
            "credential_binding_sha256":input.credential_binding_sha256,
            "model_args_sha256":input.model_args_sha256,
        }),
    )
    .await?;
    let ordinal: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(invocation_ordinal),0)+1 FROM investigation_asset_verification_invocations WHERE dynamic_session_id=$1")
        .bind(input.session_id).fetch_one(&mut *tx).await?;
    let invocation_authorization_id = Uuid::new_v5(
        &input.session_id,
        format!(
            "investigation-asset-verification-invocation-authorization-v1:{}",
            input.invocation_id
        )
        .as_bytes(),
    );
    let invocation_authorization_sha256: String = sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(jsonb_build_object(
             'domain','investigation_asset_verification_invocation_authorization.v1',
             'invocation_id',$1::UUID,'session_id',$2::UUID,
             'inventory_snapshot_id',$3::UUID,'inventory_member_id',$4::UUID,
             'wrapper_name',$5::TEXT,'selected_tool_name',$6::TEXT,
             'selected_tool_config_sha256',$7::TEXT,
             'effect_class',$8::TEXT,'risk_tier',$9::TEXT,
             'credential_binding_sha256',$10::TEXT,
             'network_request_limit',$11::BIGINT,
             'wall_time_limit_ms',$12::BIGINT,'output_byte_limit',$13::BIGINT,
             'model_args_sha256',$14::TEXT,
             'request_manifest_sha256',$15::TEXT,
             'expires_at',$16::TIMESTAMPTZ)::TEXT)"#,
    )
    .bind(input.invocation_id)
    .bind(input.session_id)
    .bind(inventory_snapshot_id)
    .bind(inventory_member_id)
    .bind(&input.wrapper_name)
    .bind(&input.selected_tool_name)
    .bind(&selected_tool_config_sha256)
    .bind(&derived.0)
    .bind(&derived.1)
    .bind(&input.credential_binding_sha256)
    .bind(derived.2)
    .bind(derived.3)
    .bind(derived.4)
    .bind(&input.model_args_sha256)
    .bind(&request_manifest_sha256)
    .bind(derived.5)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_asset_verification_invocations(
      invocation_id,stable_request_id,dynamic_session_id,invocation_ordinal,actor_call_id,actor_role,
      actor_work_item_id,actor_worker_run_id,actor_message_chain_id,inventory_snapshot_id,
      inventory_member_id,wrapper_name,selected_tool_name,selected_tool_config_sha256,
      invocation_authorization_id,invocation_authorization_sha256,
      invocation_authorization_expires_at,effect_class,risk_tier,credential_binding_sha256,
      network_request_limit,wall_time_limit_ms,output_byte_limit,model_args_redacted,
      model_args_sha256,request_manifest_sha256,started_lease_token,started_attempt_epoch,
      started_checkpoint_version,state,row_version)
      VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,
             $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,'running',0)"#,
    )
    .bind(input.invocation_id)
    .bind(input.stable_request_id)
    .bind(input.session_id)
    .bind(ordinal)
    .bind(input.actor_call_id)
    .bind(&actor.3)
    .bind(actor.4)
    .bind(input.worker_fence.worker_run_id)
    .bind(actor.6)
    .bind(inventory_snapshot_id)
    .bind(inventory_member_id)
    .bind(&input.wrapper_name)
    .bind(&input.selected_tool_name)
    .bind(&selected_tool_config_sha256)
    .bind(invocation_authorization_id)
    .bind(invocation_authorization_sha256)
    .bind(derived.5)
    .bind(&derived.0)
    .bind(&derived.1)
    .bind(&input.credential_binding_sha256)
    .bind(derived.2)
    .bind(derived.3)
    .bind(derived.4)
    .bind(&input.model_args_redacted)
    .bind(&input.model_args_sha256)
    .bind(&request_manifest_sha256)
    .bind(input.worker_fence.lease_token)
    .bind(input.worker_fence.attempt_epoch)
    .bind(input.worker_fence.checkpoint_version)
    .execute(&mut *tx)
    .await?;
    let row = load_invocation_on(&mut tx, input.invocation_id, false).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn complete_invocation(
    pool: &PgPool,
    input: &CompleteAssetVerificationInvocationInput,
) -> Result<AssetVerificationInvocationRow> {
    if !matches!(
        input.disposition.as_str(),
        "succeeded" | "failed" | "outcome_unknown"
    ) || !valid_sha256(&input.evidence_set_sha256)
        || !valid_sha256(&input.result_sha256)
        || input.audit_evidence_ids.iter().any(|id| *id <= 0)
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as::<_, AssetVerificationInvocationRow>(LOAD_INVOCATION_SQL)
        .bind(input.invocation_id)
        .bind(true)
        .fetch_optional(&mut *tx)
        .await?
    {
        if existing.state != "running" {
            if existing.stable_request_id == input.stable_request_id
                && existing.state == input.disposition
                && existing.row_version == input.expected_row_version + 1
                && existing.capability_execution_receipt_id == input.capability_execution_receipt_id
                && existing.oracle_receipt_id == input.oracle_receipt_id
                && existing.audit_evidence_ids == input.audit_evidence_ids
                && existing.evidence_set_sha256.as_deref()
                    == Some(input.evidence_set_sha256.as_str())
                && existing.redacted_result.as_ref() == Some(&input.redacted_result)
                && existing.result_sha256.as_deref() == Some(&input.result_sha256)
            {
                tx.commit().await?;
                return Ok(existing);
            }
            return Err(fail(REPLAY_DRIFT));
        }
    } else {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let updated = sqlx::query(
        r#"UPDATE investigation_asset_verification_invocations invocation
        SET state=$1,row_version=row_version+1,capability_execution_receipt_id=$2,
            oracle_receipt_id=$3,audit_evidence_ids=$4,evidence_set_sha256=$5,
            redacted_result=$6,result_sha256=$7,completed_at=statement_timestamp()
        FROM stage_worker_runs worker,
             investigation_dynamic_verification_actor_calls actor
       WHERE invocation.invocation_id=$8 AND invocation.state='running'
         AND invocation.stable_request_id=$14
         AND actor.actor_call_id=invocation.actor_call_id
         AND invocation.dynamic_session_id=actor.session_id
         AND invocation.row_version=$9 AND worker.id=actor.worker_run_id
         AND worker.id=$10 AND worker.lease_token=$11 AND worker.attempt_epoch=$12
         AND worker.checkpoint_version=$13 AND worker.lease_expires_at>statement_timestamp()"#,
    )
    .bind(&input.disposition)
    .bind(input.capability_execution_receipt_id)
    .bind(input.oracle_receipt_id)
    .bind(&input.audit_evidence_ids)
    .bind(&input.evidence_set_sha256)
    .bind(&input.redacted_result)
    .bind(&input.result_sha256)
    .bind(input.invocation_id)
    .bind(input.expected_row_version)
    .bind(input.worker_fence.worker_run_id)
    .bind(input.worker_fence.lease_token)
    .bind(input.worker_fence.attempt_epoch)
    .bind(input.worker_fence.checkpoint_version)
    .bind(input.stable_request_id)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(fail(CAS_CONFLICT));
    }
    let row = load_invocation_on(&mut tx, input.invocation_id, false).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Debug, Clone, FromRow)]
pub struct DynamicVerificationInvocationAuthorityRow {
    pub invocation_id: Uuid,
    pub actor_call_id: Uuid,
    pub actor_ordinal: i64,
    pub specialist_role: String,
    pub state: String,
    pub capability_execution_receipt_id: Option<Uuid>,
    pub oracle_receipt_id: Option<Uuid>,
    pub audit_evidence_ids: Vec<i64>,
    pub evidence_set_sha256: Option<String>,
    pub result_sha256: Option<String>,
}

pub async fn list_dynamic_invocation_authorities(
    pool: &PgPool,
    session_id: Uuid,
    actor_call_id: Option<Uuid>,
) -> Result<Vec<DynamicVerificationInvocationAuthorityRow>> {
    validate_uuid(session_id)?;
    Ok(
        sqlx::query_as::<_, DynamicVerificationInvocationAuthorityRow>(
            r#"SELECT invocation.invocation_id,actor.actor_call_id,actor.actor_ordinal,
                  actor.specialist_role,invocation.state,
                  invocation.capability_execution_receipt_id,invocation.oracle_receipt_id,
                  invocation.audit_evidence_ids,invocation.evidence_set_sha256,
                  invocation.result_sha256
             FROM investigation_asset_verification_invocations invocation
             JOIN investigation_dynamic_verification_actor_calls actor
               ON actor.actor_call_id=invocation.actor_call_id
              AND actor.session_id=invocation.dynamic_session_id
            WHERE invocation.dynamic_session_id=$1
              AND ($2::UUID IS NULL OR invocation.actor_call_id=$2)
            ORDER BY actor.actor_ordinal,invocation.invocation_ordinal,invocation.invocation_id"#,
        )
        .bind(session_id)
        .bind(actor_call_id)
        .fetch_all(pool)
        .await?,
    )
}

#[derive(Debug, Clone, FromRow)]
pub struct AssetVerificationInvocationGuardRow {
    pub invocation_id: Uuid,
    pub session_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub target_type_at_freeze: String,
    pub target_value_at_freeze: String,
    pub target_name: String,
    pub target_project_path: String,
    pub target_ports: Value,
    pub session_authorization_id: Uuid,
    pub session_authorization_sha256: String,
    pub authorization_expires_at: DateTime<Utc>,
    pub session_budget_envelope_id: Uuid,
    pub invocation_authorization_id: Uuid,
    pub invocation_authorization_sha256: String,
    pub invocation_authorization_expires_at: DateTime<Utc>,
    pub actor_call_id: Option<Uuid>,
    pub actor_ordinal: Option<i64>,
    pub actor_subtask_id: Option<Uuid>,
    pub actor_role: String,
    pub actor_work_item_id: Uuid,
    pub actor_worker_run_id: Uuid,
    pub actor_message_chain_id: Uuid,
    pub inventory_snapshot_id: Uuid,
    pub inventory_member_id: Option<Uuid>,
    pub selected_tool_name: Option<String>,
    pub selected_tool_config_sha256: Option<String>,
}

pub async fn load_invocation_guard(
    pool: &PgPool,
    invocation_id: Uuid,
    fence: &VerificationWorkerFenceInput,
    wrapper_name: &str,
    selected_tool_name: Option<&str>,
    selected_tool_config_sha256: Option<&str>,
    model_args_sha256: &str,
) -> Result<AssetVerificationInvocationGuardRow> {
    let row = sqlx::query_as::<_, AssetVerificationInvocationGuardRow>(
        r#"SELECT
      invocation.invocation_id,session.session_id,session.operation_id,session.project_scope_id,
      session.stage_execution_id,session.stage_run_unit_id,session.scope_snapshot_id,
      session.organization_id,session.asset_lane_id,session.target_live_id,
      lane.target_type_at_freeze,lane.target_value_at_freeze,target.name AS target_name,
      target.project_path AS target_project_path,target.ports AS target_ports,
      session.session_authorization_id,authz.authorization_sha256 AS session_authorization_sha256,
      session.authorization_expires_at,session.session_budget_envelope_id,
      invocation.invocation_authorization_id,invocation.invocation_authorization_sha256,
      invocation.invocation_authorization_expires_at,invocation.actor_call_id,
      actor.actor_ordinal,actor.subtask_id AS actor_subtask_id,actor.specialist_role AS actor_role,
      actor.work_item_id AS actor_work_item_id,actor.worker_run_id AS actor_worker_run_id,
      actor.message_chain_id AS actor_message_chain_id,invocation.inventory_snapshot_id,
      invocation.inventory_member_id,invocation.selected_tool_name,
      invocation.selected_tool_config_sha256
      FROM investigation_asset_verification_invocations invocation
      JOIN investigation_dynamic_verification_rounds session
        ON session.session_id=invocation.dynamic_session_id
      JOIN investigation_dynamic_verification_actor_calls actor
        ON actor.actor_call_id=invocation.actor_call_id AND actor.session_id=session.session_id
      JOIN investigation_asset_verification_authorizations authz
        ON authz.session_authorization_id=session.session_authorization_id
      JOIN investigation_asset_lanes lane ON lane.asset_lane_id=session.asset_lane_id
      JOIN targets target ON target.id=session.target_live_id
      JOIN stage_worker_runs worker ON worker.id=actor.worker_run_id
     WHERE invocation.invocation_id=$1 AND invocation.state='running' AND session.state='open'
       AND actor.state='running'
       AND session.authorization_expires_at>statement_timestamp()
       AND invocation.invocation_authorization_expires_at>statement_timestamp()
       AND invocation.wrapper_name=$2
       AND invocation.selected_tool_name IS NOT DISTINCT FROM $3
       AND invocation.selected_tool_config_sha256 IS NOT DISTINCT FROM $4
       AND invocation.model_args_sha256=$5 AND worker.id=$6 AND worker.lease_token=$7
       AND worker.attempt_epoch=$8 AND worker.checkpoint_version=$9
       AND worker.status IN('running','waiting_background')
       AND worker.lease_expires_at>statement_timestamp()
       AND target.scope::TEXT='in' AND target.organization_id=session.organization_id
       AND target.id=lane.target_id AND target.value=lane.target_value_at_freeze"#,
    )
    .bind(invocation_id)
    .bind(wrapper_name)
    .bind(selected_tool_name)
    .bind(selected_tool_config_sha256)
    .bind(model_args_sha256)
    .bind(fence.worker_run_id)
    .bind(fence.lease_token)
    .bind(fence.attempt_epoch)
    .bind(fence.checkpoint_version)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| fail(AUTHORITY_MISMATCH))
}

#[derive(Debug, Clone)]
pub struct ResolveDynamicHypothesisInput {
    pub stable_request_id: Uuid,
    pub resolution_authority_id: Uuid,
    pub session_id: Uuid,
    pub expected_session_head_version: i64,
    pub primary_worker_fence: VerificationWorkerFenceInput,
    pub primary_turn_id: Uuid,
    pub source_tool_call_record_id: Uuid,
    pub source_provider_call_id: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct PendingHypothesisDiscoveryRow {
    pub discovery_authority_id: Uuid,
    pub resolution_authority_id: Uuid,
    pub session_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub source_hypothesis_revision_id: Uuid,
    pub discovery_ordinal: i32,
    pub subject_kind: String,
    pub subject_identity_sha256: String,
    pub semantic_key_sha256: String,
    pub canonical_proposal: Value,
    pub structured_claim: String,
    pub structured_claim_sha256: String,
    pub rationale_redacted: Value,
    pub discovery_sha256: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct DynamicHypothesisResolutionFlatRow {
    pub resolution_authority_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub primary_turn_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub expected_session_head_version: i64,
    pub primary_work_item_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub primary_message_chain_id: Uuid,
    pub primary_lease_token: Uuid,
    pub primary_attempt_epoch: i64,
    pub primary_checkpoint_version: i64,
    pub disposition: String,
    pub primary_conclusion_sha256: String,
    pub conclusion_redacted: Value,
    pub citation_count: i64,
    pub citation_set_sha256: String,
    pub resolution_sha256: String,
    pub resolved_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct DynamicHypothesisResolutionRow {
    pub resolution_authority_id: Uuid,
    pub stable_request_id: Uuid,
    pub session_id: Uuid,
    pub primary_turn_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub primary_work_item_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub primary_message_chain_id: Uuid,
    pub disposition: String,
    pub primary_conclusion_sha256: String,
    pub conclusion_redacted: Value,
    pub citation_count: i64,
    pub citation_set_sha256: String,
    pub resolution_sha256: String,
    pub new_hypothesis_proposals: Vec<PendingHypothesisDiscoveryRow>,
    pub resolved_at: DateTime<Utc>,
    pub replayed: bool,
}

async fn load_dynamic_resolution_on(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    replayed: bool,
) -> Result<DynamicHypothesisResolutionRow> {
    let row = sqlx::query_as::<_, DynamicHypothesisResolutionFlatRow>(
        "SELECT * FROM investigation_dynamic_hypothesis_resolutions \
         WHERE resolution_authority_id=$1",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    let discoveries = sqlx::query_as::<_, PendingHypothesisDiscoveryRow>(
        r#"SELECT discovery_authority_id,dynamic_resolution_authority_id AS resolution_authority_id,
          session_id,asset_lane_id,target_live_id,source_hypothesis_revision_id,
          discovery_ordinal,subject_kind,subject_identity_sha256,semantic_key_sha256,
          canonical_proposal,structured_claim,structured_claim_sha256,rationale_redacted,
          discovery_sha256 FROM investigation_pending_hypothesis_discoveries
          WHERE dynamic_resolution_authority_id=$1 ORDER BY discovery_ordinal"#,
    )
    .bind(id)
    .fetch_all(&mut **tx)
    .await?;
    Ok(DynamicHypothesisResolutionRow {
        resolution_authority_id: row.resolution_authority_id,
        stable_request_id: row.stable_request_id,
        session_id: row.session_id,
        primary_turn_id: row.primary_turn_id,
        asset_lane_id: row.asset_lane_id,
        target_live_id: row.target_live_id,
        hypothesis_revision_id: row.hypothesis_revision_id,
        primary_work_item_id: row.primary_work_item_id,
        primary_worker_run_id: row.primary_worker_run_id,
        primary_message_chain_id: row.primary_message_chain_id,
        disposition: row.disposition,
        primary_conclusion_sha256: row.primary_conclusion_sha256,
        conclusion_redacted: row.conclusion_redacted,
        citation_count: row.citation_count,
        citation_set_sha256: row.citation_set_sha256,
        resolution_sha256: row.resolution_sha256,
        new_hypothesis_proposals: discoveries,
        resolved_at: row.resolved_at,
        replayed,
    })
}

#[derive(Debug, Clone, FromRow)]
struct DynamicTerminalRevisionAuthority {
    root_id: Uuid,
    revision_ordinal: i32,
    semantic_key: Value,
    semantic_key_hash: String,
    subject_kind: String,
    subject_identity_hash: String,
    target_live_id: Option<Uuid>,
    target_type_at_time: String,
    target_value_at_time: String,
    predicate_schema: String,
    predicate_version: i32,
    normalized_arguments: Value,
    trust_boundary: String,
    polarity: String,
    structured_claim: Value,
    assumptions: Value,
    missing_facts: Value,
    priority: i32,
    risk_impact: Value,
    head_version: i64,
}

/// Close one immutable canonical head from the Primary's dynamic resolution.
/// The source revision is never edited: a terminal successor, state event,
/// head CAS and exact authority-copy census are committed in this transaction.
async fn terminalize_dynamic_hypothesis_on(
    tx: &mut Transaction<'_, Postgres>,
    round: &DynamicVerificationRoundRow,
    resolution_id: Uuid,
    resolution_sha256: &str,
    disposition: &str,
) -> Result<()> {
    let predecessor = sqlx::query_as::<_, DynamicTerminalRevisionAuthority>(
        r#"SELECT revision.root_id,revision.revision_ordinal,revision.semantic_key,
                  revision.semantic_key_hash,revision.subject_kind,
                  revision.subject_identity_hash,revision.target_live_id,
                  revision.target_type_at_time,revision.target_value_at_time,
                  revision.predicate_schema,revision.predicate_version,
                  revision.normalized_arguments,revision.trust_boundary,
                  revision.polarity,revision.structured_claim,revision.assumptions,
                  revision.missing_facts,revision.priority,revision.risk_impact,
                  head.head_version
             FROM investigation_asset_lanes lane
             JOIN attack_hypothesis_revisions revision
               ON revision.revision_id=$1
              AND revision.operation_id=$2
              AND revision.organization_id=$3
              AND revision.asset_lane_id=lane.asset_lane_id
              AND revision.target_live_id=lane.target_id
             JOIN attack_hypothesis_heads head
               ON head.root_id=revision.root_id
              AND head.operation_id=revision.operation_id
              AND head.organization_id=revision.organization_id
              AND head.head_revision_id=revision.revision_id
              AND head.head_lifecycle_state='current'
            WHERE lane.asset_lane_id=$4 AND lane.operation_id=$2
              AND lane.organization_id=$3 AND lane.target_id=$5
            FOR UPDATE OF lane,head"#,
    )
    .bind(round.hypothesis_revision_id)
    .bind(round.operation_id)
    .bind(round.organization_id)
    .bind(round.asset_lane_id)
    .bind(round.target_live_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| fail(AUTHORITY_MISMATCH))?;
    let terminal_transition_id = Uuid::new_v5(
        &resolution_id,
        b"investigation-dynamic-terminal-transition.v2",
    );
    let successor_revision_id = Uuid::new_v5(
        &terminal_transition_id,
        b"investigation-dynamic-terminal-successor.v2",
    );
    let state_event_id = Uuid::new_v5(
        &terminal_transition_id,
        b"investigation-dynamic-terminal-state-event.v2",
    );
    let revision_ingredients_hash = sha256_on(
        tx,
        &json!({
            "predecessor_revision_id":round.hypothesis_revision_id,
            "revision_ordinal":predecessor.revision_ordinal+1,
            "semantic_key_hash":predecessor.semantic_key_hash,
            "epistemic_state":disposition,
            "lifecycle_state":"closed",
            "origin_decision_hash":resolution_sha256,
        }),
    )
    .await?;
    let successor_revision_hash = sha256_on(
        tx,
        &json!({
            "revision_id":successor_revision_id,
            "revision_ingredients_hash":revision_ingredients_hash,
            "semantic_key_hash":predecessor.semantic_key_hash,
            "structured_claim":predecessor.structured_claim,
            "decision":disposition,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,predecessor_revision_id,
               revision_ordinal,semantic_key,semantic_key_hash,subject_kind,
               subject_identity_hash,target_live_id,target_type_at_time,target_value_at_time,
               predicate_schema,predicate_version,normalized_arguments,trust_boundary,
               polarity,epistemic_state,lifecycle_state,planning_readiness,structured_claim,
               assumptions,missing_facts,priority,risk_impact,origin_decision_hash,
               revision_ingredients_hash,revision_hash,asset_lane_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,
                  'closed','deferred',$20,$21,$22,$23,$24,$25,$26,$27,$28)"#,
    )
    .bind(successor_revision_id)
    .bind(predecessor.root_id)
    .bind(round.operation_id)
    .bind(round.organization_id)
    .bind(round.hypothesis_revision_id)
    .bind(predecessor.revision_ordinal + 1)
    .bind(&predecessor.semantic_key)
    .bind(&predecessor.semantic_key_hash)
    .bind(&predecessor.subject_kind)
    .bind(&predecessor.subject_identity_hash)
    .bind(predecessor.target_live_id)
    .bind(&predecessor.target_type_at_time)
    .bind(&predecessor.target_value_at_time)
    .bind(&predecessor.predicate_schema)
    .bind(predecessor.predicate_version)
    .bind(&predecessor.normalized_arguments)
    .bind(&predecessor.trust_boundary)
    .bind(&predecessor.polarity)
    .bind(disposition)
    .bind(&predecessor.structured_claim)
    .bind(&predecessor.assumptions)
    .bind(&predecessor.missing_facts)
    .bind(predecessor.priority)
    .bind(&predecessor.risk_impact)
    .bind(resolution_sha256)
    .bind(&revision_ingredients_hash)
    .bind(&successor_revision_hash)
    .bind(round.asset_lane_id)
    .execute(&mut **tx)
    .await?;
    super::hypothesis_revision_adjudications::clone_terminal_revision_authorities_on(
        tx,
        round.hypothesis_revision_id,
        successor_revision_id,
        &successor_revision_hash,
        &revision_ingredients_hash,
    )
    .await?;
    let event_kind = if disposition == "invalid" {
        "invalidated"
    } else {
        disposition
    };
    let event_hash = sha256_on(
        tx,
        &json!({
            "event_id":state_event_id,
            "predecessor_revision_id":round.hypothesis_revision_id,
            "successor_revision_id":successor_revision_id,
            "event_kind":event_kind,
            "resolution_authority_id":resolution_id,
            "resolution_sha256":resolution_sha256,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_state_events(
               event_id,operation_id,organization_id,root_id,predecessor_revision_id,
               successor_revision_id,event_kind,origin_authority,successor_epistemic_state,
               authority_receipt_kind,authority_receipt_id,authority_receipt_hash,
               event_hash,server_decision_id,server_decision_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,'dynamic_verification_resolution',$8,
                  'dynamic_resolution',$9,$10,$11,$9,$10)"#,
    )
    .bind(state_event_id)
    .bind(round.operation_id)
    .bind(round.organization_id)
    .bind(predecessor.root_id)
    .bind(round.hypothesis_revision_id)
    .bind(successor_revision_id)
    .bind(event_kind)
    .bind(disposition)
    .bind(resolution_id)
    .bind(resolution_sha256)
    .bind(event_hash)
    .execute(&mut **tx)
    .await?;
    let advanced = sqlx::query(
        r#"UPDATE attack_hypothesis_heads
              SET head_revision_id=$1,head_revision_hash=$2,
                  head_semantic_key_hash=$3,head_epistemic_state=$4,
                  head_lifecycle_state='closed',head_version=head_version+1,
                  updated_at=statement_timestamp()
            WHERE root_id=$5 AND operation_id=$6 AND organization_id=$7
              AND head_revision_id=$8 AND head_version=$9
              AND head_lifecycle_state='current'"#,
    )
    .bind(successor_revision_id)
    .bind(&successor_revision_hash)
    .bind(&predecessor.semantic_key_hash)
    .bind(disposition)
    .bind(predecessor.root_id)
    .bind(round.operation_id)
    .bind(round.organization_id)
    .bind(round.hypothesis_revision_id)
    .bind(predecessor.head_version)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if advanced != 1 {
        return Err(fail(CAS_CONFLICT));
    }
    let transition_sha256 = sha256_on(
        tx,
        &json!({
            "domain":"investigation_dynamic_hypothesis_terminal_transition.v2",
            "resolution_authority_id":resolution_id,
            "asset_lane_id":round.asset_lane_id,
            "source_revision_id":round.hypothesis_revision_id,
            "terminal_revision_id":successor_revision_id,
            "state_event_id":state_event_id,
            "disposition":disposition,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_dynamic_hypothesis_terminal_transitions(
               terminal_transition_id,stable_request_id,resolution_authority_id,asset_lane_id,
               source_revision_id,terminal_revision_id,state_event_id,disposition,
               transition_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(terminal_transition_id)
    .bind(Uuid::new_v5(
        &resolution_id,
        b"dynamic-terminal-transition-request.v2",
    ))
    .bind(resolution_id)
    .bind(round.asset_lane_id)
    .bind(round.hypothesis_revision_id)
    .bind(successor_revision_id)
    .bind(state_event_id)
    .bind(disposition)
    .bind(transition_sha256)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"WITH authority_sets(source_kind,source_count,source_set_sha256) AS (
             SELECT 'revision_source',COUNT(*),tool_truth_sha256(COALESCE(
                      jsonb_agg(member_hash ORDER BY ordinal)::TEXT,'[]'))
               FROM attack_hypothesis_revision_sources WHERE revision_id=$2
             UNION ALL
             SELECT 'verification_objective',COUNT(*),tool_truth_sha256(COALESCE(
                      jsonb_agg(objective_hash ORDER BY objective_ordinal)::TEXT,'[]'))
               FROM attack_hypothesis_verification_objectives WHERE revision_id=$2
             UNION ALL
             SELECT 'claim_component',COUNT(*),tool_truth_sha256(COALESCE(
                      jsonb_agg(member_hash ORDER BY component_ordinal)::TEXT,'[]'))
               FROM attack_hypothesis_claim_components WHERE revision_id=$2
             UNION ALL
             SELECT 'verification_contract',COUNT(*),tool_truth_sha256(COALESCE(
                      jsonb_agg(contract_hash ORDER BY objective_id)::TEXT,'[]'))
               FROM attack_hypothesis_verification_contracts WHERE revision_id=$2
             UNION ALL
             SELECT 'verification_plan',COUNT(*),tool_truth_sha256(COALESCE(
                      jsonb_agg(plan_hash ORDER BY plan_id)::TEXT,'[]'))
               FROM attack_hypothesis_verification_plans WHERE revision_id=$2)
           INSERT INTO investigation_dynamic_hypothesis_terminal_transition_sources(
               source_id,terminal_transition_id,source_revision_id,terminal_revision_id,
               source_kind,source_count,source_set_sha256)
           SELECT uuid_generate_v5($1,source_kind),$1,$3,$2,source_kind,
                  source_count,source_set_sha256 FROM authority_sets"#,
    )
    .bind(terminal_transition_id)
    .bind(successor_revision_id)
    .bind(round.hypothesis_revision_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Resolution is a Primary business conclusion, not an all-specialists
/// barrier.  Accepted children that the Primary no longer needs are closed by
/// an immutable resolution-backed receipt; an actor with live external I/O is
/// rejected before this helper is entered.
async fn archive_unfinished_dynamic_actors_on(
    tx: &mut Transaction<'_, Postgres>,
    session_id: Uuid,
    resolution_id: Uuid,
) -> Result<()> {
    let actors = sqlx::query_as::<_, (Uuid, String, Uuid, Uuid)>(
        r#"SELECT actor.actor_call_id,actor.state,actor.work_item_id,actor.worker_run_id
             FROM investigation_dynamic_verification_actor_calls actor
            WHERE actor.session_id=$1 AND actor.state IN('queued','running','parked')
            ORDER BY actor.actor_ordinal
            FOR UPDATE"#,
    )
    .bind(session_id)
    .fetch_all(&mut **tx)
    .await?;
    for (actor_call_id, prior_state, work_item_id, worker_run_id) in actors {
        let archive_id = Uuid::new_v5(
            &resolution_id,
            format!("dynamic-actor-archive:{actor_call_id}").as_bytes(),
        );
        let archive_sha256 = sha256_on(
            tx,
            &json!({
                "domain":"investigation_dynamic_verification_actor_archive.v1",
                "resolution_authority_id":resolution_id,
                "session_id":session_id,
                "actor_call_id":actor_call_id,
                "prior_state":prior_state,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO investigation_dynamic_verification_actor_archives(
                   archive_id,resolution_authority_id,session_id,actor_call_id,
                   prior_state,archive_sha256)
               VALUES($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(archive_id)
        .bind(resolution_id)
        .bind(session_id)
        .bind(actor_call_id)
        .bind(&prior_state)
        .bind(archive_sha256)
        .execute(&mut **tx)
        .await?;
        let worker_updated = sqlx::query(
            r#"UPDATE stage_worker_runs
                  SET status='superseded',terminal_at=NOW(),active_tool_call_id=NULL,
                      active_tool_started_at=NULL,lease_token=NULL,lease_owner=NULL,
                      lease_acquired_at=NULL,lease_expires_at=NULL,heartbeat_at=NULL,
                      updated_at=NOW()
                WHERE id=$1 AND status IN('queued','running','waiting_background',
                                           'recovery_required','gate_blocked')
                  AND active_tool_call_id IS NULL"#,
        )
        .bind(worker_run_id)
        .execute(&mut **tx)
        .await?;
        if worker_updated.rows_affected() == 0 {
            let already_terminal: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM stage_worker_runs WHERE id=$1 \
                 AND status IN('failed','exhausted','superseded') \
                 AND terminal_at IS NOT NULL AND active_tool_call_id IS NULL)",
            )
            .bind(worker_run_id)
            .fetch_one(&mut **tx)
            .await?;
            if !already_terminal {
                return Err(fail(CAS_CONFLICT));
            }
        }
        let item_updated = sqlx::query(
            r#"UPDATE stage_work_items
                  SET status='superseded',terminal_at=NOW(),row_version=row_version+1,
                      updated_at=NOW()
                WHERE id=$1 AND status IN('queued','claimed','running','waiting_dependency',
                                           'retry_pending','recovery_required','gate_blocked')"#,
        )
        .bind(work_item_id)
        .execute(&mut **tx)
        .await?;
        if item_updated.rows_affected() == 0 {
            let already_terminal: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM stage_work_items WHERE id=$1 \
                 AND status IN('exhausted','superseded') AND terminal_at IS NOT NULL)",
            )
            .bind(work_item_id)
            .fetch_one(&mut **tx)
            .await?;
            if !already_terminal {
                return Err(fail(CAS_CONFLICT));
            }
        }
        let actor_updated = sqlx::query(
            r#"UPDATE investigation_dynamic_verification_actor_calls
                  SET state='archived',completed_at=NOW()
                WHERE actor_call_id=$1 AND session_id=$2 AND state=$3"#,
        )
        .bind(actor_call_id)
        .bind(session_id)
        .bind(&prior_state)
        .execute(&mut **tx)
        .await?;
        if actor_updated.rows_affected() != 1 {
            return Err(fail(CAS_CONFLICT));
        }
    }
    Ok(())
}

pub async fn resolve_dynamic_hypothesis(
    pool: &PgPool,
    input: &ResolveDynamicHypothesisInput,
) -> Result<DynamicHypothesisResolutionRow> {
    if input.primary_turn_id.is_nil()
        || input.source_tool_call_record_id.is_nil()
        || input.source_provider_call_id.trim().is_empty()
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT resolution_authority_id FROM investigation_dynamic_hypothesis_resolutions \
         WHERE stable_request_id=$1",
    )
    .bind(input.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if id != input.resolution_authority_id {
            return Err(fail(REPLAY_DRIFT));
        }
        let row = load_dynamic_resolution_on(&mut tx, id, true).await?;
        let turn = load_dynamic_primary_turn_on(&mut tx, row.primary_turn_id, true).await?;
        if row.session_id != input.session_id
            || row.primary_turn_id != input.primary_turn_id
            || turn.decision_kind != "resolve"
            || turn.expected_session_head_version != input.expected_session_head_version
            || turn.source_primary_worker_run_id != input.primary_worker_fence.worker_run_id
            || turn.consumer_primary_lease_token != input.primary_worker_fence.lease_token
            || turn.consumer_primary_attempt_epoch != input.primary_worker_fence.attempt_epoch
            || turn.source_primary_checkpoint_version
                != input.primary_worker_fence.checkpoint_version
            || turn.source_tool_call_record_id != input.source_tool_call_record_id
            || turn.source_provider_call_id != input.source_provider_call_id
        {
            return Err(fail(REPLAY_DRIFT));
        }
        tx.commit().await?;
        return Ok(row);
    }
    let round = load_dynamic_round_on(&mut tx, input.session_id, false).await?;
    if round.state != "open"
        || round.head_version != input.expected_session_head_version
        || round.primary.worker_run_id != input.primary_worker_fence.worker_run_id
        || round.consumed_primary_turns >= round.maximum_primary_turns
    {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let (primary_checkpoint, source_attempt_epoch, source_lease_token, raw_turn): (
        Value,
        i64,
        Uuid,
        Value,
    ) = sqlx::query_as(
        r#"SELECT worker.checkpoint,call.attempt_epoch,call.lease_token,call.args->'result'
              FROM stage_worker_runs worker
              JOIN tool_calls call ON call.id=$7 AND call.call_id=$8
               AND call.worker_run_id=worker.id AND call.name='submit_result'
               AND call.status='finished' AND call.result IS NOT NULL
               AND call.result::JSONB->>'status'='result submitted' AND call.args ? 'result'
               AND call.operation_id=$9 AND call.stage_execution_id=$10
               AND call.stage_run_unit_id=$11 AND call.organization_id=$12
            WHERE worker.id=$1 AND worker.work_item_id=$2
              AND worker.message_chain_id=$3 AND worker.status='running'
              AND worker.lease_token=$4 AND worker.attempt_epoch=$5
              AND worker.checkpoint_version=$6
              AND worker.lease_expires_at>statement_timestamp()
              AND worker.active_tool_call_id IS NULL FOR SHARE"#,
    )
    .bind(input.primary_worker_fence.worker_run_id)
    .bind(round.primary.work_item_id)
    .bind(round.primary.message_chain_id)
    .bind(input.primary_worker_fence.lease_token)
    .bind(input.primary_worker_fence.attempt_epoch)
    .bind(input.primary_worker_fence.checkpoint_version)
    .bind(input.source_tool_call_record_id)
    .bind(&input.source_provider_call_id)
    .bind(round.operation_id)
    .bind(round.stage_execution_id)
    .bind(round.stage_run_unit_id)
    .bind(round.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    let canonical_turn_sha256 = sha256_on(&mut tx, &raw_turn).await?;
    let checkpoint_sha256 = sha256_on(&mut tx, &primary_checkpoint).await?;
    let StoredDynamicPrimaryTurn::Resolve {
        schema_version,
        session_id,
        hypothesis_revision_id,
        subtasks,
        disposition,
        conclusion,
        cited_evidence_ids,
        new_hypothesis_proposals,
    } = serde_json::from_value(raw_turn).map_err(|_| fail(CONTRACT_INVALID))?
    else {
        return Err(fail(CONTRACT_INVALID));
    };
    if schema_version != 1
        || session_id != round.session_id
        || hypothesis_revision_id != round.hypothesis_revision_id
        || !subtasks.is_empty()
        || !matches!(disposition.as_str(), "verified" | "refuted" | "invalid")
        || conclusion.trim().is_empty()
        || cited_evidence_ids.iter().any(|id| *id <= 0)
        || cited_evidence_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != cited_evidence_ids.len()
        || new_hypothesis_proposals.len() > 64
        || new_hypothesis_proposals
            .iter()
            .any(|proposal| !stored_dynamic_proposal_is_valid(proposal))
    {
        return Err(fail(CONTRACT_INVALID));
    }
    let live_io: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
             SELECT 1 FROM investigation_asset_verification_invocations invocation
              WHERE invocation.dynamic_session_id=$1 AND invocation.state='running'
             UNION ALL
             SELECT 1 FROM investigation_dynamic_verification_actor_calls actor
              JOIN stage_worker_runs worker ON worker.id=actor.worker_run_id
             WHERE actor.session_id=$1 AND worker.active_tool_call_id IS NOT NULL)"#,
    )
    .bind(round.session_id)
    .fetch_one(&mut *tx)
    .await?;
    if live_io {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let citation_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM unnest($2::BIGINT[]) evidence_id
            WHERE EXISTS(SELECT 1 FROM investigation_asset_verification_invocations invocation
              WHERE invocation.dynamic_session_id=$1 AND invocation.state='succeeded'
                AND evidence_id=ANY(invocation.audit_evidence_ids))"#,
    )
    .bind(round.session_id)
    .bind(&cited_evidence_ids)
    .fetch_one(&mut *tx)
    .await?;
    if citation_count
        != i64::try_from(cited_evidence_ids.len()).map_err(|_| fail(CONTRACT_INVALID))?
    {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let mut citation_hashes = Vec::with_capacity(cited_evidence_ids.len());
    for evidence_id in &cited_evidence_ids {
        citation_hashes.push(
            sha256_on(
                &mut tx,
                &json!({"citation_kind":"audit_evidence","audit_evidence_id":evidence_id,
                        "authority_id":Value::Null}),
            )
            .await?,
        );
    }
    let citation_set_sha256: String =
        sqlx::query_scalar("SELECT tool_truth_sha256($1::JSONB::TEXT)")
            .bind(json!(citation_hashes))
            .fetch_one(&mut *tx)
            .await?;
    let primary_conclusion_sha256 = sha256_on(&mut tx, &json!(conclusion)).await?;
    let resolution_sha256 = sha256_on(
        &mut tx,
        &json!({
            "domain":"investigation_dynamic_hypothesis_resolution.v2",
            "session_id":round.session_id,
            "hypothesis_revision_id":round.hypothesis_revision_id,
            "primary_turn_id":input.primary_turn_id,
            "canonical_turn_sha256":canonical_turn_sha256,
            "disposition":disposition,
            "primary_conclusion_sha256":primary_conclusion_sha256,
            "citation_set_sha256":citation_set_sha256,
        }),
    )
    .await?;
    // The immutable accepted Primary turn is the source authority for the
    // resolution row.  Its FK is intentionally non-deferrable, so establish
    // it before inserting the resolution that consumes it.
    let empty_actor_set_sha256 = sha256_on(&mut tx, &json!([])).await?;
    sqlx::query(
        r#"INSERT INTO investigation_dynamic_verification_primary_turns(
             primary_turn_id,stable_request_id,session_id,turn_ordinal,decision_kind,
             expected_session_head_version,source_primary_work_item_id,
             source_primary_worker_run_id,source_primary_lease_token,source_primary_attempt_epoch,
             consumer_primary_lease_token,consumer_primary_attempt_epoch,
             consumer_primary_checkpoint_version,consumer_primary_checkpoint_sha256,
             source_tool_call_record_id,source_provider_call_id,canonical_turn_sha256,
             actor_call_count,actor_call_set_sha256)
           VALUES($1,$2,$3,$4,'resolve',$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,0,$17)"#,
    )
    .bind(input.primary_turn_id)
    .bind(Uuid::new_v5(
        &input.stable_request_id,
        b"dynamic-resolve-primary-turn.v2",
    ))
    .bind(round.session_id)
    .bind(round.consumed_primary_turns + 1)
    .bind(input.expected_session_head_version)
    .bind(round.primary.work_item_id)
    .bind(round.primary.worker_run_id)
    .bind(source_lease_token)
    .bind(source_attempt_epoch)
    .bind(input.primary_worker_fence.lease_token)
    .bind(input.primary_worker_fence.attempt_epoch)
    .bind(input.primary_worker_fence.checkpoint_version)
    .bind(checkpoint_sha256)
    .bind(input.source_tool_call_record_id)
    .bind(&input.source_provider_call_id)
    .bind(&canonical_turn_sha256)
    .bind(empty_actor_set_sha256)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_dynamic_hypothesis_resolutions(
          resolution_authority_id,stable_request_id,session_id,primary_turn_id,asset_lane_id,target_live_id,
          hypothesis_revision_id,expected_session_head_version,primary_work_item_id,
          primary_worker_run_id,primary_message_chain_id,primary_lease_token,
          primary_attempt_epoch,primary_checkpoint_version,disposition,
          primary_conclusion_sha256,conclusion_redacted,citation_count,citation_set_sha256,
          resolution_sha256)
          VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)"#,
    )
    .bind(input.resolution_authority_id)
    .bind(input.stable_request_id)
    .bind(round.session_id)
    .bind(input.primary_turn_id)
    .bind(round.asset_lane_id)
    .bind(round.target_live_id)
    .bind(round.hypothesis_revision_id)
    .bind(input.expected_session_head_version)
    .bind(round.primary.work_item_id)
    .bind(input.primary_worker_fence.worker_run_id)
    .bind(round.primary.message_chain_id)
    .bind(input.primary_worker_fence.lease_token)
    .bind(input.primary_worker_fence.attempt_epoch)
    .bind(input.primary_worker_fence.checkpoint_version)
    .bind(&disposition)
    .bind(sha256_on(&mut tx, &json!(conclusion)).await?)
    .bind(json!({"conclusion":conclusion}))
    .bind(citation_count)
    .bind(&citation_set_sha256)
    .bind(&resolution_sha256)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (evidence_id, citation_sha256)) in
        cited_evidence_ids.iter().zip(citation_hashes).enumerate()
    {
        sqlx::query(
            r#"INSERT INTO investigation_dynamic_hypothesis_resolution_citations(
              citation_id,resolution_authority_id,citation_ordinal,citation_kind,
              audit_evidence_id,authority_id,citation_sha256)
              VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(Uuid::new_v5(
            &input.resolution_authority_id,
            format!("citation:{ordinal}").as_bytes(),
        ))
        .bind(input.resolution_authority_id)
        .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind("audit_evidence")
        .bind(*evidence_id)
        .bind(Option::<Uuid>::None)
        .bind(citation_sha256)
        .execute(&mut *tx)
        .await?;
    }
    for (ordinal, proposal) in new_hypothesis_proposals.iter().enumerate() {
        if proposal.predicate_schema.trim().is_empty()
            || proposal.predicate_version == 0
            || proposal.trust_boundary.trim().is_empty()
            || proposal.structured_claim.trim().is_empty()
            || proposal.impact.trim().is_empty()
            || proposal.rationale.trim().is_empty()
        {
            return Err(fail(CONTRACT_INVALID));
        }
        let (organization_id, target_value_at_freeze): (Uuid, String) = sqlx::query_as(
            "SELECT organization_id,target_value_at_freeze FROM investigation_asset_lanes \
             WHERE asset_lane_id=$1 AND target_id=$2 FOR SHARE",
        )
        .bind(round.asset_lane_id)
        .bind(round.target_live_id)
        .fetch_one(&mut *tx)
        .await?;
        let subject_identity_sha256 = sha256_on(
            &mut tx,
            &json!({"domain":"investigation_subject_identity.v1","subject_kind":"asset",
                "subject_id":round.target_live_id,"display_value":target_value_at_freeze}),
        )
        .await?;
        let mut arguments = serde_json::Map::new();
        for (key, value) in &proposal.predicate_arguments {
            if key.trim().is_empty()
                || arguments
                    .insert(key.clone(), Value::String(value.clone()))
                    .is_some()
            {
                return Err(fail(CONTRACT_INVALID));
            }
        }
        let semantic_key = HypothesisSemanticKeyV1::new(
            organization_id,
            AtTimeSubjectIdentity::new("asset".to_owned(), subject_identity_sha256.clone())
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            PredicateIdentity::new(
                proposal.predicate_schema.clone(),
                proposal.predicate_version,
                Value::Object(arguments.clone()),
            )
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            proposal.trust_boundary.clone(),
            ClaimPolarity::try_from(proposal.polarity.as_str())
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let semantic_key_sha256 = semantic_key
            .hash()
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let semantic_key_canonical_json = serde_json::to_string(&semantic_key)?;
        let structured_claim_sha256 = sha256_on(&mut tx, &json!(proposal.structured_claim)).await?;
        let canonical_proposal = json!({
            "proposal_id":Uuid::new_v5(&input.resolution_authority_id,
                format!("dynamic-discovery:{ordinal}").as_bytes()),
            "subject_kind":"asset",
            "subject_identity_hash":subject_identity_sha256,
            "predicate_schema":proposal.predicate_schema,
            "predicate_version":proposal.predicate_version,
            "predicate_arguments":Value::Object(arguments),
            "trust_boundary":proposal.trust_boundary,
            "polarity":proposal.polarity,
            "structured_claim":proposal.structured_claim,
            "preconditions":proposal.preconditions,
            "impact":proposal.impact,
            "proof_refs":[],
            "knowledge_signals":[],
            "readiness":"ready_for_strategy",
        });
        let discovery_sha256 = sha256_on(
            &mut tx,
            &json!({"domain":"investigation_pending_hypothesis_discovery.v1",
              "resolution_authority_id":input.resolution_authority_id,
              "asset_lane_id":round.asset_lane_id,"target_live_id":round.target_live_id,
              "source_hypothesis_revision_id":round.hypothesis_revision_id,
              "discovery_ordinal":ordinal,"subject_kind":"asset",
              "subject_identity_sha256":subject_identity_sha256,
              "semantic_key_sha256":semantic_key_sha256,
              "structured_claim_sha256":structured_claim_sha256,
              "canonical_proposal":canonical_proposal,
              "preconditions":proposal.preconditions,"impact":proposal.impact,
              "rationale":proposal.rationale}),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO investigation_pending_hypothesis_discoveries(
              discovery_authority_id,dynamic_resolution_authority_id,session_id,asset_lane_id,
              target_live_id,source_hypothesis_revision_id,discovery_ordinal,subject_kind,
              subject_identity_sha256,semantic_key_sha256,semantic_key_canonical_json,
              canonical_proposal,structured_claim,structured_claim_sha256,rationale_redacted,
              discovery_sha256)
              VALUES($1,$2,$3,$4,$5,$6,$7,'asset',$8,$9,$10,$11,$12,$13,$14,$15)"#,
        )
        .bind(Uuid::new_v5(
            &input.resolution_authority_id,
            format!("dynamic-discovery:{ordinal}").as_bytes(),
        ))
        .bind(input.resolution_authority_id)
        .bind(round.session_id)
        .bind(round.asset_lane_id)
        .bind(round.target_live_id)
        .bind(round.hypothesis_revision_id)
        .bind(i32::try_from(ordinal).map_err(|_| fail(CONTRACT_INVALID))?)
        .bind(&subject_identity_sha256)
        .bind(&semantic_key_sha256)
        .bind(&semantic_key_canonical_json)
        .bind(&canonical_proposal)
        .bind(&proposal.structured_claim)
        .bind(&structured_claim_sha256)
        .bind(json!({"rationale":proposal.rationale}))
        .bind(&discovery_sha256)
        .execute(&mut *tx)
        .await?;
    }
    // Make the round-to-resolution edge authoritative before inserting the
    // terminal transition: the transition trigger deliberately refuses an
    // otherwise orphaned resolution.  Every step remains in this transaction,
    // so any later terminalization failure rolls this CAS back as well.
    let updated = sqlx::query(
        "UPDATE investigation_dynamic_verification_rounds SET state='resolved',\
         head_version=head_version+1,consumed_primary_turns=consumed_primary_turns+1,\
         resolution_authority_id=$1,resolved_at=statement_timestamp() \
         WHERE session_id=$2 AND state='open' AND head_version=$3",
    )
    .bind(input.resolution_authority_id)
    .bind(round.session_id)
    .bind(input.expected_session_head_version)
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(fail(CAS_CONFLICT));
    }
    archive_unfinished_dynamic_actors_on(&mut tx, round.session_id, input.resolution_authority_id)
        .await?;
    terminalize_dynamic_hypothesis_on(
        &mut tx,
        &round,
        input.resolution_authority_id,
        &resolution_sha256,
        &disposition,
    )
    .await?;
    let row = load_dynamic_resolution_on(&mut tx, input.resolution_authority_id, false).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Debug, Clone)]
pub struct PendingDynamicPrimaryTerminalizationRow {
    pub round: DynamicVerificationRoundRow,
    pub resolution: DynamicHypothesisResolutionRow,
    pub primary_worker_fence: Option<VerificationWorkerFenceInput>,
    pub expected_work_item_row_version: i64,
    pub expected_plan_row_version: i64,
}

#[derive(Debug, FromRow)]
struct PendingDynamicPrimaryTerminalizationAuthorityRow {
    session_id: Uuid,
    resolution_authority_id: Uuid,
    lease_token: Option<Uuid>,
    attempt_epoch: i64,
    checkpoint_version: i64,
    work_item_row_version: i64,
    plan_row_version: i64,
}

pub async fn load_pending_dynamic_primary_terminalization(
    pool: &PgPool,
    operation_id: Uuid,
    asset_lane_id: Uuid,
) -> Result<Option<PendingDynamicPrimaryTerminalizationRow>> {
    validate_uuid(operation_id)?;
    validate_uuid(asset_lane_id)?;
    let mut tx = pool.begin().await?;
    let pending = sqlx::query_as::<_, PendingDynamicPrimaryTerminalizationAuthorityRow>(
        r#"SELECT dynamic_round.session_id,resolution.resolution_authority_id,
                  CASE WHEN worker.status='running'
                              AND worker.lease_expires_at>statement_timestamp()
                       THEN worker.lease_token ELSE NULL END AS lease_token,
                  worker.attempt_epoch,worker.checkpoint_version,
                  item.row_version AS work_item_row_version,
                  plan.row_version AS plan_row_version
             FROM investigation_dynamic_verification_rounds dynamic_round
             JOIN investigation_dynamic_hypothesis_resolutions resolution
               ON resolution.resolution_authority_id=dynamic_round.resolution_authority_id
              AND resolution.session_id=dynamic_round.session_id
             JOIN stage_work_items item ON item.id=dynamic_round.primary_work_item_id
             JOIN stage_worker_runs worker ON worker.id=dynamic_round.primary_worker_run_id
             JOIN stage_team_plans plan ON plan.id=dynamic_round.stage_team_plan_id
            WHERE dynamic_round.operation_id=$1 AND dynamic_round.asset_lane_id=$2
              AND dynamic_round.state='resolved'
              AND item.status IN('running','waiting_dependency')
              AND worker.status IN('running','waiting_background')
              AND plan.requests_closed_at IS NULL
              AND NOT EXISTS(SELECT 1 FROM stage_worker_outputs output
                              WHERE output.work_item_id=item.id
                                AND output.worker_run_id=worker.id)
            ORDER BY dynamic_round.resolved_at,dynamic_round.session_id LIMIT 1
            FOR SHARE OF dynamic_round,resolution,item,worker,plan"#,
    )
    .bind(operation_id)
    .bind(asset_lane_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(pending) = pending else {
        tx.commit().await?;
        return Ok(None);
    };
    let round = load_dynamic_round_on(&mut tx, pending.session_id, true).await?;
    let resolution =
        load_dynamic_resolution_on(&mut tx, pending.resolution_authority_id, true).await?;
    let worker_run_id = round.primary.worker_run_id;
    tx.commit().await?;
    Ok(Some(PendingDynamicPrimaryTerminalizationRow {
        round,
        resolution,
        primary_worker_fence: pending
            .lease_token
            .map(|lease_token| VerificationWorkerFenceInput {
                worker_run_id,
                lease_token,
                attempt_epoch: pending.attempt_epoch,
                checkpoint_version: pending.checkpoint_version,
            }),
        expected_work_item_row_version: pending.work_item_row_version,
        expected_plan_row_version: pending.plan_row_version,
    }))
}

#[derive(Debug, Clone)]
pub struct CompleteDynamicVerificationPrimaryInput {
    pub session_id: Uuid,
    pub resolution_authority_id: Uuid,
    pub primary_worker_fence: VerificationWorkerFenceInput,
    pub expected_work_item_row_version: i64,
    pub expected_plan_row_version: i64,
    pub terminal_checkpoint: Value,
}

pub async fn complete_dynamic_primary(
    pool: &PgPool,
    input: &CompleteDynamicVerificationPrimaryInput,
) -> Result<(
    DynamicHypothesisResolutionRow,
    stage_teams::CompletedStageWorkerRow,
)> {
    let mut tx = pool.begin().await?;
    let requested_terminal_checkpoint_sha256 =
        sha256_on(&mut tx, &input.terminal_checkpoint).await?;
    if let Some((
        resolution_id,
        output_id,
        worker_id,
        lease_token,
        attempt_epoch,
        checkpoint_version,
        item_version,
        plan_version,
        checkpoint_sha256,
    )) = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Uuid, i64, i64, i64, i64, String)>(
        "SELECT resolution_authority_id,stage_worker_output_id,primary_worker_run_id,\
                primary_lease_token,primary_attempt_epoch,expected_primary_checkpoint_version,\
                expected_work_item_row_version,expected_plan_row_version,\
                terminal_checkpoint_sha256 FROM \
         investigation_dynamic_verification_primary_completions WHERE session_id=$1",
    )
    .bind(input.session_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if resolution_id != input.resolution_authority_id
            || worker_id != input.primary_worker_fence.worker_run_id
            || lease_token != input.primary_worker_fence.lease_token
            || attempt_epoch != input.primary_worker_fence.attempt_epoch
            || checkpoint_version != input.primary_worker_fence.checkpoint_version
            || item_version != input.expected_work_item_row_version
            || plan_version != input.expected_plan_row_version
            || checkpoint_sha256 != requested_terminal_checkpoint_sha256
        {
            return Err(fail(REPLAY_DRIFT));
        }
        let resolution = load_dynamic_resolution_on(&mut tx, resolution_id, true).await?;
        let round = load_dynamic_round_on(&mut tx, input.session_id, true).await?;
        let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
            "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1"
        ))
        .bind(round.stage_team_plan_id)
        .fetch_one(&mut *tx)
        .await?;
        let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
            "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1"
        ))
        .bind(round.primary.work_item_id)
        .fetch_one(&mut *tx)
        .await?;
        let worker = sqlx::query_as("SELECT * FROM stage_worker_runs WHERE id=$1")
            .bind(round.primary.worker_run_id)
            .fetch_one(&mut *tx)
            .await?;
        let output = sqlx::query_as::<_, stage_teams::StageWorkerOutputRow>(&format!(
            "SELECT {OUTPUT_COLUMNS} FROM stage_worker_outputs WHERE id=$1"
        ))
        .bind(output_id)
        .fetch_one(&mut *tx)
        .await?;
        let unit = sqlx::query_as("SELECT * FROM stage_run_units WHERE id=$1")
            .bind(round.stage_run_unit_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok((
            resolution,
            stage_teams::CompletedStageWorkerRow {
                unit,
                plan,
                work_item: item,
                worker,
                output,
                replayed: true,
            },
        ));
    }
    let round = load_dynamic_round_on(&mut tx, input.session_id, false).await?;
    if round.state != "resolved"
        || round.resolution_authority_id != Some(input.resolution_authority_id)
        || round.primary.worker_run_id != input.primary_worker_fence.worker_run_id
    {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let resolution =
        load_dynamic_resolution_on(&mut tx, input.resolution_authority_id, false).await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "SELECT {PLAN_COLUMNS} FROM stage_team_plans WHERE id=$1 FOR UPDATE"
    ))
    .bind(round.stage_team_plan_id)
    .fetch_one(&mut *tx)
    .await?;
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "SELECT {ITEM_COLUMNS} FROM stage_work_items WHERE id=$1 FOR UPDATE"
    ))
    .bind(round.primary.work_item_id)
    .fetch_one(&mut *tx)
    .await?;
    if plan.row_version != input.expected_plan_row_version
        || plan.requests_closed_at.is_some()
        || item.row_version != input.expected_work_item_row_version
        || item.status != "running"
    {
        return Err(fail(CAS_CONFLICT));
    }
    let canonical_output = json!({
        "schema":"investigation_asset_verification_primary_resolution.v2",
        "session_id":round.session_id,
        "hypothesis_revision_id":round.hypothesis_revision_id,
        "resolution_authority_id":resolution.resolution_authority_id,
        "resolution_sha256":resolution.resolution_sha256,
        "disposition":resolution.disposition,
    });
    let generic = stage_teams::CompleteStageWorkerRow {
        fence: RuntimeMemoryTxFence {
            operation_id: round.operation_id,
            stage_execution_id: round.stage_execution_id,
            stage_run_unit_id: round.stage_run_unit_id,
            worker_run_id: input.primary_worker_fence.worker_run_id,
            lease_token: input.primary_worker_fence.lease_token,
            attempt_epoch: input.primary_worker_fence.attempt_epoch,
            expected_checkpoint_version: input.primary_worker_fence.checkpoint_version,
        },
        team_plan_id: plan.id,
        work_item_id: item.id,
        expected_work_item_row_version: item.row_version,
        output_schema: "investigation_asset_verification_primary_resolution.v2".into(),
        business_disposition: "artifact_recorded".into(),
        canonical_output: canonical_output.clone(),
        canonical_fact_refs: json!([]),
        evidence_ids: vec![],
        checked_empty_cells: json!([]),
        blocker_codes: vec![],
        output_hash: String::new(),
        terminal_checkpoint: input.terminal_checkpoint.clone(),
        evidence_watermark: None,
    };
    let output_hash = stage_teams::canonical_stage_worker_output_hash(&generic);
    let worker = stage_worker_runs::finish_passed_for_stage_output(
        &mut *tx,
        &generic.fence,
        &input.terminal_checkpoint,
        None,
    )
    .await
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(&format!(
        "UPDATE stage_work_items SET status='completed',row_version=row_version+1,\
         terminal_at=NOW(),updated_at=NOW() WHERE id=$1 AND status='running' AND row_version=$2 \
         RETURNING {ITEM_COLUMNS}"
    ))
    .bind(item.id)
    .bind(input.expected_work_item_row_version)
    .fetch_one(&mut *tx)
    .await?;
    let output = sqlx::query_as::<_, stage_teams::StageWorkerOutputRow>(&format!(
        r#"INSERT INTO stage_worker_outputs(id,team_plan_id,work_item_id,worker_run_id,
             operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
             output_schema,output_version,business_disposition,canonical_output,
             canonical_fact_refs,evidence_ids,checked_empty_cells,blocker_codes,output_hash)
             VALUES(uuid_generate_v5($1,'stage-worker-output-v1'),$2,$1,$3,$4,$5,$6,$7,$8,
                    'investigation_asset_verification_primary_resolution.v2',1,
                    'artifact_recorded',$9,'[]',ARRAY[]::BIGINT[],'[]',ARRAY[]::TEXT[],$10)
             RETURNING {OUTPUT_COLUMNS}"#,
    ))
    .bind(item.id)
    .bind(plan.id)
    .bind(worker.id)
    .bind(round.operation_id)
    .bind(round.stage_execution_id)
    .bind(round.stage_run_unit_id)
    .bind(round.scope_snapshot_id)
    .bind(round.organization_id)
    .bind(canonical_output)
    .bind(output_hash)
    .fetch_one(&mut *tx)
    .await?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(&format!(
        "UPDATE stage_team_plans SET requests_closed_at=NOW(),\
         final_submitter_worker_run_id=NULL,row_version=row_version+1,updated_at=NOW() \
         WHERE id=$1 AND row_version=$2 AND requests_closed_at IS NULL \
         RETURNING {PLAN_COLUMNS}"
    ))
    .bind(plan.id)
    .bind(input.expected_plan_row_version)
    .fetch_one(&mut *tx)
    .await?;
    let completion_id = Uuid::new_v5(
        &input.resolution_authority_id,
        b"investigation-dynamic-verification-primary-completion.v1",
    );
    let completion_sha256 = sha256_on(
        &mut tx,
        &json!({
            "domain":"investigation_dynamic_verification_primary_completion.v1",
            "session_id":round.session_id,
            "resolution_authority_id":input.resolution_authority_id,
            "primary_worker_run_id":input.primary_worker_fence.worker_run_id,
            "primary_lease_token":input.primary_worker_fence.lease_token,
            "primary_attempt_epoch":input.primary_worker_fence.attempt_epoch,
            "expected_primary_checkpoint_version":input.primary_worker_fence.checkpoint_version,
            "expected_work_item_row_version":input.expected_work_item_row_version,
            "expected_plan_row_version":input.expected_plan_row_version,
            "terminal_checkpoint_sha256":requested_terminal_checkpoint_sha256,
            "stage_worker_output_id":output.id,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_dynamic_verification_primary_completions(
             completion_id,session_id,resolution_authority_id,primary_worker_run_id,
             primary_lease_token,primary_attempt_epoch,expected_primary_checkpoint_version,
             expected_work_item_row_version,expected_plan_row_version,
             terminal_checkpoint_sha256,stage_worker_output_id,completion_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(completion_id)
    .bind(round.session_id)
    .bind(input.resolution_authority_id)
    .bind(input.primary_worker_fence.worker_run_id)
    .bind(input.primary_worker_fence.lease_token)
    .bind(input.primary_worker_fence.attempt_epoch)
    .bind(input.primary_worker_fence.checkpoint_version)
    .bind(input.expected_work_item_row_version)
    .bind(input.expected_plan_row_version)
    .bind(requested_terminal_checkpoint_sha256)
    .bind(output.id)
    .bind(completion_sha256)
    .execute(&mut *tx)
    .await?;
    let unit = sqlx::query_as("SELECT * FROM stage_run_units WHERE id=$1")
        .bind(round.stage_run_unit_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((
        resolution,
        stage_teams::CompletedStageWorkerRow {
            unit,
            plan,
            work_item: item,
            worker,
            output,
            replayed: false,
        },
    ))
}

#[derive(Debug, Clone, FromRow)]
pub struct PendingHypothesisDiscoveryConsumptionRow {
    pub consumption_id: Uuid,
    pub discovery_authority_id: Uuid,
    pub asset_lane_id: Uuid,
    pub target_live_id: Uuid,
    pub disposition: String,
    pub admitted_root_id: Option<Uuid>,
    pub admitted_revision_id: Option<Uuid>,
    pub compiler_receipt_id: Option<Uuid>,
    pub duplicate_of_revision_id: Option<Uuid>,
    pub consumption_sha256: String,
    pub consumed_at: DateTime<Utc>,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct AdmitOrDismissPendingHypothesisDiscoveryInput {
    pub stable_request_id: Uuid,
    pub discovery_authority_id: Uuid,
    pub expected_asset_lane_id: Uuid,
    pub expected_session_id: Uuid,
}

#[derive(Debug, FromRow)]
struct CurrentPendingDiscoveryAdmissionRow {
    root_id: Uuid,
    revision_id: Uuid,
    compiler_receipt_id: Option<Uuid>,
    route_kind: Option<String>,
    compiled_after_discovery: bool,
}

pub async fn list_pending_hypothesis_discoveries(
    pool: &PgPool,
    operation_id: Uuid,
    asset_lane_id: Uuid,
) -> Result<Vec<PendingHypothesisDiscoveryRow>> {
    Ok(sqlx::query_as::<_, PendingHypothesisDiscoveryRow>(
        r#"SELECT
      backlog.discovery_authority_id,
      COALESCE(backlog.dynamic_resolution_authority_id,backlog.resolution_authority_id)
        AS resolution_authority_id,
      backlog.session_id,
      backlog.asset_lane_id,backlog.target_live_id,backlog.source_hypothesis_revision_id,
      backlog.discovery_ordinal,backlog.subject_kind,backlog.subject_identity_sha256,
      backlog.semantic_key_sha256,backlog.canonical_proposal,backlog.structured_claim,
      backlog.structured_claim_sha256,
      backlog.rationale_redacted,backlog.discovery_sha256
      FROM investigation_pending_hypothesis_discovery_backlog backlog
      JOIN investigation_asset_lanes lane ON lane.asset_lane_id=backlog.asset_lane_id
      WHERE lane.operation_id=$1 AND backlog.asset_lane_id=$2
      ORDER BY backlog.created_at,backlog.discovery_ordinal,backlog.discovery_authority_id"#,
    )
    .bind(operation_id)
    .bind(asset_lane_id)
    .fetch_all(pool)
    .await?)
}

pub async fn admit_or_dismiss_pending_hypothesis_discovery(
    pool: &PgPool,
    input: &AdmitOrDismissPendingHypothesisDiscoveryInput,
) -> Result<PendingHypothesisDiscoveryConsumptionRow> {
    let mut tx = pool.begin().await?;
    let consumption_id = Uuid::new_v5(
        &input.discovery_authority_id,
        b"investigation-pending-hypothesis-discovery-consumption-v1",
    );
    if let Some(id)=sqlx::query_scalar::<_,Uuid>("SELECT consumption_id FROM investigation_pending_hypothesis_discovery_consumptions WHERE stable_request_id=$1")
      .bind(input.stable_request_id).fetch_optional(&mut *tx).await?{
      if id!=consumption_id{return Err(fail(REPLAY_DRIFT));}
      let row=sqlx::query_as::<_,PendingHypothesisDiscoveryConsumptionRow>("SELECT consumption.*,$2::BOOLEAN AS replayed FROM investigation_pending_hypothesis_discovery_consumptions consumption WHERE consumption_id=$1")
        .bind(id).bind(true).fetch_one(&mut *tx).await?;
      let replay_owner: Option<(Uuid,Uuid)> = sqlx::query_as(
        "SELECT discovery.asset_lane_id,discovery.session_id FROM \
         investigation_pending_hypothesis_discoveries discovery \
         WHERE discovery.discovery_authority_id=$1")
        .bind(input.discovery_authority_id).fetch_optional(&mut *tx).await?;
      if row.discovery_authority_id!=input.discovery_authority_id
        || row.asset_lane_id!=input.expected_asset_lane_id
        || replay_owner!=Some((input.expected_asset_lane_id,input.expected_session_id)) {
          return Err(fail(REPLAY_DRIFT));
      }
      tx.commit().await?;return Ok(row);
    }
    let owner:(Uuid,Uuid,Uuid,String)=sqlx::query_as("SELECT discovery.asset_lane_id,discovery.target_live_id,discovery.session_id,discovery.semantic_key_sha256 FROM investigation_pending_hypothesis_discoveries discovery LEFT JOIN investigation_pending_hypothesis_discovery_consumptions consumption ON consumption.discovery_authority_id=discovery.discovery_authority_id WHERE discovery.discovery_authority_id=$1 AND consumption.discovery_authority_id IS NULL FOR UPDATE OF discovery")
      .bind(input.discovery_authority_id).fetch_one(&mut *tx).await?;
    if owner.0 != input.expected_asset_lane_id || owner.2 != input.expected_session_id {
        return Err(fail(AUTHORITY_MISMATCH));
    }
    let current = sqlx::query_as::<_, CurrentPendingDiscoveryAdmissionRow>(
        r#"SELECT revision.root_id,revision.revision_id,
                  receipt.apply_receipt_id AS compiler_receipt_id,
                  compilation_member.route_kind,
                  COALESCE(compilation_member.created_at>=discovery.created_at,FALSE)
                    AS compiled_after_discovery
             FROM investigation_pending_hypothesis_discoveries discovery
             JOIN attack_hypothesis_revisions revision
               ON revision.asset_lane_id=discovery.asset_lane_id
              AND revision.target_live_id=discovery.target_live_id
              AND revision.semantic_key_hash=discovery.semantic_key_sha256
             JOIN attack_hypothesis_heads head ON head.head_revision_id=revision.revision_id
             LEFT JOIN investigation_hypothesis_compilation_members compilation_member
               ON compilation_member.proposal_id=discovery.discovery_authority_id
              AND compilation_member.successor_revision_id=revision.revision_id
              AND compilation_member.semantic_key_sha256=discovery.semantic_key_sha256
             LEFT JOIN investigation_hypothesis_canonical_apply_receipts receipt
               ON receipt.decision_id=compilation_member.decision_id
            WHERE discovery.discovery_authority_id=$4
              AND revision.asset_lane_id=$1 AND revision.target_live_id=$2
              AND revision.semantic_key_hash=$3
              AND head.head_lifecycle_state='current'
            ORDER BY (compilation_member.proposal_id IS NOT NULL) DESC,
                     compilation_member.created_at DESC NULLS LAST,
                     revision.created_at DESC,revision.revision_id DESC LIMIT 1
            FOR SHARE OF discovery,revision,head"#,
    )
    .bind(owner.0)
    .bind(owner.1)
    .bind(&owner.3)
    .bind(input.discovery_authority_id)
    .fetch_optional(&mut *tx)
    .await?;
    let current = current.ok_or_else(|| {
        DbError::Other(anyhow::anyhow!(
            "INVESTIGATION_PENDING_HYPOTHESIS_DISCOVERY_COMPILER_ADMISSION_REQUIRED"
        ))
    })?;
    let admitted = current.compiler_receipt_id.is_some()
        && current.route_kind.as_deref() == Some("create_initial")
        && current.compiled_after_discovery;
    let disposition = if admitted {
        "admitted"
    } else {
        "dismissed_duplicate"
    };
    let admitted_root_id = admitted.then_some(current.root_id);
    let admitted_revision_id = admitted.then_some(current.revision_id);
    let duplicate_revision_id = (!admitted).then_some(current.revision_id);
    let consumption_sha256 = sha256_on(
        &mut tx,
        &json!({
            "domain":"investigation_pending_hypothesis_discovery_consumption.v1",
            "discovery_authority_id":input.discovery_authority_id,
            "asset_lane_id":owner.0,"target_live_id":owner.1,
            "disposition":disposition,"admitted_root_id":admitted_root_id,
            "admitted_revision_id":admitted_revision_id,"compiler_receipt_id":current.compiler_receipt_id,
            "duplicate_of_revision_id":duplicate_revision_id,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_pending_hypothesis_discovery_consumptions(
      consumption_id,stable_request_id,discovery_authority_id,asset_lane_id,target_live_id,
      disposition,admitted_root_id,admitted_revision_id,compiler_receipt_id,
      duplicate_of_revision_id,consumption_sha256)
      VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(consumption_id)
    .bind(input.stable_request_id)
    .bind(input.discovery_authority_id)
    .bind(owner.0)
    .bind(owner.1)
    .bind(disposition)
    .bind(admitted_root_id)
    .bind(admitted_revision_id)
    .bind(current.compiler_receipt_id)
    .bind(duplicate_revision_id)
    .bind(consumption_sha256)
    .execute(&mut *tx)
    .await?;
    let row=sqlx::query_as::<_,PendingHypothesisDiscoveryConsumptionRow>("SELECT consumption.*,$2::BOOLEAN AS replayed FROM investigation_pending_hypothesis_discovery_consumptions consumption WHERE consumption_id=$1")
      .bind(consumption_id).bind(false).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(row)
}
