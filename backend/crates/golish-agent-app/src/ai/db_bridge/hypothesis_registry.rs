//! Trusted application-layer forwarding boundary for the Plan B registry.
//!
//! This adapter intentionally performs no identity synthesis. Every operation,
//! scope, organization, snapshot, worker, lease, attempt, and optimistic row
//! version received from the server-owned caller is forwarded unchanged to the
//! injected persistence implementation.

use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::*;
use golish_db::repo::{candidate_analysis as db, hypothesis_registry as registry};
use serde_json::Value;
use sqlx::PgPool;

fn map_db_error(error: golish_db::DbError) -> HypothesisRegistryError {
    let detail = error.to_string();
    if detail.contains("HYPOTHESIS_REGISTRY_AUTHORITY_MISMATCH") {
        HypothesisRegistryError::AuthorityMismatch(detail)
    } else if detail.contains("HYPOTHESIS_REGISTRY_ARTIFACT_KIND_FORBIDDEN") {
        HypothesisRegistryError::ArtifactKindForbidden(detail)
    } else if detail.contains("not found") || detail.contains("NOT_FOUND") {
        HypothesisRegistryError::NotFound(detail)
    } else if detail.contains("INVALID") || detail.contains("PAGE_SIZE") {
        HypothesisRegistryError::InvalidRequest(detail)
    } else if detail.contains("CONFLICT")
        || detail.contains("FENCE")
        || detail.contains("REPLAY")
        || detail.contains("CENSUS_NOT_CLOSED")
        || detail.contains("SNAPSHOT_NOT_READY")
    {
        HypothesisRegistryError::Conflict(detail)
    } else {
        HypothesisRegistryError::Storage(detail)
    }
}

fn invalid_numeric(field: &'static str) -> HypothesisRegistryError {
    HypothesisRegistryError::InvalidRequest(format!("{field} is outside the repository range"))
}

fn to_i32(value: u32, field: &'static str) -> Result<i32, HypothesisRegistryError> {
    i32::try_from(value).map_err(|_| invalid_numeric(field))
}

fn to_i64(value: u64, field: &'static str) -> Result<i64, HypothesisRegistryError> {
    i64::try_from(value).map_err(|_| invalid_numeric(field))
}

fn to_u32_i32(value: i32, field: &'static str) -> Result<u32, HypothesisRegistryError> {
    u32::try_from(value).map_err(|_| invalid_numeric(field))
}

fn to_u32_i64(value: i64, field: &'static str) -> Result<u32, HypothesisRegistryError> {
    u32::try_from(value).map_err(|_| invalid_numeric(field))
}

fn to_u64(value: i64, field: &'static str) -> Result<u64, HypothesisRegistryError> {
    u64::try_from(value).map_err(|_| invalid_numeric(field))
}

fn write_fence(
    fence: CandidateRepositoryWriteFenceV1,
) -> Result<db::CandidateWriteFenceRow, HypothesisRegistryError> {
    Ok(db::CandidateWriteFenceRow {
        operation_id: fence.operation_id,
        scope_snapshot_id: fence.scope_snapshot_id,
        organization_id: fence.organization_id,
        snapshot_id: fence.snapshot_id,
        team_plan_id: fence.team_plan_id,
        work_item_id: fence.work_item_id,
        worker_run_id: fence.worker_run_id,
        lease_token: fence.lease_token,
        lease_epoch: to_i64(fence.lease_epoch, "lease_epoch")?,
        analysis_attempt_id: fence.analysis_attempt_id,
        analysis_attempt_ordinal: to_i32(
            fence.analysis_attempt_ordinal,
            "analysis_attempt_ordinal",
        )?,
        attempt_epoch: to_i64(fence.attempt_epoch, "attempt_epoch")?,
        expected_snapshot_row_version: fence.expected_snapshot_row_version,
        expected_team_plan_row_version: fence.expected_team_plan_row_version,
        expected_work_item_row_version: fence.expected_work_item_row_version,
        expected_worker_row_version: fence.expected_worker_row_version,
        expected_attempt_row_version: fence.expected_attempt_row_version,
    })
}

fn revision_source_ref(
    source: CandidateRegistryRevisionSourceRefV1,
) -> registry::CandidateRevisionSourceRefRow {
    match source {
        CandidateRegistryRevisionSourceRefV1::ToolTruthEvidence(id) => {
            registry::CandidateRevisionSourceRefRow::ToolTruthEvidence(id)
        }
        CandidateRegistryRevisionSourceRefV1::Finding(id) => {
            registry::CandidateRevisionSourceRefRow::Finding(id)
        }
        CandidateRegistryRevisionSourceRefV1::VerificationReceipt(id) => {
            registry::CandidateRevisionSourceRefRow::VerificationReceipt(id)
        }
        CandidateRegistryRevisionSourceRefV1::ApplicationContext(id) => {
            registry::CandidateRevisionSourceRefRow::ApplicationContext(id)
        }
        CandidateRegistryRevisionSourceRefV1::KnowledgeSignal(id) => {
            registry::CandidateRevisionSourceRefRow::KnowledgeSignal(id)
        }
        CandidateRegistryRevisionSourceRefV1::Gap(id) => {
            registry::CandidateRevisionSourceRefRow::Gap(id)
        }
    }
}

fn snapshot_disposition(
    value: db::CandidateSnapshotDispositionRow,
) -> CandidateAnalysisSnapshotDispositionV1 {
    match value {
        db::CandidateSnapshotDispositionRow::SealedReady => {
            CandidateAnalysisSnapshotDispositionV1::SealedReady
        }
        db::CandidateSnapshotDispositionRow::BlockedAuthorityBundle => {
            CandidateAnalysisSnapshotDispositionV1::BlockedAuthorityBundle
        }
    }
}

fn semantic_status(
    value: &str,
) -> Result<CandidateSemanticAuthorityStatusV1, HypothesisRegistryError> {
    match value {
        "consistent" => Ok(CandidateSemanticAuthorityStatusV1::Consistent),
        "pending" => Ok(CandidateSemanticAuthorityStatusV1::Pending),
        "orphaned" => Ok(CandidateSemanticAuthorityStatusV1::Orphaned),
        "superseded" => Ok(CandidateSemanticAuthorityStatusV1::Superseded),
        _ => Err(HypothesisRegistryError::AuthorityMismatch(format!(
            "unknown persisted semantic authority status: {value}"
        ))),
    }
}

