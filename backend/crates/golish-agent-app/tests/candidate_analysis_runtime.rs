#[path = "../src/ai/candidate_analysis_projection.rs"]
mod candidate_analysis_projection;
#[path = "../src/ai/candidate_analysis_runtime.rs"]
mod candidate_analysis_runtime;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use candidate_analysis_projection::{
    project_sealed_candidate_chunk, CandidateAtTimeSubject, CandidateInputProvenance,
    CandidateKnowledgeFeedEligibility, SealedCandidateChunkProjectionRow,
};
use candidate_analysis_runtime::{live_lane_limit, PgHypothesisAnalysisStageRuntime};
use golish_agent_kit::harness::stage_spec::CandidateAnalysisTeamPolicy;
use golish_agent_kit::task_orchestrator::hypothesis_analysis::*;
use golish_pentest_domain::tool_truth::ToolTruthRootFamilyV1;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn policy() -> CandidateAnalysisTeamPolicy {
    CandidateAnalysisTeamPolicy {
        schema_version: 1,
        controller_role: "candidate_hypothesis_controller".into(),
        analyst_role: "candidate_hypothesis_analyst".into(),
        critic_role: "merge_conflict_critic".into(),
        final_submitter_role: "candidate_hypothesis_controller".into(),
        min_live_analysis_lanes: 2,
        max_live_analysis_lanes: 8,
        single_lane_input_limit: 12,
        max_inputs_per_microbatch: 24,
        chunking_contract_version: 1,
        redaction_contract_version: 1,
        attack_class_checklist_contract_version: 1,
        trust_boundary_checklist_contract_version: 1,
        coverage_partition_contract_version: 1,
        coverage_synthesis_contract_version: 1,
        hypothesis_coverage_sampling_contract_version: 1,
        require_checked_tool_truth_temporal_authority: true,
        knowledge_feed_snapshot_contract_version: 1,
        product_version_match_contract_version: 1,
        max_knowledge_feed_age_seconds: 86_400,
        require_signed_knowledge_feeds: true,
        required_tool_truth_root_families: ToolTruthRootFamilyV1::ALL.to_vec(),
        max_source_bytes_per_input: 1_048_576,
        max_chunk_bytes: 16_384,
        max_chunks_per_input: 64,
        max_chunks_per_coverage_partition: 4,
        max_coverage_subreview_work_items: 4_096,
        max_synthesis_inputs_per_partition: 32,
        max_proposals_per_artifact: 16,
        max_controller_page_size: 64,
        max_attempts_per_work_item: 2,
        max_analysis_attempts: 2,
        require_read_only_children: true,
        require_tool_free_children: true,
    }
}

#[test]
fn candidate_analysis_lane_limit_is_single_then_two_to_eight() {
    let policy = policy();
    assert_eq!(live_lane_limit(0, &policy), 1);
    assert_eq!(live_lane_limit(12, &policy), 1);
    assert_eq!(live_lane_limit(13, &policy), 2);
    assert_eq!(live_lane_limit(48, &policy), 2);
    assert_eq!(live_lane_limit(300, &policy), 8);
}

fn hash(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn stable_id(label: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, label.as_bytes())
}

fn payload_digest(payload: &CandidateBoundedPayload) -> String {
    let bytes = serde_json::to_vec(payload).unwrap();
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{digest}")
}

fn projection_row(
    input_kind: CandidateInputKind,
    payload: CandidateBoundedPayload,
    knowledge_feed_eligibility: Option<CandidateKnowledgeFeedEligibility>,
) -> SealedCandidateChunkProjectionRow {
    let input_id = stable_id("projection-input");
    SealedCandidateChunkProjectionRow {
        snapshot_ready: true,
        input_id,
        expected_input_id: input_id,
        input_key: "frozen:input".into(),
        input_kind,
        knowledge_feed_eligibility,
        provenance: CandidateInputProvenance::FrozenKnowledgeFeed,
        at_time_subject: CandidateAtTimeSubject {
            kind: "product".into(),
            identity_hash: hash('a'),
        },
        source_hash: hash('b'),
        source_size: 128,
        chunk_ordinal: 0,
        expected_chunk_ordinal: 0,
        chunk_census_hash: hash('c'),
        expected_chunk_census_hash: hash('c'),
        chunking_contract_version: 1,
        expected_chunking_contract_version: 1,
        redaction_contract_version: 1,
        expected_redaction_contract_version: 1,
        persisted_payload_hash: payload_digest(&payload),
        bounded_payload: payload,
        max_chunk_bytes: 16_384,
    }
}

