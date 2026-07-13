//! Candidate V2 manifest bridge. Model-facing code can seed observation
//! work-items and read their immutable manifest, but has no direct Candidate or
//! Finding write method.

use golish_agent_kit::db_traits::{
    AttackV2ConsolidateWave, AttackV2ReviewBarrierView, AttackV2WaveConsolidationView,
    RuntimeMemoryRepository,
};
use golish_agent_kit::harness::attack_execution::{
    CandidateManifestSnapshot, CandidateManifestWorkItem, SeedCandidateManifest,
};
use golish_db::repo::attack_candidate_work_items::{SeedAttackObservation, SeedAttackWorkItems};
use golish_db::repo::attack_waves::OpenAttackWaveUnit;

use super::GolishDbRepoProvider;

fn manifest_from_db(
    manifest: golish_db::repo::attack_candidate_work_items::CandidateManifestRow,
) -> CandidateManifestSnapshot {
    let manifest_hash =
        golish_db::repo::attack_candidate_work_items::canonical_manifest_hash(&manifest);
    CandidateManifestSnapshot {
        operation_id: manifest.operation_id,
        scope_snapshot_id: manifest.scope_snapshot_id,
        wave_run_id: manifest.wave_run_id,
        wave_unit_id: manifest.wave_unit_id,
        organization_id: manifest.organization_id,
        manifest_hash,
        work_items: manifest
            .items
            .into_iter()
            .map(|item| CandidateManifestWorkItem {
                work_item_id: item.work_item.id,
                work_item_key: item.work_item.work_item_key,
                target_live_id: item.work_item.target_live_id,
                target_type_at_time: item.work_item.target_type_at_time,
                target_value_at_time: item.work_item.target_value_at_time,
                target_identity_hash: item.work_item.target_identity_hash,
                technique: item.technique,
                evidence_ids: item.evidence_ids,
            })
            .collect(),
    }
}

