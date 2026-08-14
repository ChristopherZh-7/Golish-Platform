//! Prepared Action compilation, authorization, conflict/budget fencing and journal compounds.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    capability_execution_receipts::{
        with_all_fresh_tool_truth_authority_bundle, AllFreshToolTruthAuthorityBundle,
        CheckToolTruthAuthorityBundle, ToolTruthAuthorityBundleConsumerV1,
    },
    verification_campaigns::{
        conflict, exact_set_hash_on, json_hash_on, require_sha256, AUTHORITY_STALE,
        CONTRACT_INVALID,
    },
};
use crate::Result;

#[derive(Debug, Clone)]
pub struct PreparedActionGroupMember {
    pub canonical_request_hash: String,
    pub credential_session_binding_hash: Option<String>,
    pub barrier_cohort_hash: String,
    pub expected_start_window_ms: i64,
    pub upper_budget_hash: String,
    pub oracle_role: String,
}

#[derive(Debug, Clone)]
pub struct PersistPreparedAction {
    pub stable_request_id: Uuid,
    pub campaign_id: Uuid,
    pub round_id: Uuid,
    pub strategy_artifact_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub capability_assessment_id: Uuid,
    pub action_ordinal: i32,
    pub action_contract_kind: String,
    pub action_kind: String,
    pub canonical_request_hash: String,
    pub display_projection: Value,
    pub renderer_version: String,
    pub private_manifest: Value,
    pub private_manifest_hash: String,
    pub review_expires_at: DateTime<Utc>,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub credential_binding_hash: Option<String>,
    pub policy_snapshot_hash: String,
    pub upper_budget_set_hash: String,
    pub oracle_contract_hash: String,
    pub risk_tier: String,
    pub compile_rejection: Option<(String, Uuid)>,
    pub group_members: Vec<PreparedActionGroupMember>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedCredentialBinding {
    handle_id: Uuid,
    handle_version: u32,
    revocation_generation: i64,
    injection_origin: String,
    injection_contract_version: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct PreparedActionRow {
    pub prepared_action_id: Uuid,
    pub stable_request_id: Uuid,
    pub campaign_id: Uuid,
    pub action_ordinal: i32,
    pub action_contract_kind: String,
    pub action_kind: String,
    pub canonical_request_hash: String,
    pub state: String,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
}

const ACTION_COLUMNS: &str = r#"prepared_action_id,stable_request_id,campaign_id,
    action_ordinal,action_contract_kind,action_kind,canonical_request_hash,state,
    row_version,created_at,terminal_at"#;

pub async fn persist_compiled_prepared_action(
    pool: &PgPool,
    command: &PersistPreparedAction,
) -> Result<PreparedActionRow> {
    for hash in [
        &command.canonical_request_hash,
        &command.private_manifest_hash,
        &command.target_identity_hash,
        &command.policy_snapshot_hash,
        &command.upper_budget_set_hash,
        &command.oracle_contract_hash,
    ] {
        require_sha256(hash)?;
    }
    if let Some(hash) = &command.credential_binding_hash {
        require_sha256(hash)?;
    }
    let is_group = command.action_contract_kind == "concurrent_action_group_v1";
    if !matches!(
        command.action_contract_kind.as_str(),
        "single_action_v1" | "concurrent_action_group_v1"
    ) || command.action_kind.trim().is_empty()
        || !matches!(command.risk_tier.as_str(), "T0" | "T1" | "T2" | "T3")
        || (is_group && command.group_members.len() < 2)
        || (!is_group && !command.group_members.is_empty())
        || (command.compile_rejection.is_none() && command.target_live_id.is_none())
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let credential = command
        .private_manifest
        .get("credential")
        .filter(|value| !value.is_null())
        .cloned()
        .map(serde_json::from_value::<PersistedCredentialBinding>)
        .transpose()
        .map_err(|_| conflict(CONTRACT_INVALID))?;
    if credential.is_some() != command.credential_binding_hash.is_some() {
        return Err(conflict(CONTRACT_INVALID));
    }
    if let Some(credential) = &credential {
        if credential.handle_version == 0
            || credential.revocation_generation < 0
            || credential.injection_origin.trim().is_empty()
            || credential.injection_contract_version.trim().is_empty()
        {
            return Err(conflict(CONTRACT_INVALID));
        }
        let vault_authorized: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM vault_entries vault
                   JOIN project_scopes scope
                     ON scope.project_scope_id=$2
                    AND scope.canonical_project_path=vault.project_path
                    AND scope.retired_at IS NULL
                  WHERE vault.id=$1 AND COALESCE(vault.status::TEXT,'unknown')<>'invalid'
               )"#,
        )
        .bind(credential.handle_id)
        .bind(command.project_scope_id)
        .fetch_one(&mut *tx)
        .await?;
        if !vault_authorized {
            return Err(conflict(AUTHORITY_STALE));
        }
        sqlx::query(
            r#"INSERT INTO verification_credential_authority_heads(
                   operation_id,project_scope_id,organization_id,handle_id,handle_version,
                   revocation_generation,revoked,injection_origin,injection_contract_version
               ) VALUES($1,$2,$3,$4,$5,$6,FALSE,$7,$8)
               ON CONFLICT(operation_id,handle_id) DO NOTHING"#,
        )
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(credential.handle_id)
        .bind(i64::from(credential.handle_version))
        .bind(credential.revocation_generation)
        .bind(&credential.injection_origin)
        .bind(&credential.injection_contract_version)
        .execute(&mut *tx)
        .await?;
        let persisted: (Uuid, i64, i64, bool, String, String) = sqlx::query_as(
            r#"SELECT organization_id,handle_version,revocation_generation,revoked,
                      injection_origin,injection_contract_version
                 FROM verification_credential_authority_heads
                WHERE operation_id=$1 AND handle_id=$2 FOR SHARE"#,
        )
        .bind(command.operation_id)
        .bind(credential.handle_id)
        .fetch_one(&mut *tx)
        .await?;
        if persisted
            != (
                command.organization_id,
                i64::from(credential.handle_version),
                credential.revocation_generation,
                false,
                credential.injection_origin.clone(),
                credential.injection_contract_version.clone(),
            )
        {
            return Err(conflict(AUTHORITY_STALE));
        }
    }
    if let Some(target_live_id) = command.target_live_id {
        let target: Option<(String, String)> = sqlx::query_as(
            r#"SELECT target.target_type::TEXT,target.value
                 FROM targets target
                 JOIN project_scopes scope
                   ON scope.project_scope_id=$2
                  AND scope.canonical_project_path=target.project_path
                  AND scope.retired_at IS NULL
                WHERE target.id=$1 AND target.organization_id=$3 AND target.scope='in'"#,
        )
        .bind(target_live_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (target_type, target_value) = target.ok_or_else(|| conflict(AUTHORITY_STALE))?;
        let server_target_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "target_live_id": target_live_id,
                "project_scope_id": command.project_scope_id,
                "organization_id": command.organization_id,
                "target_type": target_type,
                "target_value": target_value,
            }),
        )
        .await?;
        if target_type != command.target_type_at_time
            || target_value != command.target_value_at_time
            || server_target_hash != command.target_identity_hash
        {
            return Err(conflict(AUTHORITY_STALE));
        }
    }
    let display_projection_hash = json_hash_on(&mut tx, &command.display_projection).await?;
    let prepared_action_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-prepared-action.v1",
    );
    let existing = sqlx::query_as::<_, PreparedActionRow>(&format!(
        "SELECT {ACTION_COLUMNS} FROM verification_prepared_actions WHERE stable_request_id=$1 FOR SHARE"
    ))
    .bind(command.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = existing {
        if row.campaign_id == command.campaign_id
            && row.action_ordinal == command.action_ordinal
            && row.canonical_request_hash == command.canonical_request_hash
        {
            tx.commit().await?;
            return Ok(row);
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    let (state, reason_code, residual_id) =
        if let Some((reason, residual)) = &command.compile_rejection {
            ("compile_rejected", Some(reason.as_str()), Some(*residual))
        } else {
            ("pending_authorization", None, None)
        };
    let row = sqlx::query_as::<_, PreparedActionRow>(&format!(
        r#"INSERT INTO verification_prepared_actions(
               prepared_action_id,stable_request_id,campaign_id,round_id,strategy_artifact_id,
               operation_id,project_scope_id,organization_id,capability_assessment_id,
               action_ordinal,action_contract_kind,action_kind,canonical_request_hash,
               display_projection,display_projection_hash,renderer_version,
               private_manifest,private_manifest_hash,review_expires_at,
               target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,
               credential_binding_hash,policy_snapshot_hash,upper_budget_set_hash,
               oracle_contract_hash,risk_tier,state,reason_code,residual_id,terminal_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,
                    $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,
                    CASE WHEN $29='compile_rejected' THEN statement_timestamp() ELSE NULL END)
           RETURNING {ACTION_COLUMNS}"#
    ))
    .bind(prepared_action_id)
    .bind(command.stable_request_id)
    .bind(command.campaign_id)
    .bind(command.round_id)
    .bind(command.strategy_artifact_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.capability_assessment_id)
    .bind(command.action_ordinal)
    .bind(&command.action_contract_kind)
    .bind(&command.action_kind)
    .bind(&command.canonical_request_hash)
    .bind(&command.display_projection)
    .bind(&display_projection_hash)
    .bind(&command.renderer_version)
    .bind(&command.private_manifest)
    .bind(&command.private_manifest_hash)
    .bind(command.review_expires_at)
    .bind(command.target_live_id)
    .bind(&command.target_type_at_time)
    .bind(&command.target_value_at_time)
    .bind(&command.target_identity_hash)
    .bind(&command.credential_binding_hash)
    .bind(&command.policy_snapshot_hash)
    .bind(&command.upper_budget_set_hash)
    .bind(&command.oracle_contract_hash)
    .bind(&command.risk_tier)
    .bind(state)
    .bind(reason_code)
    .bind(residual_id)
    .fetch_one(&mut *tx)
    .await?;
    for (ordinal, member) in command.group_members.iter().enumerate() {
        for hash in [
            &member.canonical_request_hash,
            &member.barrier_cohort_hash,
            &member.upper_budget_hash,
        ] {
            require_sha256(hash)?;
        }
        if let Some(hash) = &member.credential_session_binding_hash {
            require_sha256(hash)?;
        }
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "canonical_request_hash": member.canonical_request_hash,
                "credential_session_binding_hash": member.credential_session_binding_hash,
                "barrier_cohort_hash": member.barrier_cohort_hash,
                "expected_start_window_ms": member.expected_start_window_ms,
                "upper_budget_hash": member.upper_budget_hash,
                "oracle_role": member.oracle_role,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO verification_prepared_action_group_members(
                   group_member_id,prepared_action_id,member_ordinal,canonical_request_hash,
                   credential_session_binding_hash,barrier_cohort_hash,expected_start_window_ms,
                   upper_budget_hash,oracle_role,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(Uuid::new_v5(&prepared_action_id, member_hash.as_bytes()))
        .bind(prepared_action_id)
        .bind(ordinal as i32)
        .bind(&member.canonical_request_hash)
        .bind(&member.credential_session_binding_hash)
        .bind(&member.barrier_cohort_hash)
        .bind(member.expected_start_window_ms)
        .bind(&member.upper_budget_hash)
        .bind(&member.oracle_role)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(row)
}

#[derive(Debug, Clone)]
pub struct ConflictKeyMember {
    pub key_kind: String,
    pub key_identity_hash: String,
    pub adapter_commutativity_authority_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SealActionConflictSet {
    pub stable_request_id: Uuid,
    pub prepared_action_id: Uuid,
    pub campaign_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub members: Vec<ConflictKeyMember>,
}

pub async fn seal_action_conflict_set(
    pool: &PgPool,
    command: &SealActionConflictSet,
) -> Result<Uuid> {
    if command.members.is_empty() {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut canonical = command.members.clone();
    canonical.sort_by(|left, right| {
        (&left.key_kind, &left.key_identity_hash).cmp(&(&right.key_kind, &right.key_identity_hash))
    });
    if canonical.windows(2).any(|pair| {
        pair[0].key_kind == pair[1].key_kind
            && pair[0].key_identity_hash == pair[1].key_identity_hash
    }) {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let set_id = Uuid::new_v5(&command.stable_request_id, b"verification-conflict-set.v1");
    let mut rows = Vec::with_capacity(canonical.len());
    for (ordinal, member) in canonical.iter().enumerate() {
        let member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": ordinal,
                "key_kind": member.key_kind,
                "key_identity_hash": member.key_identity_hash,
                "adapter_commutativity_authority_hash": member.adapter_commutativity_authority_hash,
            }),
        )
        .await?;
        rows.push((member, member_hash));
    }
    let hashes = rows.iter().map(|row| row.1.clone()).collect::<Vec<_>>();
    let set_hash =
        exact_set_hash_on(&mut tx, "verification_action_conflict_set.v1", &hashes).await?;
    sqlx::query(
        r#"INSERT INTO verification_action_conflict_sets(
               conflict_set_id,stable_request_id,prepared_action_id,campaign_id,
               operation_id,project_scope_id,organization_id,member_count,member_set_hash,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,NULL)"#,
    )
    .bind(set_id)
    .bind(command.stable_request_id)
    .bind(command.prepared_action_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(rows.len() as i64)
    .bind(&set_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (member, member_hash)) in rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO verification_action_conflict_set_members(
                   conflict_set_id,member_ordinal,key_kind,key_identity_hash,
                   adapter_commutativity_authority_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(set_id)
        .bind(ordinal as i32)
        .bind(&member.key_kind)
        .bind(&member.key_identity_hash)
        .bind(&member.adapter_commutativity_authority_hash)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE verification_action_conflict_sets SET sealed_at=statement_timestamp() WHERE conflict_set_id=$1 AND sealed_at IS NULL",
    )
    .bind(set_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(set_id)
}

#[derive(Debug, Clone)]
pub struct ListPendingPreparedActions {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub campaign_id: Option<Uuid>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct PreparedActionReviewRow {
    pub prepared_action_id: Uuid,
    pub campaign_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub action_kind: String,
    pub display_projection: Value,
    pub display_projection_hash: String,
    pub renderer_version: String,
    pub private_manifest_hash: String,
    pub risk_tier: String,
    pub state: String,
    pub row_version: i64,
    pub review_expires_at: DateTime<Utc>,
    pub authorization_receipt_id: Option<Uuid>,
    pub authorization_decision: Option<String>,
    pub authorization_campaign_dispatch_generation: Option<i64>,
    pub authorization_expires_at: Option<DateTime<Utc>>,
    pub authorization_decided_at: Option<DateTime<Utc>>,
}

/// Returns only the immutable safe display projection and opaque hashes.  Raw
/// target values, canonical request material and credentials are intentionally
/// absent from both the SELECT and the public row type.
pub async fn list_pending_prepared_actions(
    pool: &PgPool,
    query: &ListPendingPreparedActions,
) -> Result<Vec<PreparedActionReviewRow>> {
    Ok(sqlx::query_as::<_, PreparedActionReviewRow>(
        r#"SELECT action.prepared_action_id,action.campaign_id,action.operation_id,
                  action.project_scope_id,action.organization_id,action.action_kind,
                  action.display_projection,action.display_projection_hash,
                  action.renderer_version,action.private_manifest_hash,action.risk_tier,
                  action.state,action.row_version,action.review_expires_at,
                  latest_auth.authorization_receipt_id,
                  latest_auth.decision AS authorization_decision,
                  latest_auth.campaign_dispatch_generation AS authorization_campaign_dispatch_generation,
                  latest_auth.expires_at AS authorization_expires_at,
                  latest_auth.decided_at AS authorization_decided_at
             FROM verification_prepared_actions action
             LEFT JOIN LATERAL (
                 SELECT receipt.authorization_receipt_id,receipt.decision,
                        receipt.campaign_dispatch_generation,
                        receipt.expires_at,receipt.decided_at
                   FROM verification_prepared_action_authorizations receipt
                  WHERE receipt.prepared_action_id=action.prepared_action_id
                  ORDER BY receipt.decided_at DESC,receipt.authorization_receipt_id DESC
                  LIMIT 1
             ) latest_auth ON TRUE
            WHERE action.operation_id=$1 AND action.project_scope_id=$2
              AND (
                  ($3::UUID IS NULL AND action.state='pending_authorization'
                       AND action.review_expires_at>statement_timestamp())
                  OR ($3::UUID IS NOT NULL AND action.campaign_id=$3)
              )
            ORDER BY action.created_at,action.prepared_action_id"#,
    )
    .bind(query.operation_id)
    .bind(query.project_scope_id)
    .bind(query.campaign_id)
    .fetch_all(pool)
    .await?)
}

/// Strongly reads the exact Campaign dispatch authority used by both JIT
/// authorization and every host-owned send check. A held lane is not an
/// authorization generation and therefore fails closed.
pub async fn current_campaign_dispatch_generation(pool: &PgPool) -> Result<i64> {
    sqlx::query_scalar(
        r#"SELECT campaign_dispatch_generation
             FROM verification_campaign_safety_holds
            WHERE singleton=TRUE AND campaign_dispatch_held=FALSE"#,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PreparedActionSendAuthorityRow {
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub authorization_receipt_id: Uuid,
    pub action_execution_id: Uuid,
    pub private_manifest: Value,
    pub private_manifest_hash: String,
    pub authorization_expires_at: DateTime<Utc>,
    pub campaign_dispatch_generation: i64,
    pub campaign_dispatch_held: bool,
    pub current_campaign_dispatch_generation: i64,
    pub operation_admission_held: bool,
    pub operation_admission_generation: i64,
    pub safety_hold_row_version: i64,
    pub quarantine_pending: bool,
    pub checked_at: DateTime<Utc>,
    pub budget_reservation_id: Uuid,
    pub credential_handle_id: Option<Uuid>,
    pub credential_handle_version: Option<i64>,
    pub credential_revocation_generation: Option<i64>,
    pub credential_revoked: Option<bool>,
    pub credential_injection_origin: Option<String>,
    pub credential_injection_contract_version: Option<String>,
    pub credential_authority_updated_at: Option<DateTime<Utc>>,
    pub credential_vault_status: Option<String>,
    pub credential_vault_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PreparedActionBudgetHeadRow {
    pub scope_kind: String,
    pub budget_contract_id: Uuid,
    pub axis_kind: String,
    pub axis_limit: i64,
    pub consumed: i64,
    pub reserved: i64,
    pub unknown_held: i64,
    pub reservation_remaining: i64,
    pub row_version: i64,
}

#[derive(Debug, Clone)]
pub struct CurrentPreparedActionSendAuthority {
    pub action: PreparedActionSendAuthorityRow,
    pub budget_heads: Vec<PreparedActionBudgetHeadRow>,
}

/// Loads one exact durable-begin authority under a repeatable-read snapshot.
/// The private manifest is never selected by UI/report readers.
pub async fn load_current_prepared_action_send_authority(
    pool: &PgPool,
    operation_id: Uuid,
    campaign_id: Uuid,
    prepared_action_id: Uuid,
    authorization_receipt_id: Uuid,
    action_execution_id: Uuid,
) -> Result<CurrentPreparedActionSendAuthority> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let action = sqlx::query_as::<_, PreparedActionSendAuthorityRow>(
        r#"SELECT action.operation_id,action.project_scope_id,action.organization_id,
                  action.campaign_id,action.prepared_action_id,
                  auth.authorization_receipt_id,execution.action_execution_id,
                  action.private_manifest,action.private_manifest_hash,
                  auth.expires_at AS authorization_expires_at,
                  auth.campaign_dispatch_generation,
                  hold.campaign_dispatch_held,
                  hold.campaign_dispatch_generation AS current_campaign_dispatch_generation,
                  hold.operation_admission_held,hold.operation_admission_generation,
                  hold.row_version AS safety_hold_row_version,
                  EXISTS(
                      SELECT 1
                        FROM verification_authority_quarantine_events quarantine
                        JOIN verification_campaign_terminal_decisions terminal
                          ON terminal.campaign_terminal_decision_id=
                             quarantine.campaign_terminal_decision_id
                       WHERE terminal.campaign_id=action.campaign_id
                  ) OR campaign.effective_valid_until<=statement_timestamp()
                    OR EXISTS(
                        SELECT 1
                          FROM tool_truth_authority_bundle_members bundle_member
                          LEFT JOIN LATERAL (
                              SELECT denominator.id
                                FROM coverage_denominators denominator
                               WHERE denominator.operation_id=action.operation_id
                                 AND denominator.organization_id=action.organization_id
                                 AND denominator.denominator_kind='root'
                                 AND denominator.stage_kind=CASE bundle_member.root_family
                                     WHEN 'ti' THEN 'target_intel'
                                     WHEN 'eas' THEN 'external_attack_surface'
                                     WHEN 'enum' THEN 'enumeration'
                                     WHEN 'vuln' THEN 'vuln_triage'
                                 END
                                 AND denominator.sealed_at IS NOT NULL
                               ORDER BY denominator.created_at DESC,denominator.id DESC
                               LIMIT 1
                          ) latest_root ON TRUE
                          LEFT JOIN tool_truth_authority_set_members authority_member
                            ON authority_member.authority_set_id=
                               bundle_member.authority_set_seal_id
                          LEFT JOIN capability_execution_receipts receipt
                            ON receipt.id=authority_member.receipt_id
                         WHERE bundle_member.bundle_seal_id=
                               campaign.tool_truth_authority_bundle_seal_id
                           AND (
                               latest_root.id IS DISTINCT FROM bundle_member.root_denominator_id
                               OR receipt.id IS NULL
                               OR receipt.current_semantic_authority_version
                                  IS DISTINCT FROM authority_member.semantic_authority_version
                               OR receipt.current_semantic_reconciliation_id
                                  IS DISTINCT FROM authority_member.reconciliation_id
                               OR receipt.current_semantic_reconciliation_hash
                                  IS DISTINCT FROM authority_member.semantic_hash
                               OR receipt.reconciliation_state<>'consistent'
                               OR receipt.valid_until IS NULL
                               OR receipt.valid_until<=statement_timestamp()
                           )
                    ) AS quarantine_pending,
                  statement_timestamp() AS checked_at,
                  execution.budget_reservation_id,
                  credential.handle_id AS credential_handle_id,
                  credential.handle_version AS credential_handle_version,
                  credential.revocation_generation AS credential_revocation_generation,
                  credential.revoked AS credential_revoked,
                  credential.injection_origin AS credential_injection_origin,
                  credential.injection_contract_version AS credential_injection_contract_version,
                  credential.updated_at AS credential_authority_updated_at,
                  vault.status AS credential_vault_status,
                  vault.updated_at AS credential_vault_updated_at
             FROM verification_prepared_actions action
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=action.campaign_id
              AND campaign.operation_id=action.operation_id
              AND campaign.organization_id=action.organization_id
              AND campaign.terminal_at IS NULL AND campaign.superseded_at IS NULL
             JOIN verification_prepared_action_authorizations auth
               ON auth.prepared_action_id=action.prepared_action_id
              AND auth.authorization_receipt_id=$4
              AND auth.decision='authorized'
             JOIN verification_action_executions execution
               ON execution.prepared_action_id=action.prepared_action_id
              AND execution.authorization_receipt_id=auth.authorization_receipt_id
              AND execution.action_execution_id=$5
              AND execution.state='started'
              AND execution.campaign_dispatch_generation=
                  auth.campaign_dispatch_generation
             LEFT JOIN verification_credential_authority_heads credential
               ON credential.operation_id=action.operation_id
              AND credential.handle_id=NULLIF(
                  action.private_manifest #>> '{credential,handle_id}',''
              )::UUID
             LEFT JOIN vault_entries vault ON vault.id=credential.handle_id
             CROSS JOIN verification_campaign_safety_holds hold
            WHERE action.operation_id=$1 AND action.campaign_id=$2
              AND action.prepared_action_id=$3 AND action.state='started'
              AND auth.expires_at IS NOT NULL AND hold.singleton=TRUE"#,
    )
    .bind(operation_id)
    .bind(campaign_id)
    .bind(prepared_action_id)
    .bind(authorization_receipt_id)
    .bind(action_execution_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let budget_heads = sqlx::query_as::<_, PreparedActionBudgetHeadRow>(
        r#"WITH RECURSIVE lineage AS (
               SELECT contract.budget_contract_id,contract.parent_contract_id,
                      contract.scope_kind,0::INTEGER AS depth
                 FROM verification_budget_contracts contract
                WHERE contract.scope_kind='action' AND contract.scope_id=$1
                  AND contract.sealed_at IS NOT NULL
               UNION ALL
               SELECT parent.budget_contract_id,parent.parent_contract_id,
                      parent.scope_kind,child.depth+1
                 FROM lineage child
                 JOIN verification_budget_contracts parent
                   ON parent.budget_contract_id=child.parent_contract_id
                  AND parent.sealed_at IS NOT NULL
           )
           SELECT lineage.scope_kind,lineage.budget_contract_id,axis.axis_kind,
                  axis.axis_limit,head.consumed,head.reserved,head.unknown_held,
                  GREATEST(COALESCE(SUM(CASE
                      WHEN ledger.entry_kind='reserve' THEN ledger.delta
                      WHEN ledger.entry_kind='consume' THEN -ledger.delta
                      ELSE 0 END),0),0)::BIGINT AS reservation_remaining,
                  head.row_version
             FROM lineage
             JOIN verification_budget_contract_axes axis
               ON axis.budget_contract_id=lineage.budget_contract_id
             JOIN verification_budget_scope_heads head
               ON head.budget_contract_id=axis.budget_contract_id
              AND head.axis_kind=axis.axis_kind
             LEFT JOIN verification_budget_ledger_entries ledger
               ON ledger.budget_reservation_id=$2
              AND ledger.ancestor_contract_id=lineage.budget_contract_id
              AND ledger.axis_kind=axis.axis_kind
            GROUP BY lineage.depth,lineage.scope_kind,lineage.budget_contract_id,
                     axis.axis_kind,axis.axis_limit,axis.axis_ordinal,
                     head.consumed,head.reserved,head.unknown_held,head.row_version
            ORDER BY lineage.depth DESC,axis.axis_ordinal"#,
    )
    .bind(prepared_action_id)
    .bind(action.budget_reservation_id)
    .fetch_all(&mut *tx)
    .await?;
    if budget_heads.is_empty() {
        return Err(conflict(AUTHORITY_STALE));
    }
    tx.commit().await?;
    Ok(CurrentPreparedActionSendAuthority {
        action,
        budget_heads,
    })
}

/// Atomically moves usage from this exact reservation into consumed counters
/// at all four ancestor layers. No external work is performed in the
/// transaction and no unreserved increment can be created here.
pub async fn consume_prepared_action_budget_before_io(
    pool: &PgPool,
    action_execution_id: Uuid,
    expected_campaign_dispatch_generation: i64,
    expected_budget_fences: [std::collections::BTreeMap<String, i64>; 4],
    delta_by_axis: &std::collections::BTreeMap<String, i64>,
) -> Result<()> {
    const AXES: [&str; 6] = [
        "requests",
        "response_bytes",
        "wall_clock_ms",
        "retries",
        "browser_steps",
        "oast_tokens",
    ];
    if delta_by_axis.is_empty()
        || delta_by_axis
            .iter()
            .any(|(axis, delta)| !AXES.contains(&axis.as_str()) || *delta < 0)
        || delta_by_axis
            .get("requests")
            .is_none_or(|value| *value <= 0)
        || expected_budget_fences.iter().any(|head| {
            head.len() != AXES.len() || AXES.iter().any(|axis| !head.contains_key(*axis))
        })
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let execution: (Uuid, Uuid, i64) = sqlx::query_as(
        r#"SELECT execution.prepared_action_id,execution.budget_reservation_id,
                  execution.campaign_dispatch_generation
             FROM verification_action_executions execution
             JOIN verification_prepared_action_authorizations auth
               ON auth.authorization_receipt_id=execution.authorization_receipt_id
              AND auth.prepared_action_id=execution.prepared_action_id
              AND auth.decision='authorized'
              AND auth.expires_at>statement_timestamp()
             JOIN verification_prepared_actions action
               ON action.prepared_action_id=execution.prepared_action_id
              AND action.state='started'
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=action.campaign_id
              AND campaign.terminal_at IS NULL AND campaign.superseded_at IS NULL
              AND campaign.effective_valid_until>statement_timestamp()
             JOIN verification_budget_reservations reservation
               ON reservation.budget_reservation_id=execution.budget_reservation_id
              AND reservation.state='active'
            WHERE execution.action_execution_id=$1 AND execution.state='started'
              AND NOT EXISTS(
                  SELECT 1 FROM verification_authority_quarantine_events quarantine
                  JOIN verification_campaign_terminal_decisions terminal
                    ON terminal.campaign_terminal_decision_id=
                       quarantine.campaign_terminal_decision_id
                 WHERE terminal.campaign_id=campaign.campaign_id
              )
              AND NOT EXISTS(
                  SELECT 1
                    FROM tool_truth_authority_bundle_members bundle_member
                    LEFT JOIN LATERAL (
                        SELECT denominator.id
                          FROM coverage_denominators denominator
                         WHERE denominator.operation_id=action.operation_id
                           AND denominator.organization_id=action.organization_id
                           AND denominator.denominator_kind='root'
                           AND denominator.stage_kind=CASE bundle_member.root_family
                               WHEN 'ti' THEN 'target_intel'
                               WHEN 'eas' THEN 'external_attack_surface'
                               WHEN 'enum' THEN 'enumeration'
                               WHEN 'vuln' THEN 'vuln_triage'
                           END
                           AND denominator.sealed_at IS NOT NULL
                         ORDER BY denominator.created_at DESC,denominator.id DESC LIMIT 1
                    ) latest_root ON TRUE
                    LEFT JOIN tool_truth_authority_set_members authority_member
                      ON authority_member.authority_set_id=bundle_member.authority_set_seal_id
                    LEFT JOIN capability_execution_receipts receipt
                      ON receipt.id=authority_member.receipt_id
                   WHERE bundle_member.bundle_seal_id=
                         campaign.tool_truth_authority_bundle_seal_id
                     AND (
                         latest_root.id IS DISTINCT FROM bundle_member.root_denominator_id
                         OR receipt.id IS NULL
                         OR receipt.current_semantic_authority_version
                            IS DISTINCT FROM authority_member.semantic_authority_version
                         OR receipt.current_semantic_reconciliation_id
                            IS DISTINCT FROM authority_member.reconciliation_id
                         OR receipt.current_semantic_reconciliation_hash
                            IS DISTINCT FROM authority_member.semantic_hash
                         OR receipt.reconciliation_state<>'consistent'
                         OR receipt.valid_until IS NULL
                         OR receipt.valid_until<=statement_timestamp()
                     )
              )
              AND (
                  COALESCE(action.private_manifest->'credential','null'::JSONB)='null'::JSONB
                  OR EXISTS(
                      SELECT 1
                        FROM verification_credential_authority_heads credential
                        JOIN vault_entries vault ON vault.id=credential.handle_id
                       WHERE credential.operation_id=action.operation_id
                         AND credential.handle_id=(
                             action.private_manifest #>> '{credential,handle_id}'
                         )::UUID
                         AND credential.handle_version=(
                             action.private_manifest #>> '{credential,handle_version}'
                         )::BIGINT
                         AND credential.revocation_generation=(
                             action.private_manifest #>> '{credential,revocation_generation}'
                         )::BIGINT
                         AND credential.injection_origin=
                             action.private_manifest #>> '{credential,injection_origin}'
                         AND credential.injection_contract_version=
                             action.private_manifest #>> '{credential,injection_contract_version}'
                         AND credential.revoked=FALSE AND vault.status<>'invalid'
                         AND vault.updated_at<=credential.updated_at
                  )
              )
            FOR UPDATE OF execution"#,
    )
    .bind(action_execution_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let hold: (bool, i64, bool) = sqlx::query_as(
        "SELECT campaign_dispatch_held,campaign_dispatch_generation,operation_admission_held FROM verification_campaign_safety_holds WHERE singleton=TRUE FOR SHARE",
    )
    .fetch_one(&mut *tx)
    .await?;
    if hold.0
        || hold.2
        || hold.1 != expected_campaign_dispatch_generation
        || execution.2 != expected_campaign_dispatch_generation
    {
        return Err(conflict(AUTHORITY_STALE));
    }
    let contracts: Vec<(Uuid, String, i32)> = sqlx::query_as(
        r#"WITH RECURSIVE lineage AS (
               SELECT budget_contract_id,parent_contract_id,scope_kind,0::INTEGER depth
                 FROM verification_budget_contracts
                WHERE scope_kind='action' AND scope_id=$1 AND sealed_at IS NOT NULL
               UNION ALL
               SELECT parent.budget_contract_id,parent.parent_contract_id,
                      parent.scope_kind,child.depth+1
                 FROM lineage child JOIN verification_budget_contracts parent
                   ON parent.budget_contract_id=child.parent_contract_id
                  AND parent.sealed_at IS NOT NULL
           ) SELECT budget_contract_id,scope_kind,depth FROM lineage ORDER BY depth DESC"#,
    )
    .bind(execution.0)
    .fetch_all(&mut *tx)
    .await?;
    if contracts.len() != 4 {
        return Err(conflict(AUTHORITY_STALE));
    }
    for (contract_index, (contract_id, _scope_kind, _depth)) in contracts.iter().enumerate() {
        for (axis_kind, delta) in delta_by_axis {
            if *delta == 0 {
                continue;
            }
            let head: (i64, i64, i64, i64, i64) = sqlx::query_as(
                r#"SELECT head.consumed,head.reserved,head.unknown_held,
                          head.row_version,axis.axis_limit
                     FROM verification_budget_scope_heads head
                     JOIN verification_budget_contract_axes axis
                       ON axis.budget_contract_id=head.budget_contract_id
                      AND axis.axis_kind=head.axis_kind
                    WHERE head.budget_contract_id=$1 AND head.axis_kind=$2 FOR UPDATE"#,
            )
            .bind(contract_id)
            .bind(axis_kind)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| conflict(AUTHORITY_STALE))?;
            if expected_budget_fences[contract_index].get(axis_kind) != Some(&head.3) {
                return Err(conflict(AUTHORITY_STALE));
            }
            let remaining: i64 = sqlx::query_scalar(
                r#"SELECT COALESCE(SUM(CASE WHEN entry_kind='reserve' THEN delta
                                             WHEN entry_kind='consume' THEN -delta
                                             ELSE 0 END),0)::BIGINT
                     FROM verification_budget_ledger_entries
                    WHERE budget_reservation_id=$1 AND ancestor_contract_id=$2
                      AND axis_kind=$3"#,
            )
            .bind(execution.1)
            .bind(contract_id)
            .bind(axis_kind)
            .fetch_one(&mut *tx)
            .await?;
            if remaining < *delta || head.1 < *delta || head.0 + head.1 + head.2 > head.4 {
                return Err(conflict("VERIFICATION_BUDGET_EXHAUSTED"));
            }
            let resulting_consumed = head.0 + delta;
            let resulting_reserved = head.1 - delta;
            let ordinal: i64 = sqlx::query_scalar(
                r#"SELECT COALESCE(MAX(entry_ordinal),0)+1
                     FROM verification_budget_ledger_entries
                    WHERE budget_reservation_id=$1 AND ancestor_contract_id=$2
                      AND axis_kind=$3"#,
            )
            .bind(execution.1)
            .bind(contract_id)
            .bind(axis_kind)
            .fetch_one(&mut *tx)
            .await?;
            let resulting_hash = json_hash_on(
                &mut tx,
                &serde_json::json!({
                    "contract_id": contract_id,
                    "axis_kind": axis_kind,
                    "consumed": resulting_consumed,
                    "reserved": resulting_reserved,
                    "unknown_held": head.2,
                    "row_version": head.3 + 1,
                }),
            )
            .await?;
            let entry_id = Uuid::new_v5(
                &action_execution_id,
                format!("{contract_id}:{axis_kind}:consume:{ordinal}").as_bytes(),
            );
            sqlx::query(
                r#"INSERT INTO verification_budget_ledger_entries(
                       budget_ledger_entry_id,budget_reservation_id,ancestor_contract_id,
                       axis_kind,entry_ordinal,entry_kind,delta,resulting_consumed,
                       resulting_reserved,resulting_unknown_held,expected_head_row_version,
                       resulting_head_hash,fence
                   ) VALUES($1,$2,$3,$4,$5,'consume',$6,$7,$8,$9,$10,$11,$12)"#,
            )
            .bind(entry_id)
            .bind(execution.1)
            .bind(contract_id)
            .bind(axis_kind)
            .bind(ordinal)
            .bind(delta)
            .bind(resulting_consumed)
            .bind(resulting_reserved)
            .bind(head.2)
            .bind(head.3)
            .bind(resulting_hash)
            .bind(head.3 + 1)
            .execute(&mut *tx)
            .await?;
            let affected = sqlx::query(
                r#"UPDATE verification_budget_scope_heads
                      SET consumed=$1,reserved=$2,row_version=row_version+1,
                          updated_at=statement_timestamp()
                    WHERE budget_contract_id=$3 AND axis_kind=$4 AND row_version=$5"#,
            )
            .bind(resulting_consumed)
            .bind(resulting_reserved)
            .bind(contract_id)
            .bind(axis_kind)
            .bind(head.3)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if affected != 1 {
                return Err(conflict(AUTHORITY_STALE));
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DecidePreparedActionAuthorization {
    pub stable_request_id: Uuid,
    pub prepared_action_id: Uuid,
    pub campaign_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub decision: String,
    pub decision_reason_code: String,
    pub expected_action_row_version: i64,
    pub campaign_dispatch_generation: i64,
    pub renderer_version: String,
    pub reviewed_action_hash: String,
    pub expected_display_projection_hash: String,
    pub expected_private_manifest_hash: String,
    pub operator_channel: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedActionAuthorizationDecision {
    pub authorization_receipt_id: Uuid,
    pub prepared_action_id: Uuid,
    pub decision: String,
    pub decided_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub residual_id: Option<Uuid>,
    pub current_action_row_version: i64,
    pub replayed: bool,
}

#[derive(sqlx::FromRow)]
struct ExistingPreparedActionAuthorization {
    authorization_receipt_id: Uuid,
    prepared_action_id: Uuid,
    campaign_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    decision: String,
    decision_reason_code: String,
    expected_action_row_version: i64,
    campaign_dispatch_generation: i64,
    renderer_version: String,
    reviewed_action_hash: String,
    expected_display_projection_hash: String,
    expected_private_manifest_hash: String,
    operator_channel: String,
    expires_at: Option<DateTime<Utc>>,
    residual_id: Option<Uuid>,
    decided_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct LockedPreparedActionAuthorization {
    hypothesis_revision_id: Uuid,
    risk_tier: String,
    renderer_version: String,
    display_projection_hash: String,
    private_manifest_hash: String,
    review_expires_at: DateTime<Utc>,
    state: String,
    row_version: i64,
    observed_at: DateTime<Utc>,
}

pub async fn decide_prepared_action_authorization(
    pool: &PgPool,
    command: &DecidePreparedActionAuthorization,
) -> Result<PreparedActionAuthorizationDecision> {
    if !matches!(command.decision.as_str(), "authorized" | "denied")
        || command.decision_reason_code.trim().is_empty()
        || (command.decision == "authorized") != command.expires_at.is_some()
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    for hash in [
        &command.reviewed_action_hash,
        &command.expected_display_projection_hash,
        &command.expected_private_manifest_hash,
    ] {
        require_sha256(hash)?;
    }
    let mut tx = pool.begin().await?;
    let residual_id = (command.decision != "authorized").then(|| {
        Uuid::new_v5(
            &command.stable_request_id,
            b"verification-prepared-action-decision-residual.v1",
        )
    });
    let existing = sqlx::query_as::<_, ExistingPreparedActionAuthorization>(
        r#"SELECT authorization_receipt_id,prepared_action_id,decision,
                  campaign_id,operation_id,project_scope_id,organization_id,
                  decision_reason_code,expected_action_row_version,
                  campaign_dispatch_generation,renderer_version,reviewed_action_hash,
                  expected_display_projection_hash,expected_private_manifest_hash,
                  operator_channel,expires_at,residual_id,decided_at
             FROM verification_prepared_action_authorizations
            WHERE stable_request_id=$1 FOR SHARE"#,
    )
    .bind(command.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = existing {
        if row.prepared_action_id == command.prepared_action_id
            && row.campaign_id == command.campaign_id
            && row.operation_id == command.operation_id
            && row.project_scope_id == command.project_scope_id
            && row.organization_id == command.organization_id
            && row.decision == command.decision
            && row.decision_reason_code == command.decision_reason_code
            && row.expected_action_row_version == command.expected_action_row_version
            && row.campaign_dispatch_generation == command.campaign_dispatch_generation
            && row.renderer_version == command.renderer_version
            && row.reviewed_action_hash == command.reviewed_action_hash
            && row.expected_display_projection_hash == command.expected_display_projection_hash
            && row.expected_private_manifest_hash == command.expected_private_manifest_hash
            && row.operator_channel == command.operator_channel
            && row.expires_at == command.expires_at
            && row.residual_id == residual_id
        {
            let current_action_row_version: i64 = sqlx::query_scalar(
                "SELECT row_version FROM verification_prepared_actions WHERE prepared_action_id=$1",
            )
            .bind(command.prepared_action_id)
            .fetch_one(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(PreparedActionAuthorizationDecision {
                authorization_receipt_id: row.authorization_receipt_id,
                prepared_action_id: row.prepared_action_id,
                decision: row.decision,
                decided_at: row.decided_at,
                expires_at: row.expires_at,
                residual_id: row.residual_id,
                current_action_row_version,
                replayed: true,
            });
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    let locked_action = sqlx::query_as::<_, LockedPreparedActionAuthorization>(
        r#"SELECT campaign.hypothesis_revision_id,action.risk_tier,
                  action.renderer_version,action.display_projection_hash,
                  action.private_manifest_hash,action.review_expires_at,
                  action.state,action.row_version,statement_timestamp() AS observed_at
             FROM verification_prepared_actions action
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=action.campaign_id
              AND campaign.operation_id=action.operation_id
              AND campaign.project_scope_id=action.project_scope_id
              AND campaign.organization_id=action.organization_id
            WHERE action.prepared_action_id=$1 AND action.campaign_id=$2
              AND action.operation_id=$3 AND action.project_scope_id=$4
              AND action.organization_id=$5
            FOR UPDATE OF action"#,
    )
    .bind(command.prepared_action_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    if !matches!(locked_action.risk_tier.as_str(), "T2" | "T3")
        || locked_action.renderer_version != command.renderer_version
        || locked_action.display_projection_hash != command.expected_display_projection_hash
        || locked_action.display_projection_hash != command.reviewed_action_hash
        || locked_action.private_manifest_hash != command.expected_private_manifest_hash
        || locked_action.review_expires_at <= locked_action.observed_at
        || locked_action.state != "pending_authorization"
        || locked_action.row_version != command.expected_action_row_version
        || command.expires_at.is_some_and(|expires_at| {
            expires_at <= locked_action.observed_at || expires_at > locked_action.review_expires_at
        })
    {
        return Err(conflict(AUTHORITY_STALE));
    }
    if let Some(residual_id) = residual_id {
        let residual_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "prepared_action_id": command.prepared_action_id,
                "decision": command.decision,
                "reason_code": command.decision_reason_code,
                "expected_action_row_version": command.expected_action_row_version,
            }),
        )
        .await?;
        let inserted = sqlx::query(
            r#"INSERT INTO hypothesis_residual_risks(
                   residual_id,operation_id,organization_id,revision_id,reason_code,
                   owner_kind,affected_inputs,next_action,residual_hash
               ) VALUES($1,$2,$3,$4,$5,'plan_c',$6,$7,$8)
               ON CONFLICT(residual_id) DO NOTHING"#,
        )
        .bind(residual_id)
        .bind(command.operation_id)
        .bind(command.organization_id)
        .bind(locked_action.hypothesis_revision_id)
        .bind(&command.decision_reason_code)
        .bind(serde_json::json!([command.prepared_action_id]))
        .bind(serde_json::json!({
            "route": "verification_campaign",
            "decision": command.decision,
            "retry": false,
        }))
        .bind(&residual_hash)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let persisted: (String, String) = sqlx::query_as(
                "SELECT reason_code,residual_hash FROM hypothesis_residual_risks WHERE residual_id=$1",
            )
            .bind(residual_id)
            .fetch_one(&mut *tx)
            .await?;
            if persisted != (command.decision_reason_code.clone(), residual_hash) {
                return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
            }
        }
    }
    let decided_by: Uuid = sqlx::query_scalar(
        "SELECT id FROM operator_principals WHERE principal_kind='local_operator' AND active FOR SHARE",
    )
    .fetch_one(&mut *tx)
    .await?;
    let authorization_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "prepared_action_id": command.prepared_action_id,
            "decision": command.decision,
            "expected_action_row_version": command.expected_action_row_version,
            "campaign_dispatch_generation": command.campaign_dispatch_generation,
            "renderer_version": command.renderer_version,
            "reviewed_action_hash": command.reviewed_action_hash,
            "expected_display_projection_hash": command.expected_display_projection_hash,
            "expected_private_manifest_hash": command.expected_private_manifest_hash,
            "decided_by": decided_by,
            "operator_channel": command.operator_channel,
            "expires_at": command.expires_at,
            "residual_id": residual_id,
        }),
    )
    .await?;
    let authorization_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-action-authorization.v1",
    );
    sqlx::query(
        r#"INSERT INTO verification_prepared_action_authorizations(
               authorization_receipt_id,stable_request_id,prepared_action_id,campaign_id,
               operation_id,project_scope_id,organization_id,decision,decision_reason_code,
               expected_action_row_version,campaign_dispatch_generation,renderer_version,
               reviewed_action_hash,expected_display_projection_hash,
               expected_private_manifest_hash,authorization_hash,decided_by,actor_kind,
               operator_channel,expires_at,residual_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                    'local_operator',$18,$19,$20)"#,
    )
    .bind(authorization_id)
    .bind(command.stable_request_id)
    .bind(command.prepared_action_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(&command.decision)
    .bind(&command.decision_reason_code)
    .bind(command.expected_action_row_version)
    .bind(command.campaign_dispatch_generation)
    .bind(&command.renderer_version)
    .bind(&command.reviewed_action_hash)
    .bind(&command.expected_display_projection_hash)
    .bind(&command.expected_private_manifest_hash)
    .bind(&authorization_hash)
    .bind(decided_by)
    .bind(&command.operator_channel)
    .bind(command.expires_at)
    .bind(residual_id)
    .execute(&mut *tx)
    .await?;
    let affected = sqlx::query(
        r#"UPDATE verification_prepared_actions
              SET state=$1,reason_code=CASE WHEN $1='authorized' THEN NULL ELSE $2 END,
                  residual_id=CASE WHEN $1='authorized' THEN NULL ELSE $3 END,
                  row_version=row_version+1,
                  terminal_at=CASE WHEN $1='authorized' THEN NULL ELSE statement_timestamp() END
            WHERE prepared_action_id=$4 AND row_version=$5 AND state='pending_authorization'"#,
    )
    .bind(&command.decision)
    .bind(&command.decision_reason_code)
    .bind(residual_id)
    .bind(command.prepared_action_id)
    .bind(command.expected_action_row_version)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if affected != 1 {
        return Err(conflict(AUTHORITY_STALE));
    }
    let decided_at: DateTime<Utc> = sqlx::query_scalar(
        "SELECT decided_at FROM verification_prepared_action_authorizations WHERE authorization_receipt_id=$1",
    )
    .bind(authorization_id)
    .fetch_one(&mut *tx)
    .await?;
    let current_action_row_version = command.expected_action_row_version + 1;
    tx.commit().await?;
    Ok(PreparedActionAuthorizationDecision {
        authorization_receipt_id: authorization_id,
        prepared_action_id: command.prepared_action_id,
        decision: command.decision.clone(),
        decided_at,
        expires_at: command.expires_at,
        residual_id,
        current_action_row_version,
        replayed: false,
    })
}

#[derive(Debug, Clone)]
pub struct ReconcilePreparedActionSchedulerAuthority {
    pub prepared_action_id: Uuid,
    pub campaign_id: Uuid,
    pub operation_id: Uuid,
    pub expected_action_row_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparedActionSchedulerAuthorityDisposition {
    Unchanged,
    AuthorizedByServerPolicy,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedActionSchedulerAuthorityReceipt {
    pub prepared_action_id: Uuid,
    pub disposition: PreparedActionSchedulerAuthorityDisposition,
    pub authorization_receipt_id: Option<Uuid>,
    pub current_action_row_version: i64,
}

#[derive(sqlx::FromRow)]
struct LockedPreparedActionSchedulerAuthority {
    hypothesis_revision_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    capability_assessment_id: Uuid,
    risk_tier: String,
    state: String,
    row_version: i64,
    renderer_version: String,
    display_projection_hash: String,
    private_manifest_hash: String,
    review_expires_at: DateTime<Utc>,
    authorization_expires_at: Option<DateTime<Utc>>,
    campaign_dispatch_held: bool,
    campaign_dispatch_generation: i64,
    observed_at: DateTime<Utc>,
}

/// Resolve only the two server-owned scheduler transitions that must not wait
/// for a human review surface:
///
/// * T0/T1 actions receive a hash-bound, generation-bound policy receipt while
///   the Campaign dispatch lane is open.
/// * an elapsed review/JIT TTL becomes an append-only `expired` receipt plus a
///   typed residual instead of remaining an immortal active-lane row.
///
/// T2/T3 actions with a live review packet are deliberately left unchanged and
/// can only be approved or denied through the local-operator CAS surface.
pub async fn reconcile_prepared_action_scheduler_authority(
    pool: &PgPool,
    command: &ReconcilePreparedActionSchedulerAuthority,
) -> Result<PreparedActionSchedulerAuthorityReceipt> {
    if command.expected_action_row_version < 0 {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let action = sqlx::query_as::<_, LockedPreparedActionSchedulerAuthority>(
        r#"SELECT campaign.hypothesis_revision_id,action.project_scope_id,
                  action.organization_id,action.capability_assessment_id,
                  action.risk_tier,action.state,action.row_version,
                  action.renderer_version,action.display_projection_hash,
                  action.private_manifest_hash,action.review_expires_at,
                  latest_auth.expires_at AS authorization_expires_at,
                  dispatch_hold.campaign_dispatch_held,
                  dispatch_hold.campaign_dispatch_generation,
                  statement_timestamp() AS observed_at
             FROM verification_prepared_actions action
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=action.campaign_id
              AND campaign.operation_id=action.operation_id
              AND campaign.project_scope_id=action.project_scope_id
              AND campaign.organization_id=action.organization_id
             CROSS JOIN verification_campaign_safety_holds dispatch_hold
             LEFT JOIN LATERAL (
                 SELECT receipt.expires_at
                   FROM verification_prepared_action_authorizations receipt
                  WHERE receipt.prepared_action_id=action.prepared_action_id
                    AND receipt.decision='authorized'
                  ORDER BY receipt.decided_at DESC,
                           receipt.authorization_receipt_id DESC
                  LIMIT 1
             ) latest_auth ON TRUE
            WHERE action.prepared_action_id=$1 AND action.campaign_id=$2
              AND action.operation_id=$3 AND dispatch_hold.singleton=TRUE
            FOR UPDATE OF action"#,
    )
    .bind(command.prepared_action_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    if action.row_version != command.expected_action_row_version {
        let replay_row_version = command
            .expected_action_row_version
            .checked_add(1)
            .ok_or_else(|| conflict(CONTRACT_INVALID))?;
        if action.row_version == replay_row_version {
            let receipts: Vec<(Uuid, String)> = sqlx::query_as(
                r#"SELECT authorization_receipt_id,decision
                     FROM verification_prepared_action_authorizations
                    WHERE prepared_action_id=$1 AND campaign_id=$2 AND operation_id=$3
                      AND expected_action_row_version=$4 AND actor_kind='server_policy'
                    ORDER BY decided_at,authorization_receipt_id
                    LIMIT 2"#,
            )
            .bind(command.prepared_action_id)
            .bind(command.campaign_id)
            .bind(command.operation_id)
            .bind(command.expected_action_row_version)
            .fetch_all(&mut *tx)
            .await?;
            if let [(authorization_receipt_id, decision)] = receipts.as_slice() {
                let disposition = match (action.state.as_str(), decision.as_str()) {
                    ("authorized", "authorized") => {
                        Some(PreparedActionSchedulerAuthorityDisposition::AuthorizedByServerPolicy)
                    }
                    ("expired", "expired") => {
                        Some(PreparedActionSchedulerAuthorityDisposition::Expired)
                    }
                    _ => None,
                };
                if let Some(disposition) = disposition {
                    let authorization_receipt_id = *authorization_receipt_id;
                    tx.commit().await?;
                    return Ok(PreparedActionSchedulerAuthorityReceipt {
                        prepared_action_id: command.prepared_action_id,
                        disposition,
                        authorization_receipt_id: Some(authorization_receipt_id),
                        current_action_row_version: replay_row_version,
                    });
                }
            }
        }
        return Err(conflict(AUTHORITY_STALE));
    }

    let expiry_reason = match action.state.as_str() {
        "pending_authorization" if action.review_expires_at <= action.observed_at => {
            Some("server_policy_review_expired")
        }
        "authorized"
            if action
                .authorization_expires_at
                .is_some_and(|expires_at| expires_at <= action.observed_at) =>
        {
            Some("server_policy_authorization_expired")
        }
        _ => None,
    };
    let server_policy_authorization = action.state == "pending_authorization"
        && matches!(action.risk_tier.as_str(), "T0" | "T1")
        && action.review_expires_at > action.observed_at
        && !action.campaign_dispatch_held;
    if expiry_reason.is_none() && !server_policy_authorization {
        tx.commit().await?;
        return Ok(PreparedActionSchedulerAuthorityReceipt {
            prepared_action_id: command.prepared_action_id,
            disposition: PreparedActionSchedulerAuthorityDisposition::Unchanged,
            authorization_receipt_id: None,
            current_action_row_version: action.row_version,
        });
    }

    let decision = if server_policy_authorization {
        "authorized"
    } else {
        "expired"
    };
    let reason_code = if server_policy_authorization {
        "server_policy_auto_authorized_t0_t1"
    } else {
        expiry_reason.expect("expiry transition has a reason")
    };
    let stable_request_id = Uuid::new_v5(
        &command.prepared_action_id,
        format!(
            "verification-scheduler-authority.v1:{}:{}:{}",
            decision, action.row_version, action.campaign_dispatch_generation
        )
        .as_bytes(),
    );
    let residual_id = (decision == "expired").then(|| {
        Uuid::new_v5(
            &stable_request_id,
            b"verification-prepared-action-decision-residual.v1",
        )
    });
    if let Some(residual_id) = residual_id {
        let residual_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "prepared_action_id": command.prepared_action_id,
                "decision": decision,
                "reason_code": reason_code,
                "expected_action_row_version": action.row_version,
            }),
        )
        .await?;
        let inserted = sqlx::query(
            r#"INSERT INTO hypothesis_residual_risks(
                   residual_id,operation_id,organization_id,revision_id,reason_code,
                   owner_kind,affected_inputs,next_action,residual_hash
               ) VALUES($1,$2,$3,$4,$5,'plan_c',$6,$7,$8)
               ON CONFLICT(residual_id) DO NOTHING"#,
        )
        .bind(residual_id)
        .bind(command.operation_id)
        .bind(action.organization_id)
        .bind(action.hypothesis_revision_id)
        .bind(reason_code)
        .bind(serde_json::json!([command.prepared_action_id]))
        .bind(serde_json::json!({
            "route": "verification_campaign",
            "decision": decision,
            "retry": true,
        }))
        .bind(&residual_hash)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() == 0 {
            let persisted: (String, String) = sqlx::query_as(
                "SELECT reason_code,residual_hash FROM hypothesis_residual_risks WHERE residual_id=$1",
            )
            .bind(residual_id)
            .fetch_one(&mut *tx)
            .await?;
            if persisted != (reason_code.to_owned(), residual_hash) {
                return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
            }
        }
    }
    let expires_at = server_policy_authorization.then_some(action.review_expires_at);
    let authorization_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "prepared_action_id": command.prepared_action_id,
            "decision": decision,
            "expected_action_row_version": action.row_version,
            "campaign_dispatch_generation": action.campaign_dispatch_generation,
            "renderer_version": action.renderer_version,
            "reviewed_action_hash": action.display_projection_hash,
            "expected_display_projection_hash": action.display_projection_hash,
            "expected_private_manifest_hash": action.private_manifest_hash,
            "decided_by": serde_json::Value::Null,
            "actor_kind": "server_policy",
            "operator_channel": "server_policy",
            "expires_at": expires_at,
            "residual_id": residual_id,
            "capability_assessment_id": action.capability_assessment_id,
        }),
    )
    .await?;
    let authorization_receipt_id =
        Uuid::new_v5(&stable_request_id, b"verification-action-authorization.v1");
    sqlx::query(
        r#"INSERT INTO verification_prepared_action_authorizations(
               authorization_receipt_id,stable_request_id,prepared_action_id,campaign_id,
               operation_id,project_scope_id,organization_id,decision,decision_reason_code,
               expected_action_row_version,campaign_dispatch_generation,renderer_version,
               reviewed_action_hash,expected_display_projection_hash,
               expected_private_manifest_hash,authorization_hash,decided_by,actor_kind,
               operator_channel,expires_at,residual_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                    NULL,'server_policy','server_policy',$17,$18)"#,
    )
    .bind(authorization_receipt_id)
    .bind(stable_request_id)
    .bind(command.prepared_action_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(action.project_scope_id)
    .bind(action.organization_id)
    .bind(decision)
    .bind(reason_code)
    .bind(action.row_version)
    .bind(action.campaign_dispatch_generation)
    .bind(&action.renderer_version)
    .bind(&action.display_projection_hash)
    .bind(&action.display_projection_hash)
    .bind(&action.private_manifest_hash)
    .bind(&authorization_hash)
    .bind(expires_at)
    .bind(residual_id)
    .execute(&mut *tx)
    .await?;
    let next_version = action
        .row_version
        .checked_add(1)
        .ok_or_else(|| conflict(CONTRACT_INVALID))?;
    let affected = sqlx::query(
        r#"UPDATE verification_prepared_actions
              SET state=$1,reason_code=CASE WHEN $1='authorized' THEN NULL ELSE $2 END,
                  residual_id=CASE WHEN $1='authorized' THEN NULL ELSE $3 END,
                  row_version=$4,
                  terminal_at=CASE WHEN $1='authorized' THEN NULL ELSE statement_timestamp() END
            WHERE prepared_action_id=$5 AND campaign_id=$6 AND operation_id=$7
              AND row_version=$8 AND state=$9"#,
    )
    .bind(decision)
    .bind(reason_code)
    .bind(residual_id)
    .bind(next_version)
    .bind(command.prepared_action_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(action.row_version)
    .bind(&action.state)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if affected != 1 {
        return Err(conflict(AUTHORITY_STALE));
    }
    tx.commit().await?;
    Ok(PreparedActionSchedulerAuthorityReceipt {
        prepared_action_id: command.prepared_action_id,
        disposition: if server_policy_authorization {
            PreparedActionSchedulerAuthorityDisposition::AuthorizedByServerPolicy
        } else {
            PreparedActionSchedulerAuthorityDisposition::Expired
        },
        authorization_receipt_id: Some(authorization_receipt_id),
        current_action_row_version: next_version,
    })
}

#[derive(Debug, Clone)]
pub struct BudgetContractAxis {
    pub axis_kind: String,
    pub axis_limit: i64,
}

#[derive(Debug, Clone)]
pub struct SealBudgetContract {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub scope_kind: String,
    pub scope_id: Uuid,
    pub parent_contract_id: Option<Uuid>,
    pub contract_version: String,
    pub axes: Vec<BudgetContractAxis>,
}

/// Persists one immutable budget layer and initializes its mutable CAS heads.
/// The DB seal guard validates exact axes, ownership and the
/// operation->wave->campaign->action parent chain before authority can be used.
pub async fn seal_budget_contract(pool: &PgPool, command: &SealBudgetContract) -> Result<Uuid> {
    const AXIS_ORDER: [&str; 6] = [
        "requests",
        "response_bytes",
        "wall_clock_ms",
        "retries",
        "browser_steps",
        "oast_tokens",
    ];
    if command.stable_request_id.is_nil()
        || command.scope_id.is_nil()
        || command.contract_version.trim().is_empty()
        || command.axes.is_empty()
        || !matches!(
            command.scope_kind.as_str(),
            "operation" | "wave" | "campaign" | "action"
        )
        || (command.scope_kind == "operation") != command.parent_contract_id.is_none()
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut axes = command.axes.clone();
    axes.sort_by_key(|axis| {
        AXIS_ORDER
            .iter()
            .position(|candidate| *candidate == axis.axis_kind.as_str())
            .unwrap_or(AXIS_ORDER.len())
    });
    if axes
        .iter()
        .any(|axis| axis.axis_limit < 0 || !AXIS_ORDER.contains(&axis.axis_kind.as_str()))
        || axes
            .windows(2)
            .any(|pair| pair[0].axis_kind == pair[1].axis_kind)
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    let mut member_hashes = Vec::with_capacity(axes.len());
    for (ordinal, axis) in axes.iter().enumerate() {
        member_hashes.push(
            json_hash_on(
                &mut tx,
                &serde_json::json!({
                    "axis_ordinal": ordinal,
                    "axis_kind": axis.axis_kind,
                    "axis_limit": axis.axis_limit,
                }),
            )
            .await?,
        );
    }
    let member_set_hash =
        exact_set_hash_on(&mut tx, "verification_budget_contract.v1", &member_hashes).await?;
    let contract_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "operation_id": command.operation_id,
            "project_scope_id": command.project_scope_id,
            "organization_id": command.organization_id,
            "scope_kind": command.scope_kind,
            "scope_id": command.scope_id,
            "parent_contract_id": command.parent_contract_id,
            "contract_version": command.contract_version,
            "member_set_hash": member_set_hash,
        }),
    )
    .await?;
    let existing: Option<(Uuid, String, Option<DateTime<Utc>>)> = sqlx::query_as(
        r#"SELECT budget_contract_id,contract_hash,sealed_at
              FROM verification_budget_contracts WHERE stable_request_id=$1 FOR SHARE"#,
    )
    .bind(command.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((id, existing_hash, sealed_at)) = existing {
        if existing_hash == contract_hash && sealed_at.is_some() {
            tx.commit().await?;
            return Ok(id);
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    let contract_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-budget-contract.v1",
    );
    sqlx::query(
        r#"INSERT INTO verification_budget_contracts(
               budget_contract_id,stable_request_id,operation_id,project_scope_id,
               organization_id,scope_kind,scope_id,parent_contract_id,contract_version,
               contract_hash,member_count,member_set_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(contract_id)
    .bind(command.stable_request_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(&command.scope_kind)
    .bind(command.scope_id)
    .bind(command.parent_contract_id)
    .bind(&command.contract_version)
    .bind(&contract_hash)
    .bind(axes.len() as i64)
    .bind(&member_set_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (axis, member_hash)) in axes.iter().zip(member_hashes.iter()).enumerate() {
        sqlx::query(
            r#"INSERT INTO verification_budget_contract_axes(
                   budget_contract_id,axis_kind,axis_ordinal,axis_limit,member_hash
               ) VALUES($1,$2,$3,$4,$5)"#,
        )
        .bind(contract_id)
        .bind(&axis.axis_kind)
        .bind(ordinal as i32)
        .bind(axis.axis_limit)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO verification_budget_scope_heads(budget_contract_id,axis_kind)
               VALUES($1,$2)"#,
        )
        .bind(contract_id)
        .bind(&axis.axis_kind)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE verification_budget_contracts SET sealed_at=statement_timestamp() WHERE budget_contract_id=$1 AND sealed_at IS NULL",
    )
    .bind(contract_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(contract_id)
}

#[derive(Debug, Clone)]
pub struct BeginAuthorizedAction {
    pub stable_request_id: Uuid,
    pub prepared_action_id: Uuid,
    pub authorization_receipt_id: Uuid,
    pub conflict_set_id: Uuid,
    pub operation_id: Uuid,
    pub project_scope_id: Uuid,
    pub organization_id: Uuid,
    pub execution_ordinal: i32,
    pub execution_kind: String,
    pub campaign_dispatch_generation: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableActionBegin {
    pub action_execution_id: Uuid,
    pub budget_reservation_id: Uuid,
    pub durable_begin_hash: String,
}

#[derive(Debug, Clone, Copy)]
pub struct BeginAuthorizedActionFromAuthority {
    pub stable_consumer_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub authorization_receipt_id: Uuid,
    pub expected_action_row_version: i64,
    pub expected_campaign_dispatch_generation: i64,
}

/// Public short compound: selects all action execution material under the
/// same all-fresh transaction that reserves budgets and conflict keys.
pub async fn begin_authorized_action_with_fresh_tool_truth(
    pool: &PgPool,
    request: BeginAuthorizedActionFromAuthority,
) -> Result<DurableActionBegin> {
    let organization_id: Uuid = sqlx::query_scalar(
        r#"SELECT organization_id FROM verification_prepared_actions
            WHERE prepared_action_id=$1 AND operation_id=$2 AND campaign_id=$3"#,
    )
    .bind(request.prepared_action_id)
    .bind(request.operation_id)
    .bind(request.campaign_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let authority_request = CheckToolTruthAuthorityBundle {
        stable_consumer_request_id: request.stable_consumer_request_id,
        operation_id: request.operation_id,
        organization_id,
        consumer_kind: ToolTruthAuthorityBundleConsumerV1::VerificationCampaign,
    };
    with_all_fresh_tool_truth_authority_bundle(pool, &authority_request, move |tx, authority| {
        Box::pin(async move {
            #[derive(sqlx::FromRow)]
            struct BeginAuthority {
                project_scope_id: Uuid,
                organization_id: Uuid,
                conflict_set_id: Uuid,
                execution_kind: String,
                execution_ordinal: i32,
                campaign_dispatch_generation: i64,
            }
            let selected = sqlx::query_as::<_, BeginAuthority>(
                r#"SELECT action.project_scope_id,action.organization_id,
                          conflict_set.conflict_set_id,
                          action.action_contract_kind AS execution_kind,
                          COALESCE(MAX(execution.execution_ordinal),0)::INT+1 AS execution_ordinal,
                          auth.campaign_dispatch_generation
                     FROM verification_prepared_actions action
                     JOIN verification_prepared_action_authorizations auth
                       ON auth.authorization_receipt_id=$4
                      AND auth.prepared_action_id=action.prepared_action_id
                      AND auth.decision='authorized'
                      AND auth.expires_at>statement_timestamp()
                     JOIN verification_action_conflict_sets conflict_set
                       ON conflict_set.prepared_action_id=action.prepared_action_id
                      AND conflict_set.sealed_at IS NOT NULL
                     LEFT JOIN verification_action_executions execution
                       ON execution.prepared_action_id=action.prepared_action_id
                      AND execution.authorization_receipt_id=auth.authorization_receipt_id
                    WHERE action.prepared_action_id=$1 AND action.operation_id=$2
                      AND action.campaign_id=$3 AND action.state='authorized'
                      AND action.row_version=$5
                    GROUP BY action.project_scope_id,action.organization_id,
                             conflict_set.conflict_set_id,action.action_contract_kind,
                             auth.campaign_dispatch_generation"#,
            )
            .bind(request.prepared_action_id)
            .bind(request.operation_id)
            .bind(request.campaign_id)
            .bind(request.authorization_receipt_id)
            .bind(request.expected_action_row_version)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| conflict(AUTHORITY_STALE))?;
            if selected.campaign_dispatch_generation
                != request.expected_campaign_dispatch_generation
            {
                return Err(conflict(AUTHORITY_STALE));
            }
            begin_authorized_action(
                tx,
                authority,
                &BeginAuthorizedAction {
                    stable_request_id: Uuid::new_v5(
                        &request.stable_consumer_request_id,
                        b"verification-authorized-action-begin.v1",
                    ),
                    prepared_action_id: request.prepared_action_id,
                    authorization_receipt_id: request.authorization_receipt_id,
                    conflict_set_id: selected.conflict_set_id,
                    operation_id: request.operation_id,
                    project_scope_id: selected.project_scope_id,
                    organization_id: selected.organization_id,
                    execution_ordinal: selected.execution_ordinal,
                    execution_kind: selected.execution_kind,
                    campaign_dispatch_generation: selected.campaign_dispatch_generation,
                },
            )
            .await
        })
    })
    .await
}

/// Called only inside Plan A's all-fresh authority callback.  The guard cannot
/// be constructed by callers, so budget reserve, all-key fences and durable
/// begin cannot be reached with a stale boolean/hash surrogate.
pub async fn begin_authorized_action(
    tx: &mut Transaction<'_, Postgres>,
    authority: &AllFreshToolTruthAuthorityBundle<'_>,
    command: &BeginAuthorizedAction,
) -> Result<DurableActionBegin> {
    if authority.checked().operation_id() != command.operation_id
        || authority.checked().organization_id() != command.organization_id
        || command.execution_ordinal <= 0
        || !matches!(
            command.execution_kind.as_str(),
            "single_action_v1" | "concurrent_action_group_v1"
        )
    {
        return Err(conflict(AUTHORITY_STALE));
    }
    let axes: Vec<(Uuid, String, i64, i64, i32, String, String)> = sqlx::query_as(
        r#"WITH RECURSIVE lineage AS (
               SELECT contract.budget_contract_id,contract.parent_contract_id,
                      contract.scope_kind,contract.contract_hash,0::INTEGER AS depth
                 FROM verification_budget_contracts contract
                 JOIN verification_prepared_actions prepared
                   ON prepared.prepared_action_id=$1
                  AND contract.scope_kind='action'
                  AND contract.scope_id=prepared.prepared_action_id
                  AND contract.contract_hash=prepared.upper_budget_set_hash
                WHERE contract.sealed_at IS NOT NULL
               UNION ALL
               SELECT parent.budget_contract_id,parent.parent_contract_id,
                      parent.scope_kind,parent.contract_hash,child.depth+1
                 FROM lineage child
                 JOIN verification_budget_contracts parent
                   ON parent.budget_contract_id=child.parent_contract_id
                  AND parent.sealed_at IS NOT NULL
           ), action_axes AS (
               SELECT axis.axis_kind,axis.axis_limit,axis.axis_ordinal
                 FROM lineage action_contract
                 JOIN verification_budget_contract_axes axis
                   ON axis.budget_contract_id=action_contract.budget_contract_id
                WHERE action_contract.depth=0
           )
           SELECT lineage.budget_contract_id,action_axes.axis_kind,
                  action_axes.axis_limit,ancestor_axis.axis_limit,lineage.depth,
                  lineage.scope_kind,lineage.contract_hash
             FROM lineage
             JOIN action_axes ON TRUE
             JOIN verification_budget_contract_axes ancestor_axis
               ON ancestor_axis.budget_contract_id=lineage.budget_contract_id
              AND ancestor_axis.axis_kind=action_axes.axis_kind
            ORDER BY lineage.depth DESC,action_axes.axis_ordinal"#,
    )
    .bind(command.prepared_action_id)
    .fetch_all(&mut **tx)
    .await?;
    let axis_count = axes.iter().filter(|axis| axis.4 == 0).count();
    if axis_count == 0 || axes.len() != axis_count * 4 {
        return Err(conflict(AUTHORITY_STALE));
    }
    let expected_scopes = [
        (0, "action"),
        (1, "campaign"),
        (2, "wave"),
        (3, "operation"),
    ];
    if expected_scopes.iter().any(|(depth, scope)| {
        axes.iter().filter(|axis| axis.4 == *depth).count() != axis_count
            || axes
                .iter()
                .filter(|axis| axis.4 == *depth)
                .any(|axis| axis.5.as_str() != *scope || axis.2 < 0 || axis.2 > axis.3)
    }) {
        return Err(conflict(AUTHORITY_STALE));
    }
    let mut contract_hashes = Vec::with_capacity(4);
    for depth in (0..=3).rev() {
        let contract_hash = axes
            .iter()
            .find(|axis| axis.4 == depth)
            .map(|axis| axis.6.clone())
            .ok_or_else(|| conflict(AUTHORITY_STALE))?;
        contract_hashes.push(contract_hash);
    }
    let contract_set_hash =
        exact_set_hash_on(tx, "verification_budget_contract_set.v1", &contract_hashes).await?;
    let mut upper_bound_member_hashes = Vec::with_capacity(axes.len());
    for axis in &axes {
        upper_bound_member_hashes.push(
            json_hash_on(
                tx,
                &serde_json::json!({
                    "ancestor_contract_id": axis.0,
                    "axis_kind": axis.1,
                    "reserved_upper_bound": axis.2,
                }),
            )
            .await?,
        );
    }
    let upper_bound_membership_hash = exact_set_hash_on(
        tx,
        "verification_budget_upper_bound_membership.v1",
        &upper_bound_member_hashes,
    )
    .await?;
    let reservation_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-budget-reservation.v1",
    );
    let execution_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-action-execution.v1",
    );
    let durable_begin_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "prepared_action_id": command.prepared_action_id,
            "authorization_receipt_id": command.authorization_receipt_id,
            "tool_truth_authority_bundle_seal_id": authority.bundle_seal_id(),
            "conflict_set_id": command.conflict_set_id,
            "contract_set_hash": contract_set_hash,
            "upper_bound_membership_hash": upper_bound_membership_hash,
            "execution_ordinal": command.execution_ordinal,
        }),
    )
    .await?;
    let existing: Option<(Uuid, Uuid, Uuid, Uuid, String)> = sqlx::query_as(
        r#"SELECT execution.action_execution_id,execution.budget_reservation_id,
                  execution.prepared_action_id,execution.authorization_receipt_id,
                  execution.durable_begin_hash
             FROM verification_action_executions execution
            WHERE execution.stable_request_id=$1 FOR SHARE"#,
    )
    .bind(command.stable_request_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((existing_execution, existing_reservation, action, authorization, begin_hash)) =
        existing
    {
        if action == command.prepared_action_id
            && authorization == command.authorization_receipt_id
            && begin_hash == durable_begin_hash
        {
            return Ok(DurableActionBegin {
                action_execution_id: existing_execution,
                budget_reservation_id: existing_reservation,
                durable_begin_hash: begin_hash,
            });
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    sqlx::query(
        r#"INSERT INTO verification_budget_reservations(
               budget_reservation_id,stable_request_id,prepared_action_id,
               authorization_receipt_id,operation_id,project_scope_id,organization_id,
               contract_set_hash,upper_bound_membership_hash,state
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'active')"#,
    )
    .bind(reservation_id)
    .bind(command.stable_request_id)
    .bind(command.prepared_action_id)
    .bind(command.authorization_receipt_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(&contract_set_hash)
    .bind(&upper_bound_membership_hash)
    .execute(&mut **tx)
    .await?;

    for axis in &axes {
        let head: (i64, i64, i64, i64, i64) = sqlx::query_as(
            r#"SELECT head.consumed,head.reserved,head.unknown_held,head.row_version,axis.axis_limit
                 FROM verification_budget_scope_heads head
                 JOIN verification_budget_contract_axes axis
                   ON axis.budget_contract_id=head.budget_contract_id AND axis.axis_kind=head.axis_kind
                WHERE head.budget_contract_id=$1 AND head.axis_kind=$2 FOR UPDATE"#,
        )
        .bind(axis.0)
        .bind(&axis.1)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict(AUTHORITY_STALE))?;
        if head.0 + head.1 + head.2 + axis.2 > head.4 {
            return Err(conflict("VERIFICATION_BUDGET_EXHAUSTED"));
        }
        let resulting_reserved = head.1 + axis.2;
        let resulting_hash = json_hash_on(
            tx,
            &serde_json::json!({
                "contract_id": axis.0,
                "axis_kind": axis.1,
                "consumed": head.0,
                "reserved": resulting_reserved,
                "unknown_held": head.2,
                "row_version": head.3 + 1,
            }),
        )
        .await?;
        let entry_ordinal: i64 = sqlx::query_scalar(
            r#"SELECT COALESCE(MAX(entry_ordinal),0)+1
                 FROM verification_budget_ledger_entries
                WHERE budget_reservation_id=$1 AND ancestor_contract_id=$2
                  AND axis_kind=$3"#,
        )
        .bind(reservation_id)
        .bind(axis.0)
        .bind(&axis.1)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO verification_budget_ledger_entries(
                   budget_ledger_entry_id,budget_reservation_id,ancestor_contract_id,
                   axis_kind,entry_ordinal,entry_kind,delta,resulting_consumed,
                   resulting_reserved,resulting_unknown_held,expected_head_row_version,
                   resulting_head_hash,fence
               ) VALUES($1,$2,$3,$4,$5,'reserve',$6,$7,$8,$9,$10,$11,$12)"#,
        )
        .bind(Uuid::new_v5(
            &reservation_id,
            format!("{}:{}:reserve", axis.0, axis.1).as_bytes(),
        ))
        .bind(reservation_id)
        .bind(axis.0)
        .bind(&axis.1)
        .bind(entry_ordinal)
        .bind(axis.2)
        .bind(head.0)
        .bind(resulting_reserved)
        .bind(head.2)
        .bind(head.3)
        .bind(resulting_hash)
        .bind(head.3 + 1)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            r#"UPDATE verification_budget_scope_heads
                  SET reserved=$1,row_version=row_version+1,updated_at=statement_timestamp()
                WHERE budget_contract_id=$2 AND axis_kind=$3 AND row_version=$4"#,
        )
        .bind(resulting_reserved)
        .bind(axis.0)
        .bind(&axis.1)
        .bind(head.3)
        .execute(&mut **tx)
        .await?;
    }

    let keys: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT member.key_kind,member.key_identity_hash
             FROM verification_action_conflict_set_members member
             JOIN verification_action_conflict_sets conflict_set
               ON conflict_set.conflict_set_id=member.conflict_set_id
            WHERE member.conflict_set_id=$1 AND conflict_set.sealed_at IS NOT NULL
            ORDER BY member.key_kind,member.key_identity_hash"#,
    )
    .bind(command.conflict_set_id)
    .fetch_all(&mut **tx)
    .await?;
    for (key_kind, key_hash) in keys {
        sqlx::query(
            r#"INSERT INTO verification_conflict_key_heads(
                   operation_id,project_scope_id,organization_id,key_kind,key_identity_hash,state
               ) VALUES($1,$2,$3,$4,$5,'free') ON CONFLICT DO NOTHING"#,
        )
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(&key_kind)
        .bind(&key_hash)
        .execute(&mut **tx)
        .await?;
        let head: (String, i64, i64) = sqlx::query_as(
            r#"SELECT state,fencing_token,row_version FROM verification_conflict_key_heads
                WHERE operation_id=$1 AND organization_id=$2 AND key_kind=$3
                  AND key_identity_hash=$4 FOR UPDATE"#,
        )
        .bind(command.operation_id)
        .bind(command.organization_id)
        .bind(&key_kind)
        .bind(&key_hash)
        .fetch_one(&mut **tx)
        .await?;
        if head.0 != "free" {
            return Err(conflict("VERIFICATION_CONFLICT_KEY_UNAVAILABLE"));
        }
        let event_hash = json_hash_on(
            tx,
            &serde_json::json!({
                "operation_id": command.operation_id,
                "organization_id": command.organization_id,
                "key_kind": key_kind,
                "key_identity_hash": key_hash,
                "expected_fencing_token": head.1,
                "new_fencing_token": head.1 + 1,
                "prepared_action_id": command.prepared_action_id,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO verification_conflict_key_events(
                   conflict_event_id,operation_id,project_scope_id,organization_id,key_kind,
                   key_identity_hash,event_ordinal,event_kind,expected_fencing_token,
                   new_fencing_token,owner_campaign_id,owner_prepared_action_id,
                   reason_code,event_hash
               ) SELECT $1,$2,$3,$4,$5,$6,COALESCE(MAX(event_ordinal),0)+1,'acquire',
                        $7,$8,action.campaign_id,$9,'durable_begin',$10
                   FROM verification_prepared_actions action
                   LEFT JOIN verification_conflict_key_events prior
                     ON prior.operation_id=$2 AND prior.organization_id=$4
                    AND prior.key_kind=$5 AND prior.key_identity_hash=$6
                  WHERE action.prepared_action_id=$9 GROUP BY action.campaign_id"#,
        )
        .bind(Uuid::new_v5(&execution_id, event_hash.as_bytes()))
        .bind(command.operation_id)
        .bind(command.project_scope_id)
        .bind(command.organization_id)
        .bind(&key_kind)
        .bind(&key_hash)
        .bind(head.1)
        .bind(head.1 + 1)
        .bind(command.prepared_action_id)
        .bind(&event_hash)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            r#"UPDATE verification_conflict_key_heads head
                  SET state='active',owner_campaign_id=action.campaign_id,
                      owner_prepared_action_id=$1,fencing_token=$2,
                      row_version=head.row_version+1
                 FROM verification_prepared_actions action
                WHERE head.operation_id=$3 AND head.organization_id=$4
                  AND head.key_kind=$5 AND head.key_identity_hash=$6
                  AND head.row_version=$7 AND action.prepared_action_id=$1"#,
        )
        .bind(command.prepared_action_id)
        .bind(head.1 + 1)
        .bind(command.operation_id)
        .bind(command.organization_id)
        .bind(&key_kind)
        .bind(&key_hash)
        .bind(head.2)
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query(
        r#"INSERT INTO verification_action_executions(
               action_execution_id,stable_request_id,prepared_action_id,authorization_receipt_id,
               budget_reservation_id,conflict_set_id,operation_id,project_scope_id,organization_id,
               execution_ordinal,execution_kind,state,campaign_dispatch_generation,durable_begin_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'started',$12,$13)"#,
    )
    .bind(execution_id)
    .bind(command.stable_request_id)
    .bind(command.prepared_action_id)
    .bind(command.authorization_receipt_id)
    .bind(reservation_id)
    .bind(command.conflict_set_id)
    .bind(command.operation_id)
    .bind(command.project_scope_id)
    .bind(command.organization_id)
    .bind(command.execution_ordinal)
    .bind(&command.execution_kind)
    .bind(command.campaign_dispatch_generation)
    .bind(&durable_begin_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE verification_prepared_actions SET state='started',row_version=row_version+1 WHERE prepared_action_id=$1 AND state='authorized'",
    )
    .bind(command.prepared_action_id)
    .execute(&mut **tx)
    .await?;
    Ok(DurableActionBegin {
        action_execution_id: execution_id,
        budget_reservation_id: reservation_id,
        durable_begin_hash,
    })
}

