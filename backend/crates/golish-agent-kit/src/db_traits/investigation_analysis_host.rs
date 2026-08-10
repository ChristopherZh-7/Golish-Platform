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
    pub admission_objective_count: u32,
    pub verification_task_ids: Vec<Uuid>,
    pub campaign_ids: Vec<Uuid>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareInvestigationVerificationTaskSubject {
    pub stable_request_id: Uuid,
    pub identity: UnifiedInvestigationUnitIdentity,
    pub verification_task_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationBoundedContextRefV1 {
    pub kind: String,
    pub id: Uuid,
    pub authority_sha256: String,
}

/// Canonical reservation binding one VerificationTask Campaign to the exact
/// plan objective and verification objective it is allowed to serve. This is
/// identity/hash authority only and cannot carry executable material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationVerificationCampaignSubjectV1 {
    pub campaign_id: Uuid,
    pub plan_objective_id: Uuid,
    pub objective_id: Uuid,
    pub reservation_sha256: String,
    pub capability_assessment_set_sha256: String,
    /// Exact closed capability ids whose current sealed assessment is
    /// `available` for this Campaign. This is cognitive selection authority,
    /// never executable target/argument/credential material.
    pub available_capability_ids: Vec<String>,
}

/// Closed, non-executable capability choice for a VerificationTask strategy.
/// These values name host registry contracts; the host still derives target,
/// arguments, budgets, credentials, network policy and Operator lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InvestigationVerificationCapabilityV1 {
    AnonymousAuthenticatedDifferential,
    DirectoryFingerprint,
    NucleiExactReplay,
    ConcurrentRaceDifferential,
}

/// Read-only input for running a VerificationTask through the same PentAGI
/// runner. It names exact durable authority and exposes only bounded hashes;
/// no SQL handle, raw action, credential, URL, or execution lease is present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInvestigationVerificationTaskSubject {
    pub subject_kind: UnifiedInvestigationSubjectKind,
    pub verification_task_id: Uuid,
    pub hypothesis_revision_id: Uuid,
    pub hypothesis_revision_sha256: String,
    pub verification_plan_id: Uuid,
    pub verification_plan_sha256: String,
    pub assignment_set_id: Uuid,
    pub assignment_set_sha256: String,
    /// Compatibility/index view; `campaigns` is the canonical objective-bound
    /// authority used for strategy validation.
    pub campaign_ids: Vec<Uuid>,
    pub campaigns: Vec<InvestigationVerificationCampaignSubjectV1>,
    pub campaign_denominator_sha256: String,
    pub subject_fingerprint_sha256: String,
    pub bounded_context: Vec<InvestigationBoundedContextRefV1>,
    pub replayed: bool,
}

/// Cognitive strategy only. Capability/control identifiers are resolved by
/// the host registry; this type cannot carry target, protocol arguments,
/// credentials, commands, request bodies, budgets, or execution authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationVerificationStrategyV1 {
    pub strategy_id: Uuid,
    pub campaign_id: Uuid,
    pub objective_id: Uuid,
    pub capability: InvestigationVerificationCapabilityV1,
    pub purpose_code: String,
    pub required_control_codes: Vec<String>,
    pub evidence_authority_refs: Vec<String>,
}

/// Verification-task-local action intent. It references a typed strategy and
/// Campaign, never an analysis proposal, and still carries no executable
/// target, request, credential, command, argument, or budget material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationVerificationActionIntentV1 {
    pub intent_id: Uuid,
    pub strategy_id: Uuid,
    pub campaign_id: Uuid,
    pub capability: InvestigationVerificationCapabilityV1,
    pub purpose_code: String,
    pub evidence_authority_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyInvestigationVerificationTaskAdvisory {
    pub stable_request_id: Uuid,
    pub identity: UnifiedInvestigationUnitIdentity,
    pub prepared_subject: PreparedInvestigationVerificationTaskSubject,
    pub task_plan_id: Uuid,
    pub delegation_census_seal_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub accepted_output_sha256: Vec<String>,
    /// Exact Primary synthesis residual set. These are cognitive reducer
    /// residuals, distinct from Campaign compilation residual receipts.
    pub primary_residual_sha256: Vec<String>,
    pub strategies: Vec<InvestigationVerificationStrategyV1>,
    pub action_intents: Vec<InvestigationVerificationActionIntentV1>,
}

/// Crash-recovery probe for a VerificationTask whose PentAGI advisory may
/// already have been frozen. The host returns `None` only when no immutable
/// advisory receipt exists; otherwise it resumes that exact envelope without
/// consulting the model again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeInvestigationVerificationTaskAdvisory {
    pub stable_request_id: Uuid,
    pub identity: UnifiedInvestigationUnitIdentity,
    pub prepared_subject: PreparedInvestigationVerificationTaskSubject,
    pub task_plan_id: Uuid,
    pub primary_worker_run_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationVerificationApplyView {
    pub verification_task_id: Uuid,
    pub campaign_ids: Vec<Uuid>,
    pub prepared_action_ids: Vec<Uuid>,
    pub residual_receipt_ids: Vec<Uuid>,
    pub primary_residual_count: u32,
    pub primary_residual_set_sha256: String,
    pub fact_delta_bundle_ids: Vec<Uuid>,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeInvestigationVerificationTasks {
    pub identity: super::UnifiedInvestigationStageIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedInvestigationVerificationTasksView {
    pub task_count: u32,
    pub terminal_count: u32,
    pub blocked_count: u32,
    pub outcome_set_ids: Vec<Uuid>,
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
    if request.output.candidate_proposals.is_empty() {
        return Err(InvestigationAnalysisHostError::InvalidRequest {
            detail: "empty advisory output requires a typed residual contract".to_owned(),
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

    async fn prepare_verification_task_subject(
        &self,
        request: PrepareInvestigationVerificationTaskSubject,
    ) -> InvestigationAnalysisHostResult<PreparedInvestigationVerificationTaskSubject>;

    async fn apply_verification_task_advisory(
        &self,
        request: ApplyInvestigationVerificationTaskAdvisory,
    ) -> InvestigationAnalysisHostResult<InvestigationVerificationApplyView>;

    async fn resume_verification_task_advisory(
        &self,
        request: ResumeInvestigationVerificationTaskAdvisory,
    ) -> InvestigationAnalysisHostResult<Option<InvestigationVerificationApplyView>> {
        let _ = request;
        Err(InvestigationAnalysisHostError::Unavailable {
            operation: "resume_verification_task_advisory",
        })
    }

    async fn finalize_verification_tasks_from_campaign_truth(
        &self,
        request: FinalizeInvestigationVerificationTasks,
    ) -> InvestigationAnalysisHostResult<FinalizedInvestigationVerificationTasksView> {
        let _ = request;
        Err(InvestigationAnalysisHostError::Unavailable {
            operation: "finalize_verification_tasks_from_campaign_truth",
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
            analysis_attempt_id: Uuid::new_v4(),
            candidate_snapshot_id: Uuid::new_v4(),
            candidate_snapshot_sha256: digest('1'),
            subject_fingerprint_sha256: digest('2'),
            binding_id: Uuid::new_v4(),
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
}
