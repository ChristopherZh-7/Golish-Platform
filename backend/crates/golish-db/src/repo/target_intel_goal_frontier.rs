//! Org-scoped V2 Target Intel frontier CAS operations.

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const FRONTIER_COLUMNS: &str = r#"
    id, operation_id, organization_id, stage_execution_id, stage_run_unit_id,
    scope_snapshot_id, team_plan_id, goal_epoch_id, goal_epoch,
    semantic_pivot_key, pivot_kind, pivot_value_sha256, intent, materiality,
    status, provenance, terminal_refs, capability_ref, reason, row_version,
    claimed_by_worker_run_id, claim_attempt_epoch, claim_lease_token,
    claim_lease_expires_at, created_at, updated_at, terminal_at
"#;

const QUALIFIED_FRONTIER_COLUMNS: &str = r#"
    frontier.id AS id, frontier.operation_id AS operation_id,
    frontier.organization_id AS organization_id,
    frontier.stage_execution_id AS stage_execution_id,
    frontier.stage_run_unit_id AS stage_run_unit_id,
    frontier.scope_snapshot_id AS scope_snapshot_id,
    frontier.team_plan_id AS team_plan_id,
    frontier.goal_epoch_id AS goal_epoch_id, frontier.goal_epoch AS goal_epoch,
    frontier.semantic_pivot_key AS semantic_pivot_key,
    frontier.pivot_kind AS pivot_kind,
    frontier.pivot_value_sha256 AS pivot_value_sha256,
    frontier.intent AS intent, frontier.materiality AS materiality,
    frontier.status AS status, frontier.provenance AS provenance,
    frontier.terminal_refs AS terminal_refs,
    frontier.capability_ref AS capability_ref, frontier.reason AS reason,
    frontier.row_version AS row_version,
    frontier.claimed_by_worker_run_id AS claimed_by_worker_run_id,
    frontier.claim_attempt_epoch AS claim_attempt_epoch,
    frontier.claim_lease_token AS claim_lease_token,
    frontier.claim_lease_expires_at AS claim_lease_expires_at,
    frontier.created_at AS created_at, frontier.updated_at AS updated_at,
    frontier.terminal_at AS terminal_at
"#;

