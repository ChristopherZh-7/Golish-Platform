//! Two-wave Candidate hypothesis-analysis phase machine.
//!
//! The repository owns all authority, census, work-item and retry identities;
//! this runtime only executes server-issued submit-only work under a rolling
//! concurrency cap.  `8` is a live-lane ceiling, never a lifetime item cap.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use futures::{stream, StreamExt, TryStreamExt};
use golish_agent_kit::harness::stage_spec::CandidateAnalysisTeamPolicy;
use golish_agent_kit::task_orchestrator::hypothesis_analysis::*;

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
                let binding = work.binding;
                let attempt = runner.run_analyst(binding.clone(), work.input).await?;
                let persisted = repository
                    .persist_analyst_artifact(&binding, &attempt)
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
                let binding = work.binding;
                let attempt = runner.run_critic(binding.clone(), work.input).await?;
                let persisted = repository
                    .persist_critic_artifact(&binding, &attempt)
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
                .open_attempt(snapshot.snapshot_id, attempt_ordinal)
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
            let dispatch = runner
                .run_controller_dispatch(
                    attempt.controller_binding.clone(),
                    attempt.controller_dispatch_input.clone(),
                )
                .await?;
            let analyst_work = self
                .repository
                .prepare_analyst_wave(&attempt, &dispatch.output, lane_limit)
                .await?;
            let analyst_workers = analyst_work
                .iter()
                .map(|work| work.binding.worker_run_id)
                .collect::<BTreeSet<_>>();
            anyhow::ensure!(
                analyst_workers.len() == analyst_work.len()
                    && !analyst_workers.contains(&attempt.controller_binding.worker_run_id),
                "candidate analyst ownership is not mutually exclusive"
            );
            analyst_work_item_count = analyst_work_item_count
                .checked_add(analyst_work.len() as u32)
                .ok_or_else(|| anyhow::anyhow!("candidate analyst work count overflow"))?;
            self.run_analyst_wave(runner, analyst_work, lane_limit)
                .await?;

            // H1 is sealed by the repository before it returns any critic
            // work.  This makes every subreview consume an immutable H1 set.
            let critic_plan = self.repository.prepare_critic_wave(&attempt).await?;
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
                .reduce_and_seal_critic_wave(&attempt)
                .await?
            {
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
                            && finalization.binding.worker_run_id
                                == attempt.controller_binding.worker_run_id,
                        "candidate final submitter is not the unique Controller"
                    );
                    let final_attempt = runner
                        .run_controller_final(finalization.binding.clone(), finalization.input)
                        .await?;
                    let persisted = self
                        .repository
                        .persist_controller_final(&finalization.binding, &final_attempt)
                        .await?;
                    let final_receipt = self
                        .resolve_persistence(final_attempt.provider_attempt_id, persisted)
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
                    });
                }
            }
        }
    }
}
