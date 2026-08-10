//! Two-wave Candidate hypothesis-analysis phase machine.
//!
//! The repository owns all authority, census, work-item and retry identities;
//! this runtime only executes server-issued submit-only work under a rolling
//! concurrency cap.  `8` is a live-lane ceiling, never a lifetime item cap.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use futures::{stream, StreamExt, TryStreamExt};
use golish_agent_kit::db_traits::{FreezeCandidateAnalysisSnapshot, HypothesisRegistryRepository};
use golish_agent_kit::harness::stage_spec::CandidateAnalysisTeamPolicy;
use golish_agent_kit::task_orchestrator::hypothesis_analysis::*;
use golish_db::repo::candidate_analysis::CandidateWriteFenceRow;
use golish_db::repo::candidate_analysis_runtime as runtime_db;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::candidate_analysis_gate::{
    compile_candidate_host_recipe, validate_controller_decision_route_confirmation,
    validate_controller_proposal_pages, AtomicCandidateFinalizer,
    CandidateAtomicFinalizationRequest, CandidateHostCompilationAuthority,
};
use super::candidate_analysis_projection::{
    project_sealed_candidate_chunk, validate_candidate_chunk_ref, SealedCandidateChunkProjectionRow,
};

pub(crate) fn project_chunk_value(value: Value) -> anyhow::Result<Value> {
    let sealed: SealedCandidateChunkProjectionRow = serde_json::from_value(value)?;
    let envelope = project_sealed_candidate_chunk(sealed)?;
    let projected = CandidateChunkRef::from(&envelope);
    validate_candidate_chunk_ref(&projected)?;
    Ok(serde_json::to_value(projected)?)
}

pub(crate) fn project_analyst_input(mut value: Value) -> anyhow::Result<CandidateAnalystInput> {
    let chunks = value
        .get_mut("chunks")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("candidate analyst sealed chunk set is absent"))?;
    for chunk in chunks {
        *chunk = project_chunk_value(std::mem::take(chunk))?;
    }
    let input: CandidateAnalystInput = serde_json::from_value(value)?;
    validate_analyst_input(&input)?;
    Ok(input)
}

pub(crate) fn project_critic_input(mut value: Value) -> anyhow::Result<CandidateCriticInput> {
    if value.get("mode").and_then(Value::as_str) == Some("coverage_subreview") {
        let chunks = value
            .get_mut("designated_chunks")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow::anyhow!("candidate critic sealed chunk set is absent"))?;
        for chunk in chunks {
            *chunk = project_chunk_value(std::mem::take(chunk))?;
        }
    }
    let input: CandidateCriticInput = serde_json::from_value(value)?;
    validate_critic_input(&input)?;
    Ok(input)
}

fn validate_analyst_input(input: &CandidateAnalystInput) -> anyhow::Result<()> {
    anyhow::ensure!(!input.chunks.is_empty(), "candidate analyst input is empty");
    for chunk in &input.chunks {
        validate_candidate_chunk_ref(chunk)?;
    }
    Ok(())
}

fn validate_critic_input(input: &CandidateCriticInput) -> anyhow::Result<()> {
    if let CandidateCriticInput::ProposalConflict {
        conflict_component_hash,
        proposals,
        proposal_summaries,
        ..
    } = input
    {
        anyhow::ensure!(
            conflict_component_hash.starts_with("sha256:"),
            "candidate conflict component authority hash is invalid"
        );
        anyhow::ensure!(
            !proposals.is_empty() && proposals.len() <= 64,
            "candidate conflict component is empty or oversized"
        );
        let proposal_ids = proposals
            .iter()
            .map(|proposal| (proposal.proposal_id, proposal.proposal_hash.as_str()))
            .collect::<BTreeSet<_>>();
        let summary_ids = proposal_summaries
            .iter()
            .map(|proposal| (proposal.proposal_id, proposal.proposal_hash.as_str()))
            .collect::<BTreeSet<_>>();
        anyhow::ensure!(
            proposal_ids.len() == proposals.len()
                && summary_ids.len() == proposal_summaries.len()
                && proposal_ids == summary_ids,
            "candidate conflict semantic summary exact set is invalid"
        );
        for summary in proposal_summaries {
            anyhow::ensure!(
                summary.proposal_hash.starts_with("sha256:")
                    && summary.predicate_version > 0
                    && !summary.subject_kind.trim().is_empty()
                    && !summary.subject_identity_hash.trim().is_empty()
                    && !summary.predicate_schema.trim().is_empty()
                    && !summary.structured_claim.trim().is_empty()
                    && !summary.impact.trim().is_empty(),
                "candidate conflict semantic summary is invalid"
            );
            anyhow::ensure!(
                summary.predicate_arguments.len() <= 32
                    && summary.preconditions.len() <= 16
                    && summary.proof_input_ids.len() <= 64
                    && summary.application_context_input_ids.len() <= 64
                    && summary.gap_input_ids.len() <= 64
                    && summary.knowledge_signals.len() <= 32
                    && summary.structured_claim.len() <= 4096
                    && summary.impact.len() <= 4096
                    && summary
                        .preconditions
                        .iter()
                        .all(|value| !value.trim().is_empty() && value.len() <= 1024)
                    && summary.predicate_arguments.iter().all(|(key, value)| {
                        !key.trim().is_empty()
                            && !value.trim().is_empty()
                            && key.len() <= 256
                            && value.len() <= 1024
                    }),
                "candidate conflict semantic summary exceeds its server bounds"
            );
        }
    }
    if let CandidateCriticInput::CoverageSubreview {
        designated_chunks, ..
    } = input
    {
        anyhow::ensure!(
            !designated_chunks.is_empty(),
            "candidate critic partition is empty"
        );
        for chunk in designated_chunks {
            validate_candidate_chunk_ref(chunk)?;
        }
    }
    Ok(())
}

/// Production authority adapter for the part of the Candidate runtime that is
/// already backed by the Plan B repository. Snapshot scope, Checked Tool Truth
/// bundle and feed disposition are all derived by the server; no caller list or
/// guard token enters this boundary.
///
#[derive(Clone)]
pub struct PgHypothesisAnalysisRuntimeRepository {
    pool: Arc<PgPool>,
    registry: Arc<dyn HypothesisRegistryRepository>,
    finalizer: AtomicCandidateFinalizer,
}

impl PgHypothesisAnalysisRuntimeRepository {
    pub fn new(
        pool: Arc<PgPool>,
        registry: Arc<dyn HypothesisRegistryRepository>,
        finalizer: AtomicCandidateFinalizer,
    ) -> Self {
        Self {
            pool,
            registry,
            finalizer,
        }
    }

