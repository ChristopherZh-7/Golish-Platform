//! SQL-free host boundary for unified Investigation hypothesis analysis.
//!
//! The port opens one Candidate Registry snapshot/ordinal-zero attempt for an
//! already registered Investigation analysis work item. Cognitive model output
//! remains advisory: only typed Candidate proposals and non-executable action
//! intents can cross this boundary. Canonical Registry mutations and executable
//! operator material are deliberately absent from the model-facing envelope.

use std::collections::BTreeSet;

use async_trait::async_trait;
use uuid::Uuid;

use crate::task_orchestrator::hypothesis_analysis::CandidateHypothesisProposal;

use super::{UnifiedInvestigationSubjectKind, UnifiedInvestigationUnitIdentity};

pub const INVESTIGATION_COGNITIVE_OUTPUT_SCHEMA_V1: &str = "investigation_cognitive_output.v1";

pub type InvestigationAnalysisHostResult<T> = Result<T, InvestigationAnalysisHostError>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvestigationAnalysisHostError {
    #[error("investigation_analysis_host_unavailable: {operation}")]
    Unavailable { operation: &'static str },
    #[error("investigation_analysis_host_invalid_request: {detail}")]
    InvalidRequest { detail: String },
    #[error("investigation_analysis_host_not_found: {detail}")]
    NotFound { detail: String },
    #[error("investigation_analysis_host_conflict: {detail}")]
    Conflict { detail: String },
    #[error("investigation_analysis_host_authority_mismatch: {detail}")]
    AuthorityMismatch { detail: String },
    #[error("investigation_analysis_host_snapshot_blocked: {detail}")]
    SnapshotBlocked { detail: String },
    #[error(
        "investigation_analysis_host_revalidation_required: retry_mode=tool_truth_revalidation operation_id={operation_id} revalidation_obligation_ids={revalidation_obligation_ids:?} stale_roots={stale_roots:?}"
    )]
    RevalidationRequired {
        operation_id: Uuid,
        revalidation_obligation_ids: Vec<Uuid>,
        stale_roots: Vec<String>,
    },
    #[error("investigation_analysis_host_infrastructure: {detail}")]
    Infrastructure { detail: String },
}

impl InvestigationAnalysisHostError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "investigation_analysis_host_unavailable",
            Self::InvalidRequest { .. } => "investigation_analysis_host_invalid_request",
            Self::NotFound { .. } => "investigation_analysis_host_not_found",
            Self::Conflict { .. } => "investigation_analysis_host_conflict",
            Self::AuthorityMismatch { .. } => "investigation_analysis_host_authority_mismatch",
            Self::SnapshotBlocked { .. } => "investigation_analysis_host_snapshot_blocked",
            Self::RevalidationRequired { .. } => {
                "investigation_analysis_host_revalidation_required"
            }
            Self::Infrastructure { .. } => "investigation_analysis_host_infrastructure",
        }
    }
}

/// Server-built request. The caller cannot provide snapshot, attempt, bundle,
/// hash, generation, or Candidate scheduler identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareInvestigationAnalysisSubject {
    pub stable_request_id: Uuid,
    pub identity: UnifiedInvestigationUnitIdentity,
    pub work_id: Uuid,
    /// Exact current asset lane. New Investigation analysis has no
    /// organization-wide fallback.
    pub asset_lane_id: Uuid,
    pub pending_evolution_authority_id: Option<Uuid>,
}

/// One exact immutable chunk selector belonging to a frozen Candidate input.
/// The selector is proof authority only; it grants no live-source or tool
/// access and cannot be supplied by the cognitive model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAnalysisAuthorityChunkV1 {
    pub chunk_id: Uuid,
    pub chunk_ordinal: u32,
    pub chunk_sha256: String,
}