fn snapshot_view(
    value: db::CandidateSnapshotRowView,
) -> Result<CandidateAnalysisSnapshotView, HypothesisRegistryError> {
    let authority_roots = value
        .authority_roots
        .into_iter()
        .map(|root| {
            Ok(CandidateToolTruthAuthorityRootViewV1 {
                ordinal: to_u32_i32(root.ordinal, "authority_root.ordinal")?,
                root_family: root.root_family,
                root_denominator_id: root.root_denominator_id,
                root_denominator_hash: root.root_denominator_hash,
                authority_set_seal_id: root.authority_set_seal_id,
                authority_set_graph_hash: root.authority_set_graph_hash,
                authority_set_semantic_hash: root.authority_set_semantic_hash,
                authority_set_freshness_hash: root.authority_set_freshness_hash,
                temporal_validity_policy_set_hash: root.temporal_validity_policy_set_hash,
                temporal_validity_decision_set_hash: root.temporal_validity_decision_set_hash,
                target_state_epoch_set_hash: root.target_state_epoch_set_hash,
                receipt_count: to_u32_i64(root.receipt_count, "authority_root.receipt_count")?,
                receipt_set_hash: root.receipt_set_hash,
                semantic_status: semantic_status(&root.semantic_status)?,
                temporal_status: root.temporal_status,
                temporal_policies: root.temporal_policies,
                member_hash: root.member_hash,
            })
        })
        .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;

    Ok(CandidateAnalysisSnapshotView {
        snapshot_id: value.snapshot_id,
        stable_consumer_request_id: value.stable_consumer_request_id,
        operation_id: value.operation_id,
        scope_snapshot_id: value.scope_snapshot_id,
        organization_id: value.organization_id,
        disposition: snapshot_disposition(value.disposition),
        snapshot_hash: value.snapshot_hash,
        candidate_snapshot_authority_hash: value.candidate_snapshot_authority_hash,
        tool_truth_authority_bundle_seal_id: value.tool_truth_authority_bundle_seal_id,
        tool_truth_authority_root_count: to_u32_i64(
            value.tool_truth_authority_root_count,
            "tool_truth_authority_root_count",
        )?,
        tool_truth_authority_root_set_hash: value.tool_truth_authority_root_set_hash,
        tool_truth_authority_bundle_member_count: to_u32_i64(
            value.tool_truth_authority_bundle_member_count,
            "tool_truth_authority_bundle_member_count",
        )?,
        tool_truth_authority_bundle_member_set_hash: value
            .tool_truth_authority_bundle_member_set_hash,
        tool_truth_authority_receipt_count: to_u32_i64(
            value.tool_truth_authority_receipt_count,
            "tool_truth_authority_receipt_count",
        )?,
        tool_truth_authority_receipt_set_hash: value.tool_truth_authority_receipt_set_hash,
        denominator_graph_bundle_hash: value.denominator_graph_bundle_hash,
        semantic_authority_bundle_hash: value.semantic_authority_bundle_hash,
        freshness_attestation_bundle_hash: value.freshness_attestation_bundle_hash,
        temporal_validity_bundle_hash: value.temporal_validity_bundle_hash,
        temporal_validity_policy_set_hash: value.temporal_validity_policy_set_hash,
        temporal_validity_decision_set_hash: value.temporal_validity_decision_set_hash,
        observation_window_hash: value.observation_window_hash,
        target_state_epoch_set_hash: value.target_state_epoch_set_hash,
        authority_roots,
        knowledge_feed_catalog_policy_seal_hash: value.knowledge_feed_catalog_policy_seal_hash,
        knowledge_feed_required_member_set_hash: value.knowledge_feed_required_member_set_hash,
        knowledge_feed_signature_algorithm_set_hash: value
            .knowledge_feed_signature_algorithm_set_hash,
        knowledge_feed_trust_store_hash: value.knowledge_feed_trust_store_hash,
        knowledge_feed_key_revocation_epoch_hash: value.knowledge_feed_key_revocation_epoch_hash,
        knowledge_feed_snapshot_set_hash: value.knowledge_feed_snapshot_set_hash,
        product_version_census_hash: value.product_version_census_hash,
        knowledge_feed_match_census_hash: value.knowledge_feed_match_census_hash,
        stale_revalidation_obligation_set_hash: value.stale_revalidation_obligation_set_hash,
        knowledge_feed_obligation_set_hash: value.knowledge_feed_obligation_set_hash,
        row_version: value.row_version,
        sealed_at: value.sealed_at,
    })
}

fn snapshot_input_kind(
    value: &str,
) -> Result<CandidateSnapshotInputKindV1, HypothesisRegistryError> {
    match value {
        "tool_truth_fact" => Ok(CandidateSnapshotInputKindV1::ToolTruthFact),
        "tool_truth_observation" => Ok(CandidateSnapshotInputKindV1::ToolTruthObservation),
        "tool_truth_evidence" | "tool_truth_bundle" => {
            Ok(CandidateSnapshotInputKindV1::ToolTruthEvidence)
        }
        "technique_outcome" => Ok(CandidateSnapshotInputKindV1::TechniqueOutcome),
        "knowledge_signal" | "managed_knowledge_feed" => {
            Ok(CandidateSnapshotInputKindV1::KnowledgeSignal)
        }
        "previous_generation" => Ok(CandidateSnapshotInputKindV1::PreviousGeneration),
        "fact_delta"
        | "expected_fact_deltas"
        | "unconsumed_fact_deltas"
        | "consumed_fact_deltas" => Ok(CandidateSnapshotInputKindV1::FactDelta),
        "relation" | "relations" => Ok(CandidateSnapshotInputKindV1::Relation),
        "residual_risk" | "state_events" => Ok(CandidateSnapshotInputKindV1::ResidualRisk),
        "open_obligation" | "open_obligations" => Ok(CandidateSnapshotInputKindV1::OpenObligation),
        _ => Err(HypothesisRegistryError::AuthorityMismatch(format!(
            "unknown persisted candidate input kind: {value}"
        ))),
    }
}

fn canonical_body(
    schema: String,
    value: &Value,
    persisted_hash: String,
) -> Result<CandidateSnapshotReadBodyV1, HypothesisRegistryError> {
    let body = serde_json::to_string(value)
        .map_err(|error| HypothesisRegistryError::Storage(error.to_string()))?;
    Ok(CandidateSnapshotReadBodyV1::CanonicalRedactedText {
        schema,
        schema_version: 1,
        body,
        body_hash: persisted_hash,
    })
}