    async fn scope_snapshot_id(
        &self,
        operation_id: Uuid,
        organization_id: Uuid,
        supplied: Uuid,
    ) -> anyhow::Result<Uuid> {
        let derived: Uuid = sqlx::query_scalar(
            r#"SELECT snapshot.id
                 FROM operation_org_scope_snapshots snapshot
                WHERE snapshot.operation_id=$1
                  AND snapshot.sealed_at IS NOT NULL
                  AND EXISTS (
                      SELECT 1 FROM operation_org_scope_units unit
                       WHERE unit.snapshot_id=snapshot.id
                         AND unit.organization_id=$2
                  )
                ORDER BY snapshot.sealed_at DESC,snapshot.id DESC
                LIMIT 1"#,
        )
        .bind(operation_id)
        .bind(organization_id)
        .fetch_optional(self.pool.as_ref())
        .await?
        .ok_or_else(|| anyhow::anyhow!("CANDIDATE_ANALYSIS_SCOPE_SNAPSHOT_UNAVAILABLE"))?;
        anyhow::ensure!(
            supplied.is_nil() || supplied == derived,
            "CANDIDATE_ANALYSIS_SCOPE_SNAPSHOT_MISMATCH"
        );
        Ok(derived)
    }
}

fn db_fence(value: &CandidateRuntimeWorkAuthority) -> anyhow::Result<CandidateWriteFenceRow> {
    let fence = &value.fence;
    Ok(CandidateWriteFenceRow {
        operation_id: fence.operation_id,
        scope_snapshot_id: fence.scope_snapshot_id,
        organization_id: fence.organization_id,
        snapshot_id: fence.snapshot_id,
        team_plan_id: fence.team_plan_id,
        work_item_id: fence.work_item_id,
        worker_run_id: fence.worker_run_id,
        lease_token: fence.lease_token,
        lease_epoch: i64::try_from(fence.lease_epoch)?,
        analysis_attempt_id: fence.analysis_attempt_id,
        analysis_attempt_ordinal: i32::try_from(fence.analysis_attempt_ordinal)?,
        attempt_epoch: i64::try_from(fence.attempt_epoch)?,
        expected_snapshot_row_version: fence.expected_snapshot_row_version,
        expected_team_plan_row_version: fence.expected_team_plan_row_version,
        expected_work_item_row_version: fence.expected_work_item_row_version,
        expected_worker_row_version: fence.expected_worker_row_version,
        expected_attempt_row_version: fence.expected_attempt_row_version,
    })
}

fn runtime_authority(
    value: CandidateWriteFenceRow,
) -> anyhow::Result<CandidateRuntimeWorkAuthority> {
    Ok(CandidateRuntimeWorkAuthority {
        fence: golish_agent_kit::db_traits::CandidateRepositoryWriteFenceV1 {
            operation_id: value.operation_id,
            scope_snapshot_id: value.scope_snapshot_id,
            organization_id: value.organization_id,
            snapshot_id: value.snapshot_id,
            team_plan_id: value.team_plan_id,
            work_item_id: value.work_item_id,
            worker_run_id: value.worker_run_id,
            lease_token: value.lease_token,
            lease_epoch: u64::try_from(value.lease_epoch)?,
            analysis_attempt_id: value.analysis_attempt_id,
            analysis_attempt_ordinal: u32::try_from(value.analysis_attempt_ordinal)?,
            attempt_epoch: u64::try_from(value.attempt_epoch)?,
            expected_snapshot_row_version: value.expected_snapshot_row_version,
            expected_team_plan_row_version: value.expected_team_plan_row_version,
            expected_work_item_row_version: value.expected_work_item_row_version,
            expected_worker_row_version: value.expected_worker_row_version,
            expected_attempt_row_version: value.expected_attempt_row_version,
        },
    })
}

fn runtime_binding(
    authority: &CandidateRuntimeWorkAuthority,
    role: CandidateAnalysisAgentRole,
    lane_ordinal: i32,
) -> anyhow::Result<CandidateAnalysisAgentBinding> {
    Ok(CandidateAnalysisAgentBinding {
        analysis_attempt_id: authority.fence.analysis_attempt_id,
        analysis_attempt_ordinal: authority.fence.analysis_attempt_ordinal,
        work_item_id: authority.fence.work_item_id,
        worker_run_id: authority.fence.worker_run_id,
        role,
        lane_ordinal: u32::try_from(lane_ordinal)?,
        read_only: true,
        allowed_tools: vec!["submit_result".to_owned()],
    })
}

fn runtime_receipt(value: runtime_db::CandidateArtifactReceiptRow) -> CandidateArtifactReceipt {
    CandidateArtifactReceipt {
        artifact_id: value.artifact_id,
        artifact_hash: value.artifact_hash,
    }
}

