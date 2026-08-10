//! IntelGoalV1 semantic collection landing.
//!
//! Provider output first becomes an immutable Observation. Only a candidate
//! with host-owned owned attribution and a fresh low-impact reachability
//! receipt may be promoted into the formal Target catalog.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::organizations::OrganizationCandidate;
use golish_pentest_domain::models::{AssetIntelPivot, AssetIntelPivotKind};

use super::agent_intel::PassiveIntelSummary;

#[derive(Debug, Clone, sqlx::FromRow)]
struct GoalLandingContext {
    team_plan_id: Uuid,
    goal_epoch_id: Uuid,
    goal_epoch: i64,
    controller_worker_run_id: Uuid,
    controller_message_chain_id: Uuid,
    producer_session_id: Uuid,
    canonical_legal_name: String,
    scope_policy: Value,
    identity_payload: Value,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExistingGoalObservation {
    id: Uuid,
    promotion_target_id: Option<Uuid>,
    asset_kind: String,
    canonical_value: String,
    provider_id: String,
    attribution_disposition: String,
    reachability_state: String,
    evidence_id: i64,
    reachability_evidence_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct ReachabilityReceipt {
    state: &'static str,
    method: &'static str,
    checked_at: DateTime<Utc>,
    valid_until: Option<DateTime<Utc>>,
    detail: Value,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SemanticPivotLandingContext<'a> {
    pub kind: &'a str,
    pub value: &'a str,
    pub authorization_basis: &'a str,
    pub has_scope_authority: bool,
    pub candidate_only: bool,
}

pub(crate) async fn land_semantic_goal_observations(
    pool: &PgPool,
    summary: &PassiveIntelSummary,
    pivot: SemanticPivotLandingContext<'_>,
    workspace: &Path,
) -> anyhow::Result<Value> {
    let pivot_kind = pivot.kind;
    let pivot_value = pivot.value;
    let pivot_authorization_basis = pivot.authorization_basis;
    let pivot_has_scope_authority = pivot.has_scope_authority;
    let pivot_candidate_only = pivot.candidate_only;
    let tool = golish_core::current_agent_tool_context()
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_TRUSTED_TOOL_CONTEXT_MISSING"))?;
    anyhow::ensure!(
        tool.tool_name == "recon_search_intel",
        "TARGET_INTEL_TOOL_CONTEXT_MISMATCH"
    );
    let operation_id = tool
        .operation_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_OPERATION_CONTEXT_MISSING"))?;
    let organization_id = tool
        .organization_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_ORGANIZATION_CONTEXT_MISSING"))?;
    let worker = tool
        .worker_lease
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_WORKER_FENCE_MISSING"))?;
    let tool_call_id = tool
        .tool_call_record_id
        .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_TOOL_CALL_ID_MISSING"))?;
    let context = sqlx::query_as::<_, GoalLandingContext>(
        r#"SELECT epoch.team_plan_id,
                  epoch.id AS goal_epoch_id,epoch.epoch AS goal_epoch,
                  epoch.controller_worker_run_id,epoch.controller_message_chain_id,
                  producer.session_id AS producer_session_id,
                  identity.canonical_legal_name,identity.scope_policy,identity.identity_payload
             FROM target_intel_goal_epochs epoch
             JOIN target_intel_goal_company_identity_bindings binding
               ON binding.operation_id=epoch.operation_id
              AND binding.organization_id=epoch.organization_id
             JOIN scoping_company_identity_receipts identity
              ON identity.id=binding.company_identity_receipt_id
              AND identity.operation_id=binding.operation_id
              AND identity.organization_id=binding.organization_id
            JOIN tool_calls producer
              ON producer.id=$4
             AND producer.operation_id=epoch.operation_id
             AND producer.organization_id=epoch.organization_id
             AND producer.stage_execution_id=epoch.stage_execution_id
             AND producer.stage_run_unit_id=epoch.stage_run_unit_id
             AND producer.worker_run_id=epoch.controller_worker_run_id
             AND producer.name='recon_search_intel'
             AND producer.status='running'
            WHERE epoch.operation_id=$1 AND epoch.organization_id=$2
              AND epoch.stage_run_unit_id=$3 AND epoch.status='open'
            ORDER BY epoch.epoch DESC LIMIT 1"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(worker.stage_run_unit_id)
    .bind(tool_call_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("TARGET_INTEL_OPEN_GOAL_EPOCH_MISSING"))?;
    // Evidence inherits the awaited producer row's durable session authority.
    // The ambient UI/chat session can be an opaque `stage-run-*` key and is not
    // the database identity used by `tool_calls`; deriving a UUID from it would
    // make an otherwise exact receipt visible to review but uncloseable by the
    // finalizer's producer/evidence join.
    let artifact_session_id = context.producer_session_id;
    let session_id = artifact_session_id.to_string();
    let trusted_roots = trusted_roots(&context.scope_policy, &context.identity_payload);
    let mut observation_refs = Vec::new();
    let mut promoted_target_refs = Vec::new();
    let mut ambiguous_refs = Vec::new();
    let mut unreachable_refs = Vec::new();
    // Every completed semantic query needs its own citable receipt, including
    // a genuinely checked-empty provider pass. Observation evidence alone is
    // insufficient because an empty run has no Observation to attach it to;
    // without this query receipt, a bounded worker can only either fabricate
    // an inherited id or return an unaudited `checked_empty` disposition.
    let query_subject = format!("{pivot_kind}:sha256:{}", sha256_hex(pivot_value.as_bytes()));
    let query_outcome = if summary.current_run_candidates.targets.is_empty()
        && summary.current_run_profile_fields.is_empty()
    {
        "checked_empty"
    } else {
        "observed"
    };
    let query_evidence = golish_db::repo::audit::log_evidence(
        pool,
        "target_intel_semantic_query",
        "target_intel",
        "intel.semantic_query_receipt.v1",
        workspace.to_str(),
        "harness",
        None,
        Some(&session_id),
        Some("recon_search_intel"),
        &json!({
            "kind": "target_intel.semantic_query",
            "operation_id": operation_id,
            "organization_id": organization_id,
            "goal_epoch_id": context.goal_epoch_id,
            "producer_worker_run_id": worker.worker_run_id,
            "producer_tool_call_id": tool_call_id,
            "provider_run_id": summary.run_id,
            "pivot_kind": pivot_kind,
            "pivot_value_sha256": sha256_hex(pivot_value.as_bytes()),
            "pivot_authorization_basis": pivot_authorization_basis,
            "pivot_has_scope_authority": pivot_has_scope_authority,
            "pivot_candidate_only": pivot_candidate_only,
            "result_status": summary.status,
            "provider_status": summary.provider_status,
            "technique_status": summary.technique_status,
            "counts": {
                "candidate_targets": summary.current_run_candidates.targets.len(),
                "profile_fields": summary.current_run_profile_fields.len(),
            },
        }),
        Some(operation_id),
        None,
        Some(&query_subject),
        Some(query_outcome),
    )
    .await?;
    let mut evidence_ids = vec![query_evidence.id];
    let mut semantic_observations = Vec::new();
    let mut discovered_pivots = Vec::new();

    for field in &summary.current_run_profile_fields {
        let Some(pivot) = profile_field_pivot(field) else {
            continue;
        };
        let stable_observation_key = format!(
            "profile-observation:v1:{}:{}:{}",
            field.provider_id,
            pivot_kind,
            sha256_hex(
                format!(
                    "{}\u{1f}{}\u{1f}{}",
                    field.target_field,
                    pivot.kind.as_str(),
                    pivot.value
                )
                .as_bytes()
            )
        );
        if let Some(existing) = sqlx::query_as::<_, ExistingGoalObservation>(
            r#"SELECT id,promotion_target_id,asset_kind,canonical_value,provider_id,
                      attribution_disposition,reachability_state,evidence_id,
                      reachability_evidence_id
                 FROM target_intel_asset_observations
                WHERE operation_id=$1 AND organization_id=$2 AND stable_observation_key=$3"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(&stable_observation_key)
        .fetch_optional(pool)
        .await?
        {
            observation_refs.push(existing.id.to_string());
            evidence_ids.push(existing.evidence_id);
            discovered_pivots.push(pivot.clone());
            semantic_observations.push(json!({
                "observation_id": existing.id,
                "asset_kind": existing.asset_kind,
                "canonical_value": existing.canonical_value,
                "provider_id": existing.provider_id,
                "attribution": existing.attribution_disposition,
                "reachability": existing.reachability_state,
                "promotion_target_id": existing.promotion_target_id,
                "evidence_ids": [existing.evidence_id],
                "discovered_pivot": pivot,
                "replayed": true,
            }));
            continue;
        }
        let provider_fields = json!({
            field.provider_id.clone(): {
                field.target_field.clone(): field.value.clone(),
            }
        });
        let artifact_payload = json!({
            "provider": field.provider_id,
            "pivot": {
                "kind": pivot_kind,
                "value_sha256": sha256_hex(pivot_value.as_bytes()),
                "authorization_basis": pivot_authorization_basis,
                "has_scope_authority": pivot_has_scope_authority,
                "candidate_only": pivot_candidate_only,
            },
            "profile_field": {
                "target_kind": format!("{:?}", field.target_kind).to_ascii_lowercase(),
                "target_field": field.target_field,
                "value": field.value,
            }
        });
        let artifact_sha256 = sha256_hex(&serde_json::to_vec(&artifact_payload)?);
        let artifact_ref = format!("intel-artifact:sha256:{artifact_sha256}");
        golish_db::repo::target_intel_semantic_artifacts::put_redacted(
            pool,
            &golish_db::repo::target_intel_semantic_artifacts::TargetIntelSemanticArtifactRow {
                artifact_ref: artifact_ref.clone(),
                operation_id,
                organization_id,
                session_id: artifact_session_id,
                artifact_sha256: artifact_sha256.clone(),
                redacted_payload: redact_provider_payload(&artifact_payload),
            },
        )
        .await?;
        let evidence = golish_db::repo::audit::log_evidence(
            pool,
            "target_intel_observation",
            "target_intel",
            "intel.semantic_profile_observation.v1",
            workspace.to_str(),
            "harness",
            None,
            Some(&session_id),
            Some("recon_search_intel"),
            &json!({
                "kind": "target_intel.semantic_profile_pivot",
                "organization_id": organization_id,
                "pivot_kind": pivot.kind.as_str(),
                "pivot_value_sha256": sha256_hex(pivot.value.as_bytes()),
                "raw_output": {"artifact_ref": artifact_ref, "artifact_sha256": artifact_sha256},
            }),
            Some(operation_id),
            None,
            Some(&pivot.value),
            Some("observed"),
        )
        .await?;
        evidence_ids.push(evidence.id);
        let stable_query_key = format!(
            "semantic-profile:v1:{}:{}:{}",
            pivot_kind,
            field.provider_id,
            sha256_hex(pivot.value.as_bytes())
        );
        let receipt = golish_db::repo::audit::log_operation(
            pool,
            "target_intel_semantic_receipt",
            "target_intel",
            "intel.semantic_pivot_receipt.v1",
            workspace.to_str(),
            "target_intel_goal",
            None,
            Some(&session_id),
            Some("recon_search_intel"),
            "succeeded",
            &json!({
                "operation_id": operation_id,
                "organization_id": organization_id,
                "stable_query_key": stable_query_key,
                "provider_id": field.provider_id,
                "query_type": pivot_kind,
                "adapter_version": "semantic_profile_landing.v1",
                "pivot_authorization_basis": pivot_authorization_basis,
                "pivot_candidate_only": pivot_candidate_only,
                "artifact_ref": artifact_ref,
                "artifact_sha256": artifact_sha256,
                "evidence_ref": format!("audit:{}", evidence.id),
                "unauthorized_promotion_refs": [],
            }),
        )
        .await?;
        let observation_id = Uuid::new_v5(
            &operation_id,
            format!("{organization_id}:{stable_observation_key}").as_bytes(),
        );
        let canonical_identity = json!({"kind": pivot.kind.as_str(), "value": pivot.value});
        let row =
            golish_db::repo::target_intel_asset_observations::TargetIntelAssetObservationRow {
                id: observation_id,
                stable_observation_key,
                operation_id,
                organization_id,
                team_plan_id: context.team_plan_id,
                goal_epoch_id: context.goal_epoch_id,
                goal_epoch: context.goal_epoch,
                producer_worker_run_id: worker.worker_run_id,
                producer_tool_call_id: Some(tool_call_id),
                semantic_receipt_audit_id: Some(receipt.id),
                evidence_id: evidence.id,
                artifact_ref,
                artifact_sha256,
                provider_id: field.provider_id.clone(),
                provider_query_type: pivot_kind.to_string(),
                adapter_version: "semantic_profile_landing.v1".to_string(),
                stable_query_key,
                provider_record_ordinal: 0,
                provider_fetched_at: Utc::now(),
                asset_kind: pivot.kind.as_str().to_string(),
                canonical_value: pivot.value.clone(),
                canonical_identity: canonical_identity.clone(),
                canonical_identity_sha256: prefixed_sha256(&canonical_identity),
                typed_core: json!({"target_field": field.target_field, "value": field.value}),
                provider_fields,
                provider_metadata: json!({
                    "run_id": summary.run_id,
                    "source": "normalized_profile_field",
                    "pivot_authorization_basis": pivot_authorization_basis,
                    "pivot_candidate_only": pivot_candidate_only,
                }),
                observation_sha256: prefixed_sha256(&json!({
                    "provider": field.provider_id,
                    "identity": canonical_identity,
                    "evidence_id": evidence.id,
                })),
                attribution_disposition: "unassessed".to_string(),
                attribution_method: None,
                attribution_basis: None,
                attribution_decided_at: None,
                reachability_state: "unverified".to_string(),
                reachability_method: None,
                reachability_tool_call_id: None,
                reachability_evidence_id: None,
                reachability_checked_at: None,
                reachability_valid_until: None,
                promotion_target_id: None,
                promoted_at: None,
                row_version: 0,
                observed_at: Utc::now(),
            };
        golish_db::repo::target_intel_asset_observations::insert(pool, &row).await?;
        golish_db::repo::target_intel_asset_observations::record_attribution(
            pool,
            &golish_db::repo::target_intel_asset_observations::RecordAttribution {
                observation_id,
                expected_row_version: 0,
                disposition: "ambiguous".to_string(),
                method: "semantic_profile_pivot_requires_corroboration_v1".to_string(),
                basis: json!({"reason": "non-address provider fact is a search pivot, not scope authority"}),
                evidence_refs: json!([format!("audit:{}", evidence.id)]),
            },
        )
        .await?;
        observation_refs.push(observation_id.to_string());
        ambiguous_refs.push(observation_id.to_string());
        discovered_pivots.push(pivot.clone());
        semantic_observations.push(json!({
            "observation_id": observation_id,
            "asset_kind": pivot.kind.as_str(),
            "canonical_value": pivot.value,
            "provider_id": field.provider_id,
            "attribution": "ambiguous",
            "reachability": "unverified",
            "promotion_target_id": Value::Null,
            "evidence_ids": [evidence.id],
            "discovered_pivot": pivot,
            "replayed": false,
        }));
    }

    for (ordinal, candidate) in summary.current_run_candidates.targets.iter().enumerate() {
        let canonical_value = canonical_asset_value(&candidate.value);
        if canonical_value.is_empty() {
            continue;
        }
        let asset_kind = asset_kind(&canonical_value);
        let canonical_identity = json!({"kind": asset_kind, "value": canonical_value});
        let canonical_identity_sha256 = prefixed_sha256(&canonical_identity);
        let stable_observation_key = format!(
            "observation:v1:{}:{}:{}",
            candidate.source,
            pivot_kind,
            sha256_hex(format!("{pivot_value}\u{1f}{canonical_value}").as_bytes())
        );
        if let Some(existing) = sqlx::query_as::<_, ExistingGoalObservation>(
            r#"SELECT id,promotion_target_id,asset_kind,canonical_value,provider_id,
                      attribution_disposition,reachability_state,evidence_id,
                      reachability_evidence_id
                 FROM target_intel_asset_observations
                WHERE operation_id=$1 AND organization_id=$2 AND stable_observation_key=$3"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(&stable_observation_key)
        .fetch_optional(pool)
        .await?
        {
            observation_refs.push(existing.id.to_string());
            evidence_ids.push(existing.evidence_id);
            if let Some(evidence_id) = existing.reachability_evidence_id {
                evidence_ids.push(evidence_id);
            }
            if let Some(target_id) = existing.promotion_target_id {
                promoted_target_refs.push(target_id.to_string());
            }
            let existing_evidence_ids = [
                Some(existing.evidence_id),
                existing.reachability_evidence_id,
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            semantic_observations.push(json!({
                "observation_id": existing.id,
                "asset_kind": existing.asset_kind,
                "canonical_value": existing.canonical_value,
                "provider_id": existing.provider_id,
                "attribution": existing.attribution_disposition,
                "reachability": existing.reachability_state,
                "promotion_target_id": existing.promotion_target_id,
                "evidence_ids": existing_evidence_ids,
                "replayed": true,
            }));
            continue;
        }

        let redacted_provider_fields = redact_provider_payload(&candidate.evidence);
        let provider_fields = Value::Object(Map::from_iter([(
            candidate.source.clone(),
            redacted_provider_fields.clone(),
        )]));
        let artifact_payload = json!({
            "provider": candidate.source,
            "pivot": {
                "kind": pivot_kind,
                "value_sha256": sha256_hex(pivot_value.as_bytes()),
                "authorization_basis": pivot_authorization_basis,
                "has_scope_authority": pivot_has_scope_authority,
            },
            "candidate": {
                "label": candidate.label,
                "value": canonical_value,
                "confidence": candidate.confidence,
                "provider_fields": redacted_provider_fields,
            }
        });
        let artifact_sha256 = sha256_hex(&serde_json::to_vec(&artifact_payload)?);
        let artifact_ref = format!("intel-artifact:sha256:{artifact_sha256}");
        golish_db::repo::target_intel_semantic_artifacts::put_redacted(
            pool,
            &golish_db::repo::target_intel_semantic_artifacts::TargetIntelSemanticArtifactRow {
                artifact_ref: artifact_ref.clone(),
                operation_id,
                organization_id,
                session_id: artifact_session_id,
                artifact_sha256: artifact_sha256.clone(),
                redacted_payload: artifact_payload,
            },
        )
        .await?;
        let evidence_detail = json!({
            "kind": "target_intel.semantic_pivot",
            "organization_id": organization_id,
            "pivot_kind": pivot_kind,
            "pivot_value_sha256": sha256_hex(pivot_value.as_bytes()),
            "raw_output": {"artifact_ref": artifact_ref, "artifact_sha256": artifact_sha256},
        });
        let evidence = golish_db::repo::audit::log_evidence(
            pool,
            "target_intel_observation",
            "target_intel",
            "intel.semantic_observation.v1",
            workspace.to_str(),
            "harness",
            None,
            Some(&session_id),
            Some("recon_search_intel"),
            &evidence_detail,
            Some(operation_id),
            None,
            Some(&canonical_value),
            Some("observed"),
        )
        .await?;
        evidence_ids.push(evidence.id);
        let stable_query_key = format!(
            "semantic:v1:{}:{}:{}:{}",
            pivot_kind,
            sha256_hex(pivot_value.as_bytes()),
            candidate.source,
            ordinal
        );
        let receipt_detail = json!({
            "operation_id": operation_id,
            "organization_id": organization_id,
            "stable_query_key": stable_query_key,
            "provider_id": candidate.source,
            "query_type": pivot_kind,
            "adapter_version": "semantic_goal_landing.v1",
            "pivot_authorization_basis": pivot_authorization_basis,
            "pivot_candidate_only": pivot_candidate_only,
            "artifact_ref": artifact_ref,
            "artifact_sha256": artifact_sha256,
            "evidence_ref": format!("audit:{}", evidence.id),
            "unauthorized_promotion_refs": [],
        });
        let receipt = golish_db::repo::audit::log_operation(
            pool,
            "target_intel_semantic_receipt",
            "target_intel",
            "intel.semantic_pivot_receipt.v1",
            workspace.to_str(),
            "target_intel_goal",
            None,
            Some(&session_id),
            Some("recon_search_intel"),
            "succeeded",
            &receipt_detail,
        )
        .await?;
        let observed_at =
            DateTime::from_timestamp_millis(candidate.created_at as i64).unwrap_or_else(Utc::now);
        let observation_id = Uuid::new_v5(
            &operation_id,
            format!("{organization_id}:{stable_observation_key}").as_bytes(),
        );
        let row =
            golish_db::repo::target_intel_asset_observations::TargetIntelAssetObservationRow {
                id: observation_id,
                stable_observation_key,
                operation_id,
                organization_id,
                team_plan_id: context.team_plan_id,
                goal_epoch_id: context.goal_epoch_id,
                goal_epoch: context.goal_epoch,
                producer_worker_run_id: worker.worker_run_id,
                producer_tool_call_id: Some(tool_call_id),
                semantic_receipt_audit_id: Some(receipt.id),
                evidence_id: evidence.id,
                artifact_ref,
                artifact_sha256,
                provider_id: candidate.source.clone(),
                provider_query_type: pivot_kind.to_string(),
                adapter_version: "semantic_goal_landing.v1".to_string(),
                stable_query_key,
                provider_record_ordinal: i32::try_from(ordinal).unwrap_or(i32::MAX),
                provider_fetched_at: observed_at,
                asset_kind: asset_kind.to_string(),
                canonical_value: canonical_value.clone(),
                canonical_identity,
                canonical_identity_sha256: canonical_identity_sha256.clone(),
                typed_core: json!({
                    "label": candidate.label,
                    "confidence": candidate.confidence,
                    "host": canonical_value,
                }),
                provider_fields,
                provider_metadata: json!({
                    "candidate_id": candidate.id,
                    "run_id": summary.run_id,
                    "pivot_authorization_basis": pivot_authorization_basis,
                    "pivot_candidate_only": pivot_candidate_only,
                }),
                observation_sha256: prefixed_sha256(&json!({
                    "provider": candidate.source,
                    "identity": canonical_identity_sha256,
                    "evidence_id": evidence.id,
                })),
                attribution_disposition: "unassessed".to_string(),
                attribution_method: None,
                attribution_basis: None,
                attribution_decided_at: None,
                reachability_state: "unverified".to_string(),
                reachability_method: None,
                reachability_tool_call_id: None,
                reachability_evidence_id: None,
                reachability_checked_at: None,
                reachability_valid_until: None,
                promotion_target_id: None,
                promoted_at: None,
                row_version: 0,
                observed_at,
            };
        golish_db::repo::target_intel_asset_observations::insert(pool, &row).await?;
        observation_refs.push(observation_id.to_string());

        let (disposition, method, basis) = candidate_attribution(
            candidate,
            &canonical_value,
            &context.canonical_legal_name,
            &trusted_roots,
            pivot_authorization_basis,
            pivot_candidate_only,
        );
        golish_db::repo::target_intel_asset_observations::record_attribution(
            pool,
            &golish_db::repo::target_intel_asset_observations::RecordAttribution {
                observation_id,
                expected_row_version: 0,
                disposition: disposition.to_string(),
                method: method.to_string(),
                basis,
                evidence_refs: json!([format!("audit:{}", evidence.id)]),
            },
        )
        .await?;
        if disposition != "owned" {
            ambiguous_refs.push(observation_id.to_string());
            semantic_observations.push(json!({
                "observation_id": observation_id,
                "asset_kind": asset_kind,
                "canonical_value": canonical_value,
                "provider_id": candidate.source,
                "attribution": disposition,
                "reachability": "unverified",
                "promotion_target_id": Value::Null,
                "evidence_ids": [evidence.id],
                "replayed": false,
            }));
            continue;
        }

        let reachability = probe_reachability(candidate, &canonical_value).await;
        let reachability_evidence = golish_db::repo::audit::log_evidence(
            pool,
            "target_intel_reachability",
            "target_intel",
            "intel.reachability_receipt.v1",
            workspace.to_str(),
            "harness",
            None,
            Some(&session_id),
            Some("recon_search_intel"),
            &json!({
                "kind": "target_intel.reachability",
                "organization_id": organization_id,
                "observation_id": observation_id,
                "state": reachability.state,
                "method": reachability.method,
                "detail": reachability.detail,
            }),
            Some(operation_id),
            None,
            Some(&canonical_value),
            Some(reachability.state),
        )
        .await?;
        evidence_ids.push(reachability_evidence.id);
        golish_db::repo::target_intel_asset_observations::record_reachability(
            pool,
            &golish_db::repo::target_intel_asset_observations::RecordReachability {
                observation_id,
                expected_row_version: 1,
                state: reachability.state.to_string(),
                method: reachability.method.to_string(),
                tool_call_id: Some(tool_call_id),
                evidence_id: reachability_evidence.id,
                checked_at: reachability.checked_at,
                valid_until: reachability.valid_until,
            },
        )
        .await?;
        if reachability.state != "reachable" {
            unreachable_refs.push(observation_id.to_string());
            semantic_observations.push(json!({
                "observation_id": observation_id,
                "asset_kind": asset_kind,
                "canonical_value": canonical_value,
                "provider_id": candidate.source,
                "attribution": disposition,
                "reachability": reachability.state,
                "promotion_target_id": Value::Null,
                "evidence_ids": [evidence.id, reachability_evidence.id],
                "replayed": false,
            }));
            continue;
        }
        let already_promoted: Option<Uuid> = sqlx::query_scalar(
            r#"SELECT promotion_target_id FROM target_intel_asset_observations
                WHERE operation_id=$1 AND organization_id=$2
                  AND canonical_identity_sha256=$3 AND promotion_target_id IS NOT NULL
                LIMIT 1"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .bind(&canonical_identity_sha256)
        .fetch_optional(pool)
        .await?;
        let target_id = if let Some(target_id) = already_promoted {
            target_id
        } else {
            golish_db::repo::target_intel_asset_observations::promote_owned_reachable(
                pool,
                observation_id,
                2,
            )
            .await?
        };
        promoted_target_refs.push(target_id.to_string());
        semantic_observations.push(json!({
            "observation_id": observation_id,
            "asset_kind": asset_kind,
            "canonical_value": canonical_value,
            "provider_id": candidate.source,
            "attribution": disposition,
            "reachability": reachability.state,
            "promotion_target_id": target_id,
            "evidence_ids": [evidence.id, reachability_evidence.id],
            "replayed": false,
        }));
    }

    observation_refs.sort();
    promoted_target_refs.sort();
    promoted_target_refs.dedup();
    ambiguous_refs.sort();
    unreachable_refs.sort();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    semantic_observations.sort_by(|left, right| {
        left.get("observation_id")
            .and_then(Value::as_str)
            .cmp(&right.get("observation_id").and_then(Value::as_str))
    });
    discovered_pivots.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.value.cmp(&right.value))
    });
    discovered_pivots.dedup();
    let journal_payload = json!({
        "tool": "recon_search_intel",
        "pivot_kind": pivot_kind,
        "pivot_value_sha256": sha256_hex(pivot_value.as_bytes()),
        "pivot_authorization_basis": pivot_authorization_basis,
        "pivot_has_scope_authority": pivot_has_scope_authority,
        "pivot_candidate_only": pivot_candidate_only,
        "provider_run_id": summary.run_id,
        "observation_refs": observation_refs,
        "promoted_target_refs": promoted_target_refs,
        "ambiguous_refs": ambiguous_refs,
        "unreachable_refs": unreachable_refs,
        "evidence_ids": evidence_ids,
        "observations": semantic_observations,
        "discovered_pivots": discovered_pivots,
    });
    let next_ordinal: i64 = sqlx::query_scalar(
        r#"SELECT COALESCE(MAX(ordinal)+1,0) FROM target_intel_goal_work_journal_entries
            WHERE team_plan_id=$1"#,
    )
    .bind(context.team_plan_id)
    .fetch_one(pool)
    .await?;
    golish_db::repo::target_intel_goal_work_journal::append(
        pool,
        &golish_db::repo::target_intel_goal_work_journal::TargetIntelGoalWorkJournalEntryRow {
            id: Uuid::new_v4(),
            stable_request_id: Uuid::new_v5(
                &tool_call_id,
                format!("intel-goal-tool-result:{}", context.goal_epoch).as_bytes(),
            ),
            operation_id,
            organization_id,
            team_plan_id: context.team_plan_id,
            goal_epoch_id: context.goal_epoch_id,
            goal_epoch: context.goal_epoch,
            controller_worker_run_id: context.controller_worker_run_id,
            controller_message_chain_id: context.controller_message_chain_id,
            ordinal: next_ordinal,
            entry_kind: "tool_result".to_string(),
            payload: journal_payload.clone(),
            related_frontier_refs: json!([]),
            evidence_refs: json!(evidence_ids),
            tool_call_refs: json!([tool_call_id]),
            observation_refs: json!(observation_refs),
            entry_sha256: prefixed_sha256(&journal_payload),
        },
    )
    .await?;
    Ok(journal_payload)
}

fn profile_field_pivot(field: &super::ObservedProfileField) -> Option<AssetIntelPivot> {
    let kind = match field.target_field.as_str() {
        "domains" | "app_domains" => AssetIntelPivotKind::Domain,
        "asns" => AssetIntelPivotKind::Asn,
        "certificates" => AssetIntelPivotKind::Certificate,
        "icp_records" | "icp" => AssetIntelPivotKind::Icp,
        "email_domains" => AssetIntelPivotKind::EmailDomain,
        "github_orgs" | "github_org" => AssetIntelPivotKind::GithubOrg,
        "code_repository" | "code_repositories" => AssetIntelPivotKind::Repository,
        "app_id" | "app_ids" | "mobile_apps" => AssetIntelPivotKind::AppId,
        _ => return None,
    };
    AssetIntelPivot::parse(kind, &field.value).ok()
}

fn attribution(
    candidate: &OrganizationCandidate,
    canonical_value: &str,
    legal_name: &str,
    trusted_roots: &BTreeSet<String>,
) -> (&'static str, Value) {
    let ownership_value = url::Url::parse(canonical_value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| canonical_value.to_string());
    if let Some(root) = trusted_roots
        .iter()
        .find(|root| ownership_value == **root || ownership_value.ends_with(&format!(".{root}")))
    {
        return (
            "owned",
            json!({"authority": "frozen_company_identity_scope_policy", "matched_root": root}),
        );
    }
    if candidate.confidence >= 0.7 && json_contains_text(&candidate.evidence, legal_name) {
        return (
            "owned",
            json!({"authority": "provider_corporate_identity_match", "legal_name": legal_name}),
        );
    }
    (
        "ambiguous",
        json!({"reason": "no strong frozen company identity ownership evidence"}),
    )
}

fn candidate_attribution(
    candidate: &OrganizationCandidate,
    canonical_value: &str,
    legal_name: &str,
    trusted_roots: &BTreeSet<String>,
    pivot_authorization_basis: &str,
    pivot_candidate_only: bool,
) -> (&'static str, &'static str, Value) {
    if pivot_candidate_only {
        return (
            "ambiguous",
            "identity_hypothesis_requires_corroboration_v1",
            json!({
                "reason": "model-originated identity hypothesis is a passive search term, not ownership or scope authority",
                "pivot_authorization_basis": pivot_authorization_basis,
            }),
        );
    }
    let (disposition, basis) = attribution(candidate, canonical_value, legal_name, trusted_roots);
    (disposition, "company_identity_scope_policy_v1", basis)
}

fn trusted_roots(scope_policy: &Value, identity_payload: &Value) -> BTreeSet<String> {
    fn visit(key: Option<&str>, value: &Value, roots: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (child_key, child) in map {
                    visit(Some(child_key), child, roots);
                }
            }
            Value::Array(values) => {
                for child in values {
                    visit(key, child, roots);
                }
            }
            Value::String(text)
                if key.is_some_and(|key| {
                    matches!(
                        key,
                        "domain"
                            | "domains"
                            | "root_domains"
                            | "trusted_roots"
                            | "authorized_roots"
                    )
                }) =>
            {
                let root = canonical_asset_value(text);
                if !root.is_empty() && root.parse::<IpAddr>().is_err() {
                    roots.insert(root);
                }
            }
            _ => {}
        }
    }
    let mut roots = BTreeSet::new();
    visit(None, scope_policy, &mut roots);
    visit(None, identity_payload, &mut roots);
    roots
}

fn json_contains_text(value: &Value, needle: &str) -> bool {
    if needle.trim().is_empty() {
        return false;
    }
    match value {
        Value::String(text) => text.trim().eq_ignore_ascii_case(needle.trim()),
        Value::Array(values) => values.iter().any(|value| json_contains_text(value, needle)),
        Value::Object(map) => map.values().any(|value| json_contains_text(value, needle)),
        _ => false,
    }
}

fn observed_service_port(evidence: &Value) -> Option<u16> {
    fn visit(value: &Value) -> Option<u16> {
        match value {
            Value::Object(map) => {
                for key in ["port", "service_port", "open_port"] {
                    if let Some(port) = map.get(key).and_then(|value| {
                        value
                            .as_u64()
                            .and_then(|value| u16::try_from(value).ok())
                            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                    }) {
                        return Some(port);
                    }
                }
                map.values().find_map(visit)
            }
            Value::Array(values) => values.iter().find_map(visit),
            _ => None,
        }
    }
    visit(evidence)
}

async fn probe_reachability(
    candidate: &crate::organizations::OrganizationCandidate,
    value: &str,
) -> ReachabilityReceipt {
    let checked_at = Utc::now();
    let parsed_url = url::Url::parse(value).ok();
    let (host, explicit_port) = if let Some(url) = parsed_url.as_ref() {
        (url.host_str().unwrap_or_default().to_string(), url.port())
    } else {
        (value.to_string(), None)
    };
    let observed_port = observed_service_port(&candidate.evidence);
    let mut ports = Vec::new();
    if let Some(port) = explicit_port.or(observed_port) {
        ports.push(port);
    }
    for port in [443, 80] {
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    let mut addresses = Vec::new();
    for port in &ports {
        if let Ok(resolved) = tokio::net::lookup_host((host.as_str(), *port)).await {
            for address in resolved {
                if !prohibited_destination(address.ip()) {
                    addresses.push(address);
                }
            }
        }
    }
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() {
        return ReachabilityReceipt {
            state: "blocked",
            method: "bounded_http_probe_v1",
            checked_at,
            valid_until: None,
            detail: json!({"reason": "no public destination address"}),
        };
    }
    let mut http_origins = Vec::new();
    if let Some(url) = parsed_url.as_ref() {
        http_origins.push((
            url.scheme().to_string(),
            url.port_or_known_default().unwrap_or(443),
            value.to_string(),
        ));
    } else {
        http_origins.push(("https".to_string(), 443, format!("https://{host}/")));
        http_origins.push(("http".to_string(), 80, format!("http://{host}/")));
        if let Some(port) = observed_port.filter(|port| !matches!(port, 80 | 443)) {
            http_origins.push(("https".to_string(), port, format!("https://{host}:{port}/")));
            http_origins.push(("http".to_string(), port, format!("http://{host}:{port}/")));
        }
    }
    for (scheme, port, url) in http_origins {
        let Some(address) = addresses.iter().find(|address| address.port() == port) else {
            continue;
        };
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(true);
        builder = builder.resolve(&host, SocketAddr::new(address.ip(), port));
        let Ok(client) = builder.build() else {
            continue;
        };
        if let Ok(response) = client.get(&url).send().await {
            return ReachabilityReceipt {
                state: "reachable",
                method: "bounded_http_probe_v1",
                checked_at,
                valid_until: Some(checked_at + TimeDelta::hours(24)),
                detail: json!({
                    "scheme": scheme,
                    "port": port,
                    "status": response.status().as_u16(),
                    "selected_address": address.ip().to_string(),
                }),
            };
        }
    }
    if let Some(port) = observed_port.or(explicit_port) {
        for address in addresses.iter().filter(|address| address.port() == port) {
            if tokio::time::timeout(
                Duration::from_secs(4),
                tokio::net::TcpStream::connect(address),
            )
            .await
            .is_ok_and(|result| result.is_ok())
            {
                return ReachabilityReceipt {
                    state: "reachable",
                    method: "bounded_tcp_protocol_probe_v1",
                    checked_at,
                    valid_until: Some(checked_at + TimeDelta::hours(24)),
                    detail: json!({
                        "port": port,
                        "selected_address": address.ip().to_string(),
                        "source": "provider_observed_service",
                    }),
                };
            }
        }
    }
    ReachabilityReceipt {
        state: "unreachable",
        method: "bounded_http_probe_v1",
        checked_at,
        valid_until: None,
        detail: json!({
            "attempted_addresses": addresses.iter().map(ToString::to_string).collect::<Vec<_>>()
        }),
    }
}

fn prohibited_destination(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [a, b, c, _] = address.octets();
            matches!(
                (a, b, c),
                (0, _, _)
                    | (10, _, _)
                    | (100, 64..=127, _)
                    | (127, _, _)
                    | (169, 254, _)
                    | (172, 16..=31, _)
                    | (192, 0, 0)
                    | (192, 0, 2)
                    | (192, 168, _)
                    | (198, 18..=19, _)
                    | (198, 51, 100)
                    | (203, 0, 113)
                    | (224..=255, _, _)
            )
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    }
}

fn canonical_asset_value(value: &str) -> String {
    let value = value.trim();
    if let Ok(url) = url::Url::parse(value) {
        if matches!(url.scheme(), "http" | "https") {
            if let Some(host) = url.host_str() {
                let mut origin = format!(
                    "{}://{}",
                    url.scheme().to_ascii_lowercase(),
                    host.to_ascii_lowercase()
                );
                if let Some(port) = url.port() {
                    origin.push(':');
                    origin.push_str(&port.to_string());
                }
                return origin;
            }
        }
    }
    value.trim_end_matches('.').to_ascii_lowercase()
}

fn asset_kind(value: &str) -> &'static str {
    if url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
    {
        "web_origin"
    } else if value.parse::<IpAddr>().is_ok() {
        "ip"
    } else if value.contains('/') {
        "cidr"
    } else {
        "hostname"
    }
}

fn redact_provider_payload(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = Map::new();
            for (key, value) in map {
                let normalized = key.to_ascii_lowercase();
                if normalized.contains("token")
                    || normalized.contains("secret")
                    || normalized.contains("password")
                    || normalized == "key"
                    || normalized.ends_with("_key")
                {
                    redacted.insert(key.clone(), Value::String("[REDACTED]".to_string()));
                } else {
                    redacted.insert(key.clone(), redact_provider_payload(value));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(values) => Value::Array(values.iter().map(redact_provider_payload).collect()),
        other => other.clone(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod semantic_profile_tests {
    use super::*;
    use golish_pentest::models::AssetIntelProfileFieldTarget;

    #[test]
    fn profile_facts_become_typed_search_pivots_not_targets() {
        let field = super::super::ObservedProfileField {
            provider_id: "quake".into(),
            target_kind: AssetIntelProfileFieldTarget::Scalar,
            target_field: "asns".into(),
            value: "as13335".into(),
        };
        assert_eq!(
            profile_field_pivot(&field),
            Some(AssetIntelPivot {
                kind: AssetIntelPivotKind::Asn,
                value: "AS13335".into(),
            })
        );
        let noise = super::super::ObservedProfileField {
            target_field: "quake_http_titles".into(),
            ..field
        };
        assert_eq!(profile_field_pivot(&noise), None);
    }

    #[test]
    fn provider_url_is_canonicalized_to_reachable_web_origin() {
        let value = canonical_asset_value("HTTPS://Portal.Example.com:8443/path?q=1");
        assert_eq!(value, "https://portal.example.com:8443");
        assert_eq!(asset_kind(&value), "web_origin");
    }
}

fn prefixed_sha256(value: &Value) -> String {
    format!(
        "sha256:{}",
        sha256_hex(&serde_json::to_vec(value).unwrap_or_default())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organizations::OrganizationCandidateKind;

    #[test]
    fn trusted_roots_only_read_explicit_company_identity_domain_fields() {
        let roots = trusted_roots(
            &json!({"trusted_roots": ["MoreSec.cn"]}),
            &json!({"noise": "example.org"}),
        );
        assert_eq!(roots, BTreeSet::from(["moresec.cn".to_string()]));
    }

    #[test]
    fn provider_secrets_are_not_stored_in_observation_fields() {
        let redacted = redact_provider_payload(&json!({
            "api_key": "secret",
            "nested": {"token": "secret", "title": "ok"}
        }));
        assert_eq!(redacted["api_key"], "[REDACTED]");
        assert_eq!(redacted["nested"]["token"], "[REDACTED]");
        assert_eq!(redacted["nested"]["title"], "ok");
    }

    #[test]
    fn private_and_documentation_addresses_are_never_probed() {
        assert!(prohibited_destination("127.0.0.1".parse().unwrap()));
        assert!(prohibited_destination("10.0.0.1".parse().unwrap()));
        assert!(prohibited_destination("203.0.113.1".parse().unwrap()));
        assert!(!prohibited_destination("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn reachability_uses_provider_observed_non_web_port() {
        assert_eq!(
            observed_service_port(&json!({"raw": {"service": {"port": "22"}}})),
            Some(22)
        );
        assert_eq!(
            observed_service_port(&json!({"raw": {"title": "22"}})),
            None
        );
    }

    #[test]
    fn high_confidence_identity_hypothesis_cannot_be_owned_in_its_first_call() {
        let candidate = OrganizationCandidate {
            id: "candidate-1".into(),
            kind: OrganizationCandidateKind::Target,
            label: "candidate".into(),
            value: "https://candidate.example".into(),
            organization_id: None,
            ownership_percent: None,
            source: "fixture".into(),
            confidence: 0.99,
            status: "new".into(),
            evidence: json!({"organization": "杭州默安科技有限公司"}),
            created_at: 0,
        };
        let (disposition, method, basis) = candidate_attribution(
            &candidate,
            &candidate.value,
            "杭州默安科技有限公司",
            &BTreeSet::new(),
            "identity_hypothesis_candidate_only",
            true,
        );
        assert_eq!(disposition, "ambiguous");
        assert_eq!(method, "identity_hypothesis_requires_corroboration_v1");
        assert_eq!(
            basis["pivot_authorization_basis"],
            "identity_hypothesis_candidate_only"
        );
    }
}
