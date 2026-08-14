//! Production-side Target Intel Goal review composition.
//!
//! This service reads the operation-frozen contract and DB-owned StageTeam
//! material, builds the pure four-section review bundle, and persists it only
//! through CAS repositories. The production StageTeam path freezes
//! `intel_goal_v1`, dispatches a durable read-only reviewer WorkItem, and may
//! commit PASS only through the compound Intel + StageTeam finalizer.

use std::sync::Arc;

use golish_agent_kit::db_traits::{
    FinalizeTargetIntelGoalPass, FreezeTargetIntelGoalUnitContract, FreezeTargetIntelReview,
    FrozenTargetIntelGoalUnitContractView, FrozenTargetIntelReviewView,
    ReadTargetIntelReviewSection, RecordTargetIntelReviewVerdict, RecordedTargetIntelReviewView,
    RuntimeMemoryError, TargetIntelReviewSectionView,
};
use golish_agent_kit::harness::{
    evaluate_intel_goal_finalizer, intel_goal_canonical_sha256, load_embedded_stage_spec,
    stage_methodology_md, IntelGoalCompletionAuthority, IntelGoalFinalizerDecision,
    IntelGoalFinalizerMaterial, IntelGoalOperationContract, IntelGoalRuntimeMode,
    IntelReviewBundle, IntelReviewBundleIdentity, IntelReviewDecision, StageKind,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

pub struct TargetIntelGoalCutoverService {
    pool: Arc<PgPool>,
}

pub struct FreezeTargetIntelGoalUnitContractMaterial {
    pub contract: IntelGoalOperationContract,
    pub organization_id: Uuid,
    pub team_plan_id: Uuid,
    pub goal_epoch_id: Uuid,
    pub controller_work_item_id: Uuid,
    pub controller_worker_run_id: Uuid,
    pub controller_message_chain_id: Uuid,
}

fn production_target_intel_provider_capability_manifest() -> serde_json::Value {
    json!({
        "capabilities": [{
            "batchable": true,
            "id": "intel.semantic_asset_discovery",
            "max_batch": 25,
            "risk": "passive",
            "runner": "existing_direct_tool",
            "techniques": [],
            "tool_names": ["recon_search_intel"],
            "writes": [
                "target_intel_goal_frontier_v2",
                "intel_semantic_artifacts",
                "audit_log",
                "targets_after_attribution_and_reachability"
            ]
        }],
        "schema_version": 1,
        "source": "intel_goal_v1_semantic_search_contract",
    })
}

fn production_target_intel_tool_manifest() -> serde_json::Value {
    json!({
        "goal_owner_tools": [
            "update_plan",
            "recon_search_intel",
            "stage_team_dispatch_workers",
            "stage_team_prepare_final_submission"
        ],
        "worker_tools": [
            "recon_search_intel",
            "list_recent_evidence",
            "submit_result"
        ],
        "final_submitter_tools": [
            "submit_stage_deliverable"
        ],
        "reviewer_tools": [
            "target_intel_read_review_section",
            "target_intel_record_review_verdict",
            "submit_result"
        ],
        "review_orchestration": "host_owned_after_prepare_final_submission",
        "stage_capability_ids": ["intel.semantic_asset_discovery"],
        "schema_version": 1,
    })
}

impl TargetIntelGoalCutoverService {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }

    pub async fn freeze_unit_contract(
        &self,
        input: FreezeTargetIntelGoalUnitContractMaterial,
    ) -> Result<bool, RuntimeMemoryError> {
        input
            .contract
            .validate()
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let runtime_mode = match input.contract.runtime_mode {
            IntelGoalRuntimeMode::Legacy => {
                return Err(RuntimeMemoryError::Conflict {
                    code: "TARGET_INTEL_GOAL_LEGACY_CONTRACT_MUST_BE_ABSENT",
                })
            }
            IntelGoalRuntimeMode::ObserveShadow => "observe_shadow",
            IntelGoalRuntimeMode::AdvisoryRework => "advisory_rework",
            IntelGoalRuntimeMode::IntelGoalV1 => "intel_goal_v1",
        };
        let completion_authority = match input.contract.completion_authority {
            IntelGoalCompletionAuthority::LegacySixAxisV1 => "legacy_six_axis_v1",
            IntelGoalCompletionAuthority::IntelGoalV1 => "intel_goal_v1",
        };
        let max_review_rounds = i32::try_from(input.contract.max_review_rounds).map_err(|_| {
            RuntimeMemoryError::Conflict {
                code: "TARGET_INTEL_GOAL_REVIEW_BUDGET_INVALID",
            }
        })?;
        let reviewer_retry_fuel =
            i32::try_from(input.contract.reviewer_retry_fuel).map_err(|_| {
                RuntimeMemoryError::Conflict {
                    code: "TARGET_INTEL_GOAL_REVIEW_BUDGET_INVALID",
                }
            })?;
        golish_db::repo::target_intel_goal_contracts::freeze_unit(
            &self.pool,
            &golish_db::repo::target_intel_goal_contracts::FreezeTargetIntelGoalUnit {
                contract: golish_db::repo::target_intel_goal_contracts::TargetIntelGoalOperationContractRow {
                    operation_id: input.contract.operation_id,
                    profile_id: input.contract.profile_id,
                    runtime_mode: runtime_mode.to_string(),
                    completion_authority: completion_authority.to_string(),
                    goal_contract_version: input.contract.goal_contract_version,
                    canonical_goal_contract: input.contract.canonical_goal_contract,
                    goal_contract_sha256: input.contract.goal_contract_sha256,
                    methodology_payload: input.contract.methodology_payload,
                    methodology_sha256: input.contract.methodology_sha256,
                    tool_manifest: input.contract.tool_manifest,
                    tool_manifest_sha256: input.contract.tool_manifest_sha256,
                    provider_capability_manifest: input.contract.provider_capability_manifest,
                    provider_capability_sha256: input.contract.provider_capability_sha256,
                    browser_policy: input.contract.browser_policy,
                    budget_policy: input.contract.budget_policy,
                    max_review_rounds,
                    reviewer_retry_fuel,
                },
                organization_id: input.organization_id,
                team_plan_id: input.team_plan_id,
                goal_epoch_id: input.goal_epoch_id,
                controller_work_item_id: input.controller_work_item_id,
                controller_worker_run_id: input.controller_worker_run_id,
                controller_message_chain_id: input.controller_message_chain_id,
            },
        )
        .await
        .map_err(storage)
    }

    /// Production composition root for Target Intel Goal v1. The operation,
    /// StageSpec, methodology and capability manifest are all server-owned;
    /// no model-facing payload can select completion authority or budgets.
    pub async fn freeze_production_unit_contract(
        &self,
        input: FreezeTargetIntelGoalUnitContract,
    ) -> Result<FrozenTargetIntelGoalUnitContractView, RuntimeMemoryError> {
        let operation = golish_db::repo::operation_state::get(&self.pool, input.operation_id)
            .await
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?
            .ok_or(RuntimeMemoryError::Missing {
                entity: "operation_state",
            })?;
        if operation.superseded_by.is_some()
            || operation.current_stage != StageKind::TargetIntel.as_str()
            || operation.runtime_memory_contract != "v2_only"
        {
            return Err(RuntimeMemoryError::Conflict {
                code: "TARGET_INTEL_GOAL_PRODUCTION_AUTHORITY_INVALID",
            });
        }
        let spec = load_embedded_stage_spec(StageKind::TargetIntel)
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let methodology_payload = json!({
            "methodology": stage_methodology_md(StageKind::TargetIntel).unwrap_or_default(),
            "stage_spec": spec,
        });
        let provider_capability_manifest = production_target_intel_provider_capability_manifest();
        let tool_manifest = production_target_intel_tool_manifest();
        let canonical_goal_contract = json!({
            "completion_authority": "intel_goal_v1",
            "four_section_review_required": true,
            "material_revision_match_required": true,
            "non_vacuous_terminal_receipt_required": true,
            "formal_asset_requires_owned_attribution": true,
            "formal_asset_requires_fresh_reachability": true,
            "legacy_six_axis_gate": false,
            "open_frontier_must_be_zero": true,
            "read_only_reviewer_required": true,
            "runtime_mode": "intel_goal_v1",
            "schema_version": 1,
            "stage": StageKind::TargetIntel.as_str(),
        });
        let browser_policy = json!({
            "active_navigation_allowed": false,
            "mode": "passive_only",
            "public_readonly_fallback_allowed": true,
            "provider_server_search_allowed": false,
            "schema_version": 1,
        });
        let budget_policy = json!({
            "max_review_rounds": 3,
            "reviewer_retry_fuel": 2,
            "schema_version": 1,
        });
        let contract = IntelGoalOperationContract {
            operation_id: input.operation_id,
            profile_id: operation.profile,
            runtime_mode: IntelGoalRuntimeMode::IntelGoalV1,
            completion_authority: IntelGoalCompletionAuthority::IntelGoalV1,
            goal_contract_version: "target_intel_goal.v1".to_string(),
            goal_contract_sha256: intel_goal_canonical_sha256(&canonical_goal_contract),
            canonical_goal_contract,
            methodology_sha256: intel_goal_canonical_sha256(&methodology_payload),
            methodology_payload,
            tool_manifest_sha256: intel_goal_canonical_sha256(&tool_manifest),
            tool_manifest,
            provider_capability_sha256: intel_goal_canonical_sha256(&provider_capability_manifest),
            provider_capability_manifest,
            browser_policy,
            budget_policy,
            max_review_rounds: 3,
            reviewer_retry_fuel: 2,
        };
        let operation_contract_sha256 = contract.goal_contract_sha256.clone();
        let goal_epoch_id = deterministic_goal_epoch_id(
            input.operation_id,
            input.organization_id,
            input.team_plan_id,
        );
        let replayed = self
            .freeze_unit_contract(FreezeTargetIntelGoalUnitContractMaterial {
                contract,
                organization_id: input.organization_id,
                team_plan_id: input.team_plan_id,
                goal_epoch_id,
                controller_work_item_id: input.controller_work_item_id,
                controller_worker_run_id: input.controller_worker_run_id,
                controller_message_chain_id: input.controller_message_chain_id,
            })
            .await?;
        let (current_goal_epoch_id, current_goal_epoch): (Uuid, i64) = sqlx::query_as(
            r#"SELECT id,epoch FROM target_intel_goal_epochs
                WHERE operation_id=$1 AND organization_id=$2 AND team_plan_id=$3
                  AND controller_work_item_id=$4
                  AND controller_worker_run_id=$5
                  AND controller_message_chain_id=$6
                  AND status IN ('open','sealed_for_review')
                ORDER BY epoch DESC LIMIT 1"#,
        )
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(input.team_plan_id)
        .bind(input.controller_work_item_id)
        .bind(input.controller_worker_run_id)
        .bind(input.controller_message_chain_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| storage(error.into()))?
        .ok_or(RuntimeMemoryError::Missing {
            entity: "target_intel_goal_open_epoch",
        })?;
        Ok(FrozenTargetIntelGoalUnitContractView {
            goal_epoch_id: current_goal_epoch_id,
            goal_epoch: current_goal_epoch,
            operation_contract_sha256,
            runtime_mode: IntelGoalRuntimeMode::IntelGoalV1,
            replayed,
        })
    }

    pub async fn freeze_review(
        &self,
        input: FreezeTargetIntelReview,
    ) -> Result<FrozenTargetIntelReviewView, RuntimeMemoryError> {
        let completion_claim = input.completion_claim.trim();
        if completion_claim.is_empty() || completion_claim.chars().count() > 12_000 {
            return Err(RuntimeMemoryError::Conflict {
                code: "TARGET_INTEL_REVIEW_COMPLETION_CLAIM_INVALID",
            });
        }
        let exact_replay = golish_db::repo::target_intel_goal_reviews::find_exact_freeze_replay(
            &self.pool,
            input.operation_id,
            input.organization_id,
            input.team_plan_id,
            input.expected_goal_epoch,
            input.controller_work_item_id,
            input.controller_worker_run_id,
            completion_claim,
        )
        .await
        .map_err(storage)?;
        // Freeze one structured completion checkpoint into the same durable
        // Main-AI work journal that the reviewer reads. This records the
        // auditable completion claim, not hidden model reasoning.
        let completion_checkpoint_request_id = Uuid::new_v5(
            &input.team_plan_id,
            format!("target-intel-completion:{}", input.expected_goal_epoch).as_bytes(),
        );
        let checkpoint_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM target_intel_goal_work_journal_entries WHERE stable_request_id=$1)",
        )
        .bind(completion_checkpoint_request_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| storage(error.into()))?;
        if !checkpoint_exists {
            let (goal_epoch_id, controller_message_chain_id): (Uuid, Uuid) = sqlx::query_as(
                r#"SELECT id,controller_message_chain_id
                     FROM target_intel_goal_epochs
                    WHERE operation_id=$1 AND organization_id=$2 AND team_plan_id=$3
                      AND epoch=$4 AND controller_worker_run_id=$5 AND status='open'
                    FOR SHARE"#,
            )
            .bind(input.operation_id)
            .bind(input.organization_id)
            .bind(input.team_plan_id)
            .bind(input.expected_goal_epoch)
            .bind(input.controller_worker_run_id)
            .fetch_one(&*self.pool)
            .await
            .map_err(|error| storage(error.into()))?;
            let ordinal: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(ordinal)+1,0) FROM target_intel_goal_work_journal_entries WHERE team_plan_id=$1",
            )
            .bind(input.team_plan_id)
            .fetch_one(&*self.pool)
            .await
            .map_err(|error| storage(error.into()))?;
            let payload = json!({
                "completion_claim": serde_json::from_str::<serde_json::Value>(completion_claim)
                    .unwrap_or_else(|_| json!(completion_claim)),
                "goal_epoch": input.expected_goal_epoch,
            });
            golish_db::repo::target_intel_goal_work_journal::append(
                &self.pool,
                &golish_db::repo::target_intel_goal_work_journal::TargetIntelGoalWorkJournalEntryRow {
                    id: Uuid::new_v4(),
                    stable_request_id: completion_checkpoint_request_id,
                    operation_id: input.operation_id,
                    organization_id: input.organization_id,
                    team_plan_id: input.team_plan_id,
                    goal_epoch_id,
                    goal_epoch: input.expected_goal_epoch,
                    controller_worker_run_id: input.controller_worker_run_id,
                    controller_message_chain_id,
                    ordinal,
                    entry_kind: "completion_checkpoint".to_string(),
                    payload: payload.clone(),
                    related_frontier_refs: json!([]),
                    evidence_refs: json!([]),
                    tool_call_refs: json!([]),
                    observation_refs: json!([]),
                    entry_sha256: intel_goal_canonical_sha256(&payload),
                },
            )
            .await
            .map_err(storage)?;
        }
        let mut snapshot = golish_db::repo::target_intel_goal_reviews::load_freeze_snapshot(
            &self.pool,
            input.operation_id,
            input.organization_id,
            input.stage_execution_id,
            input.stage_run_unit_id,
            input.team_plan_id,
            input.controller_work_item_id,
            input.controller_worker_run_id,
            input.expected_goal_epoch,
        )
        .await
        .map_err(storage)?;
        if let Some((review_generation, round)) = exact_replay {
            snapshot.review_generation = review_generation;
            snapshot.round = round;
        }
        if snapshot.plan_row_version != input.expected_plan_row_version {
            return Err(RuntimeMemoryError::StaleVersion {
                expected: input.expected_plan_row_version,
            });
        }
        let runtime_mode =
            IntelGoalRuntimeMode::parse(&snapshot.runtime_mode).map_err(|error| {
                RuntimeMemoryError::Conflict {
                    code: match error {
                        golish_agent_kit::harness::IntelGoalContractError::UnknownRuntimeMode => {
                            "TARGET_INTEL_GOAL_RUNTIME_MODE_UNKNOWN"
                        }
                        _ => "TARGET_INTEL_GOAL_OPERATION_CONTRACT_INVALID",
                    },
                }
            })?;
        let controller_message_chain_id =
            snapshot
                .controller_message_chain_id
                .ok_or(RuntimeMemoryError::IdentityMismatch {
                    code: "TARGET_INTEL_GOAL_CONTROLLER_CHAIN_MISSING",
                })?;
        let review_id = deterministic_review_id(
            snapshot.operation_id,
            snapshot.organization_id,
            snapshot.team_plan_id,
            snapshot.review_generation,
            snapshot.round,
            completion_claim,
        );
        let identity = IntelReviewBundleIdentity {
            review_id,
            operation_id: snapshot.operation_id,
            stage_execution_id: snapshot.stage_execution_id,
            stage_run_unit_id: snapshot.stage_run_unit_id,
            organization_id: snapshot.organization_id,
            team_plan_id: snapshot.team_plan_id,
            controller_work_item_id: snapshot.controller_work_item_id,
            controller_worker_run_id: snapshot.controller_worker_run_id,
            controller_message_chain_id,
            goal_epoch: snapshot.goal_epoch,
            review_generation: snapshot.review_generation,
            round: u32::try_from(snapshot.round).map_err(|_| RuntimeMemoryError::Conflict {
                code: "TARGET_INTEL_REVIEW_ROUND_INVALID",
            })?,
            state_revision: snapshot.state_revision,
        };
        let completion_claim = json!({"completion_claim": completion_claim});
        let bundle = IntelReviewBundle::freeze(
            identity,
            [
                snapshot.durable_state.clone(),
                snapshot.observable_actions.clone(),
                snapshot.frozen_contract.clone(),
                completion_claim.clone(),
            ],
        )
        .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let material_revision_vector = json!({
            "state_revision": snapshot.state_revision,
            "action_revision": snapshot.action_revision,
            "evidence_high_water": snapshot.evidence_high_water,
            "tool_high_water": snapshot.tool_high_water,
        });
        let inserted = golish_db::repo::target_intel_goal_reviews::insert_frozen_review(
            &self.pool,
            &golish_db::repo::target_intel_goal_reviews::InsertFrozenTargetIntelReview {
                review_id,
                expected_plan_row_version: input.expected_plan_row_version,
                completion_claim,
                material_revision_vector,
                material_state_sha256: intel_goal_canonical_sha256(&snapshot.durable_state),
                material_actions_sha256: intel_goal_canonical_sha256(&snapshot.observable_actions),
                durable_state_sha256: bundle.sections[0].sha256.clone(),
                observable_actions_sha256: bundle.sections[1].sha256.clone(),
                frozen_contract_sha256: bundle.sections[2].sha256.clone(),
                completion_claim_sha256: bundle.sections[3].sha256.clone(),
                bundle_sha256: bundle.bundle_sha256.clone(),
                snapshot,
            },
        )
        .await
        .map_err(storage)?;
        Ok(FrozenTargetIntelReviewView {
            review_id,
            reviewer_work_item_id: inserted.reviewer_work_item_id,
            review_round: bundle.identity.round,
            bundle_sha256: bundle.bundle_sha256,
            runtime_mode,
            detached_shadow: runtime_mode == IntelGoalRuntimeMode::ObserveShadow,
            replayed: inserted.replayed,
        })
    }

    pub async fn read_review_section(
        &self,
        input: ReadTargetIntelReviewSection,
    ) -> Result<TargetIntelReviewSectionView, RuntimeMemoryError> {
        // Worker attempt epoch is intentionally part of the public repository
        // fence. The DB review row binds the worker identity; scheduler wiring
        // must verify the attempt before calling this service.
        if input.expected_worker_attempt_epoch < 0 {
            return Err(RuntimeMemoryError::Conflict {
                code: "TARGET_INTEL_REVIEWER_ATTEMPT_EPOCH_INVALID",
            });
        }
        let requested = input.requested_kind;
        let row = golish_db::repo::target_intel_goal_reviews::read_section(
            &self.pool,
            input.review_id,
            input.reviewer_worker_run_id,
            input.expected_worker_attempt_epoch,
            requested.as_str(),
            &input.expected_bundle_sha256,
        )
        .await
        .map_err(storage)?;
        Ok(TargetIntelReviewSectionView {
            review_id: row.review_id,
            review_row_version: row.review_row_version,
            section_kind: requested,
            section_sha256: row.sha256,
            payload: row.payload,
            next_section: requested.next(),
            replayed: row.replayed,
        })
    }

    pub async fn record_review_verdict(
        &self,
        mut input: RecordTargetIntelReviewVerdict,
    ) -> Result<RecordedTargetIntelReviewView, RuntimeMemoryError> {
        input.verdict.stamp_finding_fingerprints();
        let inherited_material_findings =
            golish_db::repo::target_intel_goal_reviews::load_inherited_material_finding_ids(
                &self.pool,
                input.review_id,
            )
            .await
            .map_err(storage)?;
        input
            .verdict
            .validate(&inherited_material_findings)
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let verdict = serde_json::to_value(&input.verdict)
            .map_err(|error| RuntimeMemoryError::Storage(error.to_string()))?;
        let verdict_sha256 = intel_goal_canonical_sha256(&verdict);
        let decision = match input.verdict.decision {
            IntelReviewDecision::Pass => "pass",
            IntelReviewDecision::Rework => "rework",
            IntelReviewDecision::NeedsHuman => "needs_human",
        };
        let recorded = golish_db::repo::target_intel_goal_reviews::record_terminal_verdict(
            &self.pool,
            input.review_id,
            input.reviewer_worker_run_id,
            input.expected_worker_attempt_epoch,
            input.expected_review_row_version,
            &input.expected_bundle_sha256,
            decision,
            &verdict,
            &verdict_sha256,
        )
        .await
        .map_err(storage)?;
        let effective_decision = match recorded.effective_decision.as_str() {
            "pass" => IntelReviewDecision::Pass,
            "rework" => IntelReviewDecision::Rework,
            "needs_human" => IntelReviewDecision::NeedsHuman,
            _ => {
                return Err(RuntimeMemoryError::IdentityMismatch {
                    code: "TARGET_INTEL_REVIEW_EFFECTIVE_DECISION_UNKNOWN",
                })
            }
        };
        let successor_goal_epoch =
            if let Some(successor_goal_epoch_id) = recorded.successor_goal_epoch_id {
                Some(
                    sqlx::query_scalar::<_, i64>(
                        "SELECT epoch FROM target_intel_goal_epochs WHERE id=$1",
                    )
                    .bind(successor_goal_epoch_id)
                    .fetch_optional(&*self.pool)
                    .await
                    .map_err(|error| storage(anyhow::Error::new(error)))?
                    .ok_or(RuntimeMemoryError::Missing {
                        entity: "target_intel_goal_successor_epoch",
                    })?,
                )
            } else {
                None
            };
        Ok(RecordedTargetIntelReviewView {
            review_id: input.review_id,
            review_row_version: recorded.review_row_version,
            decision: effective_decision,
            verdict_sha256,
            successor_goal_epoch,
            hold_id: recorded.hold_id,
            replayed: recorded.replayed,
        })
    }

    pub async fn authorize_finalizer(
        &self,
        input: &FinalizeTargetIntelGoalPass,
    ) -> Result<IntelGoalFinalizerDecision, RuntimeMemoryError> {
        if input.expected_review_row_version < 0 {
            return Err(RuntimeMemoryError::Conflict {
                code: "TARGET_INTEL_GOAL_FINALIZER_VERSION_INVALID",
            });
        }
        let snapshot = golish_db::repo::target_intel_goal_reviews::load_finalizer_snapshot(
            &self.pool,
            input.operation_id,
            input.organization_id,
            input.review_id,
            &input.expected_bundle_sha256,
            &input.expected_verdict_sha256,
            &input.expected_operation_contract_sha256,
            input.expected_review_row_version,
        )
        .await
        .map_err(storage)?;
        let material = IntelGoalFinalizerMaterial {
            review_id: snapshot.review_id,
            operation_contract_sha256: snapshot.operation_contract_sha256,
            review_bundle_sha256: snapshot.review_bundle_sha256,
            verdict_sha256: snapshot.verdict_sha256,
            operation_contract_valid: snapshot.operation_contract_valid,
            review_is_fresh_pass: snapshot.review_is_fresh_pass,
            all_four_sections_read: snapshot.all_four_sections_read,
            material_revision_matches: snapshot.material_revision_matches,
            active_authoritative_workers: count(snapshot.active_authoritative_workers)?,
            active_authoritative_tools: count(snapshot.active_authoritative_tools)?,
            current_run_terminal_receipt_count: count(snapshot.current_run_terminal_receipt_count)?,
            valid_evidence_artifact_closure_count: count(
                snapshot.valid_evidence_artifact_closure_count,
            )?,
            pending_or_retryable_frontier_count: count(
                snapshot.pending_or_retryable_frontier_count,
            )?,
            unwaived_blocked_or_unsupported_count: count(
                snapshot.unwaived_blocked_or_unsupported_count,
            )?,
            unresolved_material_contradiction_count: count(
                snapshot.unresolved_material_contradiction_count,
            )?,
            open_material_finding_count: count(snapshot.open_material_finding_count)?,
            unauthorized_scope_promotion_count: count(snapshot.unauthorized_scope_promotion_count)?,
            needs_human_count: count(snapshot.needs_human_count)?,
        };
        Ok(evaluate_intel_goal_finalizer(&material))
    }
}