#[derive(Debug, Clone)]
pub struct RecordActionSubexecution {
    pub action_subexecution_id: Uuid,
    pub action_execution_id: Uuid,
    pub prepared_action_id: Uuid,
    pub group_member_id: Uuid,
    pub subexecution_ordinal: i32,
    pub state: String,
    pub capability_execution_receipt_id: Uuid,
    pub barrier_released_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub member_hash: String,
}

pub async fn record_action_subexecution(
    pool: &PgPool,
    command: &RecordActionSubexecution,
) -> Result<()> {
    if !matches!(
        command.state.as_str(),
        "succeeded" | "failed" | "outcome_unknown"
    ) || command.started_at < command.barrier_released_at
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    require_sha256(&command.member_hash)?;
    let inserted = sqlx::query(
        r#"INSERT INTO verification_action_subexecutions(
               action_subexecution_id,action_execution_id,prepared_action_id,group_member_id,
               subexecution_ordinal,state,capability_execution_receipt_id,barrier_released_at,
               started_at,completed_at,member_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
           ON CONFLICT(action_subexecution_id) DO NOTHING"#,
    )
    .bind(command.action_subexecution_id)
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .bind(command.group_member_id)
    .bind(command.subexecution_ordinal)
    .bind(&command.state)
    .bind(command.capability_execution_receipt_id)
    .bind(command.barrier_released_at)
    .bind(command.started_at)
    .bind(command.completed_at)
    .bind(&command.member_hash)
    .execute(pool)
    .await?;
    if inserted.rows_affected() == 0 {
        let persisted: (Uuid, Uuid, Uuid, i32, String, Uuid, String) = sqlx::query_as(
            r#"SELECT action_execution_id,prepared_action_id,group_member_id,
                      subexecution_ordinal,state,capability_execution_receipt_id,member_hash
                 FROM verification_action_subexecutions WHERE action_subexecution_id=$1"#,
        )
        .bind(command.action_subexecution_id)
        .fetch_one(pool)
        .await?;
        if persisted
            != (
                command.action_execution_id,
                command.prepared_action_id,
                command.group_member_id,
                command.subexecution_ordinal,
                command.state.clone(),
                command.capability_execution_receipt_id,
                command.member_hash.clone(),
            )
        {
            return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct BeginVerificationActionCapabilityReceipt {
    pub stable_request_id: Uuid,
    pub action_execution_id: Uuid,
    pub prepared_action_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationActionCapabilityReceiptBegin {
    pub binding_id: Uuid,
    pub capability_execution_receipt_id: Uuid,
    pub derived_denominator_id: Uuid,
    pub replayed: bool,
}

/// Derives a one-member Plan A child denominator from the Campaign's frozen
/// all-fresh root graph, seals the exact host-pinned destination policy, and
/// begins the Tool Truth receipt before any target I/O occurs.
pub async fn begin_verification_action_capability_receipt(
    pool: &PgPool,
    command: BeginVerificationActionCapabilityReceipt,
) -> Result<VerificationActionCapabilityReceiptBegin> {
    #[derive(sqlx::FromRow)]
    struct Authority {
        campaign_id: Uuid,
        operation_id: Uuid,
        project_scope_id: Uuid,
        organization_id: Uuid,
        capability: String,
        execution_ordinal: i32,
        target_live_id: Uuid,
        exact_target_url: String,
        execution_authority_id: Uuid,
        execution_authority_hash: String,
        project_path_at_freeze: String,
        scope_snapshot_id: Uuid,
        stage_execution_id: Uuid,
        stage_kind: String,
        parent_denominator_id: Uuid,
        parent_denominator_item_id: Uuid,
        scheme: String,
        normalized_host: String,
        port: i32,
        path_boundary: String,
        scope_exception_hash: Option<String>,
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "verification-action-capability-begin:{}",
            command.stable_request_id
        ))
        .execute(&mut *tx)
        .await?;
    if let Some((binding_id, receipt_id, denominator_id)) = sqlx::query_as::<_, (Uuid, Uuid, Uuid)>(
        r#"SELECT binding_id,capability_execution_receipt_id,derived_denominator_id
                 FROM verification_action_capability_receipt_bindings
                WHERE stable_request_id=$1 AND action_execution_id=$2
                  AND prepared_action_id=$3"#,
    )
    .bind(command.stable_request_id)
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(VerificationActionCapabilityReceiptBegin {
            binding_id,
            capability_execution_receipt_id: receipt_id,
            derived_denominator_id: denominator_id,
            replayed: true,
        });
    }
    let authority = sqlx::query_as::<_, Authority>(
        r#"SELECT action.campaign_id,action.operation_id,action.project_scope_id,
                  action.organization_id,action.action_kind AS capability,
                  execution.execution_ordinal,action.target_live_id,
                  action.private_manifest #>> '{exact_target_url}' AS exact_target_url,
                  parent.id AS execution_authority_id,root.execution_authority_hash,
                  root.project_path_at_freeze,root.scope_snapshot_id,
                  root.stage_execution_id,root.stage_kind,
                  root.id AS parent_denominator_id,
                  item.id AS parent_denominator_item_id,
                  action.private_manifest #>> '{network_policy,scheme}' AS scheme,
                  action.private_manifest #>> '{network_policy,normalized_host}' AS normalized_host,
                  (action.private_manifest #>> '{network_policy,port}')::INT AS port,
                  action.private_manifest #>> '{network_policy,path_boundary}' AS path_boundary,
                  NULLIF(action.private_manifest #>> '{network_policy,scope_exception_hash}','')
                      AS scope_exception_hash
             FROM verification_action_executions execution
             JOIN verification_prepared_actions action
               ON action.prepared_action_id=execution.prepared_action_id
              AND action.state='started'
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=action.campaign_id
              AND campaign.terminal_at IS NULL AND campaign.superseded_at IS NULL
             JOIN tool_truth_authority_bundle_members bundle
               ON bundle.bundle_seal_id=campaign.tool_truth_authority_bundle_seal_id
              AND bundle.member_status='consistent_fresh'
             JOIN coverage_denominators root
               ON root.id=bundle.root_denominator_id
              AND root.execution_authority_id=bundle.root_execution_authority_id
              AND root.sealed_at IS NOT NULL
             JOIN coverage_denominator_items item
               ON item.denominator_id=root.id AND item.target_id=action.target_live_id
             JOIN tool_truth_execution_authorities parent
               ON parent.id=root.execution_authority_id
            WHERE execution.action_execution_id=$1
              AND execution.prepared_action_id=$2 AND execution.state='started'
            ORDER BY bundle.ordinal,item.ordinal LIMIT 1"#,
    )
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let denominator_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-action-tool-truth-denominator.v1",
    );
    let denominator_item_id =
        Uuid::new_v5(&denominator_id, b"verification-action-tool-truth-input.v1");
    let destination_policy_id = Uuid::new_v5(
        &denominator_id,
        b"verification-action-destination-policy.v1",
    );
    let input_key = format!("verification-action:{}", command.action_execution_id);
    let denominator_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM coverage_denominators WHERE id=$1 AND sealed_at IS NOT NULL)",
    )
    .bind(denominator_id)
    .fetch_one(&mut *tx)
    .await?;
    if !denominator_exists {
        let item_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": 0,
                "input_key": &input_key,
                "target_id": authority.target_live_id,
                "exact_asset": &authority.exact_target_url,
                "technique": "verification_prepared_action.v1",
                "expected_capability": &authority.capability,
            }),
        )
        .await?;
        let member_set_hash = exact_set_hash_on(
            &mut tx,
            "verification_action_tool_truth_denominator.v1",
            std::slice::from_ref(&item_hash),
        )
        .await?;
        let input_manifest_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "prepared_action_id": command.prepared_action_id,
                "action_execution_id": command.action_execution_id,
                "member_set_hash": &member_set_hash,
            }),
        )
        .await?;
        let denominator_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "execution_authority_id": authority.execution_authority_id,
                "parent_denominator_id": authority.parent_denominator_id,
                "parent_denominator_item_id": authority.parent_denominator_item_id,
                "input_manifest_hash": &input_manifest_hash,
                "member_set_hash": &member_set_hash,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO coverage_denominators(
                   id,stable_seal_request_id,execution_authority_id,operation_id,
                   project_scope_id,project_path_at_freeze,scope_snapshot_id,
                   organization_id,stage_execution_id,stage_kind,execution_authority_hash,
                   denominator_kind,parent_denominator_id,parent_denominator_item_id,
                   derived_ordinal,contract,input_manifest_hash,denominator_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'derived_child',$12,$13,
                        $14,'receipt_v1',$15,$16)"#,
        )
        .bind(denominator_id)
        .bind(command.stable_request_id)
        .bind(authority.execution_authority_id)
        .bind(authority.operation_id)
        .bind(authority.project_scope_id)
        .bind(&authority.project_path_at_freeze)
        .bind(authority.scope_snapshot_id)
        .bind(authority.organization_id)
        .bind(authority.stage_execution_id)
        .bind(&authority.stage_kind)
        .bind(&authority.execution_authority_hash)
        .bind(authority.parent_denominator_id)
        .bind(authority.parent_denominator_item_id)
        .bind(authority.execution_ordinal)
        .bind(&input_manifest_hash)
        .bind(&denominator_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO coverage_denominator_items(
                   id,denominator_id,execution_authority_id,denominator_hash,ordinal,
                   input_key,target_id,exact_asset,technique,expected_capability,member_hash
               ) VALUES($1,$2,$3,$4,0,$5,$6,$7,'verification_prepared_action.v1',$8,$9)"#,
        )
        .bind(denominator_item_id)
        .bind(denominator_id)
        .bind(authority.execution_authority_id)
        .bind(&denominator_hash)
        .bind(&input_key)
        .bind(authority.target_live_id)
        .bind(&authority.exact_target_url)
        .bind(&authority.capability)
        .bind(item_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE coverage_denominators SET sealed_at=statement_timestamp() WHERE id=$1 AND sealed_at IS NULL",
        )
        .bind(denominator_id)
        .execute(&mut *tx)
        .await?;

        let tls_policy_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({"policy": "rustls_webpki_hostname_sni_required.v1"}),
        )
        .await?;
        let prohibited_range_policy_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "policy": "deny_loopback_private_link_local_metadata.v1",
                "scope_exception_hash": &authority.scope_exception_hash,
            }),
        )
        .await?;
        let destination_member_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "ordinal": 0,
                "destination_role": "authorized_target",
                "scheme": &authority.scheme,
                "normalized_host": &authority.normalized_host,
                "port": authority.port,
                "path_prefix": &authority.path_boundary,
                "input_binding_mode": "destination_authority",
                "exact_scope_exception_hash": &authority.scope_exception_hash,
            }),
        )
        .await?;
        let policy_hash = json_hash_on(
            &mut tx,
            &serde_json::json!({
                "denominator_id": denominator_id,
                "execution_authority_id": authority.execution_authority_id,
                "capability": &authority.capability,
                "execution_backend": "host_pinned_http",
                "governance_status": "enforced",
                "redirect_mode": "deny",
                "max_redirect_hops": 0,
                "tls_policy_hash": &tls_policy_hash,
                "prohibited_range_policy_hash": &prohibited_range_policy_hash,
                "members": [&destination_member_hash],
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO capability_execution_destination_policies(
                   id,denominator_id,execution_authority_id,capability,execution_backend,
                   governance_status,redirect_mode,max_redirect_hops,tls_policy_hash,
                   prohibited_range_policy_hash,policy_hash
               ) VALUES($1,$2,$3,$4,'host_pinned_http','enforced','deny',0,$5,$6,$7)"#,
        )
        .bind(destination_policy_id)
        .bind(denominator_id)
        .bind(authority.execution_authority_id)
        .bind(&authority.capability)
        .bind(tls_policy_hash)
        .bind(prohibited_range_policy_hash)
        .bind(policy_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO capability_execution_destination_policy_members(
                   id,policy_id,execution_authority_id,ordinal,destination_role,scheme,
                   normalized_host,port,path_prefix,input_binding_mode,
                   exact_scope_exception_hash,member_hash
               ) VALUES($1,$2,$3,0,'authorized_target',$4,$5,$6,$7,
                        'destination_authority',$8,$9)"#,
        )
        .bind(Uuid::new_v5(
            &destination_policy_id,
            b"authorized-target.v1",
        ))
        .bind(destination_policy_id)
        .bind(authority.execution_authority_id)
        .bind(&authority.scheme)
        .bind(&authority.normalized_host)
        .bind(authority.port)
        .bind(&authority.path_boundary)
        .bind(&authority.scope_exception_hash)
        .bind(destination_member_hash)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE capability_execution_destination_policies SET sealed_at=statement_timestamp() WHERE id=$1 AND sealed_at IS NULL",
        )
        .bind(destination_policy_id)
        .execute(&mut *tx)
        .await?;
    }
    let receipt_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-action-tool-truth-receipt.v1",
    );
    let (receipt, receipt_replayed) =
        match super::capability_execution_receipts::begin_managed_claim_in_connection(
            &mut tx,
            &super::capability_execution_receipts::BeginManagedCapabilityReceipt {
                id: receipt_id,
                denominator_id,
                capability: authority.capability.clone(),
                attempt_ordinal: authority.execution_ordinal,
                destination_policy_id,
            },
        )
        .await?
        {
            super::capability_execution_receipts::ManagedReceiptBeginOutcome::Created(receipt) => {
                (receipt, false)
            }
            super::capability_execution_receipts::ManagedReceiptBeginOutcome::TerminalReplay(
                receipt,
            )
            | super::capability_execution_receipts::ManagedReceiptBeginOutcome::InFlight(receipt) => {
                (receipt, true)
            }
        };
    let binding_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-action-tool-truth-binding.v1",
    );
    let binding_hash = json_hash_on(
        &mut tx,
        &serde_json::json!({
            "action_execution_id": command.action_execution_id,
            "prepared_action_id": command.prepared_action_id,
            "capability_execution_receipt_id": receipt.id,
            "derived_denominator_id": denominator_id,
            "parent_denominator_id": authority.parent_denominator_id,
            "parent_denominator_item_id": authority.parent_denominator_item_id,
            "execution_authority_id": authority.execution_authority_id,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO verification_action_capability_receipt_bindings(
               binding_id,stable_request_id,action_execution_id,prepared_action_id,campaign_id,
               operation_id,project_scope_id,organization_id,capability_execution_receipt_id,
               derived_denominator_id,parent_denominator_id,parent_denominator_item_id,
               execution_authority_id,binding_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
           ON CONFLICT(stable_request_id) DO NOTHING"#,
    )
    .bind(binding_id)
    .bind(command.stable_request_id)
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .bind(authority.campaign_id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(authority.organization_id)
    .bind(receipt.id)
    .bind(denominator_id)
    .bind(authority.parent_denominator_id)
    .bind(authority.parent_denominator_item_id)
    .bind(authority.execution_authority_id)
    .bind(binding_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(VerificationActionCapabilityReceiptBegin {
        binding_id,
        capability_execution_receipt_id: receipt.id,
        derived_denominator_id: denominator_id,
        replayed: receipt_replayed,
    })
}

#[derive(Debug, Clone)]
pub struct FinalizeVerificationActionCapabilityReceipt {
    pub stable_request_id: Uuid,
    pub action_execution_id: Uuid,
    pub prepared_action_id: Uuid,
    pub capability_execution_receipt_id: Uuid,
    pub terminal_state: String,
    pub observation: Value,
}

const DIRECTORY_FINGERPRINT_CAPABILITY_V1: &str = "verify.directory_fingerprint.v1";
const DIRECTORY_FINGERPRINT_OBSERVATION_V1: &str = "directory-soft404-fingerprint-observation.v1";
const DIRECTORY_FINGERPRINT_WITNESS_V1: &str = "complete_fingerprint_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryFingerprintOracleVerdictV1 {
    Proof,
    Refutation,
    Inconclusive,
}

impl DirectoryFingerprintOracleVerdictV1 {
    const fn observation_value(self) -> &'static str {
        match self {
            Self::Proof => "verified",
            Self::Refutation => "refuted",
            Self::Inconclusive => "inconclusive",
        }
    }

    const fn oracle_value(self) -> &'static str {
        match self {
            Self::Proof => "proof",
            Self::Refutation => "refutation",
            Self::Inconclusive => "inconclusive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerificationActionWitnessV1 {
    MetadataOnly,
    DirectoryFingerprint(DirectoryFingerprintOracleVerdictV1),
}

type DirectoryHttpFingerprintV1<'a> = (u16, u64, &'a str, Option<&'a str>);
type VerificationResidualSpecV1<'a> = (&'a str, &'a [u8], Value);
type VerificationOracleLandingPlanV1<'a> = (
    &'a str,
    &'a str,
    Option<VerificationResidualSpecV1<'a>>,
    &'a [u8],
);

impl VerificationActionWitnessV1 {
    const fn witness_completeness(self) -> &'static str {
        match self {
            Self::MetadataOnly => "metadata_only",
            Self::DirectoryFingerprint(_) => DIRECTORY_FINGERPRINT_WITNESS_V1,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryFingerprintObservationV1 {
    assessment: DirectoryFingerprintAssessmentV1,
    candidate: DirectoryFingerprintHttpObservationV1,
    capability_id: String,
    contract_version: String,
    controls: Vec<DirectoryFingerprintHttpObservationV1>,
    request_count: u32,
    witness_completeness: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryFingerprintAssessmentV1 {
    controls_consistent: bool,
    verdict: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryFingerprintHttpObservationV1 {
    final_url: String,
    hops: Vec<DirectoryFingerprintHttpHopV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryFingerprintHttpHopV1 {
    url: String,
    status: u16,
    response_bytes: u64,
    body_sha256: String,
    content_type: Option<String>,
}

fn directory_http_origin_and_terminal_fingerprint(
    observation: &DirectoryFingerprintHttpObservationV1,
) -> Option<(String, DirectoryHttpFingerprintV1<'_>)> {
    let first = observation.hops.first()?;
    let terminal = observation.hops.last()?;
    if observation.final_url != terminal.url
        || observation.hops.iter().any(|hop| {
            !hop.body_sha256.starts_with("sha256:")
                || hop.body_sha256.len() != 71
                || !hop.body_sha256[7..]
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
    {
        return None;
    }
    let origin = reqwest::Url::parse(&first.url)
        .ok()?
        .origin()
        .ascii_serialization();
    if observation.hops.iter().any(|hop| {
        reqwest::Url::parse(&hop.url)
            .map(|url| url.origin().ascii_serialization() != origin)
            .unwrap_or(true)
    }) {
        return None;
    }
    Some((
        origin,
        (
            terminal.status,
            terminal.response_bytes,
            terminal.body_sha256.as_str(),
            terminal.content_type.as_deref(),
        ),
    ))
}

fn classify_verification_action_witness(
    prepared_action_id: Uuid,
    terminal_state: &str,
    observation: &Value,
) -> Result<VerificationActionWitnessV1> {
    let declared = observation
        .get("witness_completeness")
        .and_then(Value::as_str)
        .ok_or_else(|| conflict(CONTRACT_INVALID))?;
    if declared == "metadata_only" {
        return Ok(VerificationActionWitnessV1::MetadataOnly);
    }
    if declared != DIRECTORY_FINGERPRINT_WITNESS_V1 || terminal_state != "succeeded" {
        return Err(conflict(CONTRACT_INVALID));
    }
    let typed: DirectoryFingerprintObservationV1 =
        serde_json::from_value(observation.clone()).map_err(|_| conflict(CONTRACT_INVALID))?;
    if typed.capability_id != DIRECTORY_FINGERPRINT_CAPABILITY_V1
        || typed.contract_version != DIRECTORY_FINGERPRINT_OBSERVATION_V1
        || typed.witness_completeness != DIRECTORY_FINGERPRINT_WITNESS_V1
        || typed.request_count != 4
        || typed.controls.len() != 3
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let (candidate_origin, candidate_fingerprint) =
        directory_http_origin_and_terminal_fingerprint(&typed.candidate)
            .ok_or_else(|| conflict(CONTRACT_INVALID))?;
    let candidate_first_url = reqwest::Url::parse(
        &typed
            .candidate
            .hops
            .first()
            .ok_or_else(|| conflict(CONTRACT_INVALID))?
            .url,
    )
    .map_err(|_| conflict(CONTRACT_INVALID))?;
    let parent_path = candidate_first_url
        .path()
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let nonce = prepared_action_id.simple().to_string();
    let mut control_fingerprints = Vec::with_capacity(typed.controls.len());
    for (index, control) in typed.controls.iter().enumerate() {
        let expected_path = format!("{parent_path}/.golish-soft404-{nonce}-{}", index + 1);
        let first_url = reqwest::Url::parse(
            &control
                .hops
                .first()
                .ok_or_else(|| conflict(CONTRACT_INVALID))?
                .url,
        )
        .map_err(|_| conflict(CONTRACT_INVALID))?;
        let (origin, fingerprint) = directory_http_origin_and_terminal_fingerprint(control)
            .ok_or_else(|| conflict(CONTRACT_INVALID))?;
        if origin != candidate_origin
            || first_url.origin().ascii_serialization() != candidate_origin
            || first_url.path() != expected_path
            || first_url.query().is_some()
            || first_url.fragment().is_some()
        {
            return Err(conflict(CONTRACT_INVALID));
        }
        control_fingerprints.push(fingerprint);
    }
    let controls_consistent = control_fingerprints
        .windows(2)
        .all(|window| window[0] == window[1]);
    let verdict = if !controls_consistent {
        DirectoryFingerprintOracleVerdictV1::Inconclusive
    } else if candidate_fingerprint == control_fingerprints[0] {
        DirectoryFingerprintOracleVerdictV1::Refutation
    } else {
        DirectoryFingerprintOracleVerdictV1::Proof
    };
    if typed.assessment.controls_consistent != controls_consistent
        || typed.assessment.verdict != verdict.observation_value()
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    Ok(VerificationActionWitnessV1::DirectoryFingerprint(verdict))
}

#[derive(Debug)]
struct DirectoryFingerprintToolTruthAuthorityV1 {
    temporal_census_id: Uuid,
    reconciliation_id: Uuid,
    semantic_authority_version: i64,
    semantic_reconciliation_hash: String,
}

/// Seals the non-raw directory fingerprint as a complete typed derivative.
///
/// The capability receipt deliberately remains `sampled`: Golish did not
/// retain the response bodies and must not claim a complete raw witness.  The
/// one-member derived denominator is nevertheless complete when the host has
/// independently validated all four full-body hashes.  This compound records
/// that narrower truth as an evidence-backed receipt input, temporal census
/// and semantic reconciliation so downstream all-fresh consumers do not
/// confuse "no raw bytes retained" with "the assigned verification input was
/// not executed".
async fn seal_directory_fingerprint_tool_truth_authority_v1(
    tx: &mut Transaction<'_, Postgres>,
    command: &FinalizeVerificationActionCapabilityReceipt,
    observation_hash: &str,
    finalization_hash: &str,
    observation_state: &str,
) -> Result<DirectoryFingerprintToolTruthAuthorityV1> {
    #[derive(sqlx::FromRow)]
    struct Authority {
        execution_authority_id: Uuid,
        denominator_id: Uuid,
        temporal_validity_policy_id: Uuid,
        semantic_authority_version: i64,
        predecessor_reconciliation_id: Option<Uuid>,
        observation_started_at: DateTime<Utc>,
        operation_id: Uuid,
        project_scope_id: Uuid,
        project_path_at_freeze: String,
        scope_snapshot_id: Uuid,
        organization_id: Uuid,
        stage_execution_id: Uuid,
        stage_kind: String,
        execution_authority_hash: String,
        execution_owner_kind: String,
        worker_run_id: Option<Uuid>,
        worker_attempt_epoch: Option<i64>,
        lease_token: Option<Uuid>,
        source_tool_call_id: Option<Uuid>,
        denominator_item_id: Uuid,
        input_key: String,
        technique: String,
        target_id: Uuid,
        exact_asset: String,
    }

    let authority = sqlx::query_as::<_, Authority>(
        r#"SELECT receipt.execution_authority_id,
                  receipt.denominator_id,receipt.temporal_validity_policy_id,
                  receipt.current_semantic_authority_version AS semantic_authority_version,
                  receipt.current_semantic_reconciliation_id AS predecessor_reconciliation_id,
                  receipt.observation_started_at,
                  execution_authority.operation_id,execution_authority.project_scope_id,
                  execution_authority.project_path_at_freeze,
                  execution_authority.scope_snapshot_id,execution_authority.organization_id,
                  execution_authority.stage_execution_id,execution_authority.stage_kind,
                  execution_authority.authority_hash AS execution_authority_hash,
                  execution_authority.execution_owner_kind,
                  execution_authority.worker_run_id,
                  execution_authority.worker_attempt_epoch,
                  execution_authority.lease_token,
                  execution_authority.source_tool_call_id,
                  item.id AS denominator_item_id,item.input_key,item.technique,
                  item.target_id,item.exact_asset
             FROM verification_action_capability_receipt_bindings binding
             JOIN capability_execution_receipts receipt
               ON receipt.id=binding.capability_execution_receipt_id
              AND receipt.denominator_id=binding.derived_denominator_id
              AND receipt.execution_authority_id=binding.execution_authority_id
             JOIN tool_truth_execution_authorities execution_authority
               ON execution_authority.id=receipt.execution_authority_id
             JOIN coverage_denominator_items item
               ON item.denominator_id=receipt.denominator_id
              AND item.execution_authority_id=receipt.execution_authority_id
              AND item.expected_capability=receipt.capability
            WHERE binding.action_execution_id=$1 AND binding.prepared_action_id=$2
              AND binding.capability_execution_receipt_id=$3
              AND receipt.capability=$4
              AND (SELECT count(*) FROM coverage_denominator_items exact_item
                    WHERE exact_item.denominator_id=receipt.denominator_id
                      AND exact_item.expected_capability=receipt.capability)=1
            FOR SHARE OF binding,receipt,execution_authority,item"#,
    )
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .bind(command.capability_execution_receipt_id)
    .bind(DIRECTORY_FINGERPRINT_CAPABILITY_V1)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;

    let snapshot_byte_count: i64 =
        sqlx::query_scalar("SELECT octet_length(($1::JSONB)::TEXT)::BIGINT")
            .bind(&command.observation)
            .fetch_one(&mut **tx)
            .await?;
    let mut producer = serde_json::json!({
        "organization_id": authority.organization_id,
        "stage_execution_id": authority.stage_execution_id,
        "receipt_id": command.capability_execution_receipt_id,
        "fingerprint_observation_hash": observation_hash,
        "finalization_hash": finalization_hash,
    });
    if authority.execution_owner_kind == "worker_tool" {
        let producer = producer
            .as_object_mut()
            .ok_or_else(|| conflict(CONTRACT_INVALID))?;
        producer.insert(
            "worker_run_id".to_owned(),
            serde_json::json!(authority.worker_run_id),
        );
        producer.insert(
            "worker_attempt_epoch".to_owned(),
            serde_json::json!(authority.worker_attempt_epoch),
        );
        producer.insert(
            "lease_token".to_owned(),
            serde_json::json!(authority.lease_token),
        );
        producer.insert(
            "source_tool_call_id".to_owned(),
            serde_json::json!(authority.source_tool_call_id),
        );
    }
    let audit_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO audit_log(
               action,category,details,project_path,source,status,detail,run_id,
               audit_role,evidence_technique,evidence_outcome
           ) VALUES('verification_directory_fingerprint_observed','tool_truth',
                    'Complete non-raw directory fingerprint witness',$1,
                    'tool_truth_receipt','completed',$2,$3,'evidence',$4,$5)
           RETURNING id"#,
    )
    .bind(&authority.project_path_at_freeze)
    .bind(serde_json::json!({"tool_truth_producer": producer}))
    .bind(authority.operation_id)
    .bind(&authority.technique)
    .bind(observation_state)
    .fetch_one(&mut **tx)
    .await?;
    let scope_version: i64 =
        sqlx::query_scalar("SELECT scope_rules_version FROM organizations WHERE id=$1")
            .bind(authority.organization_id)
            .fetch_one(&mut **tx)
            .await?;
    let classification_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO evidence_classifications(
               evidence_audit_id,classification,scope_version,reason,
               classified_by_session,producing_stage_run_id
           ) VALUES($1,'in_scope',$2,'sealed directory fingerprint witness',
                    'verification_action_receipt',$3)
           RETURNING id"#,
    )
    .bind(audit_id)
    .bind(scope_version)
    .bind(authority.stage_execution_id)
    .fetch_one(&mut **tx)
    .await?;
    let production_binding_id = Uuid::new_v5(
        &command.capability_execution_receipt_id,
        b"verification-fingerprint-evidence-production.v1",
    );
    let placeholder_hash =
        json_hash_on(tx, &serde_json::json!({"untrusted": "server_recomputes"})).await?;
    sqlx::query(
        r#"INSERT INTO tool_truth_evidence_production_bindings(
               id,execution_authority_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,
               evidence_audit_id,evidence_classification_id,production_binding_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
    )
    .bind(production_binding_id)
    .bind(authority.execution_authority_id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(&authority.stage_kind)
    .bind(&authority.execution_authority_hash)
    .bind(audit_id)
    .bind(classification_id)
    .bind(&placeholder_hash)
    .execute(&mut **tx)
    .await?;
    let evidence_authority_id = Uuid::new_v5(
        &command.capability_execution_receipt_id,
        b"verification-fingerprint-evidence-authority.v1",
    );
    sqlx::query(
        r#"INSERT INTO tool_truth_evidence_authorities(
               id,production_binding_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,stage_execution_id,stage_kind,execution_authority_hash,
               evidence_audit_id,evidence_classification_id,audit_row_hash,
               classification_row_hash,evidence_chain_hash,authority_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$14,$14,$14)"#,
    )
    .bind(evidence_authority_id)
    .bind(production_binding_id)
    .bind(authority.execution_authority_id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(&authority.stage_kind)
    .bind(&authority.execution_authority_hash)
    .bind(audit_id)
    .bind(classification_id)
    .bind(&placeholder_hash)
    .execute(&mut **tx)
    .await?;

    let input_id = Uuid::new_v5(
        &command.capability_execution_receipt_id,
        b"verification-fingerprint-input.v1",
    );
    sqlx::query(
        r#"INSERT INTO capability_execution_receipt_inputs(
               id,receipt_id,denominator_id,denominator_item_id,execution_authority_id,
               input_key,attempt_state,landing_state,observation_state,coverage_extent,
               coverage_gap_reason
           ) VALUES($1,$2,$3,$4,$5,$6,'succeeded','committed',$7,'complete','none')"#,
    )
    .bind(input_id)
    .bind(command.capability_execution_receipt_id)
    .bind(authority.denominator_id)
    .bind(authority.denominator_item_id)
    .bind(authority.execution_authority_id)
    .bind(&authority.input_key)
    .bind(observation_state)
    .execute(&mut **tx)
    .await?;
    let input_member_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "input_key": &authority.input_key,
            "technique": &authority.technique,
            "evidence_authority_id": evidence_authority_id,
            "observation_hash": observation_hash,
            "observation_state": observation_state,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO capability_execution_input_evidence_members(
               id,input_id,receipt_id,denominator_item_id,execution_authority_id,
               evidence_authority_id,ordinal,member_hash
           ) VALUES($1,$2,$3,$4,$5,$6,0,$7)"#,
    )
    .bind(Uuid::new_v5(&input_id, b"fingerprint-evidence:v1"))
    .bind(input_id)
    .bind(command.capability_execution_receipt_id)
    .bind(authority.denominator_item_id)
    .bind(authority.execution_authority_id)
    .bind(evidence_authority_id)
    .bind(input_member_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE capability_execution_receipt_inputs SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(input_id)
    .execute(&mut **tx)
    .await?;

    let policy_member: (Uuid, String, i64, i64) = sqlx::query_as(
        r#"SELECT id,member_hash,positive_ttl_ms,negative_ttl_ms
             FROM evidence_temporal_validity_policy_members
            WHERE policy_id=$1 AND fact_class='target_state' FOR SHARE"#,
    )
    .bind(authority.temporal_validity_policy_id)
    .fetch_one(&mut **tx)
    .await?;
    let target_scope_identity_hash: String = sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(jsonb_build_object(
               'operation_id',$1::uuid,'organization_id',$2::uuid,
               'target_id',$3::uuid,'exact_asset',$4::text
           )::TEXT)"#,
    )
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(authority.target_id)
    .bind(&authority.exact_asset)
    .fetch_one(&mut **tx)
    .await?;
    let genesis_event_id = Uuid::new_v5(
        &authority.operation_id,
        format!("target-state-epoch:{target_scope_identity_hash}:0").as_bytes(),
    );
    sqlx::query(
        r#"INSERT INTO tool_truth_target_state_epoch_events(
               id,operation_id,project_scope_id,project_path_at_freeze,
               scope_snapshot_id,organization_id,target_scope_identity_hash,
               epoch,predecessor_event_id,reason_code,source_authority_hash,event_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,0,NULL,'initial_observation',$8,$7)
           ON CONFLICT(operation_id,organization_id,target_scope_identity_hash,epoch)
           DO NOTHING"#,
    )
    .bind(genesis_event_id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(&target_scope_identity_hash)
    .bind(&authority.execution_authority_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"INSERT INTO tool_truth_target_state_epoch_heads(
               operation_id,organization_id,target_scope_identity_hash,
               current_epoch,current_event_id
           ) VALUES($1,$2,$3,0,$4)
           ON CONFLICT(operation_id,organization_id,target_scope_identity_hash)
           DO NOTHING"#,
    )
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(&target_scope_identity_hash)
    .bind(genesis_event_id)
    .execute(&mut **tx)
    .await?;
    let (target_state_epoch, target_state_epoch_event_id): (i64, Uuid) = sqlx::query_as(
        r#"SELECT current_epoch,current_event_id
             FROM tool_truth_target_state_epoch_heads
            WHERE operation_id=$1 AND organization_id=$2
              AND target_scope_identity_hash=$3 FOR SHARE"#,
    )
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(&target_scope_identity_hash)
    .fetch_one(&mut **tx)
    .await?;
    let temporal_census_id = Uuid::new_v5(
        &command.capability_execution_receipt_id,
        b"verification-fingerprint-temporal-census.v1",
    );
    let selected_ttl_ms = if observation_state == "found" {
        policy_member.2
    } else {
        policy_member.3
    };
    sqlx::query(
        r#"INSERT INTO capability_execution_temporal_censuses(
               id,receipt_id,execution_authority_id,receipt_authority_hash,
               temporal_validity_policy_id,temporal_validity_policy_hash,
               observation_window_started_at,observation_window_completed_at,
               effective_valid_until,target_state_epoch_set_hash
           ) SELECT $1,receipt.id,receipt.execution_authority_id,
                    receipt.receipt_authority_hash,receipt.temporal_validity_policy_id,
                    receipt.temporal_validity_policy_hash,$3,statement_timestamp(),
                    statement_timestamp()+$4*INTERVAL '1 millisecond',$5
               FROM capability_execution_receipts receipt WHERE receipt.id=$2"#,
    )
    .bind(temporal_census_id)
    .bind(command.capability_execution_receipt_id)
    .bind(authority.observation_started_at)
    .bind(selected_ttl_ms)
    .bind(
        json_hash_on(
            tx,
            &serde_json::json!([{
                "input_key": &authority.input_key,
                "target_scope_identity_hash": &target_scope_identity_hash,
                "target_state_epoch_event_id": target_state_epoch_event_id,
                "target_state_epoch": target_state_epoch,
            }]),
        )
        .await?,
    )
    .execute(&mut **tx)
    .await?;
    let polarity = if observation_state == "found" {
        "positive"
    } else {
        "negative"
    };
    let temporal_member_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "receipt_id": command.capability_execution_receipt_id,
            "input_key": &authority.input_key,
            "observation_hash": observation_hash,
            "polarity": polarity,
            "target_state_epoch_event_id": target_state_epoch_event_id,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO capability_execution_temporal_census_members(
               id,census_id,receipt_id,execution_authority_id,ordinal,input_key,
               observation_identity_hash,temporal_validity_policy_id,
               policy_member_id,policy_member_hash,target_state_operation_id,
               target_state_organization_id,target_scope_identity_hash,
               target_state_epoch_event_id,target_state_epoch,temporal_fact_class,
               observation_polarity,mapping_rule_id,mapping_rule_version,
               mapping_rule_digest,selected_ttl_ms,observed_at,
               effective_valid_until,member_hash
           ) VALUES($1,$2,$3,$4,0,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,
                    'target_state',$15,'verification_fingerprint.policy_v1','1',$16,$17,
                    statement_timestamp(),
                    statement_timestamp()+$17*INTERVAL '1 millisecond',$18)"#,
    )
    .bind(Uuid::new_v5(
        &temporal_census_id,
        authority.input_key.as_bytes(),
    ))
    .bind(temporal_census_id)
    .bind(command.capability_execution_receipt_id)
    .bind(authority.execution_authority_id)
    .bind(&authority.input_key)
    .bind(observation_hash)
    .bind(authority.temporal_validity_policy_id)
    .bind(policy_member.0)
    .bind(&policy_member.1)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(&target_scope_identity_hash)
    .bind(target_state_epoch_event_id)
    .bind(target_state_epoch)
    .bind(polarity)
    .bind(
        json_hash_on(
            tx,
            &serde_json::json!({"rule": "verification_fingerprint.policy_v1", "version": 1}),
        )
        .await?,
    )
    .bind(selected_ttl_ms)
    .bind(temporal_member_hash)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "UPDATE capability_execution_temporal_censuses SET sealed_at=statement_timestamp() WHERE id=$1",
    )
    .bind(temporal_census_id)
    .execute(&mut **tx)
    .await?;

    let semantic_authority_version = authority.semantic_authority_version + 1;
    let reconciliation_id = Uuid::new_v5(
        &command.capability_execution_receipt_id,
        format!("verification-fingerprint-reconciliation:{semantic_authority_version}").as_bytes(),
    );
    sqlx::query(
        r#"INSERT INTO capability_execution_reconciliations(
               id,receipt_id,execution_authority_id,semantic_authority_version,
               predecessor_reconciliation_id,reconciliation_state
           ) VALUES($1,$2,$3,$4,$5,'pending')"#,
    )
    .bind(reconciliation_id)
    .bind(command.capability_execution_receipt_id)
    .bind(authority.execution_authority_id)
    .bind(semantic_authority_version)
    .bind(authority.predecessor_reconciliation_id)
    .execute(&mut **tx)
    .await?;
    let reconciliation_member_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "source_kind": "evidence",
            "evidence_authority_id": evidence_authority_id,
            "receipt_id": command.capability_execution_receipt_id,
            "observation_hash": observation_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO capability_execution_reconciliation_members(
               id,reconciliation_id,receipt_id,execution_authority_id,ordinal,
               source_kind,evidence_authority_id,member_hash
           ) VALUES($1,$2,$3,$4,0,'evidence',$5,$6)"#,
    )
    .bind(Uuid::new_v5(&reconciliation_id, b"fingerprint-evidence:v1"))
    .bind(reconciliation_id)
    .bind(command.capability_execution_receipt_id)
    .bind(authority.execution_authority_id)
    .bind(evidence_authority_id)
    .bind(reconciliation_member_hash)
    .execute(&mut **tx)
    .await?;
    let semantic_reconciliation_hash: String = sqlx::query_scalar(
        r#"UPDATE capability_execution_reconciliations
              SET reconciliation_state='consistent',observed_artifact_sha256=$2,
                  observed_artifact_byte_count=$3,sealed_at=statement_timestamp()
            WHERE id=$1 RETURNING semantic_reconciliation_hash"#,
    )
    .bind(reconciliation_id)
    .bind(observation_hash)
    .bind(snapshot_byte_count)
    .fetch_one(&mut **tx)
    .await?;
    Ok(DirectoryFingerprintToolTruthAuthorityV1 {
        temporal_census_id,
        reconciliation_id,
        semantic_authority_version,
        semantic_reconciliation_hash,
    })
}