/// Model-readable form of one server-frozen Candidate input. `body` is the
/// exact concatenation of its immutable redacted chunks. Proposal proof refs
/// must select an `input_id`, one of its `chunks`, and this `source_sha256`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAnalysisAuthorityInputV1 {
    pub input_id: Uuid,
    pub stable_input_key: String,
    pub source_kind: String,
    pub source_sha256: String,
    pub body: String,
    pub chunks: Vec<InvestigationAnalysisAuthorityChunkV1>,
}

/// One server-owned subject identity that an analysis proposal may select.
///
/// The cognitive model may choose among these identities, but it cannot mint
/// a new subject hash. `display_value` is prompt-only context (for example an
/// exact origin or endpoint URL); canonical compilation continues to bind the
/// hash and kind selected from this frozen allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAnalysisSubjectAuthorityV1 {
    pub subject_id: Uuid,
    pub subject_kind: String,
    pub display_value: String,
    pub subject_identity_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInvestigationAnalysisSubject {
    pub subject_kind: UnifiedInvestigationSubjectKind,
    pub analysis_attempt_id: Uuid,
    pub candidate_snapshot_id: Uuid,
    pub candidate_snapshot_sha256: String,
    pub subject_fingerprint_sha256: String,
    pub binding_id: Uuid,
    pub asset_lane_id: Uuid,
    pub pending_evolution_authority_id: Option<Uuid>,
    pub authority_inputs: Vec<InvestigationAnalysisAuthorityInputV1>,
    pub subject_authorities: Vec<InvestigationAnalysisSubjectAuthorityV1>,
    pub replayed: bool,
}

/// A bounded capability suggestion. This is only an input to the server-owned
/// verification compiler: it has no target URL, credential, raw command,
/// arguments, HTTP body, or execution authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InvestigationAdvisoryCapabilityV1 {
    HttpObservation,
    BrowserObservation,
    CliObservation,
    CredentialedObservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAdvisoryActionIntentV1 {
    pub intent_id: Uuid,
    pub proposal_id: Uuid,
    pub capability: InvestigationAdvisoryCapabilityV1,
    pub purpose_code: String,
    pub evidence_authority_refs: Vec<String>,
}

/// A closed, non-authoritative explanation for why a sealed Analysis Primary
/// emitted no bounded hypothesis.  This is not evidence and cannot create a
/// Candidate, Campaign, action, or execution authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationAnalysisResidualV1 {
    pub kind: String,
    pub reason_code: String,
}

impl InvestigationAnalysisResidualV1 {
    pub const NO_BOUNDED_HYPOTHESIS_KIND: &'static str = "no_bounded_hypothesis";
    pub const SEALED_INPUT_UNSUPPORTED_REASON: &'static str =
        "sealed_input_did_not_support_a_proof_bound_hypothesis";

    pub fn is_valid_no_hypothesis_residual(&self) -> bool {
        self.kind == Self::NO_BOUNDED_HYPOTHESIS_KIND
            && self.reason_code == Self::SEALED_INPUT_UNSUPPORTED_REASON
    }
}

/// Typed advisory envelope emitted by Primary/Workers. There is intentionally
/// no CandidateRegistryMutationV1, canonical claim component, verification
/// contract/plan, Finding, URL, credential, raw command, or executable action
/// payload in this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCognitiveOutputV1 {
    pub schema: String,
    pub subject_id: Uuid,
    pub candidate_snapshot_id: Uuid,
    pub subject_fingerprint_sha256: String,
    pub candidate_proposals: Vec<CandidateHypothesisProposal>,
    pub action_intents: Vec<InvestigationAdvisoryActionIntentV1>,
    pub residuals: Vec<InvestigationAnalysisResidualV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReduceInvestigationCognitiveOutput {
    pub expected_subject: PreparedInvestigationAnalysisSubject,
    pub output: InvestigationCognitiveOutputV1,
}