#[async_trait]
impl HypothesisAnalysisRuntimeRepository for PgHypothesisAnalysisRuntimeRepository {
    async fn freeze_snapshot(
        &self,
        request: HypothesisAnalysisStageRequest,
    ) -> anyhow::Result<CandidateRuntimeSnapshot> {
        let scope_snapshot_id = self
            .scope_snapshot_id(
                request.operation_id,
                request.organization_id,
                request.scope_snapshot_id,
            )
            .await?;
        let snapshot = self
            .registry
            .freeze_candidate_snapshot(FreezeCandidateAnalysisSnapshot {
                stable_consumer_request_id: request.stable_request_id,
                operation_id: request.operation_id,
                scope_snapshot_id,
                organization_id: request.organization_id,
            })
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let disposition = match snapshot.disposition {
            golish_agent_kit::db_traits::CandidateAnalysisSnapshotDispositionV1::SealedReady => {
                CandidateRuntimeSnapshotDisposition::SealedReady
            }
            golish_agent_kit::db_traits::CandidateAnalysisSnapshotDispositionV1::SealedAnalysisReadyWithResiduals => {
                CandidateRuntimeSnapshotDisposition::SealedAnalysisReadyWithResiduals
            }
            golish_agent_kit::db_traits::CandidateAnalysisSnapshotDispositionV1::BlockedAuthorityBundle => {
                CandidateRuntimeSnapshotDisposition::BlockedAuthorityBundle
            }
        };
        let analysis_ready = matches!(
            disposition,
            CandidateRuntimeSnapshotDisposition::SealedReady
                | CandidateRuntimeSnapshotDisposition::SealedAnalysisReadyWithResiduals
        );
        let input_count = if analysis_ready {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1",
            )
            .bind(snapshot.snapshot_id)
            .fetch_one(self.pool.as_ref())
            .await?;
            u32::try_from(count)
                .map_err(|_| anyhow::anyhow!("candidate snapshot input count overflow"))?
        } else {
            0
        };
        let input_chunk_census_set_hash = if analysis_ready {
            sqlx::query_scalar(
                r#"SELECT tool_truth_sha256(to_jsonb(ARRAY(
                       SELECT census_hash
                         FROM candidate_analysis_input_chunk_censuses
                        WHERE snapshot_id=$1
                        ORDER BY snapshot_input_id
                   ))::TEXT)"#,
            )
            .bind(snapshot.snapshot_id)
            .fetch_one(self.pool.as_ref())
            .await?
        } else {
            snapshot.knowledge_feed_obligation_set_hash.clone()
        };
        let blocked_residual_hash = (disposition
            == CandidateRuntimeSnapshotDisposition::BlockedAuthorityBundle)
            .then(|| {
                if snapshot.knowledge_feed_obligation_set_hash.is_empty() {
                    snapshot.stale_revalidation_obligation_set_hash.clone()
                } else {
                    snapshot.knowledge_feed_obligation_set_hash.clone()
                }
            });
        Ok(CandidateRuntimeSnapshot {
            snapshot_id: snapshot.snapshot_id,
            disposition,
            input_count,
            snapshot_authority_hash: snapshot.candidate_snapshot_authority_hash,
            input_chunk_census_set_hash,
            blocked_residual_hash,
            stage_execution_id: request.stage_execution_id,
        })
    }

    async fn open_attempt(
        &self,
        snapshot_id: Uuid,
        attempt_ordinal: u32,
        stage_execution_id: Uuid,
    ) -> anyhow::Result<CandidateRuntimeAttempt> {
        let row = runtime_db::open_or_replay_attempt_runtime(
            self.pool.as_ref(),
            snapshot_id,
            stage_execution_id,
            i32::try_from(attempt_ordinal)?,
        )
        .await?;
        let controller_authority = runtime_authority(row.controller_fence)?;
        let controller_binding = runtime_binding(
            &controller_authority,
            CandidateAnalysisAgentRole::Controller,
            0,
        )?;
        Ok(CandidateRuntimeAttempt {
            analysis_attempt_id: row.analysis_attempt_id,
            analysis_attempt_ordinal: u32::try_from(row.analysis_attempt_ordinal)?,
            stage_execution_id,
            controller_binding,
            controller_authority,
            controller_dispatch_input: CandidateControllerDispatchInput {
                snapshot_id,
                snapshot_authority_hash: row.snapshot_authority_hash,
                input_count: u32::try_from(row.input_count)?,
                input_chunk_census_set_hash: row.input_chunk_census_set_hash,
                relationship_cross_index_hash: row.relationship_cross_index_hash,
                missed_hypothesis_signals: row
                    .missed_hypothesis_signals
                    .into_iter()
                    .map(serde_json::from_value)
                    .collect::<Result<_, _>>()?,
                missed_hypothesis_signal_set_hash: row.missed_hypothesis_signal_set_hash,
            },
            controller_dispatch_replay: row
                .dispatch_replay
                .map(|replay| -> anyhow::Result<_> {
                    Ok(CandidateAnalysisAgentAttempt {
                        provider_attempt_id: replay.provider_attempt_id,
                        output: serde_json::from_value(replay.body)?,
                    })
                })
                .transpose()?,
        })
    }

    async fn prepare_analyst_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
        dispatch: &CandidateControllerDispatchPlan,
        host_lane_limit: usize,
    ) -> anyhow::Result<Vec<CandidateAnalystWorkItem>> {
        runtime_db::prepare_analyst_work_batch(
            self.pool.as_ref(),
            attempt.controller_dispatch_input.snapshot_id,
            attempt.stage_execution_id,
            i32::try_from(attempt.analysis_attempt_ordinal)?,
            i32::try_from(dispatch.requested_inputs_per_microbatch)?,
            i32::try_from(host_lane_limit)?,
        )
        .await?
        .into_iter()
        .map(|row| {
            let authority = runtime_authority(row.fence)?;
            let input = project_analyst_input(row.input)?;
            Ok(CandidateAnalystWorkItem {
                binding: runtime_binding(
                    &authority,
                    CandidateAnalysisAgentRole::Analyst,
                    row.lane_ordinal,
                )?,
                authority,
                input,
                replayed_receipt: row.replayed_receipt.map(runtime_receipt),
            })
        })
        .collect()
    }

    async fn persist_controller_dispatch(
        &self,
        _binding: &CandidateAnalysisAgentBinding,
        authority: &CandidateRuntimeWorkAuthority,
        attempt: &CandidateAnalysisAgentAttempt<CandidateControllerDispatchPlan>,
    ) -> anyhow::Result<()> {
        runtime_db::persist_controller_dispatch(
            self.pool.as_ref(),
            &db_fence(authority)?,
            attempt.provider_attempt_id,
            &serde_json::to_value(&attempt.output)?,
        )
        .await?;
        Ok(())
    }

    async fn persist_analyst_artifact(
        &self,
        _binding: &CandidateAnalysisAgentBinding,
        authority: &CandidateRuntimeWorkAuthority,
        attempt: &CandidateAnalysisAgentAttempt<HypothesisProposalArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence> {
        let receipt = runtime_db::persist_candidate_worker_artifact(
            self.pool.as_ref(),
            runtime_db::PersistCandidateWorkerArtifact {
                fence: db_fence(authority)?,
                provider_attempt_id: attempt.provider_attempt_id,
                artifact_kind: "hypothesis_proposal.v1".to_owned(),
                artifact_body: serde_json::to_value(&attempt.output)?,
            },
        )
        .await?;
        Ok(CandidateArtifactPersistence::Committed(runtime_receipt(
            receipt,
        )))
    }

    async fn prepare_critic_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
        max_coverage_subreview_work_items: usize,
    ) -> anyhow::Result<CandidateCriticWavePlan> {
        let (h1_census_hash, rows, terminal_blocked_residual_hash) =
            runtime_db::prepare_critic_work_batch(
                self.pool.as_ref(),
                attempt.controller_dispatch_input.snapshot_id,
                attempt.stage_execution_id,
                i32::try_from(attempt.analysis_attempt_ordinal)?,
                8,
                max_coverage_subreview_work_items,
            )
            .await?;
        Ok(CandidateCriticWavePlan {
            work_items: rows
                .into_iter()
                .map(|row| {
                    let authority = runtime_authority(row.fence)?;
                    let input = project_critic_input(row.input)?;
                    Ok(CandidateCriticWorkItem {
                        binding: runtime_binding(
                            &authority,
                            CandidateAnalysisAgentRole::Critic,
                            row.lane_ordinal,
                        )?,
                        authority,
                        input,
                        replayed_receipt: row.replayed_receipt.map(runtime_receipt),
                    })
                })
                .collect::<anyhow::Result<_>>()?,
            h1_census_hash,
            terminal_blocked_residual_hash,
        })
    }

    async fn persist_critic_artifact(
        &self,
        _binding: &CandidateAnalysisAgentBinding,
        authority: &CandidateRuntimeWorkAuthority,
        attempt: &CandidateAnalysisAgentAttempt<HypothesisCriticArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence> {
        let fence = db_fence(authority)?;
        let provider_artifact_body = serde_json::to_value(&attempt.output)?;
        let artifact_kind = match &attempt.output {
            HypothesisCriticArtifact::ProposalConflict { .. } => "proposal_conflict_review.v1",
            HypothesisCriticArtifact::CoverageSubreview {
                subreview_census_member_id,
                finding,
            } => {
                let census_id: Uuid = sqlx::query_scalar(
                    "SELECT subreview_census_id FROM candidate_analysis_hypothesis_coverage_subreview_census_members WHERE subreview_census_member_id=$1",
                )
                .bind(subreview_census_member_id)
                .fetch_one(self.pool.as_ref())
                .await?;
                let outcome = match finding.outcome {
                    CandidateCriticOutcome::NoMiss => "no_local_miss",
                    CandidateCriticOutcome::MissedHypothesis => "missed_hypothesis",
                    CandidateCriticOutcome::Blocked => "blocked",
                };
                golish_db::repo::candidate_analysis::record_hypothesis_coverage_subreview(
                    self.pool.as_ref(),
                    golish_db::repo::candidate_analysis::RecordCoverageSubreviewInput {
                        fence: fence.clone(),
                        stable_review_request_id: attempt.provider_attempt_id,
                        subreview_census_id: census_id,
                        subreview_census_member_id: *subreview_census_member_id,
                        outcome: outcome.to_owned(),
                        missed_proposal_ids: finding.missed_hypothesis_refs.clone(),
                        blocker_codes: finding.blocker_codes.clone(),
                        semantic_summary: serde_json::to_value(&finding.semantic_summary)?,
                        review_notes: "typed Candidate subreview".to_owned(),
                        provider_attempt_id: Some(attempt.provider_attempt_id),
                        provider_artifact_body: Some(provider_artifact_body.clone()),
                    },
                )
                .await
                .map_err(|error| {
                    anyhow::anyhow!(
                        "Candidate subreview {} persistence failed: {error}",
                        subreview_census_member_id
                    )
                })?;
                "hypothesis_coverage_subreview.v1"
            }
            HypothesisCriticArtifact::CoverageCrossChunkSynthesis {
                synthesis_node_id,
                finding,
                ..
            }
            | HypothesisCriticArtifact::CoverageCrossInputPartition {
                synthesis_node_id,
                finding,
                ..
            }
            | HypothesisCriticArtifact::CoverageCrossInputReduce {
                synthesis_node_id,
                finding,
                ..
            }
            | HypothesisCriticArtifact::CoverageCrossDimensionReduce {
                synthesis_node_id,
                finding,
                ..
            }
            | HypothesisCriticArtifact::CoverageGlobalSemanticRoot {
                synthesis_node_id,
                finding,
                ..
            } => {
                let (census_id, node_kind): (Uuid, String) = sqlx::query_as(
                    r#"SELECT synthesis_census_id,node_kind
                         FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
                        WHERE synthesis_node_id=$1"#,
                )
                .bind(synthesis_node_id)
                .fetch_one(self.pool.as_ref())
                .await?;
                let outcome = match finding.outcome {
                    CandidateCriticOutcome::NoMiss => "no_composite_miss",
                    CandidateCriticOutcome::MissedHypothesis => "missed_hypothesis",
                    CandidateCriticOutcome::Blocked => "blocked",
                };
                golish_db::repo::candidate_analysis::record_hypothesis_coverage_synthesis_review(
                    self.pool.as_ref(),
                    golish_db::repo::candidate_analysis::RecordCoverageSynthesisReviewInput {
                        fence: fence.clone(),
                        stable_review_request_id: attempt.provider_attempt_id,
                        synthesis_census_id: census_id,
                        synthesis_census_member_id: *synthesis_node_id,
                        node_kind,
                        outcome: outcome.to_owned(),
                        missed_proposal_ids: finding.missed_hypothesis_refs.clone(),
                        blocker_codes: finding.blocker_codes.clone(),
                        semantic_summary: serde_json::to_value(&finding.semantic_summary)?,
                        review_notes: "typed Candidate synthesis".to_owned(),
                        provider_attempt_id: Some(attempt.provider_attempt_id),
                        provider_artifact_body: Some(provider_artifact_body.clone()),
                    },
                )
                .await
                .map_err(|error| {
                    anyhow::anyhow!(
                        "Candidate synthesis {} persistence failed: {error}",
                        synthesis_node_id
                    )
                })?;
                "hypothesis_coverage_synthesis.v1"
            }
        };
        if artifact_kind.starts_with("hypothesis_coverage_") {
            let receipt = runtime_db::load_artifact_receipt_by_provider_attempt(
                self.pool.as_ref(),
                attempt.provider_attempt_id,
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("CANDIDATE_ATOMIC_ARTIFACT_RECEIPT_MISSING"))?;
            return Ok(CandidateArtifactPersistence::Committed(runtime_receipt(
                receipt,
            )));
        }
        let receipt = runtime_db::persist_candidate_worker_artifact(
            self.pool.as_ref(),
            runtime_db::PersistCandidateWorkerArtifact {
                fence,
                provider_attempt_id: attempt.provider_attempt_id,
                artifact_kind: artifact_kind.to_owned(),
                artifact_body: provider_artifact_body,
            },
        )
        .await?;
        Ok(CandidateArtifactPersistence::Committed(runtime_receipt(
            receipt,
        )))
    }

    async fn load_artifact_receipt(
        &self,
        provider_attempt_id: Uuid,
    ) -> anyhow::Result<Option<CandidateArtifactReceipt>> {
        Ok(runtime_db::load_artifact_receipt_by_provider_attempt(
            self.pool.as_ref(),
            provider_attempt_id,
        )
        .await?
        .map(runtime_receipt))
    }

    async fn reduce_and_seal_critic_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
        max_coverage_subreview_work_items: usize,
    ) -> anyhow::Result<CandidateCriticReduction> {
        let terminal_ready = if let Some(terminal) =
            runtime_db::load_terminal_candidate_coverage_closure(
                self.pool.as_ref(),
                attempt.analysis_attempt_id,
            )
            .await?
        {
            match terminal {
                runtime_db::CandidateCoverageClosureRow::RetryAttempt {
                    next_attempt_ordinal,
                } => {
                    return Ok(CandidateCriticReduction::RetryAttempt {
                        next_attempt_ordinal: u32::try_from(next_attempt_ordinal)?,
                    });
                }
                runtime_db::CandidateCoverageClosureRow::Blocked { residual_hash } => {
                    return Ok(CandidateCriticReduction::Blocked { residual_hash });
                }
                ready @ runtime_db::CandidateCoverageClosureRow::Ready { .. } => Some(ready),
            }
        } else {
            None
        };
        if let Some(residual_hash) = runtime_db::load_blocked_attempt_residual(
            self.pool.as_ref(),
            attempt.analysis_attempt_id,
        )
        .await?
        {
            return Ok(CandidateCriticReduction::Blocked { residual_hash });
        }
        if runtime_db::candidate_subreview_phase_incomplete(
            self.pool.as_ref(),
            attempt.analysis_attempt_id,
        )
        .await?
        {
            return Ok(CandidateCriticReduction::MoreWork(
                self.prepare_critic_wave(attempt, max_coverage_subreview_work_items)
                    .await?,
            ));
        }
        if runtime_db::candidate_synthesis_phase_needed(
            self.pool.as_ref(),
            attempt.analysis_attempt_id,
        )
        .await?
            || runtime_db::candidate_synthesis_work_incomplete(
                self.pool.as_ref(),
                attempt.analysis_attempt_id,
            )
            .await?
        {
            let rows = runtime_db::prepare_synthesis_work_batch(
                self.pool.as_ref(),
                attempt.controller_dispatch_input.snapshot_id,
                attempt.stage_execution_id,
                i32::try_from(attempt.analysis_attempt_ordinal)?,
                8,
            )
            .await?;
            return Ok(CandidateCriticReduction::MoreWork(
                CandidateCriticWavePlan {
                    work_items: rows
                        .into_iter()
                        .map(|row| {
                            let authority = runtime_authority(row.fence)?;
                            Ok(CandidateCriticWorkItem {
                                binding: runtime_binding(
                                    &authority,
                                    CandidateAnalysisAgentRole::Critic,
                                    row.lane_ordinal,
                                )?,
                                authority,
                                input: serde_json::from_value(row.input)?,
                                replayed_receipt: row.replayed_receipt.map(runtime_receipt),
                            })
                        })
                        .collect::<anyhow::Result<_>>()?,
                    h1_census_hash: attempt
                        .controller_dispatch_input
                        .input_chunk_census_set_hash
                        .clone(),
                    terminal_blocked_residual_hash: None,
                },
            ));
        }
        let closure = if let Some(ready) = terminal_ready {
            ready
        } else {
            runtime_db::reduce_coverage_and_seal_h2(
                self.pool.as_ref(),
                attempt.controller_dispatch_input.snapshot_id,
                attempt.stage_execution_id,
                i32::try_from(attempt.analysis_attempt_ordinal)?,
            )
            .await?
        };
        match closure {
            runtime_db::CandidateCoverageClosureRow::RetryAttempt {
                next_attempt_ordinal,
            } => Ok(CandidateCriticReduction::RetryAttempt {
                next_attempt_ordinal: u32::try_from(next_attempt_ordinal)?,
            }),
            runtime_db::CandidateCoverageClosureRow::Blocked { residual_hash } => {
                Ok(CandidateCriticReduction::Blocked { residual_hash })
            }
            runtime_db::CandidateCoverageClosureRow::Ready {
                proposal_census_hash: _,
                critic_census_hash: _,
                coverage_review_set_hash: _,
            } => {
                let refreshed = runtime_db::open_or_replay_attempt_runtime(
                    self.pool.as_ref(),
                    attempt.controller_dispatch_input.snapshot_id,
                    attempt.stage_execution_id,
                    i32::try_from(attempt.analysis_attempt_ordinal)?,
                )
                .await?;
                let recipe = runtime_db::load_host_compiler_recipe(
                    self.pool.as_ref(),
                    &refreshed.controller_fence,
                )
                .await?;
                let compiled = compile_candidate_host_recipe(&recipe)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                let mutations = Value::Array(
                    compiled
                        .mutations
                        .iter()
                        .map(|mutation| {
                            let item = recipe["items"]
                                .as_array()
                                .and_then(|items| {
                                    items.iter().find(|item| {
                                        item["proposal_id"]
                                            .as_str()
                                            .and_then(|value| Uuid::parse_str(value).ok())
                                            == Some(mutation.proposal_id)
                                    })
                                })
                                .expect("compiled mutation came from recipe");
                            serde_json::json!({
                                "proposal_id":mutation.proposal_id,
                                "organization_id":mutation.organization_id,
                                "semantic_key_hash":mutation.semantic_key_hash,
                                "operator_rank":mutation.operator_rank,
                                "state":mutation.state.as_str(),
                                "proof_refs":item["proof_refs"],
                                "refutation_refs":item["refutation_refs"],
                                "generation_transition_hash":mutation.generation_transition_hash,
                                "mutation_hash":mutation.mutation_hash,
                                "route":item["route"],
                            })
                        })
                        .collect(),
                );
                let prepared = runtime_db::persist_host_compilation_material_and_prepare_final(
                    self.pool.as_ref(),
                    runtime_db::PersistHostCompilationMaterial {
                        snapshot_id: attempt.controller_dispatch_input.snapshot_id,
                        stage_execution_id: attempt.stage_execution_id,
                        attempt_ordinal: i32::try_from(attempt.analysis_attempt_ordinal)?,
                        compiler_recipe: recipe,
                        mutations,
                        mutation_count: i64::try_from(compiled.mutations.len())?,
                        mutation_set_hash: compiled.mutation_set_hash.clone(),
                        claim_component_count: i64::try_from(compiled.claim_components.len())?,
                        claim_component_set_hash: compiled.claim_component_set_hash.clone(),
                        verification_contract_count: i64::try_from(
                            compiled.verification_contracts.len(),
                        )?,
                        verification_contract_set_hash: compiled
                            .verification_contract_set_hash
                            .clone(),
                        verification_plan_count: i64::try_from(compiled.verification_plans.len())?,
                        verification_plan_set_hash: compiled.verification_plan_set_hash.clone(),
                        generation_transition_set_hash: compiled
                            .generation_transition_set_hash
                            .clone(),
                    },
                )
                .await?;
                let authority = runtime_authority(prepared.controller_fence)?;
                let controller_final_input: CandidateControllerFinalInput =
                    serde_json::from_value(prepared.controller_final_input)?;
                validate_controller_proposal_pages(
                    &controller_final_input,
                    &compiled.mutation_routes,
                )
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                let controller_final_replay: Option<(Uuid, serde_json::Value)> = sqlx::query_as(
                    r#"SELECT provider_attempt_id,artifact_body
                         FROM candidate_analysis_provider_attempts
                        WHERE stage_work_item_id=$1 AND artifact_kind='controller_decision.v1'"#,
                )
                .bind(authority.fence.work_item_id)
                .fetch_optional(self.pool.as_ref())
                .await?;
                Ok(CandidateCriticReduction::Ready(Box::new(
                    CandidateFinalizationPlan {
                        binding: runtime_binding(
                            &authority,
                            CandidateAnalysisAgentRole::Controller,
                            0,
                        )?,
                        authority,
                        input: controller_final_input,
                        claim_component_compilation: CandidateClaimComponentCompilation {
                            claim_component_count: u32::try_from(compiled.claim_components.len())?,
                            planned_component_count: u32::try_from(
                                compiled.claim_components.len(),
                            )?,
                            claim_component_set_hash: compiled.claim_component_set_hash.clone(),
                            planned_component_set_hash: compiled.claim_component_set_hash,
                            plan_scope: CandidateVerificationPlanScope::ExactOriginalClaim,
                            incomplete_residual_hash: prepared.material_hash,
                        },
                        controller_final_replay: controller_final_replay
                            .map(|(provider_attempt_id, body)| -> anyhow::Result<_> {
                                Ok(CandidateAnalysisAgentAttempt {
                                    provider_attempt_id,
                                    output: serde_json::from_value(body)?,
                                })
                            })
                            .transpose()?,
                    },
                )))
            }
        }
    }

    async fn revalidate_authority(
        &self,
        snapshot_id: Uuid,
        analysis_attempt_id: Uuid,
    ) -> anyhow::Result<CandidateAuthorityRevalidation> {
        Ok(
            match runtime_db::revalidate_candidate_runtime_authority(
                self.pool.as_ref(),
                snapshot_id,
                analysis_attempt_id,
            )
            .await?
            {
                runtime_db::CandidateAuthorityRevalidationRow::Fresh => {
                    CandidateAuthorityRevalidation::Fresh
                }
                runtime_db::CandidateAuthorityRevalidationRow::Invalidated {
                    replacement_snapshot_id,
                    residual_hash,
                } => CandidateAuthorityRevalidation::Invalidated {
                    replacement_snapshot_id,
                    residual_hash,
                },
            },
        )
    }

    async fn persist_controller_final(
        &self,
        _binding: &CandidateAnalysisAgentBinding,
        authority: &CandidateRuntimeWorkAuthority,
        attempt: &CandidateAnalysisAgentAttempt<CandidateControllerDecisionArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence> {
        let receipt = runtime_db::persist_candidate_worker_artifact(
            self.pool.as_ref(),
            runtime_db::PersistCandidateWorkerArtifact {
                fence: db_fence(authority)?,
                provider_attempt_id: attempt.provider_attempt_id,
                artifact_kind: "controller_decision.v1".to_owned(),
                artifact_body: serde_json::to_value(&attempt.output)?,
            },
        )
        .await?;
        Ok(CandidateArtifactPersistence::Committed(runtime_receipt(
            receipt,
        )))
    }

    async fn validate_controller_final_binding(
        &self,
        attempt: &CandidateRuntimeAttempt,
        finalization: &CandidateFinalizationPlan,
    ) -> anyhow::Result<()> {
        runtime_db::validate_controller_final_authority_binding(
            self.pool.as_ref(),
            &db_fence(&attempt.controller_authority)?,
            &db_fence(&finalization.authority)?,
        )
        .await?;
        Ok(())
    }

    async fn finalize_generation(
        &self,
        attempt: &CandidateRuntimeAttempt,
        finalization: &CandidateFinalizationPlan,
        final_receipt: &CandidateArtifactReceipt,
    ) -> anyhow::Result<CandidateGenerationSealOutcome> {
        let (recipe, stable_compilation_request_id, stable_apply_request_id, dispositions): (
            Value,
            Uuid,
            Uuid,
            Value,
        ) = sqlx::query_as(
            r#"SELECT compiler_recipe,stable_compilation_request_id,
                      stable_apply_request_id,input_dispositions
                 FROM candidate_analysis_host_compilation_materials
                WHERE analysis_attempt_id=$1 AND final_submitter_worker_run_id=$2"#,
        )
        .bind(attempt.analysis_attempt_id)
        .bind(finalization.authority.fence.worker_run_id)
        .fetch_one(self.pool.as_ref())
        .await?;
        let controller_body: Value = sqlx::query_scalar(
            r#"SELECT provider.artifact_body
                 FROM candidate_analysis_artifacts artifact
                 JOIN candidate_analysis_provider_attempts provider
                   ON provider.artifact_id=artifact.artifact_id
                WHERE artifact.artifact_id=$1 AND artifact.artifact_hash=$2
                  AND provider.analysis_attempt_id=$3
                  AND provider.stage_work_item_id=$4
                  AND provider.worker_run_id=$5
                  AND provider.artifact_kind='controller_decision.v1'"#,
        )
        .bind(final_receipt.artifact_id)
        .bind(&final_receipt.artifact_hash)
        .bind(attempt.analysis_attempt_id)
        .bind(finalization.authority.fence.work_item_id)
        .bind(finalization.authority.fence.worker_run_id)
        .fetch_optional(self.pool.as_ref())
        .await?
        .ok_or_else(|| anyhow::anyhow!("CANDIDATE_FINAL_CONTROLLER_RECEIPT_MISMATCH"))?;
        let controller_artifact: CandidateControllerDecisionArtifact =
            serde_json::from_value(controller_body)
                .map_err(|_| anyhow::anyhow!("CANDIDATE_FINAL_CONTROLLER_ARTIFACT_INVALID"))?;
        let compiled = compile_candidate_host_recipe(&recipe)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        validate_controller_decision_route_confirmation(
            &controller_artifact,
            &compiled.mutation_routes,
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let mut reason_codes = BTreeMap::new();
        for disposition in dispositions
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("CANDIDATE_INPUT_DISPOSITION_MATERIAL_INVALID"))?
        {
            let input_id = disposition["input_id"]
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or_else(|| anyhow::anyhow!("CANDIDATE_INPUT_DISPOSITION_MATERIAL_INVALID"))?;
            let reason = disposition["reason_code"]
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("CANDIDATE_INPUT_DISPOSITION_MATERIAL_INVALID"))?;
            reason_codes.insert(input_id, reason.to_owned());
        }
        let expected_source_head_version = recipe["expected_source_head_version"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("CANDIDATE_SOURCE_HEAD_FENCE_MISSING"))?;
        let final_fence = runtime_db::refresh_controller_final_write_fence(
            self.pool.as_ref(),
            &db_fence(&finalization.authority)?,
        )
        .await?;
        let sealed = self
            .finalizer
            .finalize(CandidateAtomicFinalizationRequest {
                fence: runtime_authority(final_fence)?.fence,
                expected_source_head_version,
                host: CandidateHostCompilationAuthority {
                    stable_compilation_request_id,
                    stable_apply_request_id,
                    mutation_set_hash: compiled.mutation_set_hash,
                    claim_component_set_hash: compiled.claim_component_set_hash,
                    verification_contract_set_hash: compiled.verification_contract_set_hash,
                    verification_plan_set_hash: compiled.verification_plan_set_hash,
                    generation_transition_set_hash: compiled.generation_transition_set_hash,
                    mutation_routes: compiled.mutation_routes,
                    input_disposition_reason_codes: reason_codes,
                },
            })
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let post_seal_route = match sealed.post_seal_route.as_str() {
            "verification_campaign_admission" => {
                CandidatePostSealRoute::VerificationCampaignAdmission
            }
            "historical_reporting_placeholder" => {
                CandidatePostSealRoute::HistoricalReportingPlaceholder
            }
            "true_zero_reporting" => CandidatePostSealRoute::TrueZeroReporting,
            _ => anyhow::bail!("CANDIDATE_POST_SEAL_ROUTE_INVALID"),
        };
        Ok(CandidateGenerationSealOutcome {
            generation_id: sealed.generation_id,
            generation_ordinal: sealed.generation_ordinal,
            generation_seal_id: sealed.generation_seal_id,
            generation_member_count: sealed.generation_member_count,
            generation_member_set_hash: sealed.generation_member_set_hash,
            generation_event_set_hash: sealed.generation_event_set_hash,
            open_obligation_set_hash: sealed.open_obligation_set_hash,
            projection_outbox_batch_id: sealed.projection_outbox_batch_id,
            projection_source_batch_seq: sealed.projection_source_batch_seq,
            projection_outbox_member_set_hash: sealed.projection_outbox_member_set_hash,
            post_seal_route,
            replayed: sealed.replayed,
        })
    }
}