fn binding(
    attempt_ordinal: u32,
    role: CandidateAnalysisAgentRole,
    item: usize,
) -> CandidateAnalysisAgentBinding {
    CandidateAnalysisAgentBinding {
        analysis_attempt_id: stable_id(&format!("attempt-{attempt_ordinal}")),
        analysis_attempt_ordinal: attempt_ordinal,
        work_item_id: stable_id(&format!("work-{attempt_ordinal}-{item}-{role:?}")),
        worker_run_id: stable_id(&format!("worker-{attempt_ordinal}-{item}-{role:?}")),
        role,
        lane_ordinal: item as u32,
        read_only: true,
        allowed_tools: vec!["submit_result".into()],
    }
}

struct FakeRunner {
    active: AtomicUsize,
    peak: AtomicUsize,
    calls: AtomicUsize,
    final_calls: AtomicUsize,
    miss_first_attempt: bool,
    critic_trace: Mutex<Vec<String>>,
}

impl FakeRunner {
    fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            final_calls: AtomicUsize::new(0),
            miss_first_attempt: false,
            critic_trace: Mutex::new(Vec::new()),
        }
    }

    fn with_missed_hypothesis_on_first_attempt(mut self) -> Self {
        self.miss_first_attempt = true;
        self
    }

    async fn enter(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        tokio::task::yield_now().await;
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl HypothesisAnalysisAgentRunner for FakeRunner {
    async fn run_controller_dispatch(
        &self,
        _binding: CandidateAnalysisAgentBinding,
        _input: CandidateControllerDispatchInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDispatchPlan>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: stable_id("dispatch"),
            output: CandidateControllerDispatchPlan {
                requested_live_lanes: 99,
                requested_inputs_per_microbatch: 99,
                objective_clusters: vec!["all".into()],
            },
        })
    }

    async fn run_analyst(
        &self,
        binding: CandidateAnalysisAgentBinding,
        _input: CandidateAnalystInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisProposalArtifact>> {
        self.enter().await;
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: Uuid::new_v5(&binding.worker_run_id, b"analyst"),
            output: HypothesisProposalArtifact {
                proposals: Vec::new(),
                blocked_input_ids: Vec::new(),
                blocker_codes: Vec::new(),
            },
        })
    }

    async fn run_critic(
        &self,
        binding: CandidateAnalysisAgentBinding,
        input: CandidateCriticInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<HypothesisCriticArtifact>> {
        self.enter().await;
        let no_miss = || CandidateLocalCoverageFinding {
            outcome: CandidateCriticOutcome::NoMiss,
            missed_hypothesis_refs: Vec::new(),
            blocker_codes: Vec::new(),
            context_truncated: false,
        };
        let (trace, output) = match input {
            CandidateCriticInput::ProposalConflict {
                conflict_component_id,
                ..
            } => {
                let outcome = if self.miss_first_attempt && binding.analysis_attempt_ordinal == 0 {
                    CandidateCriticOutcome::MissedHypothesis
                } else {
                    CandidateCriticOutcome::NoMiss
                };
                (
                    "proposal_conflict".to_string(),
                    HypothesisCriticArtifact::ProposalConflict {
                        conflict_component_id,
                        outcome,
                        related_proposal_ids: Vec::new(),
                    },
                )
            }
            CandidateCriticInput::CoverageSubreview {
                subreview_census_member_id,
                checklist_member_id,
                chunk_partition_id,
                ..
            } => (
                format!("subreview:{checklist_member_id}:{chunk_partition_id}"),
                HypothesisCriticArtifact::CoverageSubreview {
                    subreview_census_member_id,
                    finding: no_miss(),
                },
            ),
            CandidateCriticInput::CoverageCrossChunkSynthesis { node } => (
                "cross_chunk".to_string(),
                HypothesisCriticArtifact::CoverageCrossChunkSynthesis {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: no_miss(),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                },
            ),
            CandidateCriticInput::CoverageCrossInputPartition { node } => (
                "cross_input_partition".to_string(),
                HypothesisCriticArtifact::CoverageCrossInputPartition {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: no_miss(),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                },
            ),
            CandidateCriticInput::CoverageCrossInputReduce { node } => (
                "cross_input_reduce".to_string(),
                HypothesisCriticArtifact::CoverageCrossInputReduce {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: no_miss(),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                },
            ),
            CandidateCriticInput::CoverageCrossDimensionReduce { node } => (
                "cross_dimension_reduce".to_string(),
                HypothesisCriticArtifact::CoverageCrossDimensionReduce {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: no_miss(),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                },
            ),
            CandidateCriticInput::CoverageGlobalSemanticRoot { node } => (
                "global_semantic_root".to_string(),
                HypothesisCriticArtifact::CoverageGlobalSemanticRoot {
                    synthesis_node_id: node.synthesis_node_id,
                    finding: no_miss(),
                    descendant_worker_set_hash: node.descendant_worker_set_hash,
                },
            ),
        };
        self.critic_trace.lock().unwrap().push(trace);
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: Uuid::new_v5(&binding.worker_run_id, b"critic"),
            output,
        })
    }

    async fn run_controller_final(
        &self,
        _binding: CandidateAnalysisAgentBinding,
        _input: CandidateControllerFinalInput,
    ) -> anyhow::Result<CandidateAnalysisAgentAttempt<CandidateControllerDecisionArtifact>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.final_calls.fetch_add(1, Ordering::SeqCst);
        Ok(CandidateAnalysisAgentAttempt {
            provider_attempt_id: stable_id("final"),
            output: CandidateControllerDecisionArtifact {
                decisions: Vec::new(),
            },
        })
    }
}