/// Advisory proposals that passed the SQL-free boundary. This is not a
/// canonical revision/generation seal and grants no execution authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCognitiveAdvisoryView {
    pub subject_id: Uuid,
    pub candidate_snapshot_id: Uuid,
    pub subject_fingerprint_sha256: String,
    pub candidate_proposals: Vec<CandidateHypothesisProposal>,
    pub action_intents: Vec<InvestigationAdvisoryActionIntentV1>,
    pub residuals: Vec<InvestigationAnalysisResidualV1>,
}

/// Server-callable compound input. No canonical mutation, route, revision,
/// verification contract/plan, generation id, Campaign id, executable action,
/// or authority hash can be supplied by the cognitive model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileSealAndAdmitInvestigationGeneration {
    pub stable_compilation_request_id: Uuid,
    pub stable_apply_request_id: Uuid,
    pub stable_admission_request_id: Uuid,
    pub identity: UnifiedInvestigationUnitIdentity,
    pub work_id: Uuid,
    pub prepared_subject: PreparedInvestigationAnalysisSubject,
    pub task_plan_id: Uuid,
    pub delegation_census_seal_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub advisory: InvestigationCognitiveAdvisoryView,
}

/// Exact crash-resume authorization for an Analysis Primary whose typed
/// synthesis is already checkpointed and sealed into the PentAGI pipeline, but
/// whose first canonical compiler attempt durably terminalized the Analysis
/// work as an authority mismatch. This operation never accepts cognitive
/// output and never invokes a model; it only reopens that one witnessed work
/// row so the caller can replay the ordinary reducer/compiler boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeInvestigationAnalysisPostSynthesis {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub work_id: Uuid,
    pub prepared_subject: PreparedInvestigationAnalysisSubject,
    pub task_plan_id: Uuid,
    pub recovery_work_item_id: Uuid,
    pub recovery_worker_run_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub primary_synthesis_event_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumedInvestigationAnalysisPostSynthesisView {
    pub work_id: Uuid,
    pub current_state: String,
    pub head_version: i64,
    pub latest_event_id: Uuid,
    pub checkpoint_sha256: String,
    pub replayed: bool,
}

/// Read-only crash-resume authority for the window after the canonical
/// compiler transaction committed but before the Analysis work and recovery
/// Primary were durably terminalized. Every identity is server-derived; the
/// host either returns the complete frozen admission or fails closed on any
/// partial/drifted artifact set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadCommittedInvestigationAnalysisPostSynthesisAdmission {
    pub stable_compilation_request_id: Uuid,
    pub stable_apply_request_id: Uuid,
    pub stable_admission_request_id: Uuid,
    pub identity: UnifiedInvestigationUnitIdentity,
    pub work_id: Uuid,
    pub prepared_subject: PreparedInvestigationAnalysisSubject,
    pub task_plan_id: Uuid,
    pub delegation_census_seal_id: Uuid,
    pub recovery_work_item_id: Uuid,
    pub recovery_worker_run_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub primary_synthesis_event_sha256: String,
}

/// The normal (non-recovery-item) counterpart of the committed admission
/// lookup. The physical completed Primary is also the logical synthesis actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadCommittedInvestigationAnalysisPrimaryPostSynthesisAdmission {
    pub stable_compilation_request_id: Uuid,
    pub stable_apply_request_id: Uuid,
    pub stable_admission_request_id: Uuid,
    pub identity: UnifiedInvestigationUnitIdentity,
    pub work_id: Uuid,
    pub prepared_subject: PreparedInvestigationAnalysisSubject,
    pub task_plan_id: Uuid,
    pub delegation_census_seal_id: Uuid,
    pub primary_work_item_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub primary_synthesis_event_sha256: String,
}

/// Exact append-only rearm for a normal completed Analysis Primary whose
/// synthesis is sealed but whose first compiler application failed before any
/// canonical decision committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeInvestigationAnalysisPrimaryPostSynthesis {
    pub identity: UnifiedInvestigationUnitIdentity,
    pub work_id: Uuid,
    pub prepared_subject: PreparedInvestigationAnalysisSubject,
    pub task_plan_id: Uuid,
    pub delegation_census_seal_id: Uuid,
    pub primary_work_item_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub primary_synthesis_event_sha256: String,
}