pub fn live_lane_limit(input_count: usize, policy: &CandidateAnalysisTeamPolicy) -> usize {
    if input_count <= policy.single_lane_input_limit as usize {
        return 1;
    }
    let batch_size = (policy.max_inputs_per_microbatch as usize).max(1);
    let microbatches = input_count.div_ceil(batch_size);
    microbatches.clamp(
        policy.min_live_analysis_lanes as usize,
        policy.max_live_analysis_lanes as usize,
    )
}

fn critic_phase_rank(input: &CandidateCriticInput) -> u32 {
    match input {
        CandidateCriticInput::ProposalConflict { .. }
        | CandidateCriticInput::CoverageSubreview { .. } => 0,
        CandidateCriticInput::CoverageCrossChunkSynthesis { .. } => 100,
        CandidateCriticInput::CoverageCrossInputPartition { node } => 200 + node.level,
        CandidateCriticInput::CoverageCrossInputReduce { node } => 300 + node.level,
        CandidateCriticInput::CoverageCrossDimensionReduce { node } => 500 + node.level,
        CandidateCriticInput::CoverageGlobalSemanticRoot { .. } => 1_000,
    }
}

pub struct PgHypothesisAnalysisStageRuntime {
    repository: Arc<dyn HypothesisAnalysisRuntimeRepository>,
    policy: CandidateAnalysisTeamPolicy,
}