/// Finalization preserves the exact host observation in Tool Truth. Generic
/// and failed observations remain metadata-only; the directory-fingerprint
/// contract is accepted as complete only after this repository independently
/// revalidates its four-request shape, same-origin control paths, full-body
/// hashes, control consistency and claimed verdict.
pub async fn finalize_verification_action_capability_receipt(
    pool: &PgPool,
    command: &FinalizeVerificationActionCapabilityReceipt,
) -> Result<String> {
    let mut tx = pool.begin().await?;
    let result =
        finalize_verification_action_capability_receipt_in_transaction(&mut tx, command).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn finalize_verification_action_capability_receipt_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    command: &FinalizeVerificationActionCapabilityReceipt,
) -> Result<String> {
    if !matches!(
        command.terminal_state.as_str(),
        "succeeded" | "failed" | "outcome_unknown"
    ) || !command.observation.is_object()
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let witness = classify_verification_action_witness(
        command.prepared_action_id,
        &command.terminal_state,
        &command.observation,
    )?;
    let witness_completeness = witness.witness_completeness();
    let binding: (Uuid, Uuid, String, i64, Option<DateTime<Utc>>) = sqlx::query_as(
        r#"SELECT binding.binding_id,binding.execution_authority_id,
                  receipt.attempt_state,receipt.row_version,receipt.finalized_at
             FROM verification_action_capability_receipt_bindings binding
             JOIN capability_execution_receipts receipt
               ON receipt.id=binding.capability_execution_receipt_id
              AND receipt.denominator_id=binding.derived_denominator_id
              AND receipt.execution_authority_id=binding.execution_authority_id
            WHERE binding.action_execution_id=$1 AND binding.prepared_action_id=$2
              AND binding.capability_execution_receipt_id=$3 FOR UPDATE OF receipt"#,
    )
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .bind(command.capability_execution_receipt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let observation_hash = json_hash_on(tx, &command.observation).await?;
    let finalization_hash = json_hash_on(
        tx,
        &serde_json::json!({
            "binding_id": binding.0,
            "action_execution_id": command.action_execution_id,
            "prepared_action_id": command.prepared_action_id,
            "capability_execution_receipt_id": command.capability_execution_receipt_id,
            "terminal_state": &command.terminal_state,
            "witness_completeness": witness_completeness,
            "observation_hash": &observation_hash,
        }),
    )
    .await?;
    if binding.4.is_some() {
        let existing: Option<(String, String)> = sqlx::query_as(
            r#"SELECT observation_hash,finalization_hash
                 FROM verification_action_capability_receipt_finalizations
                WHERE stable_request_id=$1"#,
        )
        .bind(command.stable_request_id)
        .fetch_optional(&mut **tx)
        .await?;
        if existing
            .as_ref()
            .is_some_and(|(stored_observation, stored_finalization)| {
                stored_observation == &observation_hash && stored_finalization == &finalization_hash
            })
        {
            return Ok(finalization_hash);
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    let typed_landing = serde_json::json!({
        "contract_version": "verification-action-observation.v1",
        "witness_completeness": witness_completeness,
        "observation_hash": &observation_hash,
        "observation": &command.observation,
    });
    let (
        landing_state,
        observation_state,
        coverage_extent,
        coverage_gap_reason,
        security_interpretation,
    ) = match witness {
        VerificationActionWitnessV1::DirectoryFingerprint(
            DirectoryFingerprintOracleVerdictV1::Proof,
        ) => ("committed", "found", "sampled", "none", "signal"),
        VerificationActionWitnessV1::DirectoryFingerprint(
            DirectoryFingerprintOracleVerdictV1::Refutation,
        ) => ("committed", "no_match", "sampled", "none", "signal"),
        VerificationActionWitnessV1::DirectoryFingerprint(
            DirectoryFingerprintOracleVerdictV1::Inconclusive,
        ) => (
            "committed",
            "indeterminate",
            "sampled",
            "none",
            "inconclusive",
        ),
        VerificationActionWitnessV1::MetadataOnly => (
            if command.terminal_state == "succeeded" {
                "partial"
            } else {
                "failed"
            },
            "indeterminate",
            "partial",
            if command.terminal_state == "outcome_unknown" {
                "transport"
            } else {
                "unsupported"
            },
            "inconclusive",
        ),
    };
    let fingerprint_authority = if matches!(
        witness,
        VerificationActionWitnessV1::DirectoryFingerprint(_)
    ) {
        Some(
            seal_directory_fingerprint_tool_truth_authority_v1(
                tx,
                command,
                &observation_hash,
                &finalization_hash,
                observation_state,
            )
            .await?,
        )
    } else {
        None
    };
    let updated = sqlx::query(
        r#"UPDATE capability_execution_receipts
              SET attempt_state=$1,landing_state=$2,observation_state=$3,
                  coverage_extent=$4,coverage_gap_reason=$5,
                  reconciliation_state='consistent',security_interpretation=$6,
                  typed_landing=$7,finalization_request_hash=$8,row_version=row_version+1,
                  temporal_census_id=$9,
                  current_semantic_authority_version=COALESCE($10,current_semantic_authority_version),
                  current_semantic_reconciliation_id=$11,
                  current_semantic_reconciliation_hash=$12,
                  observation_completed_at=statement_timestamp(),
                  valid_until=COALESCE(
                      (SELECT effective_valid_until
                         FROM capability_execution_temporal_censuses WHERE id=$9),
                      statement_timestamp()+INTERVAL '60 seconds'
                  ),
                  finalized_at=statement_timestamp()
            WHERE id=$13 AND row_version=$14 AND finalized_at IS NULL"#,
    )
    .bind(&command.terminal_state)
    .bind(landing_state)
    .bind(observation_state)
    .bind(coverage_extent)
    .bind(coverage_gap_reason)
    .bind(security_interpretation)
    .bind(&typed_landing)
    .bind(&finalization_hash)
    .bind(fingerprint_authority.as_ref().map(|value| value.temporal_census_id))
    .bind(
        fingerprint_authority
            .as_ref()
            .map(|value| value.semantic_authority_version),
    )
    .bind(
        fingerprint_authority
            .as_ref()
            .map(|value| value.reconciliation_id),
    )
    .bind(
        fingerprint_authority
            .as_ref()
            .map(|value| value.semantic_reconciliation_hash.as_str()),
    )
    .bind(command.capability_execution_receipt_id)
    .bind(binding.3)
    .execute(&mut **tx)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(conflict(AUTHORITY_STALE));
    }
    sqlx::query(
        r#"INSERT INTO verification_action_capability_receipt_finalizations(
               finalization_id,stable_request_id,binding_id,action_execution_id,
               prepared_action_id,capability_execution_receipt_id,terminal_state,
               witness_completeness,observation_hash,finalization_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
    )
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"verification-action-tool-truth-finalization.v1",
    ))
    .bind(command.stable_request_id)
    .bind(binding.0)
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .bind(command.capability_execution_receipt_id)
    .bind(&command.terminal_state)
    .bind(witness_completeness)
    .bind(&observation_hash)
    .bind(&finalization_hash)
    .execute(&mut **tx)
    .await?;
    Ok(finalization_hash)
}