fn artifact_kind(value: &str) -> Result<CandidateAnalysisArtifactKindV1, HypothesisRegistryError> {
    match value {
        "hypothesis_proposal.v1" => Ok(CandidateAnalysisArtifactKindV1::HypothesisProposal),
        "proposal_conflict_review.v1" => {
            Ok(CandidateAnalysisArtifactKindV1::ProposalConflictReview)
        }
        "controller_decision.v1" => Ok(CandidateAnalysisArtifactKindV1::ControllerDecision),
        "hypothesis_coverage_subreview.v1" => {
            Ok(CandidateAnalysisArtifactKindV1::HypothesisCoverageSubreview)
        }
        "hypothesis_coverage_synthesis.v1" => {
            Ok(CandidateAnalysisArtifactKindV1::HypothesisCoverageSynthesis)
        }
        "hypothesis_coverage_review.v1" => {
            Ok(CandidateAnalysisArtifactKindV1::HypothesisCoverageReview)
        }
        _ => Err(HypothesisRegistryError::AuthorityMismatch(format!(
            "unknown persisted analysis artifact kind: {value}"
        ))),
    }
}

/// Production adapter. Its method implementation is kept in this app layer so
/// `golish-db` never needs a reverse dependency on `golish-agent-kit`.
#[derive(Clone)]
pub struct PgHypothesisRegistryRepository {
    pool: Arc<PgPool>,
}

impl PgHypothesisRegistryRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HypothesisRegistryRepository for PgHypothesisRegistryRepository {
    async fn freeze_candidate_snapshot(
        &self,
        request: FreezeCandidateAnalysisSnapshot,
    ) -> Result<CandidateAnalysisSnapshotView, HypothesisRegistryError> {
        let row = db::freeze_candidate_snapshot(
            self.pool.as_ref(),
            db::FreezeCandidateSnapshotInput {
                stable_consumer_request_id: request.stable_consumer_request_id,
                operation_id: request.operation_id,
                scope_snapshot_id: request.scope_snapshot_id,
                organization_id: request.organization_id,
            },
        )
        .await
        .map_err(map_db_error)?;
        snapshot_view(row)
    }