impl PgHypothesisAnalysisStageRuntime {
    pub fn new(
        repository: Arc<dyn HypothesisAnalysisRuntimeRepository>,
        policy: CandidateAnalysisTeamPolicy,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            policy.min_live_analysis_lanes >= 2
                && policy.min_live_analysis_lanes <= policy.max_live_analysis_lanes
                && policy.max_live_analysis_lanes <= 8
                && policy.single_lane_input_limit > 0
                && policy.max_inputs_per_microbatch > 0,
            "candidate live-lane policy is invalid"
        );
        anyhow::ensure!(
            policy.require_read_only_children && policy.require_tool_free_children,
            "candidate children must be read-only and tool-free"
        );
        anyhow::ensure!(
            policy.controller_role == policy.final_submitter_role,
            "candidate Controller must be the unique final submitter"
        );
        Ok(Self { repository, policy })
    }

    async fn resolve_persistence(
        &self,
        provider_attempt_id: uuid::Uuid,
        persisted: CandidateArtifactPersistence,
    ) -> anyhow::Result<CandidateArtifactReceipt> {
        match persisted {
            CandidateArtifactPersistence::Committed(receipt) => Ok(receipt),
            CandidateArtifactPersistence::ResponseLost => self
                .repository
                .load_artifact_receipt(provider_attempt_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("candidate artifact response lost before commit")),
        }
    }

    async fn run_analyst_wave(
        &self,
        runner: &dyn HypothesisAnalysisAgentRunner,
        work_items: Vec<CandidateAnalystWorkItem>,
        lane_limit: usize,
    ) -> anyhow::Result<()> {
        let repository = &self.repository;
        stream::iter(work_items)
            .map(|work| async move {
                work.binding.validate_tool_free()?;
                anyhow::ensure!(
                    work.binding.role == CandidateAnalysisAgentRole::Analyst,
                    "non-analyst binding entered analyst wave"
                );
                if work.replayed_receipt.is_some() {
                    return Ok::<(), anyhow::Error>(());
                }
                let binding = work.binding;
                let authority = work.authority;
                let attempt = runner.run_analyst(binding.clone(), work.input).await?;
                let persisted = repository
                    .persist_analyst_artifact(&binding, &authority, &attempt)
                    .await?;
                self.resolve_persistence(attempt.provider_attempt_id, persisted)
                    .await?;
                Ok::<(), anyhow::Error>(())
            })
            .buffer_unordered(lane_limit.max(1))
            .try_collect::<Vec<_>>()
            .await?;
        Ok(())
    }

    async fn run_critic_wave(
        &self,
        runner: &dyn HypothesisAnalysisAgentRunner,
        work_items: Vec<CandidateCriticWorkItem>,
        lane_limit: usize,
    ) -> anyhow::Result<()> {
        let repository = &self.repository;
        stream::iter(work_items)
            .map(|work| async move {
                work.binding.validate_tool_free()?;
                anyhow::ensure!(
                    work.binding.role == CandidateAnalysisAgentRole::Critic,
                    "non-critic binding entered critic wave"
                );
                if work.replayed_receipt.is_some() {
                    return Ok::<(), anyhow::Error>(());
                }
                let binding = work.binding;
                let authority = work.authority;
                let attempt = runner.run_critic(binding.clone(), work.input).await?;
                let persisted = repository
                    .persist_critic_artifact(&binding, &authority, &attempt)
                    .await?;
                self.resolve_persistence(attempt.provider_attempt_id, persisted)
                    .await?;
                Ok::<(), anyhow::Error>(())
            })
            .buffer_unordered(lane_limit.max(1))
            .try_collect::<Vec<_>>()
            .await?;
        Ok(())
    }
}

