//! Explicit local-operator authority packet for an exact Investigation resume.
//!
//! This surface is intentionally unavailable to fresh runs, model tools and
//! `--auto-approve`. It requires the complete persisted resume identity and
//! reuses the canonical safety-hold and Prepared Action repositories so every
//! transition remains CAS-bound and append-only. The same contract applies to
//! a retained ephemeral database and the local application database.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

const PACKET_SCHEMA: &str = "stage_run_campaign_authority.v1";
const MAX_PACKET_BYTES: u64 = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CampaignAuthorityPacket {
    schema: String,
    operation_id: Uuid,
    campaign_dispatch_hold_release: Option<CampaignDispatchHoldRelease>,
    #[serde(default)]
    prepared_action_decisions: Vec<PreparedActionDecision>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CampaignDispatchHoldRelease {
    expected_generation: i64,
    expected_row_version: i64,
    reason_code: String,
    evidence_manifest_hash: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DecisionKind {
    Approve,
    Deny,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedActionDecision {
    campaign_id: Uuid,
    prepared_action_id: Uuid,
    decision: DecisionKind,
    private_manifest_hash: String,
    display_projection_hash: String,
    renderer_version: String,
    expected_row_version: i64,
    stable_request_id: Uuid,
    requested_expiry: Option<DateTime<Utc>>,
}

fn read_packet(path: &Path) -> Result<CampaignAuthorityPacket> {
    anyhow::ensure!(
        path.is_absolute(),
        "--stage-run-campaign-authority must be an absolute path"
    );
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect Campaign authority packet {}", path.display()))?;
    anyhow::ensure!(
        metadata.file_type().is_file() && !metadata.file_type().is_symlink(),
        "Campaign authority packet must be a regular non-symlink file"
    );
    anyhow::ensure!(
        metadata.len() <= MAX_PACKET_BYTES,
        "Campaign authority packet exceeds {MAX_PACKET_BYTES} bytes"
    );
    let bytes = std::fs::read(path)
        .with_context(|| format!("read Campaign authority packet {}", path.display()))?;
    let packet: CampaignAuthorityPacket =
        serde_json::from_slice(&bytes).context("parse Campaign authority packet")?;
    anyhow::ensure!(
        packet.schema == PACKET_SCHEMA,
        "unsupported Campaign authority packet schema"
    );
    anyhow::ensure!(
        packet.campaign_dispatch_hold_release.is_some()
            || !packet.prepared_action_decisions.is_empty(),
        "Campaign authority packet contains no transition"
    );
    Ok(packet)
}

pub(super) async fn apply_exact_resume_campaign_authority(
    pool: &PgPool,
    packet_path: &Path,
    expected_operation_id: Uuid,
) -> Result<()> {
    let packet = read_packet(packet_path)?;
    anyhow::ensure!(
        packet.operation_id == expected_operation_id,
        "Campaign authority packet operation does not match the exact resume"
    );
    let operation = golish_db::repo::operation_state::get(pool, expected_operation_id)
        .await?
        .context("Campaign authority operation is missing")?;
    let project_scope_id = operation
        .project_scope_id
        .context("Campaign authority operation has no active project scope")?;

    let mut hold_receipt_id = None;
    if let Some(release) = packet.campaign_dispatch_hold_release {
        anyhow::ensure!(
            release.expected_generation >= 0 && release.expected_row_version >= 0,
            "Campaign hold release carries a negative CAS value"
        );
        let principal = golish_db::repo::operator_principals::current_local(pool).await?;
        let mut tx = pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
            .execute(&mut *tx)
            .await?;
        let receipt = golish_db::repo::operation_rollout_safety_hold::set_operation_safety_hold(
            &mut tx,
            golish_db::repo::operation_rollout_safety_hold::SetOperationSafetyHold {
                scope: golish_db::repo::operation_rollout_safety_hold::OperationSafetyHoldScope::CampaignDispatch,
                next_held: false,
                expected_generation: release.expected_generation,
                expected_row_version: release.expected_row_version,
                reason_code: release.reason_code,
                evidence_manifest_hash: release.evidence_manifest_hash,
                principal_id: principal.id,
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.code()))?;
        hold_receipt_id = Some(receipt.event_id);
        tx.commit().await?;
    }

    let mut decision_receipt_ids = Vec::new();
    for decision in packet.prepared_action_decisions {
        anyhow::ensure!(
            decision.expected_row_version >= 0,
            "Prepared Action decision carries a negative row version"
        );
        let rows = golish_db::repo::verification_prepared_actions::list_pending_prepared_actions(
            pool,
            &golish_db::repo::verification_prepared_actions::ListPendingPreparedActions {
                operation_id: expected_operation_id,
                project_scope_id,
                campaign_id: Some(decision.campaign_id),
            },
        )
        .await?;
        let row = rows
            .into_iter()
            .find(|row| row.prepared_action_id == decision.prepared_action_id)
            .context("Prepared Action is absent from the exact Campaign")?;
        anyhow::ensure!(
            row.operation_id == expected_operation_id
                && row.campaign_id == decision.campaign_id
                && row.project_scope_id == project_scope_id
                && matches!(row.risk_tier.as_str(), "T2" | "T3")
                && row.renderer_version == decision.renderer_version
                && row.display_projection_hash == decision.display_projection_hash
                && row.private_manifest_hash == decision.private_manifest_hash
                && (row.row_version == decision.expected_row_version
                    || (matches!(row.state.as_str(), "authorized" | "denied")
                        && row.row_version.checked_sub(1) == Some(decision.expected_row_version))),
            "Prepared Action decision authority is stale"
        );
        let (decision_name, reason_code, expires_at) = match decision.decision {
            DecisionKind::Approve => {
                let expiry = row.authorization_expires_at.unwrap_or_else(|| {
                    decision
                        .requested_expiry
                        .map(|requested| requested.min(row.review_expires_at))
                        .unwrap_or(row.review_expires_at)
                });
                anyhow::ensure!(
                    expiry > Utc::now(),
                    "Prepared Action review packet has expired"
                );
                (
                    "authorized",
                    "operator_authorized_exact_action",
                    Some(expiry),
                )
            }
            DecisionKind::Deny => {
                anyhow::ensure!(
                    decision.requested_expiry.is_none(),
                    "Prepared Action deny decision cannot carry an expiry"
                );
                ("denied", "operator_denied_exact_action", None)
            }
        };
        let campaign_dispatch_generation =
            match row.authorization_campaign_dispatch_generation {
                Some(generation) => generation,
                None => golish_db::repo::verification_prepared_actions::current_campaign_dispatch_generation(pool)
                    .await?,
            };
        let receipt =
            golish_db::repo::verification_prepared_actions::decide_prepared_action_authorization(
                pool,
                &golish_db::repo::verification_prepared_actions::DecidePreparedActionAuthorization {
                    stable_request_id: decision.stable_request_id,
                    prepared_action_id: decision.prepared_action_id,
                    campaign_id: decision.campaign_id,
                    operation_id: expected_operation_id,
                    project_scope_id,
                    organization_id: row.organization_id,
                    decision: decision_name.to_owned(),
                    decision_reason_code: reason_code.to_owned(),
                    expected_action_row_version: decision.expected_row_version,
                    campaign_dispatch_generation,
                    renderer_version: decision.renderer_version,
                    reviewed_action_hash: decision.display_projection_hash.clone(),
                    expected_display_projection_hash: decision.display_projection_hash,
                    expected_private_manifest_hash: decision.private_manifest_hash,
                    operator_channel: "local_cli".to_owned(),
                    expires_at,
                },
            )
            .await?;
        decision_receipt_ids.push(receipt.authorization_receipt_id);
    }

    eprintln!(
        "[stage-run-resume] applied explicit Campaign authority packet: {}",
        serde_json::json!({
            "schema": PACKET_SCHEMA,
            "operationId": expected_operation_id,
            "campaignDispatchHoldEventId": hold_receipt_id,
            "preparedActionAuthorizationReceiptIds": decision_receipt_ids,
        })
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn campaign_authority_packet_is_exact_and_contains_no_raw_action_body() {
        let packet: CampaignAuthorityPacket = serde_json::from_value(serde_json::json!({
            "schema": PACKET_SCHEMA,
            "operationId": Uuid::from_u128(1),
            "campaignDispatchHoldRelease": {
                "expectedGeneration": 0,
                "expectedRowVersion": 0,
                "reasonCode": "controlled_fixture_acceptance",
                "evidenceManifestHash": format!("sha256:{}", "a".repeat(64)),
            },
            "preparedActionDecisions": [{
                "campaignId": Uuid::from_u128(2),
                "preparedActionId": Uuid::from_u128(3),
                "decision": "approve",
                "privateManifestHash": format!("sha256:{}", "b".repeat(64)),
                "displayProjectionHash": format!("sha256:{}", "c".repeat(64)),
                "rendererVersion": "verification-action-renderer.v1",
                "expectedRowVersion": 0,
                "stableRequestId": Uuid::from_u128(4),
                "requestedExpiry": null,
            }],
        }))
        .expect("parse exact authority packet");
        assert_eq!(packet.operation_id, Uuid::from_u128(1));
        assert_eq!(packet.prepared_action_decisions.len(), 1);

        let rejected = serde_json::from_value::<CampaignAuthorityPacket>(serde_json::json!({
            "schema": PACKET_SCHEMA,
            "operationId": Uuid::from_u128(1),
            "campaignDispatchHoldRelease": null,
            "preparedActionDecisions": [{
                "campaignId": Uuid::from_u128(2),
                "preparedActionId": Uuid::from_u128(3),
                "decision": "approve",
                "privateManifestHash": format!("sha256:{}", "b".repeat(64)),
                "displayProjectionHash": format!("sha256:{}", "c".repeat(64)),
                "rendererVersion": "verification-action-renderer.v1",
                "expectedRowVersion": 0,
                "stableRequestId": Uuid::from_u128(4),
                "requestedExpiry": null,
                "canonicalRequest": {"method": "GET"},
            }],
        }));
        assert!(
            rejected.is_err(),
            "raw action material must be rejected by deny_unknown_fields"
        );
    }
}