fn deterministic_review_id(
    operation_id: Uuid,
    organization_id: Uuid,
    team_plan_id: Uuid,
    generation: i64,
    round: i32,
    completion_claim: &str,
) -> Uuid {
    let claim_sha256 =
        intel_goal_canonical_sha256(&json!({"completion_claim": completion_claim.trim()}));
    let digest = Sha256::digest(
        format!(
            "target-intel-review:v1:{operation_id}:{organization_id}:{team_plan_id}:{generation}:{round}:{claim_sha256}"
        )
        .as_bytes(),
    );
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn deterministic_goal_epoch_id(
    operation_id: Uuid,
    organization_id: Uuid,
    team_plan_id: Uuid,
) -> Uuid {
    let digest = Sha256::digest(
        format!("target-intel-goal-epoch:v1:{operation_id}:{organization_id}:{team_plan_id}:0")
            .as_bytes(),
    );
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn storage(error: anyhow::Error) -> RuntimeMemoryError {
    RuntimeMemoryError::Storage(error.to_string())
}

fn count(value: i64) -> Result<usize, RuntimeMemoryError> {
    usize::try_from(value).map_err(|_| RuntimeMemoryError::Conflict {
        code: "TARGET_INTEL_GOAL_FINALIZER_COUNT_INVALID",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_names<'a>(manifest: &'a serde_json::Value, field: &str) -> Vec<&'a str> {
        manifest[field]
            .as_array()
            .expect("tool manifest field must be an array")
            .iter()
            .map(|value| value.as_str().expect("tool name must be a string"))
            .collect()
    }

    #[test]
    fn production_tool_contract_is_semantic_goal_owned_not_legacy_worklist() {
        let manifest = production_target_intel_tool_manifest();
        assert_eq!(
            tool_names(&manifest, "goal_owner_tools"),
            [
                "update_plan",
                "recon_search_intel",
                "stage_team_dispatch_workers",
                "stage_team_prepare_final_submission",
            ]
        );
        assert_eq!(
            tool_names(&manifest, "worker_tools"),
            [
                "recon_search_intel",
                "list_recent_evidence",
                "submit_result"
            ]
        );
        assert_eq!(
            tool_names(&manifest, "final_submitter_tools"),
            ["submit_stage_deliverable"]
        );

        let serialized = manifest.to_string();
        for retired in [
            "recon_list_providers",
            "recon_map_assets",
            "recon_lookup_whois",
            "check_stage_asset_coverage",
            "stage_worklist_status",
            "stage_worklist_next",
            "stage_team_spawn_intel_subagents",
            "stage_team_request_intel_review",
        ] {
            assert!(
                !serialized.contains(retired),
                "production contract must not advertise retired tool {retired}"
            );
        }
    }

    #[test]
    fn production_capability_contract_exposes_semantic_search_only() {
        let manifest = production_target_intel_provider_capability_manifest();
        let capabilities = manifest["capabilities"]
            .as_array()
            .expect("capabilities must be an array");
        assert_eq!(capabilities.len(), 1);
        assert_eq!(
            tool_names(&capabilities[0], "tool_names"),
            ["recon_search_intel"]
        );
        assert_eq!(manifest["source"], "intel_goal_v1_semantic_search_contract");
    }
}