#[derive(Debug, Clone)]
pub struct CloseoutActionExecution {
    pub action_execution_id: Uuid,
    pub prepared_action_id: Uuid,
    pub capability_execution_receipt_id: Uuid,
    pub state: String,
    pub closeout_body: Value,
    pub residual_id: Option<Uuid>,
    pub cleanup_complete: bool,
    pub budget_actuals: Vec<BudgetActualAxis>,
}

#[derive(Debug, Clone)]
pub struct FinalizeVerificationActionSemanticLanding {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub authorization_receipt_id: Uuid,
    pub action_execution_id: Uuid,
    pub capability_execution_receipt_id: Uuid,
    pub terminal_state: String,
    pub observation: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationActionSemanticLanding {
    pub oracle_assessment_id: Uuid,
    pub residual_id: Option<Uuid>,
    pub closeout_hash: String,
    pub execution_row_version: i64,
    pub terminal_state: String,
    pub replayed: bool,
}

/// Atomically lands the bounded Tool Truth observation, typed residual,
/// deterministic Oracle and execution closeout after external I/O. No network
/// call is made while this transaction is open. The terminal execution row is
/// therefore the commit marker for the complete semantic landing.
pub async fn finalize_verification_action_semantic_landing(
    pool: &PgPool,
    command: &FinalizeVerificationActionSemanticLanding,
) -> Result<VerificationActionSemanticLanding> {
    if [
        command.stable_request_id,
        command.operation_id,
        command.campaign_id,
        command.prepared_action_id,
        command.authorization_receipt_id,
        command.action_execution_id,
        command.capability_execution_receipt_id,
    ]
    .into_iter()
    .any(|id| id.is_nil())
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "verification-action-semantic-landing:{}",
            command.action_execution_id
        ))
        .execute(&mut *tx)
        .await?;
    let initial_state: String = sqlx::query_scalar(
        r#"SELECT state FROM verification_action_executions
            WHERE action_execution_id=$1 AND prepared_action_id=$2
              AND authorization_receipt_id=$3 AND operation_id=$4 FOR UPDATE"#,
    )
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .bind(command.authorization_receipt_id)
    .bind(command.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;

    finalize_verification_action_capability_receipt_in_transaction(
        &mut tx,
        &FinalizeVerificationActionCapabilityReceipt {
            stable_request_id: Uuid::new_v5(
                &command.stable_request_id,
                b"verification-action-capability-receipt-finalize.v1",
            ),
            action_execution_id: command.action_execution_id,
            prepared_action_id: command.prepared_action_id,
            capability_execution_receipt_id: command.capability_execution_receipt_id,
            terminal_state: command.terminal_state.clone(),
            observation: command.observation.clone(),
        },
    )
    .await?;

    #[derive(sqlx::FromRow)]
    struct LandingAuthority {
        project_scope_id: Uuid,
        organization_id: Uuid,
        action_kind: String,
        campaign_coverage_member_id: Uuid,
        control_binding_kind: String,
        expected_oracle_kind: String,
        oracle_contract_version: String,
        oracle_contract_hash: String,
        observation_receipt_hash: String,
        receipt_attempt_state: String,
        typed_landing: Value,
        budget_reservation_id: Uuid,
    }
    #[derive(Clone, sqlx::FromRow)]
    struct LandingOracleMember {
        campaign_coverage_member_id: Uuid,
        control_binding_kind: String,
        expected_oracle_kind: String,
    }
    let authority = sqlx::query_as::<_, LandingAuthority>(
        r#"SELECT action.project_scope_id,action.organization_id,action.action_kind,
                  member.campaign_coverage_member_id,member.control_binding_kind,
                  member.expected_oracle_kind,
                  action.private_manifest #>> '{oracle_contract_version}' AS oracle_contract_version,
                  action.oracle_contract_hash,
                  receipt.receipt_authority_hash AS observation_receipt_hash,
                  receipt.attempt_state AS receipt_attempt_state,receipt.typed_landing,
                  execution.budget_reservation_id
             FROM verification_prepared_actions action
             JOIN verification_action_executions execution
               ON execution.action_execution_id=$1
              AND execution.prepared_action_id=action.prepared_action_id
              AND execution.authorization_receipt_id=$2
             JOIN verification_campaign_coverage_denominators denominator
               ON denominator.campaign_id=action.campaign_id AND denominator.sealed_at IS NOT NULL
             JOIN verification_campaign_coverage_members member
               ON member.campaign_denominator_id=denominator.campaign_denominator_id
              AND member.member_hash=action.private_manifest #>> '{coverage_member_hash}'
             JOIN capability_execution_receipts receipt
               ON receipt.id=$3 AND receipt.finalized_at IS NOT NULL
            WHERE action.prepared_action_id=$4 AND action.operation_id=$5
              AND action.campaign_id=$6"#,
    )
    .bind(command.action_execution_id)
    .bind(command.authorization_receipt_id)
    .bind(command.capability_execution_receipt_id)
    .bind(command.prepared_action_id)
    .bind(command.operation_id)
    .bind(command.campaign_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    if authority.receipt_attempt_state != command.terminal_state
        || !matches!(
            authority.receipt_attempt_state.as_str(),
            "succeeded" | "failed" | "outcome_unknown"
        )
    {
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }

    // A Campaign intentionally owns one active action lane, while one sealed
    // strategy may contain several component obligations observed by that
    // same request. Bind the single immutable observation to every exact
    // applied coverage member. Legacy/direct repository callers retain the
    // manifest-bound member as a one-member fallback.
    let mut oracle_members = sqlx::query_as::<_, LandingOracleMember>(
        r#"SELECT member.campaign_coverage_member_id,member.control_binding_kind,
                  member.expected_oracle_kind
             FROM verification_prepared_actions landing_action
             JOIN investigation_verification_task_advisory_receipts advisory
               ON advisory.operation_id=landing_action.operation_id
             JOIN investigation_verification_task_advisory_seals advisory_seal
               ON advisory_seal.advisory_receipt_id=advisory.advisory_receipt_id
             JOIN investigation_verification_advisory_campaign_applies apply
               ON apply.advisory_receipt_id=advisory.advisory_receipt_id
              AND apply.campaign_id=landing_action.campaign_id
              AND apply.strategy_artifact_id=landing_action.strategy_artifact_id
              AND apply.result_kind='prepared_action'
             JOIN verification_campaign_coverage_members member
               ON member.campaign_denominator_id=apply.campaign_denominator_id
              AND member.campaign_coverage_member_id=apply.campaign_coverage_member_id
            WHERE landing_action.campaign_id=$1
              AND landing_action.prepared_action_id=$2
              AND landing_action.operation_id=$3
              AND advisory.status='applied'
              AND advisory.applied_at IS NOT NULL
            ORDER BY (member.campaign_coverage_member_id=$4) DESC,
                     member.member_ordinal,member.campaign_coverage_member_id"#,
    )
    .bind(command.campaign_id)
    .bind(command.prepared_action_id)
    .bind(command.operation_id)
    .bind(authority.campaign_coverage_member_id)
    .fetch_all(&mut *tx)
    .await?;
    if oracle_members.is_empty() {
        oracle_members.push(LandingOracleMember {
            campaign_coverage_member_id: authority.campaign_coverage_member_id,
            control_binding_kind: authority.control_binding_kind.clone(),
            expected_oracle_kind: authority.expected_oracle_kind.clone(),
        });
    } else if oracle_members.first().is_none_or(|member| {
        member.campaign_coverage_member_id != authority.campaign_coverage_member_id
    }) {
        return Err(conflict(AUTHORITY_STALE));
    }

    let witness = classify_verification_action_witness(
        command.prepared_action_id,
        &command.terminal_state,
        &command.observation,
    )?;
    if matches!(
        witness,
        VerificationActionWitnessV1::DirectoryFingerprint(_)
    ) && (authority.action_kind != DIRECTORY_FINGERPRINT_CAPABILITY_V1
        || oracle_members
            .iter()
            .any(|member| member.expected_oracle_kind != "directory_soft404_fingerprint.v1"))
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let (oracle_verdict, precondition_validity, residual_spec, oracle_request_domain):
        VerificationOracleLandingPlanV1<'_> = match witness {
        VerificationActionWitnessV1::DirectoryFingerprint(
            DirectoryFingerprintOracleVerdictV1::Proof,
        ) => (
            "proof",
            "valid",
            None,
            b"verification-action-directory-fingerprint-oracle.v1",
        ),
        VerificationActionWitnessV1::DirectoryFingerprint(
            DirectoryFingerprintOracleVerdictV1::Refutation,
        ) => (
            "refutation",
            "valid",
            None,
            b"verification-action-directory-fingerprint-oracle.v1",
        ),
        VerificationActionWitnessV1::DirectoryFingerprint(
            DirectoryFingerprintOracleVerdictV1::Inconclusive,
        ) => (
            "inconclusive",
            "valid",
            Some((
                "directory_soft404_controls_inconsistent",
                b"verification-action-directory-controls-inconsistent.v1",
                serde_json::json!({
                    "kind": "repeat_directory_fingerprint_after_control_stability",
                    "oracle_contract_version": &authority.oracle_contract_version,
                }),
            )),
            b"verification-action-directory-fingerprint-oracle.v1",
        ),
        VerificationActionWitnessV1::MetadataOnly => (
            "inconclusive",
            "unknown",
            Some((
                "raw_witness_incomplete",
                b"verification-action-raw-witness-incomplete.v1",
                serde_json::json!({
                    "kind": "install_complete_capability_raw_witness_contract",
                    "oracle_contract_version": &authority.oracle_contract_version,
                }),
            )),
            b"verification-action-inconclusive-oracle.v1",
        ),
    };
    let residual_reason_code = residual_spec.as_ref().map(|spec| spec.0);
    let mut primary_oracle_assessment_id = None;
    let mut primary_residual_id = None;
    for (index, member) in oracle_members.iter().enumerate() {
        let member_request_id = if index == 0 {
            command.stable_request_id
        } else {
            Uuid::new_v5(
                &command.stable_request_id,
                format!("coverage-member:{}", member.campaign_coverage_member_id).as_bytes(),
            )
        };
        let affected_inputs = serde_json::json!([
            command.prepared_action_id,
            command.action_execution_id,
            command.capability_execution_receipt_id,
        ]);
        let residual_id =
            if let Some((reason_code, residual_domain, next_action)) = residual_spec.as_ref() {
                let residual_id = Uuid::new_v5(&member_request_id, residual_domain);
                let residual_hash = json_hash_on(
                    &mut tx,
                    &serde_json::json!({
                        "prepared_action_id": command.prepared_action_id,
                        "action_execution_id": command.action_execution_id,
                        "capability_execution_receipt_id": command.capability_execution_receipt_id,
                        "reason_code": reason_code,
                    }),
                )
                .await?;
                sqlx::query(
                    r#"INSERT INTO hypothesis_residual_risks(
                       residual_id,operation_id,organization_id,reason_code,owner_kind,
                       affected_inputs,next_action,residual_hash
                   ) VALUES($1,$2,$3,$4,'plan_c',$5,$6,$7)
                   ON CONFLICT(residual_id) DO NOTHING"#,
                )
                .bind(residual_id)
                .bind(command.operation_id)
                .bind(authority.organization_id)
                .bind(*reason_code)
                .bind(&affected_inputs)
                .bind(next_action)
                .bind(&residual_hash)
                .execute(&mut *tx)
                .await?;
                let stored_residual: (Uuid, Uuid, String, Value, Value, String) = sqlx::query_as(
                    r#"SELECT operation_id,organization_id,reason_code,affected_inputs,
                          next_action,residual_hash
                     FROM hypothesis_residual_risks WHERE residual_id=$1 FOR SHARE"#,
                )
                .bind(residual_id)
                .fetch_one(&mut *tx)
                .await?;
                if stored_residual
                    != (
                        command.operation_id,
                        authority.organization_id,
                        (*reason_code).to_owned(),
                        affected_inputs,
                        next_action.clone(),
                        residual_hash,
                    )
                {
                    return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
                }
                Some(residual_id)
            } else {
                None
            };
        let oracle_assessment_body = match witness {
            VerificationActionWitnessV1::MetadataOnly => serde_json::json!({
                "contract_version": "verification-action-oracle-assessment.v1",
                "witness_completeness": "metadata_only",
                "reason_code": "raw_witness_incomplete",
                "typed_landing": authority.typed_landing.clone(),
            }),
            VerificationActionWitnessV1::DirectoryFingerprint(verdict) => serde_json::json!({
                "contract_version": "verification-action-oracle-assessment.v1",
                "witness_completeness": DIRECTORY_FINGERPRINT_WITNESS_V1,
                "reason_code": residual_reason_code,
                "recomputed_verdict": verdict.oracle_value(),
                "typed_landing": authority.typed_landing.clone(),
            }),
        };
        let control_validity = match witness {
            VerificationActionWitnessV1::DirectoryFingerprint(
                DirectoryFingerprintOracleVerdictV1::Inconclusive,
            ) => "invalid",
            VerificationActionWitnessV1::DirectoryFingerprint(_) => {
                if member.control_binding_kind == "explicit_no_control" {
                    "not_required"
                } else {
                    "valid"
                }
            }
            VerificationActionWitnessV1::MetadataOnly => {
                if member.control_binding_kind == "explicit_no_control" {
                    "not_required"
                } else {
                    "not_assessed"
                }
            }
        };
        let oracle_assessment_id =
            super::verification_oracles::record_action_oracle_in_transaction(
                &mut tx,
                &super::verification_oracles::RecordActionOracle {
                    stable_request_id: Uuid::new_v5(&member_request_id, oracle_request_domain),
                    campaign_id: command.campaign_id,
                    prepared_action_id: command.prepared_action_id,
                    action_execution_id: command.action_execution_id,
                    campaign_coverage_member_id: member.campaign_coverage_member_id,
                    operation_id: command.operation_id,
                    project_scope_id: authority.project_scope_id,
                    organization_id: authority.organization_id,
                    oracle_revision_ordinal: i32::try_from(index + 1)
                        .map_err(|_| conflict(CONTRACT_INVALID))?,
                    oracle_contract_version: authority.oracle_contract_version.clone(),
                    oracle_contract_hash: authority.oracle_contract_hash.clone(),
                    observation_receipt_hash: authority.observation_receipt_hash.clone(),
                    precondition_validity: precondition_validity.to_owned(),
                    control_validity: control_validity.to_owned(),
                    verdict: oracle_verdict.to_owned(),
                    assessment_body: oracle_assessment_body,
                    residual_id,
                },
            )
            .await?;
        if index == 0 {
            primary_oracle_assessment_id = Some(oracle_assessment_id);
            primary_residual_id = residual_id;
        }
    }
    let oracle_assessment_id =
        primary_oracle_assessment_id.ok_or_else(|| conflict(AUTHORITY_STALE))?;
    let residual_id = primary_residual_id;

    let budget_actuals = sqlx::query_as::<_, (Uuid, String, i64)>(
        r#"SELECT reserve.ancestor_contract_id,reserve.axis_kind,
                  COALESCE(SUM(consumed.delta),0)::BIGINT AS actual
             FROM verification_budget_ledger_entries reserve
             LEFT JOIN verification_budget_ledger_entries consumed
               ON consumed.budget_reservation_id=reserve.budget_reservation_id
              AND consumed.ancestor_contract_id=reserve.ancestor_contract_id
              AND consumed.axis_kind=reserve.axis_kind
              AND consumed.entry_kind='consume'
            WHERE reserve.budget_reservation_id=$1 AND reserve.entry_kind='reserve'
            GROUP BY reserve.ancestor_contract_id,reserve.axis_kind
            ORDER BY reserve.ancestor_contract_id,reserve.axis_kind"#,
    )
    .bind(authority.budget_reservation_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(
        |(ancestor_contract_id, axis_kind, actual)| BudgetActualAxis {
            ancestor_contract_id,
            axis_kind,
            actual,
        },
    )
    .collect::<Vec<_>>();
    let closeout_body = serde_json::json!({
        "contract_version": "verification-action-closeout.v1",
        "action_execution_id": command.action_execution_id,
        "capability_execution_receipt_id": command.capability_execution_receipt_id,
        "receipt_authority_hash": authority.observation_receipt_hash,
        "typed_landing": authority.typed_landing,
    });
    let closeout_hash = closeout_action_execution_in_transaction(
        &mut tx,
        &CloseoutActionExecution {
            action_execution_id: command.action_execution_id,
            prepared_action_id: command.prepared_action_id,
            capability_execution_receipt_id: command.capability_execution_receipt_id,
            state: command.terminal_state.clone(),
            closeout_body,
            residual_id: None,
            cleanup_complete: true,
            budget_actuals,
        },
    )
    .await?;
    let execution_row_version: i64 = sqlx::query_scalar(
        "SELECT row_version FROM verification_action_executions WHERE action_execution_id=$1",
    )
    .bind(command.action_execution_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(VerificationActionSemanticLanding {
        oracle_assessment_id,
        residual_id,
        closeout_hash,
        execution_row_version,
        terminal_state: command.terminal_state.clone(),
        replayed: initial_state != "started",
    })
}

#[derive(Debug, Clone)]
pub struct BudgetActualAxis {
    pub ancestor_contract_id: Uuid,
    pub axis_kind: String,
    pub actual: i64,
}

pub async fn closeout_action_execution(
    pool: &PgPool,
    command: &CloseoutActionExecution,
) -> Result<String> {
    let mut tx = pool.begin().await?;
    let result = closeout_action_execution_in_transaction(&mut tx, command).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn closeout_action_execution_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    command: &CloseoutActionExecution,
) -> Result<String> {
    if !matches!(
        command.state.as_str(),
        "succeeded" | "failed" | "outcome_unknown"
    ) || command.residual_id.is_some()
        || (!command.cleanup_complete && command.state != "outcome_unknown")
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let mut actual_keys: Vec<(Uuid, &str)> = command
        .budget_actuals
        .iter()
        .map(|axis| (axis.ancestor_contract_id, axis.axis_kind.as_str()))
        .collect();
    actual_keys.sort_unstable();
    if command.budget_actuals.iter().any(|axis| axis.actual < 0)
        || actual_keys.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(conflict(CONTRACT_INVALID));
    }
    let closeout_hash = json_hash_on(tx, &command.closeout_body).await?;
    let execution: (Uuid, Uuid, String, Option<Uuid>, Option<String>) = sqlx::query_as(
        r#"SELECT budget_reservation_id,conflict_set_id,state,
                  capability_execution_receipt_id,closeout_hash
             FROM verification_action_executions
            WHERE action_execution_id=$1 AND prepared_action_id=$2 FOR UPDATE"#,
    )
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    if execution.2 != "started" {
        if execution.2 == command.state
            && execution.3 == Some(command.capability_execution_receipt_id)
            && execution.4.as_deref() == Some(closeout_hash.as_str())
        {
            return Ok(closeout_hash);
        }
        return Err(conflict(super::verification_campaigns::REPLAY_DRIFT));
    }
    let unknown = command.state == "outcome_unknown" || !command.cleanup_complete;
    let group_census: (String, i64, i64, i64) = sqlx::query_as(
        r#"SELECT action.action_contract_kind,
                  COUNT(DISTINCT member.group_member_id),
                  COUNT(DISTINCT sub.action_subexecution_id),
                  COUNT(DISTINCT sub.action_subexecution_id) FILTER (
                      WHERE sub.started_at>sub.barrier_released_at
                            +(member.expected_start_window_ms*INTERVAL '1 millisecond')
                  )
             FROM verification_prepared_actions action
             LEFT JOIN verification_prepared_action_group_members member
               ON member.prepared_action_id=action.prepared_action_id
             LEFT JOIN verification_action_subexecutions sub
               ON sub.action_execution_id=$2
              AND sub.prepared_action_id=action.prepared_action_id
              AND sub.group_member_id=member.group_member_id
            WHERE action.prepared_action_id=$1
            GROUP BY action.action_contract_kind"#,
    )
    .bind(command.prepared_action_id)
    .bind(command.action_execution_id)
    .fetch_one(&mut **tx)
    .await?;
    let exact_group = if group_census.0 == "single_action_v1" {
        group_census.1 == 0 && group_census.2 == 0
    } else {
        group_census.1 >= 2 && group_census.1 == group_census.2 && group_census.3 == 0
    };
    if !unknown && !exact_group {
        return Err(conflict(AUTHORITY_STALE));
    }
    let reserve_entries: Vec<(Uuid, String, i64, i64)> = sqlx::query_as(
        r#"SELECT reserve.ancestor_contract_id,reserve.axis_kind,reserve.delta,
                  COALESCE(SUM(consumed.delta),0)::BIGINT AS consumed_by_reservation
             FROM verification_budget_ledger_entries reserve
             LEFT JOIN verification_budget_ledger_entries consumed
               ON consumed.budget_reservation_id=reserve.budget_reservation_id
              AND consumed.ancestor_contract_id=reserve.ancestor_contract_id
              AND consumed.axis_kind=reserve.axis_kind
              AND consumed.entry_kind='consume'
            WHERE reserve.budget_reservation_id=$1 AND reserve.entry_kind='reserve'
            GROUP BY reserve.ancestor_contract_id,reserve.axis_kind,reserve.delta
            ORDER BY reserve.ancestor_contract_id,reserve.axis_kind"#,
    )
    .bind(execution.0)
    .fetch_all(&mut **tx)
    .await?;
    if !unknown && reserve_entries.len() != command.budget_actuals.len() {
        return Err(conflict(CONTRACT_INVALID));
    }
    for (contract_id, axis_kind, reserved_amount, consumed_by_reservation) in reserve_entries {
        if consumed_by_reservation < 0 || consumed_by_reservation > reserved_amount {
            return Err(conflict(AUTHORITY_STALE));
        }
        let remaining_reservation = reserved_amount - consumed_by_reservation;
        let actual = if unknown {
            consumed_by_reservation
        } else {
            command
                .budget_actuals
                .iter()
                .find(|item| {
                    item.ancestor_contract_id == contract_id && item.axis_kind == axis_kind
                })
                .map(|item| item.actual)
                .ok_or_else(|| conflict(CONTRACT_INVALID))?
        };
        if actual < consumed_by_reservation || actual > reserved_amount {
            return Err(conflict(CONTRACT_INVALID));
        }
        let additional_consumed = actual - consumed_by_reservation;
        let head: (i64, i64, i64, i64) = sqlx::query_as(
            r#"SELECT consumed,reserved,unknown_held,row_version
                 FROM verification_budget_scope_heads
                WHERE budget_contract_id=$1 AND axis_kind=$2 FOR UPDATE"#,
        )
        .bind(contract_id)
        .bind(&axis_kind)
        .fetch_one(&mut **tx)
        .await?;
        if head.1 < remaining_reservation {
            return Err(conflict(AUTHORITY_STALE));
        }
        let resulting_consumed = head.0 + if unknown { 0 } else { additional_consumed };
        let resulting_reserved = head.1 - remaining_reservation;
        let resulting_unknown = head.2 + if unknown { remaining_reservation } else { 0 };
        let resulting_hash = json_hash_on(
            tx,
            &serde_json::json!({
                "contract_id": contract_id,
                "axis_kind": axis_kind,
                "consumed": resulting_consumed,
                "reserved": resulting_reserved,
                "unknown_held": resulting_unknown,
                "row_version": head.3 + 1,
            }),
        )
        .await?;
        let entry_ordinal: i64 = sqlx::query_scalar(
            r#"SELECT COALESCE(MAX(entry_ordinal),0)+1
                 FROM verification_budget_ledger_entries
                WHERE budget_reservation_id=$1 AND ancestor_contract_id=$2
                  AND axis_kind=$3"#,
        )
        .bind(execution.0)
        .bind(contract_id)
        .bind(&axis_kind)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            r#"INSERT INTO verification_budget_ledger_entries(
                   budget_ledger_entry_id,budget_reservation_id,ancestor_contract_id,
                   axis_kind,entry_ordinal,entry_kind,delta,resulting_consumed,
                   resulting_reserved,resulting_unknown_held,expected_head_row_version,
                   resulting_head_hash,fence
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
        )
        .bind(Uuid::new_v5(
            &command.action_execution_id,
            format!("{contract_id}:{axis_kind}:closeout").as_bytes(),
        ))
        .bind(execution.0)
        .bind(contract_id)
        .bind(&axis_kind)
        .bind(entry_ordinal)
        .bind(if unknown { "hold_unknown" } else { "settle" })
        .bind(if unknown {
            remaining_reservation
        } else {
            additional_consumed
        })
        .bind(resulting_consumed)
        .bind(resulting_reserved)
        .bind(resulting_unknown)
        .bind(head.3)
        .bind(resulting_hash)
        .bind(head.3 + 1)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            r#"UPDATE verification_budget_scope_heads
                  SET consumed=$1,reserved=$2,unknown_held=$3,row_version=row_version+1,
                      updated_at=statement_timestamp()
                WHERE budget_contract_id=$4 AND axis_kind=$5 AND row_version=$6"#,
        )
        .bind(resulting_consumed)
        .bind(resulting_reserved)
        .bind(resulting_unknown)
        .bind(contract_id)
        .bind(&axis_kind)
        .bind(head.3)
        .execute(&mut **tx)
        .await?;
    }
    let action_scope: (Uuid, Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT campaign_id,operation_id,project_scope_id,organization_id
             FROM verification_prepared_actions WHERE prepared_action_id=$1 FOR SHARE"#,
    )
    .bind(command.prepared_action_id)
    .fetch_one(&mut **tx)
    .await?;
    let conflict_keys: Vec<(String, String)> = sqlx::query_as(
        r#"SELECT member.key_kind,member.key_identity_hash
             FROM verification_action_conflict_set_members member
            WHERE member.conflict_set_id=$1 ORDER BY member.key_kind,member.key_identity_hash"#,
    )
    .bind(execution.1)
    .fetch_all(&mut **tx)
    .await?;
    for (key_kind, key_hash) in conflict_keys {
        let head: (i64, i64) = sqlx::query_as(
            r#"SELECT fencing_token,row_version FROM verification_conflict_key_heads
                WHERE operation_id=$1 AND organization_id=$2 AND key_kind=$3
                  AND key_identity_hash=$4 AND owner_prepared_action_id=$5 FOR UPDATE"#,
        )
        .bind(action_scope.1)
        .bind(action_scope.3)
        .bind(&key_kind)
        .bind(&key_hash)
        .bind(command.prepared_action_id)
        .fetch_one(&mut **tx)
        .await?;
        let new_fence = if unknown { head.0 + 1 } else { head.0 };
        let event_kind = if unknown { "recovery_hold" } else { "release" };
        let event_hash = json_hash_on(
            tx,
            &serde_json::json!({
                "action_execution_id": command.action_execution_id,
                "key_kind": key_kind,
                "key_identity_hash": key_hash,
                "event_kind": event_kind,
                "expected_fence": head.0,
                "new_fence": new_fence,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO verification_conflict_key_events(
                   conflict_event_id,operation_id,project_scope_id,organization_id,
                   key_kind,key_identity_hash,event_ordinal,event_kind,
                   expected_fencing_token,new_fencing_token,owner_campaign_id,
                   owner_prepared_action_id,reason_code,residual_id,event_hash
               ) SELECT $1,$2,$3,$4,$5,$6,COALESCE(MAX(event_ordinal),0)+1,$7,$8,$9,
                        $10,$11,$12,$13,$14
                   FROM verification_conflict_key_events prior
                  WHERE prior.operation_id=$2 AND prior.organization_id=$4
                    AND prior.key_kind=$5 AND prior.key_identity_hash=$6"#,
        )
        .bind(Uuid::new_v5(
            &command.action_execution_id,
            event_hash.as_bytes(),
        ))
        .bind(action_scope.1)
        .bind(action_scope.2)
        .bind(action_scope.3)
        .bind(&key_kind)
        .bind(&key_hash)
        .bind(event_kind)
        .bind(head.0)
        .bind(new_fence)
        .bind(action_scope.0)
        .bind(command.prepared_action_id)
        .bind(if unknown {
            "closeout_recovery_hold"
        } else {
            "closeout_release"
        })
        .bind(command.residual_id)
        .bind(&event_hash)
        .execute(&mut **tx)
        .await?;
        if unknown {
            sqlx::query(
                r#"UPDATE verification_conflict_key_heads
                      SET state='recovery_hold',fencing_token=$1,row_version=row_version+1,
                          updated_at=statement_timestamp()
                    WHERE operation_id=$2 AND organization_id=$3 AND key_kind=$4
                      AND key_identity_hash=$5 AND row_version=$6"#,
            )
            .bind(new_fence)
            .bind(action_scope.1)
            .bind(action_scope.3)
            .bind(&key_kind)
            .bind(&key_hash)
            .bind(head.1)
            .execute(&mut **tx)
            .await?;
        } else {
            sqlx::query(
                r#"UPDATE verification_conflict_key_heads
                      SET state='free',owner_campaign_id=NULL,owner_prepared_action_id=NULL,
                          expires_at=NULL,row_version=row_version+1,
                          updated_at=statement_timestamp()
                    WHERE operation_id=$1 AND organization_id=$2 AND key_kind=$3
                      AND key_identity_hash=$4 AND row_version=$5"#,
            )
            .bind(action_scope.1)
            .bind(action_scope.3)
            .bind(&key_kind)
            .bind(&key_hash)
            .bind(head.1)
            .execute(&mut **tx)
            .await?;
        }
    }
    sqlx::query(
        r#"UPDATE verification_budget_reservations
              SET state=$1,row_version=row_version+1,settled_at=statement_timestamp()
            WHERE budget_reservation_id=$2 AND state='active'"#,
    )
    .bind(if unknown { "unknown_held" } else { "settled" })
    .bind(execution.0)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"UPDATE verification_action_executions
              SET state=$1,capability_execution_receipt_id=$2,closeout_hash=$3,
                  row_version=row_version+1,completed_at=statement_timestamp()
            WHERE action_execution_id=$4 AND state='started'"#,
    )
    .bind(&command.state)
    .bind(command.capability_execution_receipt_id)
    .bind(&closeout_hash)
    .bind(command.action_execution_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        r#"UPDATE verification_prepared_actions
              SET state=$1,reason_code=$2,residual_id=$3,row_version=row_version+1,
                  terminal_at=CASE WHEN $1='outcome_unknown' THEN NULL
                                   ELSE statement_timestamp() END
            WHERE prepared_action_id=$4 AND state='started'"#,
    )
    .bind(&command.state)
    .bind(if unknown {
        Some("outcome_unknown")
    } else {
        Some("execution_closed")
    })
    .bind(command.residual_id)
    .bind(command.prepared_action_id)
    .execute(&mut **tx)
    .await?;
    Ok(closeout_hash)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverUnknownActionDisposition {
    OutcomeUnknown,
    ReconciledSucceeded,
    ReconciledFailed,
    ManuallyBlocked,
}

impl RecoverUnknownActionDisposition {
    fn as_str(self) -> &'static str {
        match self {
            Self::OutcomeUnknown => "outcome_unknown",
            Self::ReconciledSucceeded => "reconciled_succeeded",
            Self::ReconciledFailed => "reconciled_failed",
            Self::ManuallyBlocked => "manually_blocked",
        }
    }

    fn execution_state(self) -> &'static str {
        match self {
            Self::ReconciledSucceeded => "succeeded",
            Self::ReconciledFailed => "failed",
            Self::OutcomeUnknown | Self::ManuallyBlocked => "outcome_unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecoverUnknownAction {
    pub stable_request_id: Uuid,
    pub operation_id: Uuid,
    pub campaign_id: Uuid,
    pub prepared_action_id: Uuid,
    pub action_execution_id: Uuid,
    pub disposition: RecoverUnknownActionDisposition,
    pub expected_execution_row_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableUnknownActionRecovery {
    pub recovery_receipt_id: Uuid,
    pub recovery_hash: String,
    pub execution_row_version: i64,
    pub replayed: bool,
}

/// Reconcile an outcome-unknown action without ever reopening its network
/// authority.  A positive/negative reconciliation consumes the full remaining
/// unknown upper-bound (the conservative accounting rule); a manual block
/// releases conflict leases but retains the unknown budget hold and records a
/// residual.  Merely re-observing `outcome_unknown` changes no mutable head.
pub async fn recover_unknown_action(
    pool: &PgPool,
    command: &RecoverUnknownAction,
) -> Result<DurableUnknownActionRecovery> {
    let receipt_id = Uuid::new_v5(
        &command.stable_request_id,
        b"verification-action-recovery.v1",
    );
    let mut tx = pool.begin().await?;
    if let Some(existing) = sqlx::query_as::<_, (Uuid, String, i64)>(
        r#"SELECT receipt.recovery_receipt_id,receipt.recovery_hash,execution.row_version
             FROM verification_action_recovery_receipts receipt
             JOIN verification_action_executions execution
               ON execution.action_execution_id=receipt.action_execution_id
            WHERE receipt.stable_request_id=$1"#,
    )
    .bind(command.stable_request_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        tx.commit().await?;
        return Ok(DurableUnknownActionRecovery {
            recovery_receipt_id: existing.0,
            recovery_hash: existing.1,
            execution_row_version: existing.2,
            replayed: true,
        });
    }

    #[derive(sqlx::FromRow)]
    struct RecoveryAuthority {
        budget_reservation_id: Uuid,
        conflict_set_id: Uuid,
        project_scope_id: Uuid,
        organization_id: Uuid,
        execution_state: String,
        execution_row_version: i64,
        prior_closeout_hash: Option<String>,
        capability_execution_receipt_id: Option<Uuid>,
        action_state: String,
    }
    let authority = sqlx::query_as::<_, RecoveryAuthority>(
        r#"SELECT execution.budget_reservation_id,execution.conflict_set_id,
                  execution.project_scope_id,execution.organization_id,
                  execution.state AS execution_state,
                  execution.row_version AS execution_row_version,
                  execution.closeout_hash AS prior_closeout_hash,
                  execution.capability_execution_receipt_id,
                  action.state AS action_state
             FROM verification_action_executions execution
             JOIN verification_prepared_actions action
               ON action.prepared_action_id=execution.prepared_action_id
              AND action.campaign_id=$2 AND action.operation_id=$3
            WHERE execution.action_execution_id=$1
              AND execution.prepared_action_id=$4
            FOR UPDATE OF execution,action"#,
    )
    .bind(command.action_execution_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(command.prepared_action_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_STALE))?;
    if authority.execution_state != "outcome_unknown"
        || authority.action_state != "outcome_unknown"
        || authority.execution_row_version != command.expected_execution_row_version
        || authority.prior_closeout_hash.is_none()
        || authority.capability_execution_receipt_id.is_none()
    {
        return Err(conflict(AUTHORITY_STALE));
    }

    let residual_id = if command.disposition == RecoverUnknownActionDisposition::ManuallyBlocked {
        let id = Uuid::new_v5(&command.stable_request_id, b"manual-block-residual.v1");
        let residual_body = serde_json::json!({
            "reason_code": "verification_action_outcome_manually_blocked",
            "action_execution_id": command.action_execution_id,
            "prepared_action_id": command.prepared_action_id,
            "next_action": "operator_reconcile_external_effects",
        });
        let residual_hash = json_hash_on(&mut tx, &residual_body).await?;
        sqlx::query(
            r#"INSERT INTO hypothesis_residual_risks(
                   residual_id,operation_id,organization_id,reason_code,owner_kind,
                   affected_inputs,next_action,residual_hash
               ) VALUES($1,$2,$3,'verification_action_outcome_manually_blocked','plan_c',$4,$5,$6)"#,
        )
        .bind(id)
        .bind(command.operation_id)
        .bind(authority.organization_id)
        .bind(serde_json::json!([{
            "kind": "prepared_action",
            "id": command.prepared_action_id,
        }]))
        .bind(serde_json::json!({
            "kind": "operator_reconcile_external_effects",
            "action_execution_id": command.action_execution_id,
        }))
        .bind(residual_hash)
        .execute(&mut *tx)
        .await?;
        Some(id)
    } else {
        None
    };
    let budget_settlement_kind = match command.disposition {
        RecoverUnknownActionDisposition::ReconciledSucceeded
        | RecoverUnknownActionDisposition::ReconciledFailed => "consume_unknown_hold",
        RecoverUnknownActionDisposition::OutcomeUnknown
        | RecoverUnknownActionDisposition::ManuallyBlocked => "retain_unknown_hold",
    };
    let recovery_body = serde_json::json!({
        "contract_version": "verification-action-recovery.v1",
        "action_execution_id": command.action_execution_id,
        "prepared_action_id": command.prepared_action_id,
        "disposition": command.disposition.as_str(),
        "execution_result_state": command.disposition.execution_state(),
        "budget_settlement_kind": budget_settlement_kind,
        "prior_closeout_hash": authority.prior_closeout_hash,
        "residual_id": residual_id,
    });
    let recovery_hash = json_hash_on(&mut tx, &recovery_body).await?;
    sqlx::query(
        r#"INSERT INTO verification_action_recovery_receipts(
               recovery_receipt_id,stable_request_id,action_execution_id,
               prepared_action_id,operation_id,project_scope_id,organization_id,
               recovery_disposition,execution_result_state,budget_settlement_kind,
               prior_closeout_hash,recovery_hash,residual_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
    )
    .bind(receipt_id)
    .bind(command.stable_request_id)
    .bind(command.action_execution_id)
    .bind(command.prepared_action_id)
    .bind(command.operation_id)
    .bind(authority.project_scope_id)
    .bind(authority.organization_id)
    .bind(command.disposition.as_str())
    .bind(command.disposition.execution_state())
    .bind(budget_settlement_kind)
    .bind(authority.prior_closeout_hash.as_deref().unwrap_or_default())
    .bind(&recovery_hash)
    .bind(residual_id)
    .execute(&mut *tx)
    .await?;

    let terminal_reconciliation = matches!(
        command.disposition,
        RecoverUnknownActionDisposition::ReconciledSucceeded
            | RecoverUnknownActionDisposition::ReconciledFailed
    );
    if terminal_reconciliation {
        let held_axes: Vec<(Uuid, String, i64, i64, i64, i64, i64)> = sqlx::query_as(
            r#"SELECT reserve.ancestor_contract_id,reserve.axis_kind,
                      GREATEST(reserve.delta-COALESCE(SUM(consume.delta),0),0)::BIGINT,
                      head.consumed,head.reserved,head.unknown_held,head.row_version
                 FROM verification_budget_ledger_entries reserve
                 LEFT JOIN verification_budget_ledger_entries consume
                   ON consume.budget_reservation_id=reserve.budget_reservation_id
                  AND consume.ancestor_contract_id=reserve.ancestor_contract_id
                  AND consume.axis_kind=reserve.axis_kind
                  AND consume.entry_kind='consume'
                 JOIN verification_budget_scope_heads head
                   ON head.budget_contract_id=reserve.ancestor_contract_id
                  AND head.axis_kind=reserve.axis_kind
                WHERE reserve.budget_reservation_id=$1 AND reserve.entry_kind='reserve'
                GROUP BY reserve.ancestor_contract_id,reserve.axis_kind,reserve.delta,
                         head.consumed,head.reserved,head.unknown_held,head.row_version
                ORDER BY reserve.ancestor_contract_id,reserve.axis_kind"#,
        )
        .bind(authority.budget_reservation_id)
        .fetch_all(&mut *tx)
        .await?;
        for (contract_id, axis_kind, held, consumed, reserved, unknown_held, row_version) in
            held_axes
        {
            if held > unknown_held {
                return Err(conflict(AUTHORITY_STALE));
            }
            let resulting_consumed = consumed + held;
            let resulting_unknown = unknown_held - held;
            let resulting_hash = json_hash_on(
                &mut tx,
                &serde_json::json!({
                    "contract_id": contract_id,
                    "axis_kind": axis_kind,
                    "consumed": resulting_consumed,
                    "reserved": reserved,
                    "unknown_held": resulting_unknown,
                    "row_version": row_version + 1,
                    "recovery_receipt_id": receipt_id,
                }),
            )
            .await?;
            let ordinal: i64 = sqlx::query_scalar(
                r#"SELECT COALESCE(MAX(entry_ordinal),0)+1
                     FROM verification_budget_ledger_entries
                    WHERE budget_reservation_id=$1 AND ancestor_contract_id=$2
                      AND axis_kind=$3"#,
            )
            .bind(authority.budget_reservation_id)
            .bind(contract_id)
            .bind(&axis_kind)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query(
                r#"INSERT INTO verification_budget_ledger_entries(
                       budget_ledger_entry_id,budget_reservation_id,ancestor_contract_id,
                       axis_kind,entry_ordinal,entry_kind,delta,resulting_consumed,
                       resulting_reserved,resulting_unknown_held,expected_head_row_version,
                       resulting_head_hash,fence
                   ) VALUES($1,$2,$3,$4,$5,'consume',$6,$7,0,$8,$9,$10,$11)"#,
            )
            .bind(Uuid::new_v5(
                &receipt_id,
                format!("{contract_id}:{axis_kind}").as_bytes(),
            ))
            .bind(authority.budget_reservation_id)
            .bind(contract_id)
            .bind(&axis_kind)
            .bind(ordinal)
            .bind(held)
            .bind(resulting_consumed)
            .bind(resulting_unknown)
            .bind(row_version)
            .bind(resulting_hash)
            .bind(row_version + 1)
            .execute(&mut *tx)
            .await?;
            let updated = sqlx::query(
                r#"UPDATE verification_budget_scope_heads
                      SET consumed=$1,unknown_held=$2,row_version=row_version+1,
                          updated_at=statement_timestamp()
                    WHERE budget_contract_id=$3 AND axis_kind=$4
                      AND row_version=$5 AND reserved=0"#,
            )
            .bind(resulting_consumed)
            .bind(reserved)
            .bind(resulting_unknown)
            .bind(contract_id)
            .bind(&axis_kind)
            .bind(row_version)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if updated != 1 {
                return Err(conflict(AUTHORITY_STALE));
            }
        }
        sqlx::query(
            r#"UPDATE verification_budget_reservations
                  SET state='settled',row_version=row_version+1,
                      settled_at=statement_timestamp()
                WHERE budget_reservation_id=$1 AND state='unknown_held'"#,
        )
        .bind(authority.budget_reservation_id)
        .execute(&mut *tx)
        .await?;
    }

    if command.disposition != RecoverUnknownActionDisposition::OutcomeUnknown {
        let conflict_keys: Vec<(String, String, i64, i64)> = sqlx::query_as(
            r#"SELECT head.key_kind,head.key_identity_hash,
                      head.fencing_token,head.row_version
                 FROM verification_action_conflict_set_members member
                 JOIN verification_conflict_key_heads head
                   ON head.operation_id=$2 AND head.organization_id=$3
                  AND head.key_kind=member.key_kind
                  AND head.key_identity_hash=member.key_identity_hash
                  AND head.owner_prepared_action_id=$4
                WHERE member.conflict_set_id=$1 AND head.state='recovery_hold'
                ORDER BY head.key_kind,head.key_identity_hash
                FOR UPDATE OF head"#,
        )
        .bind(authority.conflict_set_id)
        .bind(command.operation_id)
        .bind(authority.organization_id)
        .bind(command.prepared_action_id)
        .fetch_all(&mut *tx)
        .await?;
        for (key_kind, key_hash, fence, row_version) in conflict_keys {
            let event_hash = json_hash_on(
                &mut tx,
                &serde_json::json!({
                    "recovery_receipt_id": receipt_id,
                    "key_kind": key_kind,
                    "key_identity_hash": key_hash,
                    "event_kind": "release",
                    "fencing_token": fence,
                }),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO verification_conflict_key_events(
                       conflict_event_id,operation_id,project_scope_id,organization_id,
                       key_kind,key_identity_hash,event_ordinal,event_kind,
                       expected_fencing_token,new_fencing_token,owner_campaign_id,
                       owner_prepared_action_id,reason_code,event_hash
                   ) SELECT $1,$2,$3,$4,$5,$6,COALESCE(MAX(event_ordinal),0)+1,
                            'release',$7,$7,$8,$9,'outcome_unknown_reconciled',$10
                       FROM verification_conflict_key_events prior
                      WHERE prior.operation_id=$2 AND prior.organization_id=$4
                        AND prior.key_kind=$5 AND prior.key_identity_hash=$6"#,
            )
            .bind(Uuid::new_v5(&receipt_id, event_hash.as_bytes()))
            .bind(command.operation_id)
            .bind(authority.project_scope_id)
            .bind(authority.organization_id)
            .bind(&key_kind)
            .bind(&key_hash)
            .bind(fence)
            .bind(command.campaign_id)
            .bind(command.prepared_action_id)
            .bind(event_hash)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                r#"UPDATE verification_conflict_key_heads
                      SET state='free',owner_campaign_id=NULL,
                          owner_prepared_action_id=NULL,expires_at=NULL,
                          row_version=row_version+1,updated_at=statement_timestamp()
                    WHERE operation_id=$1 AND organization_id=$2 AND key_kind=$3
                      AND key_identity_hash=$4 AND row_version=$5"#,
            )
            .bind(command.operation_id)
            .bind(authority.organization_id)
            .bind(&key_kind)
            .bind(&key_hash)
            .bind(row_version)
            .execute(&mut *tx)
            .await?;
        }
    }

    if terminal_reconciliation {
        sqlx::query(
            r#"UPDATE verification_action_executions
                  SET state=$1,closeout_hash=$2,row_version=row_version+1,
                      completed_at=statement_timestamp()
                WHERE action_execution_id=$3 AND state='outcome_unknown'
                  AND row_version=$4"#,
        )
        .bind(command.disposition.execution_state())
        .bind(&recovery_hash)
        .bind(command.action_execution_id)
        .bind(command.expected_execution_row_version)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"UPDATE verification_prepared_actions
                  SET state=$1,reason_code='outcome_unknown_reconciled',
                      row_version=row_version+1,terminal_at=statement_timestamp()
                WHERE prepared_action_id=$2 AND state='outcome_unknown'"#,
        )
        .bind(command.disposition.execution_state())
        .bind(command.prepared_action_id)
        .execute(&mut *tx)
        .await?;
    } else if command.disposition == RecoverUnknownActionDisposition::ManuallyBlocked {
        sqlx::query(
            r#"UPDATE verification_prepared_actions
                  SET state='manually_blocked',reason_code='outcome_unknown_manually_blocked',
                      residual_id=$1,row_version=row_version+1,
                      terminal_at=statement_timestamp()
                WHERE prepared_action_id=$2 AND state='outcome_unknown'"#,
        )
        .bind(residual_id)
        .bind(command.prepared_action_id)
        .execute(&mut *tx)
        .await?;
    }
    let execution_row_version: i64 = sqlx::query_scalar(
        "SELECT row_version FROM verification_action_executions WHERE action_execution_id=$1",
    )
    .bind(command.action_execution_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(DurableUnknownActionRecovery {
        recovery_receipt_id: receipt_id,
        recovery_hash,
        execution_row_version,
        replayed: false,
    })
}