#[async_trait]
impl HypothesisAnalysisStageRuntime for PgHypothesisAnalysisStageRuntime {
    async fn run(
        &self,
        request: HypothesisAnalysisStageRequest,
        runner: &dyn HypothesisAnalysisAgentRunner,
    ) -> anyhow::Result<HypothesisAnalysisStageOutcome> {
        let snapshot = self.repository.freeze_snapshot(request).await?;
        if snapshot.disposition == CandidateRuntimeSnapshotDisposition::BlockedAuthorityBundle {
            return Ok(HypothesisAnalysisStageOutcome::BlockedAuthorityBundle {
                snapshot_id: snapshot.snapshot_id,
                residual_hash: snapshot.blocked_residual_hash.ok_or_else(|| {
                    anyhow::anyhow!("blocked authority snapshot lacks residual authority")
                })?,
            });
        }
        anyhow::ensure!(
            snapshot.disposition == CandidateRuntimeSnapshotDisposition::SealedReady,
            "legacy Candidate runtime cannot consume a unified residual-ready snapshot"
        );
        anyhow::ensure!(
            snapshot.blocked_residual_hash.is_none(),
            "ready snapshot carries blocked residual"
        );
        let lane_limit = live_lane_limit(snapshot.input_count as usize, &self.policy);
        let mut attempt_ordinal = 0u32;
        let mut analyst_work_item_count = 0u32;
        let mut critic_work_item_count = 0u32;
        loop {
            anyhow::ensure!(
                attempt_ordinal < self.policy.max_analysis_attempts,
                "candidate analysis attempt budget exhausted"
            );
            let attempt = self
                .repository
                .open_attempt(
                    snapshot.snapshot_id,
                    attempt_ordinal,
                    snapshot.stage_execution_id,
                )
                .await?;
            anyhow::ensure!(
                attempt.analysis_attempt_ordinal == attempt_ordinal,
                "candidate repository opened a non-contiguous attempt"
            );
            attempt.controller_binding.validate_tool_free()?;
            anyhow::ensure!(
                attempt.controller_binding.role == CandidateAnalysisAgentRole::Controller,
                "candidate dispatch is not owned by Controller"
            );
            let dispatch = if let Some(replayed) = attempt.controller_dispatch_replay.clone() {
                replayed
            } else {
                let produced = runner
                    .run_controller_dispatch(
                        attempt.controller_binding.clone(),
                        attempt.controller_dispatch_input.clone(),
                    )
                    .await?;
                self.repository
                    .persist_controller_dispatch(
                        &attempt.controller_binding,
                        &attempt.controller_authority,
                        &produced,
                    )
                    .await?;
                produced
            };
            let mut analyst_workers = BTreeSet::new();
            loop {
                let analyst_work = self
                    .repository
                    .prepare_analyst_wave(&attempt, &dispatch.output, lane_limit)
                    .await?;
                if analyst_work.is_empty() {
                    break;
                }
                let wave_workers = analyst_work
                    .iter()
                    .map(|work| work.binding.worker_run_id)
                    .collect::<BTreeSet<_>>();
                anyhow::ensure!(
                    wave_workers.len() == analyst_work.len()
                        && !wave_workers.contains(&attempt.controller_binding.worker_run_id)
                        && analyst_workers.is_disjoint(&wave_workers),
                    "candidate analyst ownership is not mutually exclusive"
                );
                analyst_workers.extend(&wave_workers);
                analyst_work_item_count = analyst_work_item_count
                    .checked_add(analyst_work.len() as u32)
                    .ok_or_else(|| anyhow::anyhow!("candidate analyst work count overflow"))?;
                self.run_analyst_wave(runner, analyst_work, lane_limit)
                    .await?;
            }

            // H1 is sealed by the repository before it returns any critic
            // work.  This makes every subreview consume an immutable H1 set.
            let mut critic_plan = self
                .repository
                .prepare_critic_wave(
                    &attempt,
                    usize::try_from(self.policy.max_coverage_subreview_work_items)?,
                )
                .await?;
            if let Some(residual_hash) = critic_plan.terminal_blocked_residual_hash.take() {
                return Ok(HypothesisAnalysisStageOutcome::BlockedAnalysis {
                    snapshot_id: snapshot.snapshot_id,
                    residual_hash,
                });
            }
            let reduction = loop {
                let critic_workers = critic_plan
                    .work_items
                    .iter()
                    .map(|work| work.binding.worker_run_id)
                    .collect::<BTreeSet<_>>();
                anyhow::ensure!(
                    critic_workers.len() == critic_plan.work_items.len()
                        && analyst_workers.is_disjoint(&critic_workers)
                        && !critic_workers.contains(&attempt.controller_binding.worker_run_id),
                    "candidate critic worker separation is invalid"
                );
                critic_work_item_count = critic_work_item_count
                    .checked_add(critic_plan.work_items.len() as u32)
                    .ok_or_else(|| anyhow::anyhow!("candidate critic work count overflow"))?;
                let mut critic_phases = BTreeMap::<u32, Vec<CandidateCriticWorkItem>>::new();
                for work in critic_plan.work_items {
                    critic_phases
                        .entry(critic_phase_rank(&work.input))
                        .or_default()
                        .push(work);
                }
                for work_items in critic_phases.into_values() {
                    self.run_critic_wave(runner, work_items, lane_limit).await?;
                }
                match self
                    .repository
                    .reduce_and_seal_critic_wave(
                        &attempt,
                        usize::try_from(self.policy.max_coverage_subreview_work_items)?,
                    )
                    .await?
                {
                    CandidateCriticReduction::MoreWork(next) => critic_plan = next,
                    terminal => break terminal,
                }
            };
            match reduction {
                CandidateCriticReduction::MoreWork(_) => {
                    unreachable!("critic loop removes MoreWork")
                }
                CandidateCriticReduction::RetryAttempt {
                    next_attempt_ordinal,
                } => {
                    anyhow::ensure!(
                        next_attempt_ordinal == attempt_ordinal + 1,
                        "candidate attempt chain is not contiguous"
                    );
                    attempt_ordinal = next_attempt_ordinal;
                }
                CandidateCriticReduction::Blocked { residual_hash } => {
                    return Ok(HypothesisAnalysisStageOutcome::BlockedAnalysis {
                        snapshot_id: snapshot.snapshot_id,
                        residual_hash,
                    });
                }
                CandidateCriticReduction::Ready(finalization) => {
                    let compilation = &finalization.claim_component_compilation;
                    anyhow::ensure!(
                        compilation.planned_component_count <= compilation.claim_component_count,
                        "candidate verification plan covers unknown claim components"
                    );
                    if compilation.plan_scope == CandidateVerificationPlanScope::ExactOriginalClaim
                        && (compilation.planned_component_count
                            != compilation.claim_component_count
                            || compilation.planned_component_set_hash
                                != compilation.claim_component_set_hash)
                    {
                        return Ok(HypothesisAnalysisStageOutcome::BlockedAnalysis {
                            snapshot_id: snapshot.snapshot_id,
                            residual_hash: compilation.incomplete_residual_hash.clone(),
                        });
                    }
                    match self
                        .repository
                        .revalidate_authority(snapshot.snapshot_id, attempt.analysis_attempt_id)
                        .await?
                    {
                        CandidateAuthorityRevalidation::Invalidated {
                            replacement_snapshot_id,
                            residual_hash,
                        } => {
                            return Ok(HypothesisAnalysisStageOutcome::AuthorityInvalidated {
                                snapshot_id: snapshot.snapshot_id,
                                replacement_snapshot_id,
                                residual_hash,
                            });
                        }
                        CandidateAuthorityRevalidation::Fresh => {}
                    }
                    finalization.binding.validate_tool_free()?;
                    anyhow::ensure!(
                        finalization.binding.role == CandidateAnalysisAgentRole::Controller
                            && finalization.binding.analysis_attempt_id
                                == attempt.analysis_attempt_id
                            && finalization.binding.analysis_attempt_ordinal
                                == attempt.analysis_attempt_ordinal,
                        "candidate final submitter is not the unique Controller"
                    );
                    self.repository
                        .validate_controller_final_binding(&attempt, &finalization)
                        .await?;
                    let final_attempt =
                        if let Some(replayed) = finalization.controller_final_replay.clone() {
                            replayed
                        } else {
                            runner
                                .run_controller_final(
                                    finalization.binding.clone(),
                                    finalization.input.clone(),
                                )
                                .await?
                        };
                    let persisted = self
                        .repository
                        .persist_controller_final(
                            &finalization.binding,
                            &finalization.authority,
                            &final_attempt,
                        )
                        .await?;
                    let final_receipt = self
                        .resolve_persistence(final_attempt.provider_attempt_id, persisted)
                        .await?;
                    let generation = self
                        .repository
                        .finalize_generation(&attempt, &finalization, &final_receipt)
                        .await?;
                    return Ok(HypothesisAnalysisStageOutcome::AnalysisArtifactsReady {
                        snapshot_id: snapshot.snapshot_id,
                        analysis_attempt_id: attempt.analysis_attempt_id,
                        analysis_attempt_ordinal: attempt_ordinal,
                        analyst_work_item_count,
                        critic_work_item_count,
                        peak_live_lanes: lane_limit
                            .min((analyst_work_item_count.max(critic_work_item_count)) as usize)
                            as u32,
                        final_receipt,
                        generation,
                    });
                }
            }
        }
    }
}