/// Only committed identities escape the host. Canonical compiler material and
/// prepared Operator inputs remain behind the application boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationGenerationAdmissionView {
    pub compilation_decision_id: Uuid,
    pub generation_id: Uuid,
    pub generation_ordinal: u32,
    pub generation_seal_id: Uuid,
    pub generation_member_count: u32,
    pub verification_task_ids: Vec<Uuid>,
    /// The pending evolution batch was closed without a successor generation;
    /// generation fields identify the source authority, not newly admitted work.
    pub evolution_fixed_point: bool,
    pub replayed: bool,
}

/// Reduce untrusted cognitive output into a bounded advisory view. Canonical
/// compilation/sealing is a separate host operation so this function cannot
/// accidentally acquire persistence or execution authority.
pub fn reduce_investigation_cognitive_output(
    request: ReduceInvestigationCognitiveOutput,
) -> InvestigationAnalysisHostResult<InvestigationCognitiveAdvisoryView> {
    if request.output.schema != INVESTIGATION_COGNITIVE_OUTPUT_SCHEMA_V1 {
        return Err(InvestigationAnalysisHostError::InvalidRequest {
            detail: "unsupported cognitive output schema".to_owned(),
        });
    }
    if request.expected_subject.subject_kind != UnifiedInvestigationSubjectKind::AnalysisAttempt
        || request.output.subject_id != request.expected_subject.analysis_attempt_id
        || request.output.candidate_snapshot_id != request.expected_subject.candidate_snapshot_id
        || request.output.subject_fingerprint_sha256
            != request.expected_subject.subject_fingerprint_sha256
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "cognitive output subject authority drifted".to_owned(),
        });
    }
    let valid_residual_shape = if request.output.candidate_proposals.is_empty() {
        request.output.action_intents.is_empty()
            && request.output.residuals.len() == 1
            && request.output.residuals[0].is_valid_no_hypothesis_residual()
    } else {
        request.output.residuals.is_empty()
    };
    if !valid_residual_shape {
        return Err(InvestigationAnalysisHostError::InvalidRequest {
            detail: "proposal/residual exact set is invalid".to_owned(),
        });
    }
    let mut proposal_ids = BTreeSet::new();
    let subject_authorities = request
        .expected_subject
        .subject_authorities
        .iter()
        .map(|subject| {
            (
                subject.subject_kind.as_str(),
                subject.subject_identity_hash.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    for proposal in &request.output.candidate_proposals {
        if proposal.proposal_id.is_nil()
            || !proposal_ids.insert(proposal.proposal_id)
            || !valid_sha256(&proposal.subject_identity_hash)
            || !subject_authorities.contains(&(
                proposal.subject_kind.as_str(),
                proposal.subject_identity_hash.as_str(),
            ))
            || proposal
                .proof_refs
                .iter()
                .any(|reference| !valid_sha256(&reference.source_hash))
        {
            return Err(InvestigationAnalysisHostError::InvalidRequest {
                detail: "candidate proposal identity/hash set is invalid".to_owned(),
            });
        }
    }
    let mut intent_ids = BTreeSet::new();
    for intent in &request.output.action_intents {
        if intent.intent_id.is_nil()
            || !intent_ids.insert(intent.intent_id)
            || !proposal_ids.contains(&intent.proposal_id)
            || intent.purpose_code.trim().is_empty()
            || intent
                .evidence_authority_refs
                .iter()
                .any(|reference| !valid_sha256(reference))
        {
            return Err(InvestigationAnalysisHostError::InvalidRequest {
                detail: "advisory action intent identity/evidence set is invalid".to_owned(),
            });
        }
    }
    Ok(InvestigationCognitiveAdvisoryView {
        subject_id: request.output.subject_id,
        candidate_snapshot_id: request.output.candidate_snapshot_id,
        subject_fingerprint_sha256: request.output.subject_fingerprint_sha256,
        candidate_proposals: request.output.candidate_proposals,
        action_intents: request.output.action_intents,
        residuals: request.output.residuals,
    })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[async_trait]
pub trait InvestigationAnalysisHostRepository: Send + Sync {
    async fn prepare_analysis_subject(
        &self,
        request: PrepareInvestigationAnalysisSubject,
    ) -> InvestigationAnalysisHostResult<PreparedInvestigationAnalysisSubject>;

    fn reduce_cognitive_output(
        &self,
        request: ReduceInvestigationCognitiveOutput,
    ) -> InvestigationAnalysisHostResult<InvestigationCognitiveAdvisoryView> {
        reduce_investigation_cognitive_output(request)
    }

    async fn compile_seal_and_admit(
        &self,
        request: CompileSealAndAdmitInvestigationGeneration,
    ) -> InvestigationAnalysisHostResult<InvestigationGenerationAdmissionView>;

    async fn resume_analysis_post_synthesis(
        &self,
        request: ResumeInvestigationAnalysisPostSynthesis,
    ) -> InvestigationAnalysisHostResult<ResumedInvestigationAnalysisPostSynthesisView> {
        let _ = request;
        Err(InvestigationAnalysisHostError::Unavailable {
            operation: "resume_analysis_post_synthesis",
        })
    }

    async fn load_committed_analysis_post_synthesis_admission(
        &self,
        request: LoadCommittedInvestigationAnalysisPostSynthesisAdmission,
    ) -> InvestigationAnalysisHostResult<Option<InvestigationGenerationAdmissionView>> {
        let _ = request;
        Err(InvestigationAnalysisHostError::Unavailable {
            operation: "load_committed_analysis_post_synthesis_admission",
        })
    }

    async fn load_committed_analysis_primary_post_synthesis_admission(
        &self,
        request: LoadCommittedInvestigationAnalysisPrimaryPostSynthesisAdmission,
    ) -> InvestigationAnalysisHostResult<Option<InvestigationGenerationAdmissionView>> {
        let _ = request;
        Err(InvestigationAnalysisHostError::Unavailable {
            operation: "load_committed_analysis_primary_post_synthesis_admission",
        })
    }

    async fn resume_analysis_primary_post_synthesis(
        &self,
        request: ResumeInvestigationAnalysisPrimaryPostSynthesis,
    ) -> InvestigationAnalysisHostResult<ResumedInvestigationAnalysisPostSynthesisView> {
        let _ = request;
        Err(InvestigationAnalysisHostError::Unavailable {
            operation: "resume_analysis_primary_post_synthesis",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(nibble: char) -> String {
        format!("sha256:{}", nibble.to_string().repeat(64))
    }

    fn subject() -> PreparedInvestigationAnalysisSubject {
        PreparedInvestigationAnalysisSubject {
            subject_kind: UnifiedInvestigationSubjectKind::AnalysisAttempt,
            asset_lane_id: Uuid::new_v4(),
            analysis_attempt_id: Uuid::new_v4(),
            candidate_snapshot_id: Uuid::new_v4(),
            candidate_snapshot_sha256: digest('1'),
            subject_fingerprint_sha256: digest('2'),
            binding_id: Uuid::new_v4(),
            pending_evolution_authority_id: None,
            authority_inputs: Vec::new(),
            subject_authorities: vec![InvestigationAnalysisSubjectAuthorityV1 {
                subject_id: Uuid::new_v4(),
                subject_kind: "web_origin".to_owned(),
                display_value: "https://example.invalid".to_owned(),
                subject_identity_hash: digest('3'),
            }],
            replayed: false,
        }
    }

    fn proposal() -> CandidateHypothesisProposal {
        CandidateHypothesisProposal {
            proposal_id: Uuid::new_v4(),
            subject_kind: "web_origin".to_owned(),
            subject_identity_hash: digest('3'),
            predicate_schema: "http.exposure".to_owned(),
            predicate_version: 1,
            predicate_arguments: vec![("origin".to_owned(), "redacted".to_owned())],
            trust_boundary: "internet_to_web".to_owned(),
            polarity: "positive".to_owned(),
            structured_claim: "typed advisory claim".to_owned(),
            preconditions: Vec::new(),
            impact: "bounded impact".to_owned(),
            proof_refs: Vec::new(),
            knowledge_signals: Vec::new(),
            readiness: crate::task_orchestrator::hypothesis_analysis::CandidateProposalReadiness::ReadyForStrategy,
        }
    }

    fn request(
        subject: &PreparedInvestigationAnalysisSubject,
    ) -> ReduceInvestigationCognitiveOutput {
        ReduceInvestigationCognitiveOutput {
            expected_subject: subject.clone(),
            output: InvestigationCognitiveOutputV1 {
                schema: INVESTIGATION_COGNITIVE_OUTPUT_SCHEMA_V1.to_owned(),
                subject_id: subject.analysis_attempt_id,
                candidate_snapshot_id: subject.candidate_snapshot_id,
                subject_fingerprint_sha256: subject.subject_fingerprint_sha256.clone(),
                candidate_proposals: vec![proposal()],
                action_intents: Vec::new(),
                residuals: Vec::new(),
            },
        }
    }

    #[test]
    fn advisory_only_reducer_accepts_typed_candidate_proposals() {
        let subject = subject();
        let reduced = reduce_investigation_cognitive_output(request(&subject))
            .expect("typed advisory proposal is accepted");
        assert_eq!(reduced.subject_id, subject.analysis_attempt_id);
        assert_eq!(reduced.candidate_proposals.len(), 1);
    }

    #[test]
    fn advisory_intent_must_reference_an_exact_proposal() {
        let subject = subject();
        let mut request = request(&subject);
        request
            .output
            .action_intents
            .push(InvestigationAdvisoryActionIntentV1 {
                intent_id: Uuid::new_v4(),
                proposal_id: Uuid::new_v4(),
                capability: InvestigationAdvisoryCapabilityV1::HttpObservation,
                purpose_code: "collect_response_metadata".to_owned(),
                evidence_authority_refs: vec![digest('4')],
            });
        assert!(matches!(
            reduce_investigation_cognitive_output(request),
            Err(InvestigationAnalysisHostError::InvalidRequest { .. })
        ));
    }

    #[test]
    fn subject_drift_and_untyped_empty_output_fail_closed() {
        let subject = subject();
        let mut drifted = request(&subject);
        drifted.output.subject_id = Uuid::new_v4();
        assert!(matches!(
            reduce_investigation_cognitive_output(drifted),
            Err(InvestigationAnalysisHostError::AuthorityMismatch { .. })
        ));

        let mut empty = request(&subject);
        empty.output.candidate_proposals.clear();
        assert!(matches!(
            reduce_investigation_cognitive_output(empty),
            Err(InvestigationAnalysisHostError::InvalidRequest { .. })
        ));
    }

    #[test]
    fn typed_no_hypothesis_residual_is_a_legal_zero_proposal_advisory() {
        let subject = subject();
        let mut empty = request(&subject);
        empty.output.candidate_proposals.clear();
        empty.output.residuals = vec![InvestigationAnalysisResidualV1 {
            kind: InvestigationAnalysisResidualV1::NO_BOUNDED_HYPOTHESIS_KIND.to_owned(),
            reason_code: InvestigationAnalysisResidualV1::SEALED_INPUT_UNSUPPORTED_REASON
                .to_owned(),
        }];

        let reduced = reduce_investigation_cognitive_output(empty)
            .expect("typed no-hypothesis residual is a legal sealed advisory");
        assert!(reduced.candidate_proposals.is_empty());
        assert_eq!(reduced.residuals.len(), 1);
    }
}