#[cfg(test)]
mod directory_fingerprint_witness_tests {
    use super::{
        classify_verification_action_witness, DirectoryFingerprintOracleVerdictV1,
        VerificationActionWitnessV1,
    };
    use serde_json::json;
    use uuid::Uuid;

    fn http_observation(url: String, body_byte: char) -> serde_json::Value {
        json!({
            "final_url": url,
            "hops": [{
                "url": url,
                "status": 200,
                "response_bytes": 8,
                "body_sha256": format!("sha256:{}", body_byte.to_string().repeat(64)),
                "content_type": "text/html",
            }],
        })
    }

    fn complete_directory_observation(
        prepared_action_id: Uuid,
        candidate_body: char,
        control_bodies: [char; 3],
        claimed_verdict: &str,
        claimed_controls_consistent: bool,
    ) -> serde_json::Value {
        let nonce = prepared_action_id.simple().to_string();
        json!({
            "assessment": {
                "controls_consistent": claimed_controls_consistent,
                "verdict": claimed_verdict,
            },
            "candidate": http_observation("https://example.test/admin".to_owned(), candidate_body),
            "capability_id": "verify.directory_fingerprint.v1",
            "contract_version": "directory-soft404-fingerprint-observation.v1",
            "controls": control_bodies.into_iter().enumerate().map(|(index, body)| {
                http_observation(
                    format!("https://example.test/.golish-soft404-{nonce}-{}", index + 1),
                    body,
                )
            }).collect::<Vec<_>>(),
            "request_count": 4,
            "witness_completeness": "complete_fingerprint_v1",
        })
    }

