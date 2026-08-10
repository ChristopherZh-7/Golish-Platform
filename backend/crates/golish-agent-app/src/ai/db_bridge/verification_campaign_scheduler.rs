//! Deterministic host driver for authoritative Plan C Campaigns.
//!
//! The agent runtime can ask this module to advance durable Campaign state,
//! but cannot provide action bodies, URLs, credentials, budgets, policy or
//! oracle claims. Those values are selected or compiled by the canonical
//! repositories. The driver deliberately stops when a prepared action needs
//! JIT operator authorization.

use std::sync::Arc;

use anyhow::Context;
use golish_agent_kit::db_traits::verification_campaign::{
    AdjudicateHypothesisRevision, BeginPreparedAction, CloseCampaignObjective, OpenCampaignRound,
    ProposePreparedAction, SealCampaignCoverageDenominator, SealOracleCensus,
    VerificationCampaignRepository,
};
use golish_agent_kit::db_traits::VerificationCampaignSchedulerView;
use golish_agent_kit::db_traits::{
    RecordVerificationConsultTerminal, VerificationConsultTerminalState,
    VerificationConsultWorkItemView,
};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::verification_campaign::PgVerificationCampaignRepository;

#[derive(Debug, sqlx::FromRow)]
struct CampaignRow {
    campaign_id: Uuid,
    verification_objective_id: Uuid,
    verification_contract_id: Uuid,
    state: String,
    row_version: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct PreparedActionState {
    prepared_action_id: Uuid,
    state: String,
    row_version: i64,
    authorization_receipt_id: Option<Uuid>,
    campaign_dispatch_generation: Option<i64>,
    action_execution_id: Option<Uuid>,
}

async fn load_latest_prepared_action(
    pool: &PgPool,
    operation_id: Uuid,
    campaign_id: Uuid,
) -> anyhow::Result<Option<PreparedActionState>> {
    Ok(sqlx::query_as::<_, PreparedActionState>(
        r#"SELECT action.prepared_action_id,action.state,action.row_version,
                  latest_authorization.authorization_receipt_id,
                  latest_authorization.campaign_dispatch_generation,
                  execution.action_execution_id
             FROM verification_prepared_actions action
             LEFT JOIN LATERAL (
                 SELECT receipt.authorization_receipt_id,
                        receipt.campaign_dispatch_generation
                   FROM verification_prepared_action_authorizations receipt
                  WHERE receipt.prepared_action_id=action.prepared_action_id
                    AND receipt.campaign_id=action.campaign_id
                    AND receipt.operation_id=action.operation_id
                    AND receipt.decision='authorized'
                  ORDER BY receipt.decided_at DESC,
                           receipt.authorization_receipt_id DESC LIMIT 1
             ) latest_authorization ON TRUE
             LEFT JOIN LATERAL (
                 SELECT current_execution.action_execution_id
                  FROM verification_action_executions current_execution
                  WHERE current_execution.prepared_action_id=action.prepared_action_id
                    AND current_execution.operation_id=action.operation_id
                  ORDER BY current_execution.execution_ordinal DESC LIMIT 1
             ) execution ON TRUE
            WHERE action.campaign_id=$1 AND action.operation_id=$2
            ORDER BY action.action_ordinal DESC,action.created_at DESC,
                     action.prepared_action_id DESC LIMIT 1"#,
    )
    .bind(campaign_id)
    .bind(operation_id)
    .fetch_optional(pool)
    .await?)
}

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct ExistingRound {
    round_id: Uuid,
}

#[derive(Debug, sqlx::FromRow)]
struct PendingConsultRow {
    operation_id: Uuid,
    campaign_id: Uuid,
    round_id: Uuid,
    consult_id: Uuid,
    verification_objective_id: Uuid,
    role_kind: String,
    round_input_hash: String,
    request_packet: serde_json::Value,
}

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct DurableConsultRow {
    consult_id: Uuid,
    role_kind: String,
    response_artifact: Option<serde_json::Value>,
    terminal_state: Option<String>,
    round_input_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct ExistingConsultTerminalRow {
    stable_request_id: Uuid,
    consult_id: Uuid,
    terminal_state: String,
    response_artifact: Option<serde_json::Value>,
    response_artifact_hash: Option<String>,
    reason_code: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StrategyObligationRow {
    strategy_artifact_id: Uuid,
    obligation_id: Uuid,
}

#[derive(Clone, Copy, Debug, sqlx::FromRow)]
struct SealedCompileResidualDisposition {
    apply_count: i64,
    residual_count: i64,
    valid_residual_count: i64,
    prepared_action_count: i64,
    planned_obligation_count: i64,
}

fn is_exact_terminal_compile_residual(disposition: &SealedCompileResidualDisposition) -> bool {
    disposition.apply_count > 0
        && disposition.apply_count == disposition.residual_count
        && disposition.apply_count == disposition.valid_residual_count
        && disposition.apply_count == disposition.planned_obligation_count
        && disposition.prepared_action_count == 0
}

#[derive(Debug, sqlx::FromRow)]
struct WaveRow {
    wave_denominator_id: Uuid,
    generation_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    member_count: i64,
    source_snapshot_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct WaveSelectedResultRow {
    campaign_coverage_receipt_id: Uuid,
    campaign_receipt_hash: String,
    wave_coverage_member_id: Uuid,
    result_hash: String,
    coverage_status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct WaveFactDeltaRow {
    fact_delta_bundle_id: Uuid,
    fact_delta_hash: String,
    delta_kind: String,
}

#[derive(Debug, sqlx::FromRow)]
struct RevisionAdjudicationSubject {
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    generation_seal_id: Uuid,
    hypothesis_revision_id: Uuid,
    verification_plan_id: Uuid,
}

fn stable(campaign_id: Uuid, label: &[u8]) -> Uuid {
    Uuid::new_v5(&campaign_id, label)
}

fn authoritative_campaign_owner(
    current_stage: &str,
    stage_topology_contract: &str,
    tool_truth_contract: &str,
    investigation_rollout_mode: &str,
) -> bool {
    tool_truth_contract == "receipt_v1"
        && matches!(
            investigation_rollout_mode,
            "registry_authoritative_legacy_projection" | "new_only"
        )
        && matches!(
            (stage_topology_contract, current_stage),
            ("legacy_candidate_verification_v1", "verification")
                | ("unified_investigation_v1", "investigation")
        )
}

/// Verification Campaigns have two operation-frozen owners. Legacy operations
/// run them in the outer `verification` stage; unified operations run the same
/// host-owned Campaign/JIT/Operator kernel inside their single
/// `investigation` stage. Requiring the topology/stage pair prevents either
/// runtime from borrowing the other's stage state.
async fn require_authoritative_verification_operation(
    pool: &PgPool,
    operation_id: Uuid,
) -> anyhow::Result<()> {
    let operation = sqlx::query_as::<_, (String, String, String, String)>(
        r#"SELECT current_stage,stage_topology_contract,
                  tool_truth_contract,investigation_rollout_mode
             FROM operation_state WHERE operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_optional(pool)
    .await?;
    let operation_is_authoritative =
        operation.is_some_and(|(current_stage, topology, tool_truth, rollout)| {
            authoritative_campaign_owner(&current_stage, &topology, &tool_truth, &rollout)
        });
    anyhow::ensure!(
        operation_is_authoritative,
        "Verification Campaign scheduler requires an operation-frozen verification owner stage"
    );
    Ok(())
}

#[cfg(test)]
mod owner_tests {
    use super::authoritative_campaign_owner;

    #[test]
    fn campaign_owner_is_the_operation_frozen_outer_or_unified_stage() {
        assert!(authoritative_campaign_owner(
            "verification",
            "legacy_candidate_verification_v1",
            "receipt_v1",
            "new_only",
        ));
        assert!(authoritative_campaign_owner(
            "investigation",
            "unified_investigation_v1",
            "receipt_v1",
            "new_only",
        ));
        for (stage, topology) in [
            ("verification", "unified_investigation_v1"),
            ("investigation", "legacy_candidate_verification_v1"),
            ("attack_candidate", "legacy_candidate_verification_v1"),
        ] {
            assert!(!authoritative_campaign_owner(
                stage,
                topology,
                "receipt_v1",
                "new_only",
            ));
        }
        assert!(!authoritative_campaign_owner(
            "investigation",
            "unified_investigation_v1",
            "legacy_v1",
            "new_only",
        ));
    }
}

/// Commit the whole bounded consult census before any provider call and return
/// only lanes that do not yet have an immutable terminal outcome.
pub async fn prepare_authoritative_verification_consults(
    pool: Arc<PgPool>,
    operation_id: Uuid,
) -> anyhow::Result<Vec<VerificationConsultWorkItemView>> {
    require_authoritative_verification_operation(&pool, operation_id).await?;
    let campaigns = sqlx::query_as::<_, CampaignRow>(
        r#"SELECT campaign_id,verification_objective_id,verification_contract_id,
                  state,row_version
             FROM verification_campaigns
            WHERE operation_id=$1 AND state IN ('admitted','running')
              AND terminal_at IS NULL AND superseded_at IS NULL
            ORDER BY organization_id,hypothesis_revision_id,
                     verification_objective_id,campaign_id"#,
    )
    .bind(operation_id)
    .fetch_all(&*pool)
    .await?;
    let repository = PgVerificationCampaignRepository::new(pool.clone());
    for campaign in campaigns {
        let round_request_id = stable(campaign.campaign_id, b"verification-scheduler-round.v1");
        let round_exists: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM verification_campaign_rounds
                    WHERE stable_request_id=$1 AND campaign_id=$2 AND operation_id=$3
               )"#,
        )
        .bind(round_request_id)
        .bind(campaign.campaign_id)
        .bind(operation_id)
        .fetch_one(&*pool)
        .await?;
        if !round_exists {
            repository
                .open_round(OpenCampaignRound {
                    stable_request_id: round_request_id,
                    operation_id,
                    campaign_id: campaign.campaign_id,
                    expected_campaign_row_version: campaign.row_version,
                })
                .await
                .map_err(anyhow::Error::new)?;
        }
    }
    let rows = sqlx::query_as::<_, PendingConsultRow>(
        r#"SELECT consult.operation_id,consult.campaign_id,consult.round_id,
                  consult.consult_id,campaign.verification_objective_id,
                  consult.role_kind,round.round_input_hash,consult.request_packet
             FROM verification_consults consult
             JOIN verification_campaign_rounds round
               ON round.round_id=consult.round_id
              AND round.campaign_id=consult.campaign_id
              AND round.operation_id=consult.operation_id
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=consult.campaign_id
              AND campaign.operation_id=consult.operation_id
             LEFT JOIN verification_consult_terminals terminal
               ON terminal.consult_id=consult.consult_id
            WHERE consult.operation_id=$1 AND consult.disposition='pending'
              AND terminal.consult_id IS NULL
              AND campaign.state='running' AND campaign.terminal_at IS NULL
              AND campaign.superseded_at IS NULL
            ORDER BY campaign.organization_id,campaign.verification_objective_id,
                     consult.consult_ordinal,consult.consult_id"#,
    )
    .bind(operation_id)
    .fetch_all(&*pool)
    .await?;
    let work = rows
        .into_iter()
        .map(|row| VerificationConsultWorkItemView {
            operation_id: row.operation_id,
            campaign_id: row.campaign_id,
            round_id: row.round_id,
            consult_lane_id: row.consult_id,
            objective_id: row.verification_objective_id,
            role_id: row.role_kind,
            input_projection_hash: row.round_input_hash,
            request_packet: row.request_packet,
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(
        work.iter().all(|item| {
            item.operation_id == operation_id
                && golish_sub_agents::executor::verification_campaign::is_verification_campaign_role(
                    &item.role_id,
                )
                && item.request_packet.get("campaign_id")
                    == Some(&serde_json::Value::String(item.campaign_id.to_string()))
                && item.request_packet.get("round_id")
                    == Some(&serde_json::Value::String(item.round_id.to_string()))
                && item.request_packet.get("objective_id")
                    == Some(&serde_json::Value::String(item.objective_id.to_string()))
                && item.request_packet.get("role_id")
                    == Some(&serde_json::Value::String(item.role_id.clone()))
                && item.request_packet.get("input_projection_hash")
                    == Some(&serde_json::Value::String(item.input_projection_hash.clone()))
        }),
        "frozen Campaign consult work item drifted from its owner projection"
    );
    Ok(work)
}

/// Persist exactly one append-only provider outcome. Completed artifacts are
/// parsed again here; noncompleted states may carry only a closed reason code.
#[allow(clippy::type_complexity)]
pub async fn record_authoritative_verification_consult_terminal(
    pool: Arc<PgPool>,
    command: RecordVerificationConsultTerminal,
) -> anyhow::Result<()> {
    require_authoritative_verification_operation(&pool, command.operation_id).await?;
    let state = match command.state {
        VerificationConsultTerminalState::Completed => "completed",
        VerificationConsultTerminalState::Failed => "failed",
        VerificationConsultTerminalState::TimedOut => "timed_out",
        VerificationConsultTerminalState::Cancelled => "cancelled",
    };
    anyhow::ensure!(
        (command.state == VerificationConsultTerminalState::Completed
            && command.response_artifact.is_some()
            && command.reason_code.is_none())
            || (command.state != VerificationConsultTerminalState::Completed
                && command.response_artifact.is_none()
                && command
                    .reason_code
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty() && reason.len() <= 128)),
        "invalid Campaign consult terminal envelope"
    );
    let mut tx = pool.begin().await?;
    let owner: (
        Uuid,
        Uuid,
        Uuid,
        Uuid,
        Uuid,
        String,
        String,
        serde_json::Value,
    ) = sqlx::query_as(
        r#"SELECT consult.round_id,consult.campaign_id,consult.operation_id,
                      consult.project_scope_id,consult.organization_id,consult.role_kind,
                      round.round_input_hash,consult.request_packet
                 FROM verification_consults consult
                 JOIN verification_campaign_rounds round
                   ON round.round_id=consult.round_id
                  AND round.campaign_id=consult.campaign_id
                  AND round.operation_id=consult.operation_id
                WHERE consult.consult_id=$1 AND consult.disposition='pending'
                FOR UPDATE OF consult"#,
    )
    .bind(command.consult_lane_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("Campaign consult lane is not a frozen pending census member")?;
    anyhow::ensure!(
        owner.0 == command.round_id
            && owner.1 == command.campaign_id
            && owner.2 == command.operation_id
            && owner.5 == command.role_id
            && owner.6 == command.input_projection_hash,
        "Campaign consult terminal owner tuple drifted"
    );
    if let Some(artifact) = command.response_artifact.as_ref() {
        let parsed = golish_sub_agents::executor::verification_campaign::parse_campaign_artifact(
            &command.role_id,
            &command.input_projection_hash,
            &serde_json::to_vec(artifact)?,
        )
        .map_err(|error| anyhow::anyhow!("Campaign consult artifact rejected: {error}"))?;
        let objective_id = owner
            .7
            .get("objective_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            .context("Campaign consult request has no objective identity")?;
        anyhow::ensure!(
            parsed.artifact().campaign_id == command.campaign_id
                && parsed.artifact().round_id == command.round_id
                && parsed.artifact().consult_lane_id == command.consult_lane_id
                && parsed.artifact().objective_id == objective_id
                && golish_sub_agents::executor::verification_campaign::campaign_artifact_matches_frozen_request(
                    parsed.artifact(),
                    &owner.7,
                ),
            "Campaign consult artifact identity does not match its frozen lane"
        );
    }
    let response_hash = match command.response_artifact.as_ref() {
        Some(artifact) => Some(
            sqlx::query_scalar::<_, String>("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(artifact)
                .fetch_one(&mut *tx)
                .await?,
        ),
        None => None,
    };
    let existing: Option<(
        Uuid,
        Uuid,
        String,
        Option<serde_json::Value>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        r#"SELECT stable_request_id,consult_id,terminal_state,response_artifact,
                      response_artifact_hash,reason_code
                 FROM verification_consult_terminals
                WHERE stable_request_id=$1 OR consult_id=$2"#,
    )
    .bind(command.stable_request_id)
    .bind(command.consult_lane_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(existing) = existing {
        anyhow::ensure!(
            existing.0 == command.stable_request_id
                && existing.1 == command.consult_lane_id
                && existing.2 == state
                && existing.3 == command.response_artifact
                && existing.4 == response_hash
                && existing.5 == command.reason_code,
            "Campaign consult terminal replay drifted"
        );
        tx.commit().await?;
        return Ok(());
    }
    sqlx::query(
        r#"INSERT INTO verification_consult_terminals(
               consult_terminal_id,stable_request_id,consult_id,round_id,campaign_id,
               operation_id,project_scope_id,organization_id,role_kind,
               input_projection_hash,terminal_state,response_artifact,
               response_artifact_hash,reason_code
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(Uuid::new_v5(
        &command.stable_request_id,
        b"verification-consult-terminal.v1",
    ))
    .bind(command.stable_request_id)
    .bind(command.consult_lane_id)
    .bind(command.round_id)
    .bind(command.campaign_id)
    .bind(command.operation_id)
    .bind(owner.3)
    .bind(owner.4)
    .bind(&command.role_id)
    .bind(&command.input_projection_hash)
    .bind(state)
    .bind(&command.response_artifact)
    .bind(response_hash)
    .bind(&command.reason_code)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[allow(dead_code)]
async fn durable_consult_census_hash(
    pool: &PgPool,
    operation_id: Uuid,
    campaign_id: Uuid,
    round_id: Uuid,
) -> anyhow::Result<String> {
    use golish_sub_agents::executor::verification_campaign::{
        parse_campaign_artifact, seal_consult_census, ConsultLaneState, ConsultLaneTerminal,
    };

    let rows = sqlx::query_as::<_, DurableConsultRow>(
        r#"SELECT consult.consult_id,consult.role_kind,terminal.response_artifact,
                  terminal.terminal_state,round.round_input_hash
             FROM verification_consults consult
             JOIN verification_campaign_rounds round
               ON round.round_id=consult.round_id
              AND round.campaign_id=consult.campaign_id
              AND round.operation_id=consult.operation_id
             LEFT JOIN verification_consult_terminals terminal
               ON terminal.consult_id=consult.consult_id
            WHERE consult.round_id=$1 AND consult.campaign_id=$2
              AND consult.operation_id=$3
            ORDER BY consult.consult_ordinal,consult.consult_id"#,
    )
    .bind(round_id)
    .bind(campaign_id)
    .bind(operation_id)
    .fetch_all(pool)
    .await?;
    anyhow::ensure!(
        (1..=3).contains(&rows.len()),
        "Campaign round consult census is not the bounded 1..=3 exact set"
    );
    let expected_lane_ids = rows.iter().map(|row| row.consult_id).collect::<Vec<_>>();
    let mut terminals = Vec::with_capacity(rows.len());
    for row in rows {
        let terminal_state = row
            .terminal_state
            .as_deref()
            .context("Campaign strategy cannot consume a nonterminal consult lane")?;
        let (state, artifact_hash) = match terminal_state {
            "completed" => {
                let response_artifact = row
                    .response_artifact
                    .context("completed Campaign consult has no durable response artifact")?;
                let parsed = parse_campaign_artifact(
                    &row.role_kind,
                    &row.round_input_hash,
                    &serde_json::to_vec(&response_artifact)?,
                )
                .map_err(|error| {
                    anyhow::anyhow!("durable Campaign consult was rejected: {error}")
                })?;
                anyhow::ensure!(
                    parsed.artifact().campaign_id == campaign_id
                        && parsed.artifact().round_id == round_id
                        && parsed.artifact().consult_lane_id == row.consult_id,
                    "durable Campaign consult identity does not match its owner tuple"
                );
                (
                    ConsultLaneState::Completed,
                    Some(parsed.artifact_hash().to_owned()),
                )
            }
            "failed" => {
                anyhow::ensure!(
                    row.response_artifact.is_none(),
                    "failed consult has an artifact"
                );
                (ConsultLaneState::Failed, None)
            }
            "timed_out" => {
                anyhow::ensure!(
                    row.response_artifact.is_none(),
                    "timed-out consult has an artifact"
                );
                (ConsultLaneState::TimedOut, None)
            }
            "cancelled" => {
                anyhow::ensure!(
                    row.response_artifact.is_none(),
                    "cancelled consult has an artifact"
                );
                (ConsultLaneState::Cancelled, None)
            }
            _ => anyhow::bail!("Campaign consult has an unknown terminal state"),
        };
        terminals.push(ConsultLaneTerminal {
            consult_lane_id: row.consult_id,
            state,
            artifact_hash,
        });
    }
    seal_consult_census(&expected_lane_ids, &terminals)
        .map_err(|error| anyhow::anyhow!("durable Campaign consult census was rejected: {error}"))
}

async fn exact_set_hash_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    domain: &str,
    member_hashes: &[String],
) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar("SELECT investigation_exact_member_set_hash($1,$2::TEXT[])")
            .bind(domain)
            .bind(member_hashes)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn json_hash_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    value: serde_json::Value,
) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(value)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn seal_wave_partition_and_fixed_point(
    pool: &PgPool,
    operation_id: Uuid,
    wave: &WaveRow,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let locked: (i64, String) = sqlx::query_as(
        r#"SELECT member_count,source_snapshot_hash
             FROM verification_wave_coverage_denominators
            WHERE wave_denominator_id=$1 AND operation_id=$2
              AND project_scope_id=$3 AND organization_id=$4
              AND sealed_at IS NOT NULL
            FOR UPDATE"#,
    )
    .bind(wave.wave_denominator_id)
    .bind(operation_id)
    .bind(wave.project_scope_id)
    .bind(wave.organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("Wave denominator disappeared before exact-partition sealing")?;
    anyhow::ensure!(
        locked.0 == wave.member_count && locked.1 == wave.source_snapshot_hash,
        "Wave denominator authority drifted before exact-partition sealing"
    );
    let already_fixed: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM verification_wave_coverage_receipts coverage
                 JOIN hypothesis_consolidation_batches batch
                   ON batch.wave_coverage_receipt_id=coverage.wave_coverage_receipt_id
                  AND batch.sealed_at IS NOT NULL
                 JOIN hypothesis_consolidation_receipts consolidation
                   ON consolidation.consolidation_batch_id=batch.consolidation_batch_id
                  AND consolidation.disposition='fixed_point'
                 JOIN hypothesis_fixed_point_receipts fixed
                   ON fixed.consolidation_receipt_id=consolidation.consolidation_receipt_id
                  AND fixed.generation_id=batch.generation_id
                WHERE coverage.wave_denominator_id=$1
           )"#,
    )
    .bind(wave.wave_denominator_id)
    .fetch_one(&mut *tx)
    .await?;
    if already_fixed {
        tx.commit().await?;
        return Ok(());
    }

    let selected = sqlx::query_as::<_, WaveSelectedResultRow>(
        r#"SELECT campaign_receipt.campaign_coverage_receipt_id,
                  campaign_receipt.receipt_hash AS campaign_receipt_hash,
                  wave_member.wave_coverage_member_id,result.result_hash,
                  campaign_receipt.coverage_status
             FROM verification_campaigns campaign
             JOIN verification_campaign_coverage_denominators denominator
               ON denominator.campaign_id=campaign.campaign_id
              AND denominator.wave_denominator_id=$1
              AND denominator.sealed_at IS NOT NULL
             JOIN verification_campaign_coverage_receipts campaign_receipt
               ON campaign_receipt.campaign_denominator_id=denominator.campaign_denominator_id
              AND campaign_receipt.campaign_id=campaign.campaign_id
             JOIN verification_campaign_coverage_results result
               ON result.campaign_coverage_receipt_id=
                  campaign_receipt.campaign_coverage_receipt_id
             JOIN verification_campaign_coverage_members campaign_member
               ON campaign_member.campaign_coverage_member_id=
                  result.campaign_coverage_member_id
              AND campaign_member.campaign_denominator_id=
                  denominator.campaign_denominator_id
             JOIN verification_wave_coverage_members wave_member
               ON wave_member.wave_coverage_member_id=
                  campaign_member.wave_coverage_member_id
              AND wave_member.wave_denominator_id=denominator.wave_denominator_id
            WHERE campaign.operation_id=$2 AND campaign.terminal_at IS NOT NULL
              AND campaign.superseded_at IS NULL
            ORDER BY campaign.campaign_id,campaign_member.member_ordinal"#,
    )
    .bind(wave.wave_denominator_id)
    .bind(operation_id)
    .fetch_all(&mut *tx)
    .await?;
    let selected_wave_members = selected
        .iter()
        .map(|row| row.wave_coverage_member_id)
        .collect::<std::collections::BTreeSet<_>>();
    anyhow::ensure!(
        i64::try_from(selected.len()).ok() == Some(wave.member_count)
            && selected_wave_members.len() == selected.len(),
        "Wave Campaign results are not an exact, duplicate-free denominator partition"
    );
    let campaign_receipt_hashes = selected
        .iter()
        .map(|row| {
            (
                row.campaign_coverage_receipt_id,
                row.campaign_receipt_hash.clone(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();
    let result_hashes = selected
        .iter()
        .map(|row| row.result_hash.clone())
        .collect::<Vec<_>>();
    let selected_campaign_receipt_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_wave_selected_campaign_receipts.v1",
        &campaign_receipt_hashes,
    )
    .await?;
    let unassigned_result_set_hash =
        exact_set_hash_on(&mut tx, "verification_wave_unassigned_results.v1", &[]).await?;
    let result_member_set_hash = exact_set_hash_on(
        &mut tx,
        "verification_wave_coverage_results.v1",
        &result_hashes,
    )
    .await?;
    let coverage_status = if selected.iter().all(|row| row.coverage_status == "complete") {
        "complete"
    } else {
        "partial"
    };
    let wave_receipt_hash = json_hash_on(
        &mut tx,
        json!({
            "wave_denominator_id": wave.wave_denominator_id,
            "selected_campaign_receipt_set_hash": selected_campaign_receipt_set_hash,
            "unassigned_result_set_hash": unassigned_result_set_hash,
            "result_member_count": selected.len(),
            "result_member_set_hash": result_member_set_hash,
            "coverage_status": coverage_status,
        }),
    )
    .await?;
    let wave_request_id = stable(
        wave.wave_denominator_id,
        b"verification-scheduler-wave-coverage-request.v1",
    );
    let wave_coverage_receipt_id =
        stable(wave_request_id, b"verification-wave-coverage-receipt.v1");
    sqlx::query(
        r#"INSERT INTO verification_wave_coverage_receipts(
               wave_coverage_receipt_id,stable_request_id,wave_denominator_id,
               operation_id,project_scope_id,organization_id,
               selected_campaign_receipt_set_hash,unassigned_result_set_hash,
               result_member_count,result_member_set_hash,coverage_status,receipt_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(wave_coverage_receipt_id)
    .bind(wave_request_id)
    .bind(wave.wave_denominator_id)
    .bind(operation_id)
    .bind(wave.project_scope_id)
    .bind(wave.organization_id)
    .bind(&selected_campaign_receipt_set_hash)
    .bind(&unassigned_result_set_hash)
    .bind(i64::try_from(selected.len())?)
    .bind(&result_member_set_hash)
    .bind(coverage_status)
    .bind(&wave_receipt_hash)
    .execute(&mut *tx)
    .await?;

    let fact_deltas = sqlx::query_as::<_, WaveFactDeltaRow>(
        r#"SELECT bundle.fact_delta_bundle_id,bundle.fact_delta_hash,bundle.delta_kind
             FROM verification_fact_delta_bundles bundle
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=bundle.campaign_id
              AND campaign.operation_id=bundle.operation_id
            WHERE campaign.wave_denominator_id=$1
              AND campaign.operation_id=$2 AND campaign.terminal_at IS NOT NULL
              AND campaign.superseded_at IS NULL
            ORDER BY bundle.fact_delta_bundle_id"#,
    )
    .bind(wave.wave_denominator_id)
    .bind(operation_id)
    .fetch_all(&mut *tx)
    .await?;
    anyhow::ensure!(
        !fact_deltas.is_empty(),
        "Wave fixed-point consolidation has no terminal FactDelta bundles"
    );
    anyhow::ensure!(
        fact_deltas
            .iter()
            .all(|delta| matches!(delta.delta_kind.as_str(), "inconclusive" | "no_change")),
        "Material FactDelta requires the Registry evolution reducer; fixed-point closeout refused"
    );
    let mut consumption_hashes = Vec::with_capacity(fact_deltas.len());
    for delta in &fact_deltas {
        let consumption_request_id = Uuid::new_v5(
            &wave.wave_denominator_id,
            format!(
                "verification-fact-delta-no-change.v1:{}",
                delta.fact_delta_bundle_id
            )
            .as_bytes(),
        );
        let consumption_id = stable(consumption_request_id, b"fact-delta-consumption.v1");
        let consumption_hash = json_hash_on(
            &mut tx,
            json!({
                "fact_delta_bundle_id": delta.fact_delta_bundle_id,
                "generation_id": wave.generation_id,
                "disposition": "no_semantic_change",
                "residual_id": null,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO fact_delta_consumptions(
                   fact_delta_consumption_id,stable_request_id,fact_delta_bundle_id,
                   operation_id,project_scope_id,organization_id,generation_id,
                   disposition,consumption_hash,residual_id
               ) VALUES($1,$2,$3,$4,$5,$6,$7,'no_semantic_change',$8,NULL)"#,
        )
        .bind(consumption_id)
        .bind(consumption_request_id)
        .bind(delta.fact_delta_bundle_id)
        .bind(operation_id)
        .bind(wave.project_scope_id)
        .bind(wave.organization_id)
        .bind(wave.generation_id)
        .bind(&consumption_hash)
        .execute(&mut *tx)
        .await?;
        consumption_hashes.push(consumption_hash);
    }
    let fact_delta_hashes = fact_deltas
        .iter()
        .map(|delta| delta.fact_delta_hash.clone())
        .collect::<Vec<_>>();
    let fact_delta_member_set_hash = exact_set_hash_on(
        &mut tx,
        "hypothesis_consolidation_fact_deltas.v1",
        &fact_delta_hashes,
    )
    .await?;
    let applied_fact_delta_set_hash = exact_set_hash_on(
        &mut tx,
        "hypothesis_consolidation_consumptions.v1",
        &consumption_hashes,
    )
    .await?;
    let residual_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT DISTINCT residual.residual_hash
             FROM verification_campaigns campaign
             JOIN verification_campaign_coverage_denominators denominator
               ON denominator.campaign_id=campaign.campaign_id
              AND denominator.wave_denominator_id=$1
             JOIN verification_campaign_coverage_receipts receipt
               ON receipt.campaign_denominator_id=denominator.campaign_denominator_id
             JOIN verification_campaign_coverage_results result
               ON result.campaign_coverage_receipt_id=receipt.campaign_coverage_receipt_id
             JOIN hypothesis_residual_risks residual
               ON residual.residual_id=result.residual_id
            WHERE campaign.operation_id=$2
            ORDER BY residual.residual_hash"#,
    )
    .bind(wave.wave_denominator_id)
    .bind(operation_id)
    .fetch_all(&mut *tx)
    .await?;
    let residual_set_hash = exact_set_hash_on(
        &mut tx,
        "hypothesis_consolidation_residuals.v1",
        &residual_hashes,
    )
    .await?;
    let unassigned_residual_set_hash = exact_set_hash_on(
        &mut tx,
        "hypothesis_consolidation_unassigned_residuals.v1",
        &[],
    )
    .await?;
    let batch_request_id = stable(
        wave.wave_denominator_id,
        b"verification-scheduler-consolidation-batch-request.v1",
    );
    let consolidation_batch_id = stable(batch_request_id, b"hypothesis-consolidation-batch.v1");
    sqlx::query(
        r#"INSERT INTO hypothesis_consolidation_batches(
               consolidation_batch_id,stable_request_id,operation_id,project_scope_id,
               organization_id,generation_id,wave_coverage_receipt_id,
               fact_delta_member_count,fact_delta_member_set_hash,
               unassigned_residual_set_hash,source_snapshot_hash,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,statement_timestamp())"#,
    )
    .bind(consolidation_batch_id)
    .bind(batch_request_id)
    .bind(operation_id)
    .bind(wave.project_scope_id)
    .bind(wave.organization_id)
    .bind(wave.generation_id)
    .bind(wave_coverage_receipt_id)
    .bind(i64::try_from(fact_deltas.len())?)
    .bind(&fact_delta_member_set_hash)
    .bind(&unassigned_residual_set_hash)
    .bind(&wave.source_snapshot_hash)
    .execute(&mut *tx)
    .await?;
    let consolidation_request_id = stable(
        wave.wave_denominator_id,
        b"verification-scheduler-consolidation-receipt-request.v1",
    );
    let consolidation_receipt_id = stable(
        consolidation_request_id,
        b"hypothesis-consolidation-receipt.v1",
    );
    let consolidation_receipt_hash = json_hash_on(
        &mut tx,
        json!({
            "consolidation_batch_id": consolidation_batch_id,
            "disposition": "fixed_point",
            "applied_fact_delta_set_hash": applied_fact_delta_set_hash,
            "residual_set_hash": residual_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_consolidation_receipts(
               consolidation_receipt_id,stable_request_id,consolidation_batch_id,
               successor_generation_id,disposition,applied_fact_delta_set_hash,
               residual_set_hash,receipt_hash
           ) VALUES($1,$2,$3,NULL,'fixed_point',$4,$5,$6)"#,
    )
    .bind(consolidation_receipt_id)
    .bind(consolidation_request_id)
    .bind(consolidation_batch_id)
    .bind(&applied_fact_delta_set_hash)
    .bind(&residual_set_hash)
    .bind(&consolidation_receipt_hash)
    .execute(&mut *tx)
    .await?;
    let open_obligation_set_hash = exact_set_hash_on(
        &mut tx,
        "hypothesis_fixed_point_open_obligations.v1",
        &residual_hashes,
    )
    .await?;
    let fixed_point_request_id = stable(
        wave.wave_denominator_id,
        b"verification-scheduler-fixed-point-request.v1",
    );
    let fixed_point_receipt_id =
        stable(fixed_point_request_id, b"hypothesis-fixed-point-receipt.v1");
    let fixed_point_hash = json_hash_on(
        &mut tx,
        json!({
            "consolidation_receipt_id": consolidation_receipt_id,
            "generation_id": wave.generation_id,
            "open_obligation_set_hash": open_obligation_set_hash,
            "residual_set_hash": residual_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_fixed_point_receipts(
               fixed_point_receipt_id,stable_request_id,consolidation_receipt_id,
               generation_id,open_obligation_set_hash,residual_set_hash,fixed_point_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
    )
    .bind(fixed_point_receipt_id)
    .bind(fixed_point_request_id)
    .bind(consolidation_receipt_id)
    .bind(wave.generation_id)
    .bind(&open_obligation_set_hash)
    .bind(&residual_set_hash)
    .bind(&fixed_point_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn finalize_all_operation_waves(
    pool: &PgPool,
    operation_id: Uuid,
) -> anyhow::Result<(u32, u32)> {
    let waves = sqlx::query_as::<_, WaveRow>(
        r#"SELECT wave.wave_denominator_id,generation.generation_id,
                  wave.project_scope_id,wave.organization_id,wave.member_count,
                  wave.source_snapshot_hash
             FROM verification_wave_coverage_denominators wave
             JOIN hypothesis_generation_seals generation_seal
               ON generation_seal.seal_id=wave.generation_seal_id
             JOIN hypothesis_generations generation
               ON generation.generation_id=generation_seal.generation_id
              AND generation.operation_id=wave.operation_id
              AND generation.organization_id=wave.organization_id
            WHERE wave.operation_id=$1 AND wave.sealed_at IS NOT NULL
            ORDER BY wave.organization_id,generation.generation_ordinal,
                     wave.wave_denominator_id"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await?;
    anyhow::ensure!(
        !waves.is_empty(),
        "Authoritative Verification operation has no sealed Wave denominator"
    );
    for wave in &waves {
        seal_wave_partition_and_fixed_point(pool, operation_id, wave).await?;
    }
    let fixed_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(DISTINCT wave.wave_denominator_id)
             FROM verification_wave_coverage_denominators wave
             JOIN hypothesis_generation_seals generation_seal
               ON generation_seal.seal_id=wave.generation_seal_id
             JOIN verification_wave_coverage_receipts coverage
               ON coverage.wave_denominator_id=wave.wave_denominator_id
             JOIN hypothesis_consolidation_batches batch
               ON batch.wave_coverage_receipt_id=coverage.wave_coverage_receipt_id
              AND batch.sealed_at IS NOT NULL
             JOIN hypothesis_consolidation_receipts consolidation
               ON consolidation.consolidation_batch_id=batch.consolidation_batch_id
              AND consolidation.disposition='fixed_point'
             JOIN hypothesis_fixed_point_receipts fixed
               ON fixed.consolidation_receipt_id=consolidation.consolidation_receipt_id
              AND fixed.generation_id=batch.generation_id
            WHERE wave.operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await?;
    Ok((u32::try_from(waves.len())?, u32::try_from(fixed_count)?))
}

async fn adjudicate_all_operation_revisions(
    pool: &PgPool,
    repository: &PgVerificationCampaignRepository,
    operation_id: Uuid,
) -> anyhow::Result<(u32, u32)> {
    let subjects = sqlx::query_as::<_, RevisionAdjudicationSubject>(
        r#"SELECT DISTINCT candidate_snapshot.scope_snapshot_id,
                  generation.organization_id,generation_seal.seal_id AS generation_seal_id,
                  revision.revision_id AS hypothesis_revision_id,
                  plan.plan_id AS verification_plan_id
             FROM verification_wave_coverage_denominators wave
             JOIN hypothesis_generation_seals generation_seal
               ON generation_seal.seal_id=wave.generation_seal_id
             JOIN hypothesis_generations generation
               ON generation.generation_id=generation_seal.generation_id
              AND generation.operation_id=wave.operation_id
              AND generation.organization_id=wave.organization_id
             JOIN candidate_analysis_snapshots candidate_snapshot
               ON candidate_snapshot.snapshot_id=generation.candidate_snapshot_id
              AND candidate_snapshot.operation_id=generation.operation_id
              AND candidate_snapshot.organization_id=generation.organization_id
             JOIN hypothesis_generation_members generation_member
               ON generation_member.generation_id=generation.generation_id
              AND generation_member.operation_id=generation.operation_id
              AND generation_member.organization_id=generation.organization_id
             JOIN attack_hypothesis_revisions revision
               ON revision.revision_id=generation_member.revision_id
              AND revision.operation_id=generation.operation_id
              AND revision.organization_id=generation.organization_id
             JOIN attack_hypothesis_verification_plans plan
               ON plan.revision_id=revision.revision_id AND plan.sealed_at IS NOT NULL
            WHERE wave.operation_id=$1
            ORDER BY generation.organization_id,generation_seal.seal_id,
                     revision.revision_id,plan.plan_id"#,
    )
    .bind(operation_id)
    .fetch_all(pool)
    .await?;
    anyhow::ensure!(
        !subjects.is_empty(),
        "Authoritative Verification fixed point has no revision adjudication subjects"
    );
    for subject in &subjects {
        repository
            .adjudicate_hypothesis_revision_with_fresh_tool_truth(AdjudicateHypothesisRevision {
                stable_consumer_request_id: Uuid::new_v5(
                    &subject.hypothesis_revision_id,
                    b"verification-scheduler-revision-adjudication.v1",
                ),
                operation_id,
                scope_snapshot_id: subject.scope_snapshot_id,
                organization_id: subject.organization_id,
                generation_seal_id: subject.generation_seal_id,
                hypothesis_revision_id: subject.hypothesis_revision_id,
                verification_plan_id: subject.verification_plan_id,
            })
            .await
            .map_err(anyhow::Error::new)?;
    }
    let adjudicated_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM hypothesis_revision_adjudications adjudication
             JOIN attack_hypothesis_verification_plans plan
               ON plan.plan_id=adjudication.verification_plan_id
              AND plan.revision_id=adjudication.hypothesis_revision_id
             JOIN hypothesis_generation_members generation_member
               ON generation_member.revision_id=adjudication.hypothesis_revision_id
              AND generation_member.operation_id=adjudication.operation_id
              AND generation_member.organization_id=adjudication.organization_id
             JOIN hypothesis_generation_seals generation_seal
               ON generation_seal.generation_id=generation_member.generation_id
             JOIN verification_wave_coverage_denominators wave
               ON wave.generation_seal_id=generation_seal.seal_id
              AND wave.operation_id=adjudication.operation_id
              AND wave.organization_id=adjudication.organization_id
            WHERE adjudication.operation_id=$1"#,
    )
    .bind(operation_id)
    .fetch_one(pool)
    .await?;
    Ok((
        u32::try_from(subjects.len())?,
        u32::try_from(adjudicated_count)?,
    ))
}

async fn close_campaign_from_current_oracles(
    pool: &PgPool,
    repository: &PgVerificationCampaignRepository,
    operation_id: Uuid,
    campaign: &CampaignRow,
) -> anyhow::Result<()> {
    let (coverage_denominator_seal_id, campaign_row_version): (Uuid, i64) = sqlx::query_as(
        r#"SELECT denominator.campaign_denominator_id,campaign.row_version
             FROM verification_campaigns campaign
             JOIN verification_campaign_coverage_denominators denominator
               ON denominator.campaign_id=campaign.campaign_id
              AND denominator.sealed_at IS NOT NULL
            WHERE campaign.campaign_id=$1 AND campaign.operation_id=$2
              AND campaign.state='running' AND campaign.terminal_at IS NULL
              AND campaign.superseded_at IS NULL"#,
    )
    .bind(campaign.campaign_id)
    .bind(operation_id)
    .fetch_optional(pool)
    .await?
    .context("Campaign oracle closeout authority is not current")?;
    let census = repository
        .seal_oracle_census(SealOracleCensus {
            stable_request_id: stable(
                campaign.campaign_id,
                b"verification-scheduler-oracle-census.v1",
            ),
            operation_id,
            campaign_id: campaign.campaign_id,
            coverage_denominator_seal_id,
            expected_campaign_row_version: campaign_row_version,
        })
        .await
        .map_err(anyhow::Error::new)?;
    repository
        .close_campaign_objective(CloseCampaignObjective {
            stable_request_id: stable(
                campaign.campaign_id,
                b"verification-scheduler-campaign-closeout.v1",
            ),
            operation_id,
            campaign_id: campaign.campaign_id,
            objective_id: campaign.verification_objective_id,
            oracle_census_seal_id: census.seal_id,
            coverage_denominator_seal_id,
            expected_campaign_row_version: campaign_row_version,
        })
        .await
        .map_err(anyhow::Error::new)?;
    Ok(())
}

async fn campaign_has_exact_sealed_compile_residual(
    pool: &PgPool,
    operation_id: Uuid,
    campaign_id: Uuid,
) -> anyhow::Result<bool> {
    let rows = sqlx::query_as::<_, SealedCompileResidualDisposition>(
        r#"SELECT COUNT(*) AS apply_count,
                  COUNT(*) FILTER(WHERE apply.result_kind='residual') AS residual_count,
                  COUNT(residual.residual_id) AS valid_residual_count,
                  COUNT(*) FILTER(WHERE apply.result_kind='prepared_action')
                      AS prepared_action_count,
                  (SELECT COUNT(*)
                     FROM verification_strategy_obligations obligation
                    WHERE obligation.strategy_artifact_id=apply.strategy_artifact_id
                      AND obligation.disposition='planned') AS planned_obligation_count
             FROM investigation_verification_task_advisory_members member
             JOIN investigation_verification_task_advisory_receipts receipt
               ON receipt.advisory_receipt_id=member.advisory_receipt_id
              AND receipt.verification_task_id=member.verification_task_id
              AND receipt.operation_id=$1
              AND receipt.status='applied'
             JOIN investigation_verification_task_advisory_seals seal
               ON seal.advisory_receipt_id=receipt.advisory_receipt_id
              AND seal.verification_task_id=receipt.verification_task_id
             JOIN investigation_verification_advisory_campaign_applies apply
               ON apply.advisory_receipt_id=member.advisory_receipt_id
              AND apply.advisory_member_id=member.advisory_member_id
              AND apply.campaign_id=member.campaign_id
             LEFT JOIN hypothesis_residual_risks residual
               ON residual.residual_id=apply.result_id
              AND residual.operation_id=receipt.operation_id
              AND residual.organization_id=receipt.organization_id
              AND residual.revision_id=receipt.hypothesis_revision_id
              AND residual.reason_code='investigation_verification_action_not_compilable'
              AND residual.owner_kind='plan_c'
              AND residual.residual_hash=apply.result_sha256
              AND residual.closed_at IS NULL
            WHERE member.campaign_id=$2
            GROUP BY receipt.advisory_receipt_id,member.advisory_member_id,
                     apply.strategy_artifact_id"#,
    )
    .bind(operation_id)
    .bind(campaign_id)
    .fetch_all(pool)
    .await?;
    anyhow::ensure!(
        rows.len() <= 1,
        "Campaign has multiple sealed VerificationTask advisory authorities"
    );
    Ok(rows.first().is_some_and(is_exact_terminal_compile_residual))
}

async fn execute_authorized_action_and_close_campaign(
    pool: Arc<PgPool>,
    repository: &PgVerificationCampaignRepository,
    operation_id: Uuid,
    campaign: &CampaignRow,
    action: &PreparedActionState,
) -> anyhow::Result<()> {
    let authorization_receipt_id = action
        .authorization_receipt_id
        .context("Authorized prepared action is missing its JIT receipt")?;
    let action_execution_id = if let Some(execution_id) = action.action_execution_id {
        execution_id
    } else {
        repository
            .begin_action(BeginPreparedAction {
                stable_request_id: stable(
                    action.prepared_action_id,
                    b"verification-scheduler-action-begin.v1",
                ),
                operation_id,
                campaign_id: campaign.campaign_id,
                prepared_action_id: action.prepared_action_id,
                authorization_receipt_id,
                expected_action_row_version: action.row_version,
                expected_campaign_dispatch_generation: action
                    .campaign_dispatch_generation
                    .context("Authorized action is missing its dispatch generation")?,
            })
            .await
            .map_err(anyhow::Error::new)?
            .execution_id
    };
    let execution = super::verification_send_authority::execute_authorized_prepared_action_v1(
        pool.clone(),
        super::verification_send_authority::ExecuteAuthorizedPreparedActionV1 {
            stable_request_id: stable(
                action.prepared_action_id,
                b"verification-scheduler-action-execute.v1",
            ),
            operation_id,
            campaign_id: campaign.campaign_id,
            prepared_action_id: action.prepared_action_id,
            authorization_receipt_id,
            action_execution_id,
        },
    )
    .await?;
    anyhow::ensure!(
        !execution.capability_execution_receipt_id.is_nil()
            && !execution.oracle_assessment_id.is_nil(),
        "Authorized action returned incomplete receipt/oracle authority"
    );
    close_campaign_from_current_oracles(&pool, repository, operation_id, campaign).await
}

async fn prepare_campaign_to_jit(
    pool: &PgPool,
    repository: &PgVerificationCampaignRepository,
    operation_id: Uuid,
    campaign: &CampaignRow,
) -> anyhow::Result<Uuid> {
    let (round_id, strategy_artifact_id, advisory_request_id, current_campaign_row_version): (
        Uuid,
        Uuid,
        Uuid,
        i64,
    ) = sqlx::query_as(
        r#"SELECT strategy.round_id,strategy.strategy_artifact_id,
                  (strategy.typed_strategy->>'advisory_request_id')::UUID,
                  campaign.row_version
             FROM verification_strategy_artifacts strategy
             JOIN verification_campaign_rounds round
               ON round.round_id=strategy.round_id
              AND round.campaign_id=strategy.campaign_id
              AND round.operation_id=strategy.operation_id
             JOIN verification_campaigns campaign
               ON campaign.campaign_id=strategy.campaign_id
              AND campaign.operation_id=strategy.operation_id
            WHERE strategy.campaign_id=$1 AND strategy.operation_id=$2
              AND strategy.decision_kind='compile_action'
              AND strategy.typed_strategy->>'schema'='investigation_verification_strategy.v1'
              AND strategy.typed_strategy->>'campaign_id'=$1::TEXT
              AND strategy.typed_strategy->>'objective_id'=campaign.verification_objective_id::TEXT
              AND strategy.typed_strategy->>'capability' IN (
                  'verify.anonymous_authenticated_differential.v1',
                  'verify.directory_fingerprint.v1',
                  'verify.nuclei_exact_replay.v1',
                  'verify.concurrent_race_differential.v1'
              )
              AND campaign.state='running' AND campaign.terminal_at IS NULL
              AND campaign.superseded_at IS NULL
            ORDER BY round.round_ordinal DESC,strategy.created_at DESC
            LIMIT 1"#,
    )
    .bind(campaign.campaign_id)
    .bind(operation_id)
    .fetch_optional(pool)
    .await?
    .context(
        "Campaign has no Primary-selected sealed strategy; host fallback planning is forbidden",
    )?;

    let denominator_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM verification_campaign_coverage_denominators
                WHERE campaign_id=$1 AND operation_id=$2 AND sealed_at IS NOT NULL
           )"#,
    )
    .bind(campaign.campaign_id)
    .bind(operation_id)
    .fetch_one(pool)
    .await?;
    if !denominator_exists {
        repository
            .seal_coverage_denominator(SealCampaignCoverageDenominator {
                stable_request_id: stable(
                    campaign.campaign_id,
                    b"verification-scheduler-campaign-denominator.v1",
                ),
                operation_id,
                campaign_id: campaign.campaign_id,
                round_id,
                objective_id: campaign.verification_objective_id,
                verification_contract_id: campaign.verification_contract_id,
                expected_campaign_row_version: current_campaign_row_version,
            })
            .await
            .map_err(anyhow::Error::new)?;
    }

    let obligation = sqlx::query_as::<_, StrategyObligationRow>(
        r#"SELECT strategy.strategy_artifact_id,obligation.obligation_id
             FROM verification_strategy_artifacts strategy
             JOIN verification_strategy_obligations obligation
               ON obligation.strategy_artifact_id=strategy.strategy_artifact_id
              AND obligation.disposition='planned'
             JOIN verification_campaign_coverage_denominators denominator
               ON denominator.campaign_id=strategy.campaign_id
              AND denominator.sealed_at IS NOT NULL
             JOIN verification_campaign_coverage_members member
               ON member.campaign_denominator_id=denominator.campaign_denominator_id
              AND member.semantic_key=obligation.semantic_key
              AND member.expected_capability_kind=obligation.obligation_kind
             JOIN verification_capability_assessments assessment
               ON assessment.assessment_id=member.capability_assessment_id
              AND assessment.status='available'
            WHERE strategy.strategy_artifact_id=$1
              AND strategy.round_id=$2 AND strategy.campaign_id=$3
              AND strategy.operation_id=$4
              AND strategy.typed_strategy->>'capability'=obligation.obligation_kind
            ORDER BY obligation.obligation_ordinal
            LIMIT 1"#,
    )
    .bind(strategy_artifact_id)
    .bind(round_id)
    .bind(campaign.campaign_id)
    .bind(operation_id)
    .fetch_optional(pool)
    .await?
    .context("Campaign has no Primary-selected available strategy obligation to compile")?;

    let prior_action_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM verification_prepared_actions WHERE campaign_id=$1 AND operation_id=$2",
    )
    .bind(campaign.campaign_id)
    .bind(operation_id)
    .fetch_one(pool)
    .await?;
    let stable_name = if prior_action_count == 0 {
        format!(
            "campaign:{}:obligation:{}",
            campaign.campaign_id, obligation.obligation_id
        )
    } else {
        format!(
            "campaign:{}:obligation:{}:review-recovery:{}",
            campaign.campaign_id, obligation.obligation_id, prior_action_count
        )
    };

    let proposal = repository
        .propose_prepared_action(ProposePreparedAction {
            stable_request_id: Uuid::new_v5(&advisory_request_id, stable_name.as_bytes()),
            operation_id,
            campaign_id: campaign.campaign_id,
            round_id,
            strategy_artifact_id: obligation.strategy_artifact_id,
            strategy_obligation_id: obligation.obligation_id,
        })
        .await
        .map_err(anyhow::Error::new)?;
    Ok(proposal.prepared_action_id)
}

pub async fn drive_authoritative_verification_campaigns(
    pool: Arc<PgPool>,
    operation_id: Uuid,
) -> anyhow::Result<VerificationCampaignSchedulerView> {
    require_authoritative_verification_operation(&pool, operation_id).await?;

    let campaigns = sqlx::query_as::<_, CampaignRow>(
        r#"SELECT campaign_id,verification_objective_id,verification_contract_id,
                  state,row_version
             FROM verification_campaigns
            WHERE operation_id=$1
            ORDER BY organization_id,hypothesis_revision_id,
                     verification_objective_id,campaign_id"#,
    )
    .bind(operation_id)
    .fetch_all(&*pool)
    .await?;
    anyhow::ensure!(
        !campaigns.is_empty(),
        "Authoritative Verification stage has no admitted Campaign"
    );

    let repository = PgVerificationCampaignRepository::new(pool.clone());
    let mut view = VerificationCampaignSchedulerView {
        campaign_count: u32::try_from(campaigns.len())?,
        pending_authorization_count: 0,
        authorized_count: 0,
        started_count: 0,
        awaiting_oracle_count: 0,
        terminal_count: 0,
        blocked_count: 0,
        wave_count: 0,
        fixed_point_wave_count: 0,
        revision_count: 0,
        adjudicated_revision_count: 0,
        pending_prepared_action_ids: Vec::new(),
    };
    for campaign in campaigns {
        match campaign.state.as_str() {
            "terminal" => {
                view.terminal_count = view.terminal_count.saturating_add(1);
                continue;
            }
            "superseded" | "stopping" | "draining" => {
                view.blocked_count = view.blocked_count.saturating_add(1);
                continue;
            }
            "admitted" | "running" => {}
            _ => {
                view.blocked_count = view.blocked_count.saturating_add(1);
                continue;
            }
        }
        let mut action =
            load_latest_prepared_action(&pool, operation_id, campaign.campaign_id).await?;
        if action.is_none()
            && campaign_has_exact_sealed_compile_residual(&pool, operation_id, campaign.campaign_id)
                .await?
        {
            close_campaign_from_current_oracles(&pool, &repository, operation_id, &campaign)
                .await?;
            view.terminal_count = view.terminal_count.saturating_add(1);
            continue;
        }
        // At most one review-TTL recovery action is materialized.  The loop
        // only advances durable, host-owned transitions: create -> policy
        // decision, or expire -> one fresh review packet -> policy decision.
        for _ in 0..4 {
            if let Some(current) = action.as_ref() {
                if matches!(
                    current.state.as_str(),
                    "pending_authorization" | "authorized"
                ) {
                    let receipt = golish_db::repo::verification_prepared_actions::reconcile_prepared_action_scheduler_authority(
                        &pool,
                        &golish_db::repo::verification_prepared_actions::ReconcilePreparedActionSchedulerAuthority {
                            prepared_action_id: current.prepared_action_id,
                            campaign_id: campaign.campaign_id,
                            operation_id,
                            expected_action_row_version: current.row_version,
                        },
                    )
                    .await?;
                    if receipt.disposition
                        != golish_db::repo::verification_prepared_actions::PreparedActionSchedulerAuthorityDisposition::Unchanged
                    {
                        action = load_latest_prepared_action(
                            &pool,
                            operation_id,
                            campaign.campaign_id,
                        )
                        .await?;
                        continue;
                    }
                }
                if current.state == "expired" {
                    let action_count: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM verification_prepared_actions WHERE campaign_id=$1 AND operation_id=$2",
                    )
                    .bind(campaign.campaign_id)
                    .bind(operation_id)
                    .fetch_one(&*pool)
                    .await?;
                    if action_count == 1 {
                        prepare_campaign_to_jit(&pool, &repository, operation_id, &campaign)
                            .await?;
                        action =
                            load_latest_prepared_action(&pool, operation_id, campaign.campaign_id)
                                .await?;
                        continue;
                    }
                }
                break;
            }
            prepare_campaign_to_jit(&pool, &repository, operation_id, &campaign).await?;
            action = load_latest_prepared_action(&pool, operation_id, campaign.campaign_id).await?;
        }
        match action.as_ref().map(|action| action.state.as_str()) {
            Some("pending_authorization") => {
                view.pending_authorization_count =
                    view.pending_authorization_count.saturating_add(1);
                view.pending_prepared_action_ids
                    .push(action.expect("matched Some action").prepared_action_id);
            }
            Some("authorized") | Some("started") => {
                let action = action.as_ref().expect("matched Some action");
                execute_authorized_action_and_close_campaign(
                    pool.clone(),
                    &repository,
                    operation_id,
                    &campaign,
                    action,
                )
                .await?;
                view.terminal_count = view.terminal_count.saturating_add(1);
            }
            Some("succeeded") | Some("failed") => {
                close_campaign_from_current_oracles(&pool, &repository, operation_id, &campaign)
                    .await?;
                view.terminal_count = view.terminal_count.saturating_add(1);
            }
            Some("outcome_unknown") => {
                let action = action.as_ref().expect("matched Some action");
                let semantic_landing_complete: bool = sqlx::query_scalar(
                    r#"SELECT EXISTS(
                           SELECT 1
                             FROM verification_action_executions execution
                             JOIN capability_execution_receipts receipt
                               ON receipt.id=execution.capability_execution_receipt_id
                              AND receipt.finalized_at IS NOT NULL
                             JOIN verification_oracle_assessments oracle
                               ON oracle.action_execution_id=execution.action_execution_id
                              AND oracle.prepared_action_id=execution.prepared_action_id
                              AND oracle.observation_receipt_hash=receipt.receipt_authority_hash
                             JOIN hypothesis_residual_risks residual
                               ON residual.residual_id=oracle.residual_id
                              AND residual.operation_id=oracle.operation_id
                              AND residual.organization_id=oracle.organization_id
                            WHERE execution.action_execution_id=$1
                              AND execution.prepared_action_id=$2
                              AND execution.state='outcome_unknown'
                              AND oracle.verdict='inconclusive'
                       )"#,
                )
                .bind(action.action_execution_id)
                .bind(action.prepared_action_id)
                .fetch_one(&*pool)
                .await?;
                if semantic_landing_complete {
                    close_campaign_from_current_oracles(
                        &pool,
                        &repository,
                        operation_id,
                        &campaign,
                    )
                    .await?;
                    view.terminal_count = view.terminal_count.saturating_add(1);
                } else {
                    view.blocked_count = view.blocked_count.saturating_add(1);
                }
            }
            Some("compile_rejected")
            | Some("denied")
            | Some("expired")
            | Some("superseded")
            | Some("manually_blocked") => {
                view.blocked_count = view.blocked_count.saturating_add(1);
            }
            Some(_) => {
                view.blocked_count = view.blocked_count.saturating_add(1);
            }
            None => anyhow::bail!("Campaign scheduler failed to materialize a Prepared Action"),
        }
    }
    view.pending_prepared_action_ids.sort_unstable();
    view.pending_prepared_action_ids.dedup();
    if view.terminal_count == view.campaign_count
        && view.pending_authorization_count == 0
        && view.authorized_count == 0
        && view.started_count == 0
        && view.awaiting_oracle_count == 0
        && view.blocked_count == 0
    {
        let (revision_count, adjudicated_revision_count) =
            adjudicate_all_operation_revisions(&pool, &repository, operation_id).await?;
        view.revision_count = revision_count;
        view.adjudicated_revision_count = adjudicated_revision_count;
        let (wave_count, fixed_point_wave_count) =
            finalize_all_operation_waves(&pool, operation_id).await?;
        view.wave_count = wave_count;
        view.fixed_point_wave_count = fixed_point_wave_count;
    } else {
        view.revision_count = u32::try_from(
            sqlx::query_scalar::<_, i64>(
                r#"SELECT COUNT(DISTINCT generation_member.revision_id)
                     FROM verification_wave_coverage_denominators wave
                     JOIN hypothesis_generation_seals generation_seal
                       ON generation_seal.seal_id=wave.generation_seal_id
                     JOIN hypothesis_generation_members generation_member
                       ON generation_member.generation_id=generation_seal.generation_id
                    WHERE wave.operation_id=$1"#,
            )
            .bind(operation_id)
            .fetch_one(&*pool)
            .await?,
        )?;
        view.adjudicated_revision_count = u32::try_from(
            sqlx::query_scalar::<_, i64>(
                r#"SELECT COUNT(DISTINCT adjudication.hypothesis_revision_id)
                     FROM hypothesis_revision_adjudications adjudication
                     JOIN hypothesis_generation_members generation_member
                       ON generation_member.revision_id=adjudication.hypothesis_revision_id
                      AND generation_member.operation_id=adjudication.operation_id
                     JOIN hypothesis_generation_seals generation_seal
                       ON generation_seal.generation_id=generation_member.generation_id
                     JOIN verification_wave_coverage_denominators wave
                       ON wave.generation_seal_id=generation_seal.seal_id
                      AND wave.operation_id=adjudication.operation_id
                    WHERE adjudication.operation_id=$1"#,
            )
            .bind(operation_id)
            .fetch_one(&*pool)
            .await?,
        )?;
        view.wave_count = u32::try_from(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM verification_wave_coverage_denominators WHERE operation_id=$1 AND sealed_at IS NOT NULL",
            )
            .bind(operation_id)
            .fetch_one(&*pool)
            .await?,
        )?;
        view.fixed_point_wave_count = u32::try_from(
            sqlx::query_scalar::<_, i64>(
                r#"SELECT COUNT(*)
                     FROM hypothesis_fixed_point_receipts fixed
                     JOIN hypothesis_consolidation_receipts consolidation
                       ON consolidation.consolidation_receipt_id=fixed.consolidation_receipt_id
                     JOIN hypothesis_consolidation_batches batch
                       ON batch.consolidation_batch_id=consolidation.consolidation_batch_id
                    WHERE batch.operation_id=$1"#,
            )
            .bind(operation_id)
            .fetch_one(&*pool)
            .await?,
        )?;
    }
    Ok(view)
}

#[cfg(test)]
mod compile_residual_tests {
    use super::{is_exact_terminal_compile_residual, SealedCompileResidualDisposition};

    #[test]
    fn only_a_complete_sealed_compile_residual_set_is_terminal() {
        let exact = SealedCompileResidualDisposition {
            apply_count: 4,
            residual_count: 4,
            valid_residual_count: 4,
            prepared_action_count: 0,
            planned_obligation_count: 4,
        };
        assert!(is_exact_terminal_compile_residual(&exact));

        for drifted in [
            SealedCompileResidualDisposition {
                valid_residual_count: 3,
                ..exact
            },
            SealedCompileResidualDisposition {
                prepared_action_count: 1,
                residual_count: 3,
                ..exact
            },
            SealedCompileResidualDisposition {
                planned_obligation_count: 5,
                ..exact
            },
            SealedCompileResidualDisposition {
                apply_count: 0,
                residual_count: 0,
                valid_residual_count: 0,
                planned_obligation_count: 0,
                ..exact
            },
        ] {
            assert!(!is_exact_terminal_compile_residual(&drifted));
        }
    }

    #[test]
    fn prepared_action_lookup_avoids_the_postgres_authorization_keyword() {
        let source = include_str!("verification_campaign_scheduler.rs");
        assert!(source.contains(") latest_authorization ON TRUE"));
        assert!(source.contains("latest_authorization.authorization_receipt_id"));
        let reserved_alias = [") author", "ization ON TRUE"].concat();
        assert!(!source.contains(&reserved_alias));
    }
}