struct FakeRepository {
    input_count: u32,
    blocked: bool,
    invalidate: bool,
    claim_plan_incomplete: bool,
    coverage_shape: Option<(u32, u32)>,
    retry_once: AtomicBool,
    saw_missed_hypothesis: AtomicBool,
    response_loss_once: AtomicBool,
    receipts: Mutex<BTreeMap<Uuid, CandidateArtifactReceipt>>,
    events: Mutex<Vec<String>>,
}

impl FakeRepository {
    fn ready(input_count: u32) -> Self {
        Self {
            input_count,
            blocked: false,
            invalidate: false,
            claim_plan_incomplete: false,
            coverage_shape: None,
            retry_once: AtomicBool::new(false),
            saw_missed_hypothesis: AtomicBool::new(false),
            response_loss_once: AtomicBool::new(false),
            receipts: Mutex::new(BTreeMap::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }

    fn persist(&self, provider_attempt_id: Uuid) -> CandidateArtifactPersistence {
        let receipt = CandidateArtifactReceipt {
            artifact_id: Uuid::new_v5(&provider_attempt_id, b"artifact"),
            artifact_hash: hash('a'),
        };
        self.receipts
            .lock()
            .unwrap()
            .insert(provider_attempt_id, receipt.clone());
        if self.response_loss_once.swap(false, Ordering::SeqCst) {
            CandidateArtifactPersistence::ResponseLost
        } else {
            CandidateArtifactPersistence::Committed(receipt)
        }
    }
}

#[async_trait]
impl HypothesisAnalysisRuntimeRepository for FakeRepository {
    async fn freeze_snapshot(
        &self,
        _request: HypothesisAnalysisStageRequest,
    ) -> anyhow::Result<CandidateRuntimeSnapshot> {
        self.record("freeze");
        Ok(CandidateRuntimeSnapshot {
            snapshot_id: stable_id("snapshot"),
            disposition: if self.blocked {
                CandidateRuntimeSnapshotDisposition::BlockedAuthorityBundle
            } else {
                CandidateRuntimeSnapshotDisposition::SealedReady
            },
            input_count: self.input_count,
            snapshot_authority_hash: hash('b'),
            input_chunk_census_set_hash: hash('c'),
            blocked_residual_hash: self.blocked.then(|| hash('d')),
        })
    }

    async fn open_attempt(
        &self,
        snapshot_id: Uuid,
        attempt_ordinal: u32,
    ) -> anyhow::Result<CandidateRuntimeAttempt> {
        self.record(format!("open:{attempt_ordinal}"));
        let controller_binding =
            binding(attempt_ordinal, CandidateAnalysisAgentRole::Controller, 0);
        Ok(CandidateRuntimeAttempt {
            analysis_attempt_id: controller_binding.analysis_attempt_id,
            analysis_attempt_ordinal: attempt_ordinal,
            controller_binding,
            controller_dispatch_input: CandidateControllerDispatchInput {
                snapshot_id,
                snapshot_authority_hash: hash('b'),
                input_count: self.input_count,
                input_chunk_census_set_hash: hash('c'),
                relationship_cross_index_hash: hash('e'),
            },
        })
    }

    async fn prepare_analyst_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
        _dispatch: &CandidateControllerDispatchPlan,
        _host_lane_limit: usize,
    ) -> anyhow::Result<Vec<CandidateAnalystWorkItem>> {
        self.record("prepare_analyst");
        let count = (self.input_count as usize).div_ceil(24).max(1);
        Ok((0..count)
            .map(|ordinal| CandidateAnalystWorkItem {
                binding: binding(
                    attempt.analysis_attempt_ordinal,
                    CandidateAnalysisAgentRole::Analyst,
                    ordinal,
                ),
                input: CandidateAnalystInput {
                    microbatch_id: stable_id(&format!("microbatch-{ordinal}")),
                    microbatch_ordinal: ordinal as u32,
                    chunks: Vec::new(),
                    relationship_cross_index_hash: hash('e'),
                    trust_boundary_cross_index_hash: hash('f'),
                },
            })
            .collect())
    }

    async fn persist_analyst_artifact(
        &self,
        _binding: &CandidateAnalysisAgentBinding,
        attempt: &CandidateAnalysisAgentAttempt<HypothesisProposalArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence> {
        self.record("persist_analyst");
        Ok(self.persist(attempt.provider_attempt_id))
    }

    async fn prepare_critic_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
    ) -> anyhow::Result<CandidateCriticWavePlan> {
        self.record("seal_h1_prepare_critic");
        if let Some((checklist_count, partition_count)) = self.coverage_shape {
            let mut work_items = Vec::new();
            let mut worker_ordinal = 10_000usize;
            for checklist_ordinal in 0..checklist_count {
                for partition_ordinal in 0..partition_count {
                    let checklist_member_id = stable_id(&format!("checklist-{checklist_ordinal}"));
                    let chunk_partition_id = stable_id(&format!("partition-{partition_ordinal}"));
                    work_items.push(CandidateCriticWorkItem {
                        binding: binding(
                            attempt.analysis_attempt_ordinal,
                            CandidateAnalysisAgentRole::Critic,
                            worker_ordinal,
                        ),
                        input: CandidateCriticInput::CoverageSubreview {
                            subreview_census_id: stable_id("subreview-census"),
                            subreview_census_member_id: stable_id(&format!(
                                "subreview-{checklist_ordinal}-{partition_ordinal}"
                            )),
                            snapshot_input_id: stable_id("large-input"),
                            checklist_member_id,
                            chunk_partition_id,
                            designated_chunks: Vec::new(),
                            h1_proposal_refs: Vec::new(),
                            read_receipt_set_hash: hash('a'),
                        },
                    });
                    worker_ordinal += 1;
                }
            }
            let node = |label: &str, level: u32, partition_ordinal: u32, child_count: u32| {
                CandidateCoverageNodeInput {
                    synthesis_census_id: stable_id("synthesis-census"),
                    synthesis_node_id: stable_id(label),
                    level,
                    partition_ordinal,
                    node_hash: hash('b'),
                    child_receipt_count: child_count,
                    child_receipt_set_hash: hash('c'),
                    descendant_worker_set_hash: hash('d'),
                    relationship_cross_index_hash: hash('e'),
                }
            };
            for checklist_ordinal in 0..checklist_count {
                work_items.push(CandidateCriticWorkItem {
                    binding: binding(
                        attempt.analysis_attempt_ordinal,
                        CandidateAnalysisAgentRole::Critic,
                        worker_ordinal,
                    ),
                    input: CandidateCriticInput::CoverageCrossChunkSynthesis {
                        node: node(
                            &format!("cross-chunk-{checklist_ordinal}"),
                            0,
                            checklist_ordinal,
                            partition_count,
                        ),
                    },
                });
                worker_ordinal += 1;
            }
            let synthesis_tail = [
                CandidateCriticInput::CoverageCrossInputPartition {
                    node: node("cross-input-partition", 0, 0, checklist_count),
                },
                CandidateCriticInput::CoverageCrossInputReduce {
                    node: node("cross-input-reduce", 0, 0, 1),
                },
                CandidateCriticInput::CoverageCrossDimensionReduce {
                    node: node("cross-dimension-reduce", 0, 0, 1),
                },
                CandidateCriticInput::CoverageGlobalSemanticRoot {
                    node: node("global-semantic-root", 0, 0, 1),
                },
            ];
            for input in synthesis_tail {
                work_items.push(CandidateCriticWorkItem {
                    binding: binding(
                        attempt.analysis_attempt_ordinal,
                        CandidateAnalysisAgentRole::Critic,
                        worker_ordinal,
                    ),
                    input,
                });
                worker_ordinal += 1;
            }
            return Ok(CandidateCriticWavePlan {
                work_items,
                h1_census_hash: hash('2'),
            });
        }
        let count = (self.input_count as usize).div_ceil(16).max(1);
        Ok(CandidateCriticWavePlan {
            work_items: (0..count)
                .map(|ordinal| CandidateCriticWorkItem {
                    binding: binding(
                        attempt.analysis_attempt_ordinal,
                        CandidateAnalysisAgentRole::Critic,
                        ordinal + 10_000,
                    ),
                    input: CandidateCriticInput::ProposalConflict {
                        conflict_component_id: stable_id(&format!("conflict-{ordinal}")),
                        conflict_component_hash: hash('1'),
                        proposals: Vec::new(),
                    },
                })
                .collect(),
            h1_census_hash: hash('2'),
        })
    }

    async fn persist_critic_artifact(
        &self,
        binding: &CandidateAnalysisAgentBinding,
        attempt: &CandidateAnalysisAgentAttempt<HypothesisCriticArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence> {
        self.record("persist_critic");
        self.record(format!(
            "persist_critic_attempt:{}",
            binding.analysis_attempt_ordinal
        ));
        if attempt.output.found_miss() {
            self.saw_missed_hypothesis.store(true, Ordering::SeqCst);
        }
        Ok(self.persist(attempt.provider_attempt_id))
    }

    async fn load_artifact_receipt(
        &self,
        provider_attempt_id: Uuid,
    ) -> anyhow::Result<Option<CandidateArtifactReceipt>> {
        self.record("replay_receipt");
        Ok(self
            .receipts
            .lock()
            .unwrap()
            .get(&provider_attempt_id)
            .cloned())
    }

    async fn reduce_and_seal_critic_wave(
        &self,
        attempt: &CandidateRuntimeAttempt,
    ) -> anyhow::Result<CandidateCriticReduction> {
        self.record("reduce_seal_h2");
        if self.retry_once.swap(false, Ordering::SeqCst)
            || self.saw_missed_hypothesis.swap(false, Ordering::SeqCst)
        {
            return Ok(CandidateCriticReduction::RetryAttempt {
                next_attempt_ordinal: attempt.analysis_attempt_ordinal + 1,
            });
        }
        Ok(CandidateCriticReduction::Ready(Box::new(
            CandidateFinalizationPlan {
                binding: attempt.controller_binding.clone(),
                input: CandidateControllerFinalInput {
                    snapshot_id: stable_id("snapshot"),
                    analysis_attempt_id: attempt.analysis_attempt_id,
                    proposal_census_hash: hash('2'),
                    critic_census_hash: hash('3'),
                    coverage_review_set_hash: hash('4'),
                    claim_component_set_hash: hash('5'),
                    verification_contract_set_hash: hash('6'),
                    verification_plan_set_hash: hash('7'),
                    cluster_page_hashes: vec![hash('8')],
                },
                claim_component_compilation: CandidateClaimComponentCompilation {
                    claim_component_count: 2,
                    planned_component_count: if self.claim_plan_incomplete { 1 } else { 2 },
                    claim_component_set_hash: hash('5'),
                    planned_component_set_hash: if self.claim_plan_incomplete {
                        hash('0')
                    } else {
                        hash('5')
                    },
                    plan_scope: CandidateVerificationPlanScope::ExactOriginalClaim,
                    incomplete_residual_hash: hash('0'),
                },
            },
        )))
    }

    async fn revalidate_authority(
        &self,
        _snapshot_id: Uuid,
        _analysis_attempt_id: Uuid,
    ) -> anyhow::Result<CandidateAuthorityRevalidation> {
        self.record("revalidate");
        Ok(if self.invalidate {
            CandidateAuthorityRevalidation::Invalidated {
                replacement_snapshot_id: stable_id("replacement"),
                residual_hash: hash('9'),
            }
        } else {
            CandidateAuthorityRevalidation::Fresh
        })
    }

    async fn persist_controller_final(
        &self,
        _binding: &CandidateAnalysisAgentBinding,
        attempt: &CandidateAnalysisAgentAttempt<CandidateControllerDecisionArtifact>,
    ) -> anyhow::Result<CandidateArtifactPersistence> {
        self.record("persist_final");
        Ok(self.persist(attempt.provider_attempt_id))
    }
}

fn request() -> HypothesisAnalysisStageRequest {
    HypothesisAnalysisStageRequest {
        stable_request_id: stable_id("request"),
        operation_id: stable_id("operation"),
        scope_snapshot_id: stable_id("scope"),
        organization_id: stable_id("organization"),
    }
}

#[tokio::test]
async fn candidate_analysis_lane_rolling_300_inputs_is_not_a_lifetime_cap() {
    let repository = Arc::new(FakeRepository::ready(300));
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository.clone(), policy()).unwrap();
    let runner = FakeRunner::new();
    let outcome = runtime.run(request(), &runner).await.unwrap();
    let HypothesisAnalysisStageOutcome::AnalysisArtifactsReady {
        analyst_work_item_count,
        critic_work_item_count,
        peak_live_lanes,
        ..
    } = outcome
    else {
        panic!("expected completed runtime");
    };
    assert!(analyst_work_item_count > 8);
    assert!(critic_work_item_count > 8);
    assert_eq!(peak_live_lanes, 8);
    assert_eq!(runner.peak.load(Ordering::SeqCst), 8);
    let events = repository.events.lock().unwrap();
    let last_analyst = events
        .iter()
        .rposition(|event| event == "persist_analyst")
        .unwrap();
    let h1 = events
        .iter()
        .position(|event| event == "seal_h1_prepare_critic")
        .unwrap();
    assert!(
        last_analyst < h1,
        "H1 was sealed before the analyst wave drained"
    );
}

#[tokio::test]
async fn candidate_authority_bundle_block_never_calls_runner() {
    let repository = Arc::new(FakeRepository {
        blocked: true,
        ..FakeRepository::ready(300)
    });
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository, policy()).unwrap();
    let runner = FakeRunner::new();
    assert!(matches!(
        runtime.run(request(), &runner).await.unwrap(),
        HypothesisAnalysisStageOutcome::BlockedAuthorityBundle { .. }
    ));
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn candidate_response_loss_replays_receipt_without_provider_recall() {
    let repository = Arc::new(FakeRepository {
        response_loss_once: AtomicBool::new(true),
        ..FakeRepository::ready(12)
    });
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository.clone(), policy()).unwrap();
    let runner = FakeRunner::new();
    assert!(matches!(
        runtime.run(request(), &runner).await.unwrap(),
        HypothesisAnalysisStageOutcome::AnalysisArtifactsReady { .. }
    ));
    assert!(repository
        .events
        .lock()
        .unwrap()
        .iter()
        .any(|event| event == "replay_receipt"));
    assert_eq!(runner.final_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn candidate_temporal_recheck_blocks_old_attempt_before_final_controller() {
    let repository = Arc::new(FakeRepository {
        invalidate: true,
        ..FakeRepository::ready(24)
    });
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository, policy()).unwrap();
    let runner = FakeRunner::new();
    assert!(matches!(
        runtime.run(request(), &runner).await.unwrap(),
        HypothesisAnalysisStageOutcome::AuthorityInvalidated { .. }
    ));
    assert_eq!(runner.final_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn candidate_analysis_attempt_retry_reopens_a_contiguous_attempt_chain() {
    let repository = Arc::new(FakeRepository::ready(24));
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository.clone(), policy()).unwrap();
    let runner = FakeRunner::new().with_missed_hypothesis_on_first_attempt();
    let outcome = runtime.run(request(), &runner).await.unwrap();
    assert!(matches!(
        outcome,
        HypothesisAnalysisStageOutcome::AnalysisArtifactsReady {
            analysis_attempt_ordinal: 1,
            ..
        }
    ));
    let events = repository.events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.starts_with("open:"))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["open:0", "open:1"]
    );
    assert!(events
        .iter()
        .any(|event| event == "persist_critic_attempt:0"));
    assert!(events
        .iter()
        .any(|event| event == "persist_critic_attempt:1"));
    assert_eq!(repository.receipts.lock().unwrap().len(), 7);
    assert_eq!(runner.final_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn candidate_hypothesis_coverage_review_schema_rejects_generic_or_unknown_modes() {
    assert!(
        serde_json::from_value::<CandidateCriticInput>(serde_json::json!({
            "mode": "generic_review",
            "node": {}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<CandidateCriticInput>(serde_json::json!({
            "mode": "coverage_subreview",
            "subreview_census_id": stable_id("census"),
            "subreview_census_member_id": stable_id("member"),
            "snapshot_input_id": stable_id("input"),
            "checklist_member_id": stable_id("checklist"),
            "chunk_partition_id": stable_id("partition"),
            "designated_chunks": [],
            "h1_proposal_refs": [],
            "read_receipt_set_hash": hash('a'),
            "caller_override": true
        }))
        .is_err()
    );
    let knowledge_signal = CandidateKnowledgeSignalReference {
        feed_snapshot_id: stable_id("feed-snapshot"),
        feed_match_member_id: stable_id("feed-member"),
        feed_match_member_hash: hash('a'),
        product_version_match_hash: hash('b'),
        source_authority: CandidateKnowledgeSignalAuthority::KnowledgeSignalOnly,
    };
    assert!(serde_json::from_value::<CandidateProofReference>(
        serde_json::to_value(knowledge_signal).unwrap()
    )
    .is_err());
}

#[tokio::test]
async fn candidate_zero_proposal_special_case_still_runs_critic_and_host_finalization() {
    let repository = Arc::new(FakeRepository::ready(12));
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository, policy()).unwrap();
    let runner = FakeRunner::new();
    let outcome = runtime.run(request(), &runner).await.unwrap();
    assert!(matches!(
        outcome,
        HypothesisAnalysisStageOutcome::AnalysisArtifactsReady {
            analyst_work_item_count: 1,
            critic_work_item_count: 1,
            ..
        }
    ));
    assert_eq!(runner.final_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn candidate_hypothesis_coverage_subreview_and_recursive_synthesis_are_exact_and_ordered() {
    let repository = Arc::new(FakeRepository {
        coverage_shape: Some((2, 16)),
        ..FakeRepository::ready(1)
    });
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository, policy()).unwrap();
    let runner = FakeRunner::new();
    let outcome = runtime.run(request(), &runner).await.unwrap();
    assert!(matches!(
        outcome,
        HypothesisAnalysisStageOutcome::AnalysisArtifactsReady {
            critic_work_item_count: 38,
            peak_live_lanes: 1,
            ..
        }
    ));
    let trace = runner.critic_trace.lock().unwrap();
    let subreviews = trace
        .iter()
        .filter(|mode| mode.starts_with("subreview:"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(subreviews.len(), 2 * 16);
    assert!(trace[..32]
        .iter()
        .all(|mode| mode.starts_with("subreview:")));
    assert_eq!(&trace[32..34], ["cross_chunk", "cross_chunk"]);
    assert_eq!(
        &trace[34..],
        [
            "cross_input_partition",
            "cross_input_reduce",
            "cross_dimension_reduce",
            "global_semantic_root"
        ]
    );
    assert!(runner.peak.load(Ordering::SeqCst) <= 8);
}

#[test]
fn candidate_chunk_source_replay_uses_only_the_immutable_materialized_payload() {
    let payload = CandidateBoundedPayload::ToolTruthRecord {
        record_schema: "tool_truth_fact.v1".into(),
        redacted_fields: vec![("subject".into(), "service:443".into())],
    };
    let row = projection_row(CandidateInputKind::ToolTruthFact, payload, None);
    let first = project_sealed_candidate_chunk(row.clone()).unwrap();
    let mut unrelated_live_source = b"mutable canonical source".to_vec();
    unrelated_live_source.clear();
    drop(unrelated_live_source);
    let replay = project_sealed_candidate_chunk(row).unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.bounded_payload_hash, replay.bounded_payload_hash);
    assert_eq!(first.source_hash, replay.source_hash);
}

#[test]
fn candidate_chunk_exact_authority_rejects_ordinal_or_body_hash_tamper() {
    let payload = CandidateBoundedPayload::ToolTruthRecord {
        record_schema: "tool_truth_fact.v1".into(),
        redacted_fields: vec![("subject".into(), "service:443".into())],
    };
    let mut ordinal_tamper =
        projection_row(CandidateInputKind::ToolTruthFact, payload.clone(), None);
    ordinal_tamper.expected_chunk_ordinal = 1;
    assert!(project_sealed_candidate_chunk(ordinal_tamper).is_err());
    let mut body_tamper = projection_row(CandidateInputKind::ToolTruthFact, payload, None);
    body_tamper.persisted_payload_hash = hash('f');
    assert!(project_sealed_candidate_chunk(body_tamper).is_err());
}

#[tokio::test]
async fn candidate_hypothesis_coverage_synthesis_runs_only_after_the_subreview_census_drains() {
    let repository = Arc::new(FakeRepository {
        coverage_shape: Some((1, 2)),
        ..FakeRepository::ready(1)
    });
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository, policy()).unwrap();
    let runner = FakeRunner::new();
    assert!(matches!(
        runtime.run(request(), &runner).await.unwrap(),
        HypothesisAnalysisStageOutcome::AnalysisArtifactsReady { .. }
    ));
    let trace = runner.critic_trace.lock().unwrap();
    assert!(trace[..2].iter().all(|mode| mode.starts_with("subreview:")));
    assert_eq!(trace[2], "cross_chunk");
    assert_eq!(trace.last().unwrap(), "global_semantic_root");
}

#[test]
fn candidate_knowledge_feed_stale_or_unknown_version_is_not_projected_as_a_signal() {
    let knowledge_payload = CandidateBoundedPayload::KnowledgeFeedMatch {
        feed_snapshot_id: stable_id("feed-snapshot"),
        feed_match_member_id: stable_id("feed-member"),
        feed_kind: "cve".into(),
        feed_version: "2026.07.31".into(),
        published_at_unix_seconds: 1_785_427_200,
        content_hash: hash('a'),
        manifest_hash: hash('b'),
        provenance_hash: hash('c'),
        signature_receipt_hash: hash('d'),
        product_version_match_hash: hash('e'),
        matcher_hash: hash('f'),
        member_hash: hash('a'),
        source_authority: CandidateKnowledgeSignalAuthority::KnowledgeSignalOnly,
    };
    assert!(project_sealed_candidate_chunk(projection_row(
        CandidateInputKind::KnowledgeSignal,
        knowledge_payload.clone(),
        Some(CandidateKnowledgeFeedEligibility::CurrentKnownVersionSigned),
    ))
    .is_ok());
    for eligibility in [
        CandidateKnowledgeFeedEligibility::Stale,
        CandidateKnowledgeFeedEligibility::UnknownProductVersion,
        CandidateKnowledgeFeedEligibility::InvalidSignature,
    ] {
        assert!(project_sealed_candidate_chunk(projection_row(
            CandidateInputKind::KnowledgeSignal,
            knowledge_payload.clone(),
            Some(eligibility),
        ))
        .is_err());
    }
    let residual = CandidateBoundedPayload::ResidualOrObligation {
        reason_code: "knowledge_feed_stale_or_unknown_version".into(),
        authority_hash: hash('a'),
    };
    assert!(project_sealed_candidate_chunk(projection_row(
        CandidateInputKind::ResidualRisk,
        residual,
        Some(CandidateKnowledgeFeedEligibility::Stale),
    ))
    .is_ok());
}

#[tokio::test]
async fn candidate_claim_component_plan_cannot_seal_a_partial_wide_claim() {
    let repository = Arc::new(FakeRepository {
        claim_plan_incomplete: true,
        ..FakeRepository::ready(12)
    });
    let runtime = PgHypothesisAnalysisStageRuntime::new(repository, policy()).unwrap();
    let runner = FakeRunner::new();
    assert!(matches!(
        runtime.run(request(), &runner).await.unwrap(),
        HypothesisAnalysisStageOutcome::BlockedAnalysis { .. }
    ));
    assert_eq!(runner.final_calls.load(Ordering::SeqCst), 0);
}