impl GolishDbRepoProvider {
    pub(super) async fn attack_v2_consolidate_wave_impl(
        &self,
        input: AttackV2ConsolidateWave,
    ) -> anyhow::Result<AttackV2WaveConsolidationView> {
        let mut tx = self.pool.begin().await?;
        let result = golish_db::repo::attack_wave_consolidations::consolidate_attack_wave(
            &mut tx,
            golish_db::repo::attack_wave_consolidations::ConsolidateAttackWave {
                operation_id: input.operation_id,
                scope_snapshot_id: input.scope_snapshot_id,
                source_wave_run_id: input.source_wave_run_id,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(AttackV2WaveConsolidationView {
            operation_id: input.operation_id,
            scope_snapshot_id: input.scope_snapshot_id,
            consolidation_id: result.consolidation_id,
            source_wave_run_id: input.source_wave_run_id,
            target_wave_run_id: result.target_wave_run_id,
            decision_kind: result.decision_kind,
            accepted_fact_delta_count: result.accepted_fact_delta_ids.len(),
            rejected_fact_delta_count: result.rejected_fact_delta_ids.len(),
            residual_risk_count: result.residual_risk_ids.len(),
            replayed: result.replayed,
        })
    }

    pub(super) async fn attack_v2_verification_truth_for_operation_impl(
        &self,
        operation_id: uuid::Uuid,
        organization_id: Option<uuid::Uuid>,
    ) -> anyhow::Result<Option<golish_agent_kit::harness::attack_execution::VerificationTruthSet>>
    {
        let contract = self
            .attack_execution_contract_for_operation(operation_id)
            .await
            .map_err(anyhow::Error::new)?;
        if !contract.executes_v2_verifier() {
            return Ok(None);
        }
        let truth = golish_db::repo::verification_truth::load_for_operation(
            &self.pool,
            operation_id,
            organization_id,
        )
        .await?;
        let snapshots = truth
            .snapshots
            .into_iter()
            .map(|row| {
                Ok(
                    golish_agent_kit::harness::attack_execution::VerificationTruthSnapshot {
                        operation_id: row.operation_id,
                        scope_snapshot_id: row.scope_snapshot_id,
                        wave_run_id: row.wave_run_id,
                        wave_unit_id: row.wave_unit_id,
                        organization_id: row.organization_id,
                        review_closed: row.review_closed,
                        pending_work_items: row.pending_work_items,
                        approved_ever: row.approved_ever,
                        attempts: row
                            .attempts
                            .into_iter()
                            .map(|attempt| {
                                golish_agent_kit::harness::attack_execution::AttemptTerminalTruth {
                                    candidate_id: attempt.candidate_id,
                                    attempt_id: attempt.attempt_id,
                                    candidate_plan_hash: attempt.candidate_plan_hash,
                                    status: attempt.status,
                                    proof_evidence_ids: attempt.proof_evidence_ids,
                                    refutation_evidence_ids: attempt.refutation_evidence_ids,
                                    blocker_evidence_ids: attempt.blocker_evidence_ids,
                                    blocker_reason_code: attempt.blocker_reason_code,
                                    finding_id: attempt.finding_id,
                                    finding_lineage_exact: attempt.finding_lineage_exact,
                                }
                            })
                            .collect(),
                        residual_risks: row
                            .residual_risks
                            .into_iter()
                            .map(|risk| {
                                golish_agent_kit::harness::attack_execution::ResidualRiskTruth {
                                    residual_risk_id: risk.residual_risk_id,
                                    reason_code: risk.reason_code,
                                    disclosure_status: risk.disclosure_status,
                                }
                            })
                            .collect(),
                    },
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(
            golish_agent_kit::harness::attack_execution::VerificationTruthSet {
                authority:
                    golish_agent_kit::harness::attack_execution::VerificationTruthAuthority {
                        operation_id: truth.operation_id,
                        scope_snapshot_id: truth.scope_snapshot_id,
                        wave_run_id: truth.wave_run_id,
                        expected_units: truth
                            .expected_units
                            .into_iter()
                            .map(|unit| {
                                golish_agent_kit::harness::attack_execution::VerificationUnitAuthority {
                                    wave_unit_id: unit.wave_unit_id,
                                    organization_id: unit.organization_id,
                                }
                            })
                            .collect(),
                    },
                snapshots,
            },
        ))
    }

    pub(super) async fn attack_v2_review_barrier_for_operation_impl(
        &self,
        operation_id: uuid::Uuid,
    ) -> anyhow::Result<AttackV2ReviewBarrierView> {
        let state = golish_db::repo::attack_candidate_approvals::review_barrier_for_operation(
            &self.pool,
            operation_id,
        )
        .await?;
        Ok(AttackV2ReviewBarrierView {
            operation_id: state.operation_id,
            wave_run_id: state.wave_run_id,
            status: state.barrier.status,
            resume_version: state.barrier.resume_version,
            wave_unit_count: usize::try_from(state.wave_unit_count)?,
            review_closed_unit_count: usize::try_from(state.review_closed_unit_count)?,
            candidate_count: usize::try_from(state.candidate_count)?,
            proposed_candidate_count: usize::try_from(state.proposed_candidate_count)?,
            dispatch_is_stale: false,
        })
    }

    pub(super) async fn attack_v2_seed_candidate_manifest_for_unit_impl(
        &self,
        operation_id: uuid::Uuid,
        stage_run_unit_id: uuid::Uuid,
        organization_id: uuid::Uuid,
    ) -> anyhow::Result<CandidateManifestSnapshot> {
        let manifest =
            golish_db::repo::attack_candidate_work_items::seed_from_final_vuln_triage_handoff(
                &self.pool,
                operation_id,
                stage_run_unit_id,
                organization_id,
            )
            .await?;
        Ok(manifest_from_db(manifest))
    }

    pub(super) async fn attack_v2_seed_candidate_manifest_impl(
        &self,
        input: SeedCandidateManifest,
    ) -> anyhow::Result<CandidateManifestSnapshot> {
        let observations = input
            .observations
            .iter()
            .map(|observation| SeedAttackObservation {
                work_item_key: observation.work_item_key.clone(),
                target_live_id: observation.target_live_id,
                target_type_at_time: observation.target_type_at_time.clone(),
                target_value_at_time: observation.target_value_at_time.clone(),
                target_identity_hash: observation.target_identity_hash.clone(),
                technique: observation.technique.clone(),
                observation: observation.observation.clone(),
                observation_hash: observation.observation_hash.clone(),
                evidence_ids: observation.evidence_ids.clone(),
            })
            .collect();
        let mut tx = self.pool.begin().await?;
        golish_db::repo::attack_waves::open_from_vuln_triage_handoff(
            &mut tx,
            &OpenAttackWaveUnit {
                wave_run_id: input.wave_run_id,
                wave_unit_id: input.wave_unit_id,
                operation_id: input.operation_id,
                scope_snapshot_id: input.scope_snapshot_id,
                organization_id: input.organization_id,
                entry_stage_execution_id: input.entry_stage_execution_id,
                entry_stage_run_unit_id: input.entry_stage_run_unit_id,
                entry_deliverable_submission_id: input.entry_deliverable_submission_id,
                generation: input.wave_generation,
                ordinal: input.wave_ordinal,
                policy_snapshot: input.policy_snapshot,
                policy_hash: input.policy_hash,
                max_waves: input.max_waves,
                max_candidates_total: input.max_candidates_total,
                max_chain_depth: input.max_chain_depth,
                max_attempts_total: input.max_attempts_total,
            },
        )
        .await?;
        golish_db::repo::attack_candidate_work_items::seed_wave_work_items(
            &mut tx,
            SeedAttackWorkItems {
                operation_id: input.operation_id,
                scope_snapshot_id: input.scope_snapshot_id,
                wave_run_id: input.wave_run_id,
                wave_unit_id: input.wave_unit_id,
                organization_id: input.organization_id,
                observations,
            },
        )
        .await?;
        tx.commit().await?;
        let manifest = golish_db::repo::attack_candidate_work_items::load_for_wave_unit(
            &self.pool,
            input.operation_id,
            input.scope_snapshot_id,
            input.wave_run_id,
            input.wave_unit_id,
            input.organization_id,
        )
        .await?;
        Ok(manifest_from_db(manifest))
    }

    pub(super) async fn attack_v2_candidate_manifest_for_unit_impl(
        &self,
        operation_id: uuid::Uuid,
        stage_run_unit_id: uuid::Uuid,
        organization_id: uuid::Uuid,
    ) -> anyhow::Result<CandidateManifestSnapshot> {
        let manifest = golish_db::repo::attack_candidate_work_items::load_for_runtime_unit(
            &self.pool,
            operation_id,
            stage_run_unit_id,
            organization_id,
        )
        .await?;
        Ok(manifest_from_db(manifest))
    }
}