    async fn load_snapshot_page(
        &self,
        request: LoadCandidateAnalysisPage,
    ) -> Result<CandidateAnalysisPageView, HypothesisRegistryError> {
        let row = db::load_snapshot_page(
            self.pool.as_ref(),
            db::LoadSnapshotPageInput {
                fence: write_fence(request.fence)?,
                stable_page_request_id: request.stable_page_request_id,
                after_input_ordinal: request
                    .after_input_ordinal
                    .map(|value| to_i32(value, "after_input_ordinal"))
                    .transpose()?,
                page_size: to_i32(request.page_size, "page_size")?,
            },
        )
        .await
        .map_err(map_db_error)?;

        let items = row
            .items
            .into_iter()
            .map(|item| {
                let input_kind = snapshot_input_kind(&item.input_kind)?;
                Ok(CandidateAnalysisPageItemV1 {
                    input_id: item.input_id,
                    ordinal: to_u32_i32(item.ordinal, "page_item.ordinal")?,
                    input_kind,
                    stable_key: item.stable_key,
                    source_hash: item.source_hash,
                    source_size_bytes: to_u64(
                        item.source_size_bytes,
                        "page_item.source_size_bytes",
                    )?,
                    body: canonical_body(
                        "candidate_snapshot_input_descriptor.v1".to_owned(),
                        &item.body,
                        item.body_hash,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;

        Ok(CandidateAnalysisPageView {
            snapshot_id: row.snapshot_id,
            page_receipt_id: row.page_receipt_id,
            first_input_ordinal: row
                .first_input_ordinal
                .map(|value| to_u32_i32(value, "first_input_ordinal"))
                .transpose()?,
            last_input_ordinal: row
                .last_input_ordinal
                .map(|value| to_u32_i32(value, "last_input_ordinal"))
                .transpose()?,
            returned_count: to_u32_i64(row.returned_count, "returned_count")?,
            page_hash: row.page_hash,
            items,
            next_input_ordinal: row
                .next_input_ordinal
                .map(|value| to_u32_i32(value, "next_input_ordinal"))
                .transpose()?,
            replayed: row.replayed,
        })
    }

    async fn load_snapshot_chunk_page(
        &self,
        request: LoadCandidateInputChunkPage,
    ) -> Result<CandidateInputChunkPageView, HypothesisRegistryError> {
        let row = db::load_snapshot_chunk_page(
            self.pool.as_ref(),
            db::LoadSnapshotChunkPageInput {
                fence: write_fence(request.fence)?,
                stable_page_request_id: request.stable_page_request_id,
                input_id: request.input_id,
                chunk_census_id: request.chunk_census_id,
                chunk_census_hash: request.chunk_census_hash,
                source_size_bytes: to_i64(request.source_size_bytes, "source_size_bytes")?,
                chunking_contract_version: request.chunking_contract_version.to_string(),
                redaction_contract_version: request.redaction_contract_version.to_string(),
                first_chunk_ordinal: to_i32(request.first_chunk_ordinal, "first_chunk_ordinal")?,
                max_chunks: to_i32(request.max_chunks, "max_chunks")?,
            },
        )
        .await
        .map_err(map_db_error)?;

        let chunking_contract_version =
            row.chunking_contract_version.parse::<u32>().map_err(|_| {
                HypothesisRegistryError::AuthorityMismatch(
                    "persisted chunking contract version is not numeric".to_owned(),
                )
            })?;
        let redaction_contract_version =
            row.redaction_contract_version.parse::<u32>().map_err(|_| {
                HypothesisRegistryError::AuthorityMismatch(
                    "persisted redaction contract version is not numeric".to_owned(),
                )
            })?;
        let chunks = row
            .chunks
            .into_iter()
            .map(|chunk| {
                let chunk_hash = chunk.chunk_hash;
                Ok(CandidateInputChunkViewV1 {
                    chunk_id: chunk.chunk_id,
                    chunk_ordinal: to_u32_i32(chunk.chunk_ordinal, "chunk_ordinal")?,
                    source_range_start: to_u64(chunk.source_range_start, "source_range_start")?,
                    source_range_end: to_u64(chunk.source_range_end, "source_range_end")?,
                    chunk_hash: chunk_hash.clone(),
                    body: canonical_body(
                        "candidate_snapshot_chunk.v1".to_owned(),
                        &chunk.body,
                        chunk.body_hash,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;

        Ok(CandidateInputChunkPageView {
            snapshot_id: row.snapshot_id,
            input_id: row.input_id,
            chunk_census_id: row.chunk_census_id,
            chunk_census_hash: row.chunk_census_hash,
            source_size_bytes: to_u64(row.source_size_bytes, "source_size_bytes")?,
            chunking_contract_version,
            redaction_contract_version,
            page_receipt_id: row.page_receipt_id,
            first_chunk_ordinal: row
                .first_chunk_ordinal
                .map(|value| to_u32_i32(value, "first_chunk_ordinal"))
                .transpose()?,
            last_chunk_ordinal: row
                .last_chunk_ordinal
                .map(|value| to_u32_i32(value, "last_chunk_ordinal"))
                .transpose()?,
            returned_count: to_u32_i64(row.returned_count, "returned_count")?,
            page_hash: row.page_hash,
            chunks,
            next_chunk_ordinal: row
                .next_chunk_ordinal
                .map(|value| to_u32_i32(value, "next_chunk_ordinal"))
                .transpose()?,
            replayed: row.replayed,
        })
    }

    async fn record_analysis_artifact(
        &self,
        request: RecordCandidateAnalysisArtifact,
    ) -> Result<CandidateAnalysisArtifactReceipt, HypothesisRegistryError> {
        let artifact = match request.artifact {
            CandidateAnalysisArtifactSubmissionV1::HypothesisProposal(value) => {
                db::AnalysisArtifactBodyRow::HypothesisProposal {
                    proposal_id: value.proposal_id,
                    subject_kind: value.subject_kind,
                    subject_identity_hash: value.subject_identity_hash,
                    predicate: value.predicate,
                    trust_boundary: value.trust_boundary,
                    polarity: value.polarity,
                    prose: value.prose,
                    confidence: value.confidence,
                    priority: value.priority,
                    tags: value.tags,
                    evidence_refs: value.evidence_refs,
                }
            }
            CandidateAnalysisArtifactSubmissionV1::ProposalConflictReview(value) => {
                let outcome = match value.outcome {
                    ProposalConflictReviewOutcomeV1::NoConflict => "no_conflict",
                    ProposalConflictReviewOutcomeV1::Duplicate => "duplicate",
                    ProposalConflictReviewOutcomeV1::MergeRequired => "merge_required",
                    ProposalConflictReviewOutcomeV1::SplitRequired => "split_required",
                    ProposalConflictReviewOutcomeV1::Blocked => "blocked",
                };
                db::AnalysisArtifactBodyRow::ProposalConflictReview {
                    conflict_component_id: value.conflict_component_id,
                    proposal_ids: value.proposal_ids,
                    outcome: outcome.to_owned(),
                    rationale: value.rationale,
                }
            }
            CandidateAnalysisArtifactSubmissionV1::ControllerDecision(value) => {
                let decision = match value.decision {
                    CandidateControllerDecisionKindV1::Accept => "accept",
                    CandidateControllerDecisionKindV1::Reject => "reject",
                    CandidateControllerDecisionKindV1::AttachExisting => "attach_existing",
                    CandidateControllerDecisionKindV1::Merge => "merge",
                    CandidateControllerDecisionKindV1::Split => "split",
                    CandidateControllerDecisionKindV1::NarrowSuccessor => "narrow_successor",
                    CandidateControllerDecisionKindV1::Blocked => "blocked",
                };
                db::AnalysisArtifactBodyRow::ControllerDecision {
                    decision_id: value.decision_id,
                    proposal_id: value.proposal_id,
                    decision: decision.to_owned(),
                    related_proposal_ids: value.related_proposal_ids,
                    rationale: value.rationale,
                }
            }
        };
        let row = db::record_analysis_artifact(
            self.pool.as_ref(),
            db::RecordAnalysisArtifactInput {
                fence: write_fence(request.fence)?,
                stable_artifact_request_id: request.stable_artifact_request_id,
                artifact,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(CandidateAnalysisArtifactReceipt {
            artifact_id: row.artifact_id,
            artifact_kind: artifact_kind(&row.artifact_kind)?,
            artifact_hash: row.artifact_hash,
            artifact_row_version: row.artifact_row_version,
            replayed: row.replayed,
        })
    }

    async fn seal_analysis_census(
        &self,
        request: SealCandidateAnalysisCensus,
    ) -> Result<CandidateAnalysisCensusView, HypothesisRegistryError> {
        let census_kind = match request.census_kind {
            CandidateAnalysisCensusKindV1::Proposal => db::AnalysisCensusKindRow::Proposal,
            CandidateAnalysisCensusKindV1::Critic => db::AnalysisCensusKindRow::Critic,
        };
        let row = db::seal_analysis_census(
            self.pool.as_ref(),
            db::SealAnalysisCensusInput {
                fence: write_fence(request.fence)?,
                stable_census_request_id: request.stable_census_request_id,
                census_kind,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(CandidateAnalysisCensusView {
            census_id: row.census_id,
            analysis_attempt_id: row.analysis_attempt_id,
            census_kind: match row.census_kind {
                db::AnalysisCensusKindRow::Proposal => CandidateAnalysisCensusKindV1::Proposal,
                db::AnalysisCensusKindRow::Critic => CandidateAnalysisCensusKindV1::Critic,
            },
            member_count: to_u32_i64(row.member_count, "member_count")?,
            member_set_hash: row.member_set_hash,
            census_hash: row.census_hash,
            row_version: row.row_version,
            replayed: row.replayed,
        })
    }

    async fn seal_hypothesis_coverage_subreview_census(
        &self,
        request: SealHypothesisCoverageSubreviewCensus,
    ) -> Result<HypothesisCoverageSubreviewCensusView, HypothesisRegistryError> {
        let row = db::seal_hypothesis_coverage_subreview_census(
            self.pool.as_ref(),
            db::SealCoverageSubreviewCensusInput {
                fence: write_fence(request.fence)?,
                stable_census_request_id: request.stable_census_request_id,
                input_id: request.input_id,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(HypothesisCoverageSubreviewCensusView {
            census_id: row.census_id,
            analysis_attempt_id: row.analysis_attempt_id,
            input_id: row.input_id,
            member_count: to_u32_i64(row.member_count, "member_count")?,
            member_set_hash: row.member_set_hash,
            census_hash: row.census_hash,
            row_version: row.row_version,
            replayed: row.replayed,
        })
    }

    async fn record_hypothesis_coverage_subreview(
        &self,
        request: RecordHypothesisCoverageSubreview,
    ) -> Result<HypothesisCoverageSubreviewReceipt, HypothesisRegistryError> {
        let outcome = match request.outcome {
            HypothesisCoverageSubreviewOutcomeV1::NoLocalMiss => "no_local_miss",
            HypothesisCoverageSubreviewOutcomeV1::MissedHypothesis => "missed_hypothesis",
            HypothesisCoverageSubreviewOutcomeV1::Blocked => "blocked",
        };
        let row = db::record_hypothesis_coverage_subreview(
            self.pool.as_ref(),
            db::RecordCoverageSubreviewInput {
                fence: write_fence(request.fence)?,
                stable_review_request_id: request.stable_review_request_id,
                subreview_census_id: request.subreview_census_id,
                subreview_census_member_id: request.subreview_census_member_id,
                outcome: outcome.to_owned(),
                missed_proposal_ids: request.missed_proposal_ids,
                blocker_reason: request.blocker_reason,
                review_notes: request.review_notes,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(HypothesisCoverageSubreviewReceipt {
            subreview_id: row.subreview_id,
            subreview_census_id: row.subreview_census_id,
            subreview_census_member_id: row.subreview_census_member_id,
            subreview_hash: row.subreview_hash,
            row_version: row.row_version,
            replayed: row.replayed,
        })
    }

    async fn seal_hypothesis_coverage_synthesis_census(
        &self,
        request: SealHypothesisCoverageSynthesisCensus,
    ) -> Result<HypothesisCoverageSynthesisCensusView, HypothesisRegistryError> {
        let row = db::seal_hypothesis_coverage_synthesis_census(
            self.pool.as_ref(),
            db::SealCoverageSynthesisCensusInput {
                fence: write_fence(request.fence)?,
                stable_census_request_id: request.stable_census_request_id,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(HypothesisCoverageSynthesisCensusView {
            census_id: row.census_id,
            analysis_attempt_id: row.analysis_attempt_id,
            member_count: to_u32_i64(row.member_count, "member_count")?,
            member_set_hash: row.member_set_hash,
            census_hash: row.census_hash,
            global_semantic_root_member_id: row.global_semantic_root_member_id,
            row_version: row.row_version,
            replayed: row.replayed,
        })
    }

    async fn record_hypothesis_coverage_synthesis_review(
        &self,
        request: RecordHypothesisCoverageSynthesisReview,
    ) -> Result<HypothesisCoverageSynthesisReceipt, HypothesisRegistryError> {
        let node_kind = match request.node_kind {
            HypothesisCoverageSynthesisNodeKindV1::CrossChunk => "cross_chunk",
            HypothesisCoverageSynthesisNodeKindV1::CrossInputPartition => "cross_input_partition",
            HypothesisCoverageSynthesisNodeKindV1::CrossInputReduce => "cross_input_reduce",
            HypothesisCoverageSynthesisNodeKindV1::CrossDimensionReduce => "cross_dimension_reduce",
            HypothesisCoverageSynthesisNodeKindV1::GlobalSemanticRoot => "global_semantic_root",
        };
        let outcome = match request.outcome {
            HypothesisCoverageSynthesisOutcomeV1::NoSemanticMiss => "no_semantic_miss",
            HypothesisCoverageSynthesisOutcomeV1::MissedHypothesis => "missed_hypothesis",
            HypothesisCoverageSynthesisOutcomeV1::Blocked => "blocked",
        };
        let row = db::record_hypothesis_coverage_synthesis_review(
            self.pool.as_ref(),
            db::RecordCoverageSynthesisReviewInput {
                fence: write_fence(request.fence)?,
                stable_review_request_id: request.stable_review_request_id,
                synthesis_census_id: request.synthesis_census_id,
                synthesis_census_member_id: request.synthesis_census_member_id,
                node_kind: node_kind.to_owned(),
                outcome: outcome.to_owned(),
                missed_proposal_ids: request.missed_proposal_ids,
                blocker_reason: request.blocker_reason,
                review_notes: request.review_notes,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(HypothesisCoverageSynthesisReceipt {
            synthesis_review_id: row.synthesis_review_id,
            synthesis_census_id: row.synthesis_census_id,
            synthesis_census_member_id: row.synthesis_census_member_id,
            synthesis_hash: row.synthesis_hash,
            row_version: row.row_version,
            replayed: row.replayed,
        })
    }

    async fn reduce_hypothesis_coverage_review(
        &self,
        request: ReduceHypothesisCoverageReview,
    ) -> Result<HypothesisCoverageReviewReceipt, HypothesisRegistryError> {
        let row = db::reduce_hypothesis_coverage_review(
            self.pool.as_ref(),
            db::ReduceCoverageReviewInput {
                fence: write_fence(request.fence)?,
                stable_reduction_request_id: request.stable_reduction_request_id,
                input_id: request.input_id,
            },
        )
        .await
        .map_err(map_db_error)?;
        let outcome = match row.outcome.as_str() {
            "adequate" => HypothesisCoverageReviewOutcomeV1::Adequate,
            "missed_hypothesis" => HypothesisCoverageReviewOutcomeV1::MissedHypothesis,
            "blocked" => HypothesisCoverageReviewOutcomeV1::Blocked,
            value => {
                return Err(HypothesisRegistryError::AuthorityMismatch(format!(
                    "unknown persisted coverage review outcome: {value}"
                )));
            }
        };
        Ok(HypothesisCoverageReviewReceipt {
            coverage_review_id: row.coverage_review_id,
            input_id: row.input_id,
            outcome,
            coverage_review_hash: row.coverage_review_hash,
            row_version: row.row_version,
            replayed: row.replayed,
        })
    }

    async fn load_candidate_gate_material(
        &self,
        request: LoadCandidateGateMaterial,
    ) -> Result<CandidateGateMaterial, HypothesisRegistryError> {
        let row = db::load_candidate_gate_material(
            self.pool.as_ref(),
            db::LoadCandidateGateMaterialInput {
                operation_id: request.operation_id,
                scope_snapshot_id: request.scope_snapshot_id,
                organization_id: request.organization_id,
                snapshot_id: request.snapshot_id,
                analysis_attempt_id: request.analysis_attempt_id,
                analysis_attempt_ordinal: to_i32(
                    request.analysis_attempt_ordinal,
                    "analysis_attempt_ordinal",
                )?,
                expected_snapshot_row_version: request.expected_snapshot_row_version,
                expected_attempt_row_version: request.expected_attempt_row_version,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(CandidateGateMaterial {
            snapshot: snapshot_view(row.snapshot)?,
            active_analysis_attempt_id: row.active_analysis_attempt_id,
            active_analysis_attempt_ordinal: to_u32_i32(
                row.active_analysis_attempt_ordinal,
                "active_analysis_attempt_ordinal",
            )?,
            attempt_epoch: to_u64(row.attempt_epoch, "attempt_epoch")?,
            prior_terminal_attempt_chain_hash: row.prior_terminal_attempt_chain_hash,
            gate_temporal_reevaluation_hash: row.gate_temporal_reevaluation_hash,
            gate_knowledge_feed_reevaluation_hash: row.gate_knowledge_feed_reevaluation_hash,
            input_chunk_census_set_hash: row.input_chunk_census_set_hash,
            proposal_census_hash: row.proposal_census_hash,
            critic_census_hash: row.critic_census_hash,
            coverage_subreview_census_set_hash: row.coverage_subreview_census_set_hash,
            coverage_synthesis_census_set_hash: row.coverage_synthesis_census_set_hash,
            coverage_global_semantic_root_hash: row.coverage_global_semantic_root_hash,
            coverage_global_review_hash: row.coverage_global_review_hash,
            coverage_review_set_hash: row.coverage_review_set_hash,
            coverage_checklist_set_hash: row.coverage_checklist_set_hash,
            controller_decision_set_hash: row.controller_decision_set_hash,
            mutation_set_hash: row.mutation_set_hash,
            claim_component_set_hash: row.claim_component_set_hash,
            verification_contract_set_hash: row.verification_contract_set_hash,
            verification_plan_set_hash: row.verification_plan_set_hash,
            generation_transition_set_hash: row.generation_transition_set_hash,
            compiler_seal_hash: row.compiler_seal_hash,
            final_submitter_worker_run_id: row.final_submitter_worker_run_id,
            snapshot_row_version: row.snapshot_row_version,
            attempt_row_version: row.attempt_row_version,
        })
    }

    async fn seal_candidate_compilation(
        &self,
        request: SealCandidateCompilation,
    ) -> Result<CandidateCompilationSealView, HypothesisRegistryError> {
        let row = db::seal_candidate_compilation(
            self.pool.as_ref(),
            db::SealCandidateCompilationInput {
                fence: write_fence(request.fence)?,
                stable_compilation_request_id: request.stable_compilation_request_id,
                mutation_set_hash: request.mutation_set_hash,
                claim_component_set_hash: request.claim_component_set_hash,
                verification_contract_set_hash: request.verification_contract_set_hash,
                verification_plan_set_hash: request.verification_plan_set_hash,
                generation_transition_set_hash: request.generation_transition_set_hash,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(CandidateCompilationSealView {
            compilation_seal_id: row.compilation_seal_id,
            mutation_set_hash: row.mutation_set_hash,
            claim_component_set_hash: row.claim_component_set_hash,
            verification_contract_set_hash: row.verification_contract_set_hash,
            verification_plan_set_hash: row.verification_plan_set_hash,
            generation_transition_set_hash: row.generation_transition_set_hash,
            compiler_seal_hash: row.compiler_seal_hash,
            row_version: row.row_version,
            replayed: row.replayed,
        })
    }

    async fn apply_candidate_gate_pass(
        &self,
        request: ApplyCandidateGatePass,
    ) -> Result<CandidateGenerationSealView, HypothesisRegistryError> {
        let expected = request.gate_pass.expected_authority;
        let mutations = request
            .gate_pass
            .mutation_set
            .into_iter()
            .map(|mutation| {
                let route = match mutation.decision {
                    CandidateRegistryMutationDecisionV1::AttachCurrent {
                        root_id,
                        revision_id,
                    } => registry::CandidateMutationRouteRow::AttachCurrent {
                        root_id,
                        revision_id,
                    },
                    CandidateRegistryMutationDecisionV1::NoSemanticChange {
                        root_id,
                        revision_id,
                    } => registry::CandidateMutationRouteRow::NoSemanticChange {
                        root_id,
                        revision_id,
                    },
                    CandidateRegistryMutationDecisionV1::CreateInitial { root_id } => {
                        registry::CandidateMutationRouteRow::CreateInitial {
                            root_id,
                        }
                    }
                    CandidateRegistryMutationDecisionV1::ReopenHistorical {
                        root_id,
                        predecessor_revision_id,
                    } => registry::CandidateMutationRouteRow::ReopenHistorical {
                        root_id,
                        predecessor_revision_id,
                    },
                    CandidateRegistryMutationDecisionV1::ExplicitTransitionRequired { .. } => {
                        return Err(HypothesisRegistryError::InvalidRequest(
                            "explicit-transition-required is not an applicable gate mutation"
                                .to_owned(),
                        ));
                    }
                    CandidateRegistryMutationDecisionV1::Split {
                        parent_root_id,
                        child_root_ids,
                    } => {
                        let [child_root_id] = child_root_ids.as_slice() else {
                            return Err(HypothesisRegistryError::InvalidRequest(
                                "the DB apply contract requires exactly one split child per mutation"
                                    .to_owned(),
                            ));
                        };
                        registry::CandidateMutationRouteRow::Split {
                            parent_root_id,
                            child_root_id: *child_root_id,
                        }
                    }
                    CandidateRegistryMutationDecisionV1::Merge {
                        parent_root_ids,
                        successor_root_id,
                    } => registry::CandidateMutationRouteRow::Merge {
                        parent_root_ids,
                        successor_root_id,
                    },
                    CandidateRegistryMutationDecisionV1::Derive {
                        source_root_id,
                        source_revision_id,
                        derivation_rule_hash,
                        successor_root_id,
                    } => registry::CandidateMutationRouteRow::Derive {
                        source_root_id,
                        source_revision_id,
                        derivation_rule_hash,
                        successor_root_id,
                    },
                    CandidateRegistryMutationDecisionV1::NarrowSuccessor {
                        source_root_id,
                        source_revision_id,
                        successor_root_id,
                        covered_claim_component_set_hash,
                    } => registry::CandidateMutationRouteRow::NarrowSuccessor {
                        source_root_id,
                        source_revision_id,
                        successor_root_id,
                        covered_claim_component_set_hash,
                    },
                };
                Ok(registry::CandidateMutationRow {
                    proposal_id: mutation.proposal_id,
                    organization_id: mutation.organization_id,
                    semantic_key_hash: mutation.semantic_key_hash,
                    operator_rank: mutation.operator_rank,
                    state: mutation.state,
                    proof_refs: mutation
                        .proof_refs
                        .into_iter()
                        .map(revision_source_ref)
                        .collect(),
                    refutation_refs: mutation
                        .refutation_refs
                        .into_iter()
                        .map(revision_source_ref)
                        .collect(),
                    generation_transition_hash: mutation.generation_transition_hash,
                    mutation_hash: mutation.mutation_hash,
                    route,
                })
            })
            .collect::<Result<Vec<_>, HypothesisRegistryError>>()?;
        let input_dispositions = request
            .gate_pass
            .input_dispositions
            .into_iter()
            .map(|decision| {
                let disposition = match decision.disposition {
                    InputProcessingDispositionV1::Analyzed => "analyzed",
                    InputProcessingDispositionV1::Informational => "informational",
                    InputProcessingDispositionV1::DuplicateInput => "duplicate_input",
                    InputProcessingDispositionV1::NotSecurityRelevant => "not_security_relevant",
                    InputProcessingDispositionV1::Gap => "gap",
                    InputProcessingDispositionV1::Blocked => "blocked",
                };
                registry::InputDispositionRow {
                    input_id: decision.input_id,
                    disposition: disposition.to_owned(),
                    reason_code: decision.reason_code,
                }
            })
            .collect();
        let input_relations = request
            .gate_pass
            .input_relations
            .into_iter()
            .map(|decision| {
                let relation_kind = match decision.relation {
                    InputHypothesisRelationKindV1::CreatesHypothesis => "creates_hypothesis",
                    InputHypothesisRelationKindV1::SupportsExisting => "supports_existing",
                    InputHypothesisRelationKindV1::ContradictsExisting => "contradicts_existing",
                    InputHypothesisRelationKindV1::QualifiesExisting => "qualifies_existing",
                };
                registry::InputHypothesisRelationRow {
                    input_id: decision.input_id,
                    root_id: decision.hypothesis_root_id,
                    relation_kind: relation_kind.to_owned(),
                }
            })
            .collect();
        let row = registry::apply_candidate_gate_pass(
            self.pool.as_ref(),
            registry::ApplyCandidateGatePassInput {
                fence: write_fence(request.fence)?,
                stable_apply_request_id: request.stable_apply_request_id,
                expected_authority: registry::CandidateExpectedAuthorityRow {
                    snapshot_hash: expected.snapshot_hash,
                    candidate_snapshot_authority_hash: expected.candidate_snapshot_authority_hash,
                    tool_truth_authority_bundle_seal_id: expected
                        .tool_truth_authority_bundle_seal_id,
                    tool_truth_authority_root_set_hash: expected.tool_truth_authority_root_set_hash,
                    tool_truth_authority_bundle_member_set_hash: expected
                        .tool_truth_authority_bundle_member_set_hash,
                    tool_truth_authority_receipt_set_hash: expected
                        .tool_truth_authority_receipt_set_hash,
                    denominator_graph_bundle_hash: expected.denominator_graph_bundle_hash,
                    semantic_authority_bundle_hash: expected.semantic_authority_bundle_hash,
                    freshness_attestation_bundle_hash: expected.freshness_attestation_bundle_hash,
                    temporal_validity_bundle_hash: expected.temporal_validity_bundle_hash,
                    temporal_validity_policy_set_hash: expected.temporal_validity_policy_digest,
                    temporal_validity_decision_set_hash: expected
                        .temporal_validity_decision_set_hash,
                    target_state_epoch_set_hash: expected.target_state_epoch_set_hash,
                    gate_temporal_reevaluation_hash: expected.gate_temporal_reevaluation_hash,
                    knowledge_feed_catalog_policy_seal_hash: expected
                        .knowledge_feed_catalog_policy_seal_hash,
                    knowledge_feed_required_member_set_hash: expected
                        .knowledge_feed_required_member_set_hash,
                    knowledge_feed_signature_algorithm_set_hash: expected
                        .knowledge_feed_signature_algorithm_set_hash,
                    knowledge_feed_trust_store_hash: expected.knowledge_feed_trust_store_hash,
                    knowledge_feed_key_revocation_epoch_hash: expected
                        .knowledge_feed_key_revocation_epoch_hash,
                    knowledge_feed_snapshot_set_hash: expected.knowledge_feed_snapshot_set_hash,
                    product_version_census_hash: expected.product_version_census_hash,
                    knowledge_feed_match_census_hash: expected.knowledge_feed_match_census_hash,
                    gate_knowledge_feed_reevaluation_hash: expected
                        .gate_knowledge_feed_reevaluation_hash,
                    stale_revalidation_obligation_set_hash: expected
                        .stale_revalidation_obligation_set_hash,
                    knowledge_feed_obligation_set_hash: expected.knowledge_feed_obligation_set_hash,
                    prior_terminal_attempt_chain_hash: expected.prior_terminal_attempt_chain_hash,
                    proposal_census_hash: expected.proposal_census_hash,
                    critic_census_hash: expected.critic_census_hash,
                    controller_decision_set_hash: expected.controller_decision_set_hash,
                    input_chunk_census_set_hash: expected.input_chunk_census_set_hash,
                    coverage_subreview_census_set_hash: expected.coverage_subreview_census_set_hash,
                    coverage_synthesis_census_set_hash: expected.coverage_synthesis_census_set_hash,
                    coverage_global_semantic_root_hash: expected.coverage_global_semantic_root_hash,
                    coverage_global_review_hash: expected.coverage_global_review_hash,
                    coverage_review_set_hash: expected.coverage_review_set_hash,
                    coverage_checklist_set_hash: expected.coverage_checklist_set_hash,
                    generation_transition_set_hash: expected.generation_transition_set_hash,
                },
                active_analysis_attempt_id: request.gate_pass.active_analysis_attempt_id,
                active_analysis_attempt_ordinal: to_i32(
                    request.gate_pass.active_analysis_attempt_ordinal,
                    "active_analysis_attempt_ordinal",
                )?,
                mutations,
                mutation_set_hash: request.gate_pass.mutation_set_hash,
                claim_components: request.gate_pass.hypothesis_claim_components,
                claim_component_set_hash: request.gate_pass.hypothesis_claim_component_set_hash,
                verification_contracts: request.gate_pass.verification_contracts,
                verification_contract_set_hash: request.gate_pass.verification_contract_set_hash,
                verification_plans: request.gate_pass.hypothesis_verification_plans,
                verification_plan_set_hash: request.gate_pass.hypothesis_verification_plan_set_hash,
                input_dispositions,
                input_relations,
                final_submitter_worker_run_id: request.gate_pass.final_submitter_worker_run_id,
                expected_source_head_version: request.expected_source_head_version,
            },
        )
        .await
        .map_err(map_db_error)?;
        Ok(CandidateGenerationSealView {
            operation_id: row.operation_id,
            scope_snapshot_id: row.scope_snapshot_id,
            organization_id: row.organization_id,
            snapshot_id: row.snapshot_id,
            analysis_attempt_id: row.analysis_attempt_id,
            generation_id: row.generation_id,
            generation_ordinal: to_u32_i32(row.generation_ordinal, "generation_ordinal")?,
            generation_seal_id: row.generation_seal_id,
            generation_member_count: to_u32_i64(
                row.generation_member_count,
                "generation_member_count",
            )?,
            generation_member_set_hash: row.generation_member_set_hash,
            generation_event_set_hash: row.generation_event_set_hash,
            open_obligation_set_hash: row.open_obligation_set_hash,
            projection_outbox_batch_id: row.projection_outbox_batch_id,
            projection_source_batch_seq: row.projection_source_batch_seq,
            projection_outbox_member_set_hash: row.projection_outbox_member_set_hash,
            replayed: row.replayed,
        })
    }
}

#[derive(Clone)]
pub struct HypothesisRegistryBridge {
    repository: Arc<dyn HypothesisRegistryRepository>,
}

impl HypothesisRegistryBridge {
    pub fn new(repository: Arc<dyn HypothesisRegistryRepository>) -> Self {
        Self { repository }
    }

    pub fn repository(&self) -> Arc<dyn HypothesisRegistryRepository> {
        self.repository.clone()
    }
}

#[async_trait]
impl HypothesisRegistryRepository for HypothesisRegistryBridge {
    async fn freeze_candidate_snapshot(
        &self,
        request: FreezeCandidateAnalysisSnapshot,
    ) -> Result<CandidateAnalysisSnapshotView, HypothesisRegistryError> {
        self.repository.freeze_candidate_snapshot(request).await
    }

    async fn load_snapshot_page(
        &self,
        request: LoadCandidateAnalysisPage,
    ) -> Result<CandidateAnalysisPageView, HypothesisRegistryError> {
        self.repository.load_snapshot_page(request).await
    }

    async fn load_snapshot_chunk_page(
        &self,
        request: LoadCandidateInputChunkPage,
    ) -> Result<CandidateInputChunkPageView, HypothesisRegistryError> {
        self.repository.load_snapshot_chunk_page(request).await
    }

    async fn record_analysis_artifact(
        &self,
        request: RecordCandidateAnalysisArtifact,
    ) -> Result<CandidateAnalysisArtifactReceipt, HypothesisRegistryError> {
        self.repository.record_analysis_artifact(request).await
    }

    async fn seal_analysis_census(
        &self,
        request: SealCandidateAnalysisCensus,
    ) -> Result<CandidateAnalysisCensusView, HypothesisRegistryError> {
        self.repository.seal_analysis_census(request).await
    }

    async fn seal_hypothesis_coverage_subreview_census(
        &self,
        request: SealHypothesisCoverageSubreviewCensus,
    ) -> Result<HypothesisCoverageSubreviewCensusView, HypothesisRegistryError> {
        self.repository
            .seal_hypothesis_coverage_subreview_census(request)
            .await
    }

    async fn record_hypothesis_coverage_subreview(
        &self,
        request: RecordHypothesisCoverageSubreview,
    ) -> Result<HypothesisCoverageSubreviewReceipt, HypothesisRegistryError> {
        self.repository
            .record_hypothesis_coverage_subreview(request)
            .await
    }

    async fn seal_hypothesis_coverage_synthesis_census(
        &self,
        request: SealHypothesisCoverageSynthesisCensus,
    ) -> Result<HypothesisCoverageSynthesisCensusView, HypothesisRegistryError> {
        self.repository
            .seal_hypothesis_coverage_synthesis_census(request)
            .await
    }

    async fn record_hypothesis_coverage_synthesis_review(
        &self,
        request: RecordHypothesisCoverageSynthesisReview,
    ) -> Result<HypothesisCoverageSynthesisReceipt, HypothesisRegistryError> {
        self.repository
            .record_hypothesis_coverage_synthesis_review(request)
            .await
    }

    async fn reduce_hypothesis_coverage_review(
        &self,
        request: ReduceHypothesisCoverageReview,
    ) -> Result<HypothesisCoverageReviewReceipt, HypothesisRegistryError> {
        self.repository
            .reduce_hypothesis_coverage_review(request)
            .await
    }

    async fn seal_candidate_compilation(
        &self,
        request: SealCandidateCompilation,
    ) -> Result<CandidateCompilationSealView, HypothesisRegistryError> {
        self.repository.seal_candidate_compilation(request).await
    }

    async fn load_candidate_gate_material(
        &self,
        request: LoadCandidateGateMaterial,
    ) -> Result<CandidateGateMaterial, HypothesisRegistryError> {
        self.repository.load_candidate_gate_material(request).await
    }

    async fn apply_candidate_gate_pass(
        &self,
        request: ApplyCandidateGatePass,
    ) -> Result<CandidateGenerationSealView, HypothesisRegistryError> {
        self.repository.apply_candidate_gate_pass(request).await
    }
}