#[derive(Debug, Clone, PartialEq)]
pub struct InsertTargetIntelFrontier {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub team_plan_id: Uuid,
    pub goal_epoch_id: Uuid,
    pub semantic_pivot_key: String,
    pub pivot_kind: String,
    pub pivot_value_sha256: String,
    pub intent: String,
    pub materiality: String,
    pub provenance: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimTargetIntelFrontier {
    pub frontier_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub expected_row_version: i64,
    pub claimed_by_worker_run_id: Uuid,
    pub claim_attempt_epoch: i64,
    pub lease_token: Uuid,
    pub lease_seconds: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionClaimedTargetIntelFrontier {
    pub frontier_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub expected_row_version: i64,
    pub claimed_by_worker_run_id: Uuid,
    pub claim_attempt_epoch: i64,
    pub lease_token: Uuid,
    pub to_status: String,
    pub terminal_refs: Value,
    pub capability_ref: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WaiveTargetIntelFrontierGap {
    pub waiver_id: Uuid,
    pub frontier_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub expected_frontier_row_version: i64,
    pub authority_kind: String,
    pub authority_ref: String,
    pub evidence_refs: Value,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TargetIntelFrontierView {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub team_plan_id: Uuid,
    pub goal_epoch_id: Uuid,
    pub goal_epoch: i64,
    pub semantic_pivot_key: String,
    pub pivot_kind: String,
    pub pivot_value_sha256: String,
    pub intent: String,
    pub materiality: String,
    pub status: String,
    pub provenance: Value,
    pub terminal_refs: Value,
    pub capability_ref: Option<String>,
    pub reason: Option<String>,
    pub row_version: i64,
    pub claimed_by_worker_run_id: Option<Uuid>,
    pub claim_attempt_epoch: Option<i64>,
    pub claim_lease_token: Option<Uuid>,
    pub claim_lease_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TargetIntelFrontierWaiverView {
    pub id: Uuid,
    pub frontier_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub expected_frontier_row_version: i64,
    pub authority_kind: String,
    pub authority_ref: String,
    pub evidence_refs: Value,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub replayed: bool,
}

pub async fn insert_pending(
    pool: &PgPool,
    input: &InsertTargetIntelFrontier,
) -> Result<TargetIntelFrontierView> {
    validate_insert(input)?;
    let mut tx = pool.begin().await?;
    if let Some(replay) = select_frontier_by_semantic_key(
        &mut tx,
        input.operation_id,
        input.organization_id,
        &input.semantic_pivot_key,
        true,
    )
    .await?
    {
        if !insert_identity_matches(&replay, input) {
            bail!("TARGET_INTEL_FRONTIER_INSERT_REPLAY_MISMATCH");
        }
        tx.commit().await?;
        return Ok(replay);
    }
    let sql = format!(
        r#"INSERT INTO target_intel_goal_frontier_v2 (
               id, operation_id, organization_id, stage_execution_id,
               stage_run_unit_id, scope_snapshot_id, team_plan_id, goal_epoch_id,
               goal_epoch, semantic_pivot_key, pivot_kind, pivot_value_sha256,
               intent, materiality, status, provenance
           )
           SELECT $1,$2,$3,$4,$5,$6,$7,$8,epoch.epoch,$9,$10,$11,$12,$13,
                  'pending',$14
             FROM target_intel_goal_epochs epoch
            WHERE epoch.id=$8 AND epoch.operation_id=$2
              AND epoch.organization_id=$3 AND epoch.team_plan_id=$7
              AND epoch.stage_execution_id=$4 AND epoch.stage_run_unit_id=$5
              AND epoch.scope_snapshot_id=$6 AND epoch.status='open'
           ON CONFLICT DO NOTHING
           RETURNING {FRONTIER_COLUMNS}, FALSE AS replayed"#,
    );
    let inserted = sqlx::query_as::<_, TargetIntelFrontierView>(&sql)
        .bind(input.id)
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(input.stage_execution_id)
        .bind(input.stage_run_unit_id)
        .bind(input.scope_snapshot_id)
        .bind(input.team_plan_id)
        .bind(input.goal_epoch_id)
        .bind(&input.semantic_pivot_key)
        .bind(&input.pivot_kind)
        .bind(&input.pivot_value_sha256)
        .bind(&input.intent)
        .bind(&input.materiality)
        .bind(&input.provenance)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(inserted) = inserted {
        tx.commit().await?;
        return Ok(inserted);
    }

    let replay = select_frontier_by_semantic_key(
        &mut tx,
        input.operation_id,
        input.organization_id,
        &input.semantic_pivot_key,
        true,
    )
    .await?;
    match replay {
        Some(row) if insert_identity_matches(&row, input) => {
            tx.commit().await?;
            Ok(row)
        }
        _ => bail!("TARGET_INTEL_FRONTIER_INSERT_REPLAY_MISMATCH"),
    }
}

pub async fn claim(
    pool: &PgPool,
    input: &ClaimTargetIntelFrontier,
) -> Result<TargetIntelFrontierView> {
    validate_claim(input)?;
    let expected_replay_version = input
        .expected_row_version
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_FRONTIER_VERSION_OVERFLOW"))?;
    let mut tx = pool.begin().await?;
    let sql = format!(
        r#"UPDATE target_intel_goal_frontier_v2 frontier
              SET status='in_progress', claimed_by_worker_run_id=$1,
                  claim_attempt_epoch=$2, claim_lease_token=$3,
                  claim_lease_expires_at=NOW()+($4::double precision*INTERVAL '1 second'),
                  row_version=frontier.row_version+1, updated_at=NOW()
            WHERE frontier.id=$5 AND frontier.operation_id=$6
              AND frontier.organization_id=$7 AND frontier.row_version=$8
              AND frontier.status='pending'
              AND EXISTS (
                  SELECT 1
                    FROM stage_worker_runs worker
                    JOIN stage_work_items item ON item.id=worker.work_item_id
                   WHERE worker.id=$1
                     AND worker.operation_id=frontier.operation_id
                     AND worker.stage_execution_id=frontier.stage_execution_id
                     AND worker.stage_run_unit_id=frontier.stage_run_unit_id
                     AND worker.organization_id=frontier.organization_id
                     AND worker.attempt_epoch=$2
                     AND worker.status IN ('running','waiting_background','gate_blocked')
                     AND item.team_plan_id=frontier.team_plan_id
                     AND item.operation_id=frontier.operation_id
                     AND item.stage_execution_id=frontier.stage_execution_id
                     AND item.stage_run_unit_id=frontier.stage_run_unit_id
                     AND item.scope_snapshot_id=frontier.scope_snapshot_id
                     AND item.organization_id=frontier.organization_id
                     AND item.dispatch_epoch=frontier.goal_epoch
                     AND item.execution_profile='worker'
                     AND item.status IN ('claimed','running','waiting_dependency')
              )
        RETURNING {FRONTIER_COLUMNS}, FALSE AS replayed"#,
    );
    let updated = sqlx::query_as::<_, TargetIntelFrontierView>(&sql)
        .bind(input.claimed_by_worker_run_id)
        .bind(input.claim_attempt_epoch)
        .bind(input.lease_token)
        .bind(input.lease_seconds)
        .bind(input.frontier_id)
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(input.expected_row_version)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(updated) = updated {
        tx.commit().await?;
        return Ok(updated);
    }

    let replay_sql = format!(
        r#"SELECT {QUALIFIED_FRONTIER_COLUMNS}, TRUE AS replayed
             FROM target_intel_goal_frontier_v2 frontier
             JOIN target_intel_goal_frontier_events event
               ON event.frontier_id=frontier.id
              AND event.operation_id=frontier.operation_id
              AND event.organization_id=frontier.organization_id
            WHERE frontier.id=$1 AND frontier.operation_id=$2
              AND frontier.organization_id=$3 AND frontier.row_version=$4
              AND frontier.status='in_progress'
              AND frontier.claimed_by_worker_run_id=$5
              AND frontier.claim_attempt_epoch=$6
              AND frontier.claim_lease_token=$7
              AND frontier.claim_lease_expires_at>NOW()
              AND event.expected_row_version=$8
              AND event.from_status='pending' AND event.to_status='in_progress'
              AND event.claimed_by_worker_run_id=$5
              AND event.claim_attempt_epoch=$6 AND event.claim_lease_token=$7
              AND frontier.claim_lease_expires_at=
                  event.created_at+($9::double precision*INTERVAL '1 second')
              AND event.evidence_refs='[]'::jsonb
              AND event.capability_ref IS NULL AND event.reason IS NULL"#,
    );
    let replay = sqlx::query_as::<_, TargetIntelFrontierView>(&replay_sql)
        .bind(input.frontier_id)
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(expected_replay_version)
        .bind(input.claimed_by_worker_run_id)
        .bind(input.claim_attempt_epoch)
        .bind(input.lease_token)
        .bind(input.expected_row_version)
        .bind(input.lease_seconds)
        .fetch_optional(&mut *tx)
        .await?;
    match replay {
        Some(replay) => {
            tx.commit().await?;
            Ok(replay)
        }
        None => bail!("TARGET_INTEL_FRONTIER_CLAIM_CAS_FAILED"),
    }
}

pub async fn transition_claimed(
    pool: &PgPool,
    input: &TransitionClaimedTargetIntelFrontier,
) -> Result<TargetIntelFrontierView> {
    validate_transition(input)?;
    let expected_replay_version = input
        .expected_row_version
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_FRONTIER_VERSION_OVERFLOW"))?;
    let mut tx = pool.begin().await?;
    let sql = format!(
        r#"UPDATE target_intel_goal_frontier_v2
              SET status=$1, terminal_refs=$2, capability_ref=$3, reason=$4,
                  terminal_at=NOW(), row_version=row_version+1, updated_at=NOW()
            WHERE id=$5 AND operation_id=$6 AND organization_id=$7
              AND row_version=$8 AND status='in_progress'
              AND claimed_by_worker_run_id=$9 AND claim_attempt_epoch=$10
              AND claim_lease_token=$11 AND claim_lease_expires_at>NOW()
        RETURNING {FRONTIER_COLUMNS}, FALSE AS replayed"#,
    );
    let updated = sqlx::query_as::<_, TargetIntelFrontierView>(&sql)
        .bind(&input.to_status)
        .bind(&input.terminal_refs)
        .bind(input.capability_ref.as_deref())
        .bind(input.reason.as_deref())
        .bind(input.frontier_id)
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(input.expected_row_version)
        .bind(input.claimed_by_worker_run_id)
        .bind(input.claim_attempt_epoch)
        .bind(input.lease_token)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(updated) = updated {
        tx.commit().await?;
        return Ok(updated);
    }

    let replay_sql = format!(
        r#"SELECT {QUALIFIED_FRONTIER_COLUMNS}, TRUE AS replayed
             FROM target_intel_goal_frontier_v2 frontier
             JOIN target_intel_goal_frontier_events event
               ON event.frontier_id=frontier.id
              AND event.operation_id=frontier.operation_id
              AND event.organization_id=frontier.organization_id
            WHERE frontier.id=$1 AND frontier.operation_id=$2
              AND frontier.organization_id=$3 AND frontier.row_version=$4
              AND frontier.status=$5 AND frontier.terminal_refs=$6
              AND frontier.capability_ref IS NOT DISTINCT FROM $7
              AND frontier.reason IS NOT DISTINCT FROM $8
              AND frontier.claimed_by_worker_run_id=$9
              AND frontier.claim_attempt_epoch=$10
              AND frontier.claim_lease_token=$11
              AND event.expected_row_version=$12
              AND event.from_status='in_progress' AND event.to_status=$5
              AND event.evidence_refs=$6
              AND event.capability_ref IS NOT DISTINCT FROM $7
              AND event.reason IS NOT DISTINCT FROM $8
              AND event.claimed_by_worker_run_id=$9
              AND event.claim_attempt_epoch=$10 AND event.claim_lease_token=$11"#,
    );
    let replay = sqlx::query_as::<_, TargetIntelFrontierView>(&replay_sql)
        .bind(input.frontier_id)
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(expected_replay_version)
        .bind(&input.to_status)
        .bind(&input.terminal_refs)
        .bind(input.capability_ref.as_deref())
        .bind(input.reason.as_deref())
        .bind(input.claimed_by_worker_run_id)
        .bind(input.claim_attempt_epoch)
        .bind(input.lease_token)
        .bind(input.expected_row_version)
        .fetch_optional(&mut *tx)
        .await?;
    match replay {
        Some(replay) => {
            tx.commit().await?;
            Ok(replay)
        }
        None => bail!("TARGET_INTEL_FRONTIER_TRANSITION_CAS_FAILED"),
    }
}

pub async fn waive_terminal_gap(
    pool: &PgPool,
    input: &WaiveTargetIntelFrontierGap,
) -> Result<TargetIntelFrontierWaiverView> {
    validate_waiver(input)?;
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query_as::<_, TargetIntelFrontierWaiverView>(
        r#"INSERT INTO target_intel_goal_frontier_waivers(
               id,frontier_id,operation_id,organization_id,
               expected_frontier_row_version,authority_kind,authority_ref,
               evidence_refs,reason
           )
           SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9
             FROM target_intel_goal_frontier_v2 frontier
            WHERE frontier.id=$2 AND frontier.operation_id=$3
              AND frontier.organization_id=$4 AND frontier.row_version=$5
              AND frontier.materiality='material'
              AND frontier.status IN ('blocked','unsupported')
           ON CONFLICT DO NOTHING
           RETURNING id,frontier_id,operation_id,organization_id,
                     expected_frontier_row_version,authority_kind,authority_ref,
                     evidence_refs,reason,created_at,FALSE AS replayed"#,
    )
    .bind(input.waiver_id)
    .bind(input.frontier_id)
    .bind(input.operation_id)
    .bind(input.organization_id)
    .bind(input.expected_frontier_row_version)
    .bind(&input.authority_kind)
    .bind(&input.authority_ref)
    .bind(&input.evidence_refs)
    .bind(&input.reason)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(inserted) = inserted {
        tx.commit().await?;
        return Ok(inserted);
    }

    let replay = sqlx::query_as::<_, TargetIntelFrontierWaiverView>(
        r#"SELECT id,frontier_id,operation_id,organization_id,
                  expected_frontier_row_version,authority_kind,authority_ref,
                  evidence_refs,reason,created_at,TRUE AS replayed
             FROM target_intel_goal_frontier_waivers
            WHERE frontier_id=$1 AND operation_id=$2 AND organization_id=$3"#,
    )
    .bind(input.frontier_id)
    .bind(input.operation_id)
    .bind(input.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    match replay {
        Some(row) if waiver_matches(&row, input) => {
            tx.commit().await?;
            Ok(row)
        }
        _ => bail!("TARGET_INTEL_FRONTIER_WAIVER_REPLAY_MISMATCH"),
    }
}

async fn select_frontier_by_semantic_key(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    organization_id: Uuid,
    semantic_pivot_key: &str,
    replayed: bool,
) -> Result<Option<TargetIntelFrontierView>> {
    let sql = format!(
        "SELECT {FRONTIER_COLUMNS}, $4 AS replayed \
           FROM target_intel_goal_frontier_v2 \
          WHERE operation_id=$1 AND organization_id=$2 AND semantic_pivot_key=$3",
    );
    Ok(sqlx::query_as::<_, TargetIntelFrontierView>(&sql)
        .bind(operation_id)
        .bind(organization_id)
        .bind(semantic_pivot_key)
        .bind(replayed)
        .fetch_optional(&mut **tx)
        .await?)
}

fn insert_identity_matches(
    row: &TargetIntelFrontierView,
    input: &InsertTargetIntelFrontier,
) -> bool {
    row.id == input.id
        && row.operation_id == input.operation_id
        && row.organization_id == input.organization_id
        && row.stage_execution_id == input.stage_execution_id
        && row.stage_run_unit_id == input.stage_run_unit_id
        && row.scope_snapshot_id == input.scope_snapshot_id
        && row.team_plan_id == input.team_plan_id
        && row.goal_epoch_id == input.goal_epoch_id
        && row.semantic_pivot_key == input.semantic_pivot_key
        && row.pivot_kind == input.pivot_kind
        && row.pivot_value_sha256 == input.pivot_value_sha256
        && row.intent == input.intent
        && row.materiality == input.materiality
        && row.provenance == input.provenance
}

fn waiver_matches(
    row: &TargetIntelFrontierWaiverView,
    input: &WaiveTargetIntelFrontierGap,
) -> bool {
    row.id == input.waiver_id
        && row.frontier_id == input.frontier_id
        && row.operation_id == input.operation_id
        && row.organization_id == input.organization_id
        && row.expected_frontier_row_version == input.expected_frontier_row_version
        && row.authority_kind == input.authority_kind
        && row.authority_ref == input.authority_ref
        && row.evidence_refs == input.evidence_refs
        && row.reason == input.reason
}

fn validate_insert(input: &InsertTargetIntelFrontier) -> Result<()> {
    if input.id.is_nil()
        || input.operation_id.is_nil()
        || input.organization_id.is_nil()
        || input.stage_execution_id.is_nil()
        || input.stage_run_unit_id.is_nil()
        || input.scope_snapshot_id.is_nil()
        || input.team_plan_id.is_nil()
        || input.goal_epoch_id.is_nil()
        || input.semantic_pivot_key.trim().is_empty()
        || !matches!(
            input.pivot_kind.as_str(),
            "company_name"
                | "brand"
                | "domain"
                | "hostname"
                | "ip"
                | "cidr"
                | "asn"
                | "certificate"
                | "icp"
                | "email_domain"
                | "github_org"
                | "repository"
                | "app_id"
        )
        || !valid_sha256(&input.pivot_value_sha256)
        || !matches!(
            input.intent.as_str(),
            "discover_related_assets" | "verify_attribution" | "enrich_known_asset"
        )
        || !matches!(input.materiality.as_str(), "material" | "supporting")
        || !input.provenance.is_object()
    {
        bail!("TARGET_INTEL_FRONTIER_INSERT_INVALID");
    }
    Ok(())
}

fn validate_claim(input: &ClaimTargetIntelFrontier) -> Result<()> {
    if input.frontier_id.is_nil()
        || input.operation_id.is_nil()
        || input.organization_id.is_nil()
        || input.claimed_by_worker_run_id.is_nil()
        || input.lease_token.is_nil()
        || input.expected_row_version < 0
        || input.claim_attempt_epoch < 0
        || input.lease_seconds <= 0
    {
        bail!("TARGET_INTEL_FRONTIER_CLAIM_INVALID");
    }
    Ok(())
}

fn validate_transition(input: &TransitionClaimedTargetIntelFrontier) -> Result<()> {
    let refs_are_array = input.terminal_refs.as_array();
    let capability_present = input
        .capability_ref
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let reason_present = input
        .reason
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let terminal_payload_valid = match input.to_status.as_str() {
        "resolved" => refs_are_array.is_some_and(|refs| !refs.is_empty()),
        "blocked" | "unsupported" => capability_present && reason_present,
        "needs_human" | "rejected_noise" | "third_party" | "ambiguous" => reason_present,
        _ => false,
    };
    let optional_text_valid = input
        .capability_ref
        .as_deref()
        .is_none_or(|value| !value.trim().is_empty())
        && input
            .reason
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty());
    if input.frontier_id.is_nil()
        || input.operation_id.is_nil()
        || input.organization_id.is_nil()
        || input.claimed_by_worker_run_id.is_nil()
        || input.lease_token.is_nil()
        || input.expected_row_version < 0
        || input.claim_attempt_epoch < 0
        || refs_are_array.is_none()
        || !terminal_payload_valid
        || !optional_text_valid
    {
        bail!("TARGET_INTEL_FRONTIER_TRANSITION_INVALID");
    }
    Ok(())
}

fn validate_waiver(input: &WaiveTargetIntelFrontierGap) -> Result<()> {
    let evidence_refs_valid = input.evidence_refs.as_array().is_some_and(|refs| {
        !refs.is_empty()
            && refs.iter().all(|value| {
                value
                    .as_str()
                    .and_then(|value| value.strip_prefix("audit:"))
                    .and_then(|value| value.parse::<i64>().ok())
                    .is_some_and(|id| id > 0)
            })
    });
    if input.waiver_id.is_nil()
        || input.frontier_id.is_nil()
        || input.operation_id.is_nil()
        || input.organization_id.is_nil()
        || input.expected_frontier_row_version < 0
        || !matches!(
            input.authority_kind.as_str(),
            "operation_policy" | "human_operator"
        )
        || input.authority_ref.trim().is_empty()
        || input.reason.trim().is_empty()
        || !evidence_refs_valid
    {
        bail!("TARGET_INTEL_FRONTIER_WAIVER_INVALID");
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}
