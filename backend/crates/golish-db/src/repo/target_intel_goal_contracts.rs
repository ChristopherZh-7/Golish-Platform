//! Immutable Target Intel Goal operation contracts.

use anyhow::{bail, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TargetIntelGoalOperationContractRow {
    pub operation_id: Uuid,
    pub profile_id: String,
    pub runtime_mode: String,
    pub completion_authority: String,
    pub goal_contract_version: String,
    pub canonical_goal_contract: Value,
    pub goal_contract_sha256: String,
    pub methodology_payload: Value,
    pub methodology_sha256: String,
    pub tool_manifest: Value,
    pub tool_manifest_sha256: String,
    pub provider_capability_manifest: Value,
    pub provider_capability_sha256: String,
    pub browser_policy: Value,
    pub budget_policy: Value,
    pub max_review_rounds: i32,
    pub reviewer_retry_fuel: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FreezeTargetIntelGoalUnit {
    pub contract: TargetIntelGoalOperationContractRow,
    pub organization_id: Uuid,
    pub team_plan_id: Uuid,
    pub goal_epoch_id: Uuid,
    pub controller_work_item_id: Uuid,
    pub controller_worker_run_id: Uuid,
    pub controller_message_chain_id: Uuid,
}

pub async fn get_by_operation(
    pool: &PgPool,
    operation_id: Uuid,
) -> Result<Option<TargetIntelGoalOperationContractRow>> {
    let row = sqlx::query_as::<_, TargetIntelGoalOperationContractRow>(
        r#"SELECT operation_id, profile_id, runtime_mode, completion_authority,
                  goal_contract_version, canonical_goal_contract, goal_contract_sha256,
                  methodology_payload, methodology_sha256, tool_manifest,
                  tool_manifest_sha256, provider_capability_manifest,
                  provider_capability_sha256, browser_policy, budget_policy,
                  max_review_rounds, reviewer_retry_fuel
             FROM target_intel_goal_operation_contracts
            WHERE operation_id = $1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn insert_immutable(
    pool: &PgPool,
    row: &TargetIntelGoalOperationContractRow,
) -> Result<()> {
    if row.operation_id.is_nil()
        || row.max_review_rounds <= 0
        || canonical_sha256(&row.canonical_goal_contract) != row.goal_contract_sha256
        || canonical_sha256(&row.methodology_payload) != row.methodology_sha256
        || canonical_sha256(&row.tool_manifest) != row.tool_manifest_sha256
        || canonical_sha256(&row.provider_capability_manifest) != row.provider_capability_sha256
    {
        bail!("TARGET_INTEL_GOAL_OPERATION_CONTRACT_INVALID");
    }
    sqlx::query(
        r#"INSERT INTO target_intel_goal_operation_contracts (
               operation_id, profile_id, runtime_mode, completion_authority,
               goal_contract_version, canonical_goal_contract, goal_contract_sha256,
               methodology_payload, methodology_sha256, tool_manifest,
               tool_manifest_sha256, provider_capability_manifest,
               provider_capability_sha256, browser_policy, budget_policy,
               max_review_rounds, reviewer_retry_fuel
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17
           )"#,
    )
    .bind(row.operation_id)
    .bind(&row.profile_id)
    .bind(&row.runtime_mode)
    .bind(&row.completion_authority)
    .bind(&row.goal_contract_version)
    .bind(&row.canonical_goal_contract)
    .bind(&row.goal_contract_sha256)
    .bind(&row.methodology_payload)
    .bind(&row.methodology_sha256)
    .bind(&row.tool_manifest)
    .bind(&row.tool_manifest_sha256)
    .bind(&row.provider_capability_manifest)
    .bind(&row.provider_capability_sha256)
    .bind(&row.browser_policy)
    .bind(&row.budget_policy)
    .bind(row.max_review_rounds)
    .bind(row.reviewer_retry_fuel)
    .execute(pool)
    .await?;
    Ok(())
}

/// Freeze one operation contract and initialize the exact org/StageTeam Goal
/// epoch in a single transaction. The caller supplies server-owned ids only;
/// every ownership edge is re-read from durable StageTeam rows.
pub async fn freeze_unit(pool: &PgPool, input: &FreezeTargetIntelGoalUnit) -> Result<bool> {
    let row = &input.contract;
    if row.operation_id.is_nil()
        || input.organization_id.is_nil()
        || input.team_plan_id.is_nil()
        || input.goal_epoch_id.is_nil()
        || input.controller_work_item_id.is_nil()
        || input.controller_worker_run_id.is_nil()
        || input.controller_message_chain_id.is_nil()
    {
        bail!("TARGET_INTEL_GOAL_FREEZE_IDENTITY_INVALID");
    }
    let mut tx = pool.begin().await?;
    if canonical_sha256(&row.canonical_goal_contract) != row.goal_contract_sha256
        || canonical_sha256(&row.methodology_payload) != row.methodology_sha256
        || canonical_sha256(&row.tool_manifest) != row.tool_manifest_sha256
        || canonical_sha256(&row.provider_capability_manifest) != row.provider_capability_sha256
    {
        bail!("TARGET_INTEL_GOAL_OPERATION_CONTRACT_HASH_MISMATCH");
    }
    let owner = sqlx::query_as::<_, (Uuid, Uuid, Uuid, Uuid, Uuid, Uuid)>(
        r#"SELECT p.operation_id, p.organization_id, p.stage_execution_id,
                  p.stage_run_unit_id, p.scope_snapshot_id, worker.message_chain_id
             FROM stage_team_plans p
             JOIN stage_work_items item
               ON item.id=$2 AND item.team_plan_id=p.id
              AND item.operation_id=p.operation_id
              AND item.organization_id=p.organization_id
              AND item.role=p.leader_role
              AND item.stable_key='leader:primary'
              AND item.status='running'
             JOIN stage_worker_runs worker
               ON worker.id=$3 AND worker.work_item_id=item.id
              AND worker.operation_id=p.operation_id
              AND worker.organization_id=p.organization_id
              AND worker.status='running'
              AND worker.message_chain_id IS NOT NULL
            WHERE p.id=$1 AND p.stage_kind='target_intel'
              AND p.dispatch_epoch=0
              AND (
                    (p.requests_closed_at IS NULL
                     AND p.final_submitter_worker_run_id IS NULL)
                 OR (p.requests_closed_at IS NOT NULL
                     AND p.final_submitter_worker_run_id=$3)
              )
            FOR UPDATE OF p,item,worker"#,
    )
    .bind(input.team_plan_id)
    .bind(input.controller_work_item_id)
    .bind(input.controller_worker_run_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_GOAL_FREEZE_STAGE_TEAM_OWNER_MISSING"))?;
    if owner.0 != row.operation_id
        || owner.1 != input.organization_id
        || owner.5 != input.controller_message_chain_id
    {
        bail!("TARGET_INTEL_GOAL_FREEZE_STAGE_TEAM_OWNER_MISMATCH");
    }
    let inserted = sqlx::query(
        r#"INSERT INTO target_intel_goal_operation_contracts (
               operation_id, profile_id, runtime_mode, completion_authority,
               goal_contract_version, canonical_goal_contract, goal_contract_sha256,
               methodology_payload, methodology_sha256, tool_manifest,
               tool_manifest_sha256, provider_capability_manifest,
               provider_capability_sha256, browser_policy, budget_policy,
               max_review_rounds, reviewer_retry_fuel
           ) VALUES (
               $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17
           ) ON CONFLICT (operation_id) DO NOTHING"#,
    )
    .bind(row.operation_id)
    .bind(&row.profile_id)
    .bind(&row.runtime_mode)
    .bind(&row.completion_authority)
    .bind(&row.goal_contract_version)
    .bind(&row.canonical_goal_contract)
    .bind(&row.goal_contract_sha256)
    .bind(&row.methodology_payload)
    .bind(&row.methodology_sha256)
    .bind(&row.tool_manifest)
    .bind(&row.tool_manifest_sha256)
    .bind(&row.provider_capability_manifest)
    .bind(&row.provider_capability_sha256)
    .bind(&row.browser_policy)
    .bind(&row.budget_policy)
    .bind(row.max_review_rounds)
    .bind(row.reviewer_retry_fuel)
    .execute(&mut *tx)
    .await?;
    let persisted = sqlx::query_as::<_, TargetIntelGoalOperationContractRow>(
        r#"SELECT operation_id, profile_id, runtime_mode, completion_authority,
                  goal_contract_version, canonical_goal_contract, goal_contract_sha256,
                  methodology_payload, methodology_sha256, tool_manifest,
                  tool_manifest_sha256, provider_capability_manifest,
                  provider_capability_sha256, browser_policy, budget_policy,
                  max_review_rounds, reviewer_retry_fuel
             FROM target_intel_goal_operation_contracts
            WHERE operation_id=$1"#,
    )
    .bind(row.operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if &persisted != row {
        bail!("TARGET_INTEL_GOAL_OPERATION_CONTRACT_REPLAY_MISMATCH");
    }
    // IntelGoalV1 never derives company ownership from a free-form organization
    // name.  Bind the one immutable, confirmed Scoping receipt before opening
    // epoch zero; absence or ambiguity fails closed.
    let identity = sqlx::query_as::<_, (Uuid, String, String)>(
        r#"SELECT id,identity_sha256,scope_policy_sha256
             FROM scoping_company_identity_receipts
            WHERE operation_id=$1 AND organization_id=$2
              AND resolution_status='confirmed'
            FOR SHARE"#,
    )
    .bind(row.operation_id)
    .bind(input.organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_CONFIRMED_COMPANY_IDENTITY_MISSING"))?;
    sqlx::query(
        r#"INSERT INTO target_intel_goal_company_identity_bindings(
               operation_id,organization_id,company_identity_receipt_id,
               company_identity_sha256,scope_policy_sha256
           ) VALUES($1,$2,$3,$4,$5)
           ON CONFLICT(operation_id) DO NOTHING"#,
    )
    .bind(row.operation_id)
    .bind(input.organization_id)
    .bind(identity.0)
    .bind(&identity.1)
    .bind(&identity.2)
    .execute(&mut *tx)
    .await?;
    let persisted_identity: (Uuid, Uuid, String, String) = sqlx::query_as(
        r#"SELECT organization_id,company_identity_receipt_id,
                  company_identity_sha256,scope_policy_sha256
             FROM target_intel_goal_company_identity_bindings
            WHERE operation_id=$1"#,
    )
    .bind(row.operation_id)
    .fetch_one(&mut *tx)
    .await?;
    if persisted_identity != (input.organization_id, identity.0, identity.1, identity.2) {
        bail!("TARGET_INTEL_COMPANY_IDENTITY_BINDING_REPLAY_MISMATCH");
    }
    sqlx::query(
        r#"INSERT INTO target_intel_goal_material_revisions (
               operation_id, organization_id
           ) VALUES ($1,$2)
           ON CONFLICT (operation_id, organization_id) DO NOTHING"#,
    )
    .bind(row.operation_id)
    .bind(input.organization_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO target_intel_goal_epochs (
               id, operation_id, organization_id, team_plan_id,
               stage_execution_id, stage_run_unit_id, scope_snapshot_id,
               epoch, status, review_fuel_remaining,
               controller_work_item_id, controller_worker_run_id,
               controller_message_chain_id
           ) VALUES ($1,$2,$3,$4,$5,$6,$7,0,'open',$8,$9,$10,$11)
           ON CONFLICT (team_plan_id, epoch) DO NOTHING"#,
    )
    .bind(input.goal_epoch_id)
    .bind(row.operation_id)
    .bind(input.organization_id)
    .bind(input.team_plan_id)
    .bind(owner.2)
    .bind(owner.3)
    .bind(owner.4)
    .bind(row.max_review_rounds)
    .bind(input.controller_work_item_id)
    .bind(input.controller_worker_run_id)
    .bind(input.controller_message_chain_id)
    .execute(&mut *tx)
    .await?;
    let epoch = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            Uuid,
            i32,
            Uuid,
            Uuid,
            Option<Uuid>,
        ),
    >(
        r#"SELECT id, operation_id, organization_id, team_plan_id,
                  stage_execution_id, stage_run_unit_id, scope_snapshot_id,
                  review_fuel_remaining,
                  controller_work_item_id, controller_worker_run_id,
                  controller_message_chain_id
             FROM target_intel_goal_epochs
            WHERE team_plan_id=$1 AND epoch=0"#,
    )
    .bind(input.team_plan_id)
    .fetch_one(&mut *tx)
    .await?;
    if epoch
        != (
            input.goal_epoch_id,
            row.operation_id,
            input.organization_id,
            input.team_plan_id,
            owner.2,
            owner.3,
            owner.4,
            row.max_review_rounds,
            input.controller_work_item_id,
            input.controller_worker_run_id,
            Some(input.controller_message_chain_id),
        )
    {
        bail!("TARGET_INTEL_GOAL_EPOCH_REPLAY_MISMATCH");
    }
    tx.commit().await?;
    Ok(inserted.rows_affected() == 0)
}

fn canonical_sha256(value: &Value) -> String {
    fn write(value: &Value, output: &mut Vec<u8>) {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                output.extend(serde_json::to_vec(value).unwrap_or_default());
            }
            Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    write(value, output);
                }
                output.push(b']');
            }
            Value::Object(map) => {
                output.push(b'{');
                let mut keys = map.keys().collect::<Vec<_>>();
                keys.sort();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        output.push(b',');
                    }
                    output.extend(serde_json::to_vec(key).unwrap_or_default());
                    output.push(b':');
                    write(&map[key], output);
                }
                output.push(b'}');
            }
        }
    }
    let mut canonical = Vec::new();
    write(value, &mut canonical);
    format!(
        "sha256:{}",
        Sha256::digest(canonical)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