    #[test]
    fn directory_fingerprint_complete_witness_is_recomputed_for_all_verdicts() {
        let prepared_action_id = Uuid::new_v4();
        for (observation, expected) in [
            (
                complete_directory_observation(
                    prepared_action_id,
                    'a',
                    ['b', 'b', 'b'],
                    "verified",
                    true,
                ),
                DirectoryFingerprintOracleVerdictV1::Proof,
            ),
            (
                complete_directory_observation(
                    prepared_action_id,
                    'b',
                    ['b', 'b', 'b'],
                    "refuted",
                    true,
                ),
                DirectoryFingerprintOracleVerdictV1::Refutation,
            ),
            (
                complete_directory_observation(
                    prepared_action_id,
                    'a',
                    ['b', 'c', 'b'],
                    "inconclusive",
                    false,
                ),
                DirectoryFingerprintOracleVerdictV1::Inconclusive,
            ),
        ] {
            assert_eq!(
                classify_verification_action_witness(
                    prepared_action_id,
                    "succeeded",
                    &observation,
                )
                .expect("exact directory witness is valid"),
                VerificationActionWitnessV1::DirectoryFingerprint(expected)
            );
        }
    }

    #[test]
    fn directory_fingerprint_complete_witness_rejects_model_or_transport_drift() {
        let prepared_action_id = Uuid::new_v4();
        let forged_verdict = complete_directory_observation(
            prepared_action_id,
            'a',
            ['b', 'b', 'b'],
            "refuted",
            true,
        );
        assert!(classify_verification_action_witness(
            prepared_action_id,
            "succeeded",
            &forged_verdict,
        )
        .is_err());

        let mut forged_control = complete_directory_observation(
            prepared_action_id,
            'a',
            ['b', 'b', 'b'],
            "verified",
            true,
        );
        forged_control["controls"][0]["hops"][0]["url"] =
            json!("https://example.test/model-selected-control");
        forged_control["controls"][0]["final_url"] =
            json!("https://example.test/model-selected-control");
        assert!(classify_verification_action_witness(
            prepared_action_id,
            "succeeded",
            &forged_control,
        )
        .is_err());
    }

    #[test]
    fn metadata_only_witness_remains_inconclusive_compatible() {
        assert_eq!(
            classify_verification_action_witness(
                Uuid::new_v4(),
                "outcome_unknown",
                &json!({
                    "contract_version": "verification-action-observation.v1",
                    "witness_completeness": "metadata_only",
                    "recovery_disposition": "durable_begin_without_terminal_receipt",
                }),
            )
            .expect("legacy recovery observation remains accepted"),
            VerificationActionWitnessV1::MetadataOnly
        );
    }
}
