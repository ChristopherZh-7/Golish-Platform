//! Atomic canonical writer for Plan B Candidate Gate passes.
//!
//! This module deliberately has one pool entrypoint and one transaction.  It
//! never writes materialized projection/legacy rows and never advances the
//! projection head.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::{
    candidate_revision_id, derive_root_id, initial_root_id, merge_root_id, split_root_id,
    AtTimeSubjectIdentity, CandidateMutationEpistemicState, HypothesisSemanticKeyV1,
};
use golish_core::hypothesis_verification::{
    HypothesisClaimComponentV1, HypothesisVerificationPlanPathMemberRoleV1,
    HypothesisVerificationPlanV1,
};
use golish_core::investigation_comparison::{
    CheckedAuthorityComparisonV1, ComparisonAuthorityBasisInputV1,
    ComparisonHypothesisDispositionV1, ComparisonHypothesisReadinessV1, GenerationComparisonV1,
    InvestigationComparisonRecordInputV1, KnowledgeFeedComparisonV1,
    PlanBCheckedComparisonAuthorityInputV1, PlanCComparisonAuthorityInputV1,
};
use golish_core::investigation_projection::{
    GenerationProjectionRecordV1, HypothesisProjectionRecordV1,
    HypothesisStateEventProjectionRecordV1, HypothesisVerificationPlanProjectionRecordV1,
    ProjectionChangeKind, ProjectionSourceSnapshotV1, ProjectionSourceTimeStatusV1,
    RelationProjectionRecordV1, ResidualProjectionRecordV1,
};
use golish_core::verification_contract::VerificationContractV1;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::candidate_analysis::{
    hash_text_array_on, load_snapshot_on, lock_and_require_registry_canonical_on,
    reevaluate_candidate_gate_authority_on, validate_final_submitter_fence_on,
    validate_write_fence_on, AnalysisArtifactBodyRow, CandidateSnapshotDispositionRow,
    CandidateWriteFenceRow,
};
use super::hypothesis_legacy_projection::{
    append_projection_source_batch_on, freeze_comparison_projection_source_body_v1,
    AppendProjectionSourceBatchRow, ProjectionOutboxSourceRow, ProjectionSourceStorageV1,
};
use crate::{DbError, Result};

const AUTHORITY_MISMATCH: &str = "HYPOTHESIS_REGISTRY_AUTHORITY_MISMATCH";
const APPLY_REPLAY_DRIFT: &str = "HYPOTHESIS_REGISTRY_APPLY_REPLAY_DRIFT";
const MUTATION_SET_INVALID: &str = "HYPOTHESIS_REGISTRY_MUTATION_SET_INVALID";
const COMPILED_AUTHORITY_INCOMPLETE: &str = "HYPOTHESIS_REGISTRY_COMPILED_AUTHORITY_INCOMPLETE";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

async fn hash_json_on(tx: &mut Transaction<'_, Postgres>, value: &Value) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(value)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn investigation_set_hash_on(
    tx: &mut Transaction<'_, Postgres>,
    domain: &str,
    hashes: &[String],
) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT investigation_exact_member_set_hash($1,$2::TEXT[])")
            .bind(domain)
            .bind(hashes)
            .fetch_one(&mut **tx)
            .await?,
    )
}

fn candidate_exact_set_hash(domain: &str, member_hashes: &[String]) -> String {
    fn field(hasher: &mut Sha256, value: &[u8]) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }

    let mut hashes = member_hashes.to_vec();
    hashes.sort();
    let mut hasher = Sha256::new();
    field(&mut hasher, domain.as_bytes());
    for hash in hashes {
        field(&mut hasher, hash.as_bytes());
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn is_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn candidate_mutation_hash(mutation: &CandidateMutationRow) -> String {
    fn field(hasher: &mut Sha256, value: &[u8]) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    let proof = mutation
        .proof_refs
        .iter()
        .map(CandidateRevisionSourceRefRow::canonical_key)
        .collect::<Vec<_>>()
        .join("\u{1f}");
    let refutations = mutation
        .refutation_refs
        .iter()
        .map(CandidateRevisionSourceRefRow::canonical_key)
        .collect::<Vec<_>>()
        .join("\u{1f}");
    let mut hasher = Sha256::new();
    for value in [
        "candidate_hypothesis_mutation.v1".to_owned(),
        mutation.proposal_id.to_string(),
        mutation.organization_id.to_string(),
        mutation.semantic_key_hash.clone(),
        mutation.operator_rank.to_string(),
        mutation.state.as_str().to_owned(),
        proof,
        refutations,
        mutation.generation_transition_hash.clone(),
    ] {
        field(&mut hasher, value.as_bytes());
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn validate_compiled_exact_sets(
    input: &ApplyCandidateGatePassInput,
    pending: &[PreparedMutation],
) -> Result<()> {
    let pending_revisions = pending
        .iter()
        .map(|mutation| mutation.revision_id)
        .collect::<std::collections::BTreeSet<_>>();
    let component_hashes = input
        .claim_components
        .iter()
        .map(|component| component.member_hash().to_owned())
        .collect::<Vec<_>>();
    let contract_hashes = input
        .verification_contracts
        .iter()
        .map(|contract| contract.contract_hash().to_owned())
        .collect::<Vec<_>>();
    let plan_hashes = input
        .verification_plans
        .iter()
        .map(|plan| plan.plan_hash().to_owned())
        .collect::<Vec<_>>();
    let component_revisions = input
        .claim_components
        .iter()
        .map(|component| component.revision_id())
        .collect::<std::collections::BTreeSet<_>>();
    let contract_revisions = input
        .verification_contracts
        .iter()
        .map(|contract| contract.revision_id())
        .collect::<std::collections::BTreeSet<_>>();
    let plan_revisions = input
        .verification_plans
        .iter()
        .map(|plan| plan.revision_id())
        .collect::<std::collections::BTreeSet<_>>();
    if (pending.is_empty()
        && (!component_hashes.is_empty() || !contract_hashes.is_empty() || !plan_hashes.is_empty()))
        || (!pending.is_empty()
            && (component_hashes.is_empty()
                || contract_hashes.is_empty()
                || plan_hashes.is_empty()))
        || component_hashes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != component_hashes.len()
        || contract_hashes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != contract_hashes.len()
        || plan_hashes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != plan_hashes.len()
        || component_revisions != pending_revisions
        || contract_revisions != pending_revisions
        || plan_revisions != pending_revisions
        || candidate_exact_set_hash("candidate_claim_components.v1", &component_hashes)
            != input.claim_component_set_hash
        || candidate_exact_set_hash("candidate_contracts.v1", &contract_hashes)
            != input.verification_contract_set_hash
        || candidate_exact_set_hash("candidate_plans.v1", &plan_hashes)
            != input.verification_plan_set_hash
    {
        return Err(conflict(COMPILED_AUTHORITY_INCOMPLETE));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateExpectedAuthorityRow {
    pub snapshot_hash: String,
    pub candidate_snapshot_authority_hash: String,
    pub tool_truth_authority_bundle_seal_id: Uuid,
    pub tool_truth_authority_root_set_hash: String,
    pub tool_truth_authority_bundle_member_set_hash: String,
    pub tool_truth_authority_receipt_set_hash: String,
    pub denominator_graph_bundle_hash: String,
    pub semantic_authority_bundle_hash: String,
    pub freshness_attestation_bundle_hash: String,
    pub temporal_validity_bundle_hash: String,
    pub temporal_validity_policy_set_hash: String,
    pub temporal_validity_decision_set_hash: String,
    pub target_state_epoch_set_hash: String,
    pub gate_temporal_reevaluation_hash: String,
    pub knowledge_feed_catalog_policy_seal_hash: String,
    pub knowledge_feed_required_member_set_hash: String,
    pub knowledge_feed_signature_algorithm_set_hash: String,
    pub knowledge_feed_trust_store_hash: String,
    pub knowledge_feed_key_revocation_epoch_hash: String,
    pub knowledge_feed_snapshot_set_hash: String,
    pub product_version_census_hash: String,
    pub knowledge_feed_match_census_hash: String,
    pub gate_knowledge_feed_reevaluation_hash: String,
    pub stale_revalidation_obligation_set_hash: String,
    pub knowledge_feed_obligation_set_hash: String,
    pub prior_terminal_attempt_chain_hash: String,
    pub proposal_census_hash: String,
    pub critic_census_hash: String,
    pub controller_decision_set_hash: String,
    pub input_chunk_census_set_hash: String,
    pub coverage_subreview_census_set_hash: String,
    pub coverage_synthesis_census_set_hash: String,
    pub coverage_global_semantic_root_hash: String,
    pub coverage_global_review_hash: String,
    pub coverage_review_set_hash: String,
    pub coverage_checklist_set_hash: String,
    pub generation_transition_set_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CandidateMutationRouteRow {
    AttachCurrent {
        root_id: Uuid,
        revision_id: Uuid,
    },
    CreateInitial {
        root_id: Uuid,
    },
    ReopenHistorical {
        root_id: Uuid,
        predecessor_revision_id: Uuid,
    },
    Split {
        parent_root_id: Uuid,
        child_root_id: Uuid,
    },
    Merge {
        parent_root_ids: Vec<Uuid>,
        successor_root_id: Uuid,
    },
    Derive {
        source_root_id: Uuid,
        source_revision_id: Uuid,
        derivation_rule_hash: String,
        successor_root_id: Uuid,
    },
    NarrowSuccessor {
        source_root_id: Uuid,
        source_revision_id: Uuid,
        successor_root_id: Uuid,
        covered_claim_component_set_hash: String,
    },
    NoSemanticChange {
        root_id: Uuid,
        revision_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum CandidateRevisionSourceRefRow {
    ToolTruthEvidence(String),
    Finding(String),
    VerificationReceipt(String),
    ApplicationContext(String),
    KnowledgeSignal(String),
    Gap(String),
}

impl CandidateRevisionSourceRefRow {
    pub(super) fn canonical_key(&self) -> String {
        match self {
            Self::ToolTruthEvidence(id) => format!("tool_truth:{id}"),
            Self::Finding(id) => format!("finding:{id}"),
            Self::VerificationReceipt(id) => format!("verification:{id}"),
            Self::ApplicationContext(id) => format!("application:{id}"),
            Self::KnowledgeSignal(id) => format!("knowledge:{id}"),
            Self::Gap(id) => format!("gap:{id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateMutationRow {
    pub proposal_id: Uuid,
    pub organization_id: Uuid,
    pub semantic_key_hash: String,
    pub operator_rank: u8,
    pub state: CandidateMutationEpistemicState,
    pub proof_refs: Vec<CandidateRevisionSourceRefRow>,
    pub refutation_refs: Vec<CandidateRevisionSourceRefRow>,
    pub generation_transition_hash: String,
    pub mutation_hash: String,
    pub route: CandidateMutationRouteRow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputDispositionRow {
    pub input_id: Uuid,
    pub disposition: String,
    pub reason_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputHypothesisRelationRow {
    pub input_id: Uuid,
    pub root_id: Uuid,
    pub relation_kind: String,
}

#[derive(Debug, Clone)]
pub struct ApplyCandidateGatePassInput {
    pub fence: CandidateWriteFenceRow,
    pub stable_compilation_request_id: Uuid,
    pub stable_apply_request_id: Uuid,
    pub expected_authority: CandidateExpectedAuthorityRow,
    pub active_analysis_attempt_id: Uuid,
    pub active_analysis_attempt_ordinal: i32,
    pub mutations: Vec<CandidateMutationRow>,
    pub mutation_set_hash: String,
    pub claim_components: Vec<HypothesisClaimComponentV1>,
    pub claim_component_set_hash: String,
    pub verification_contracts: Vec<VerificationContractV1>,
    pub verification_contract_set_hash: String,
    pub verification_plans: Vec<HypothesisVerificationPlanV1>,
    pub verification_plan_set_hash: String,
    pub input_dispositions: Vec<InputDispositionRow>,
    pub input_relations: Vec<InputHypothesisRelationRow>,
    pub final_submitter_worker_run_id: Uuid,
    pub expected_source_head_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateGenerationSealRowView {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub snapshot_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub generation_id: Uuid,
    pub generation_ordinal: i32,
    pub generation_seal_id: Uuid,
    pub generation_member_count: i64,
    pub generation_member_set_hash: String,
    pub generation_event_set_hash: String,
    pub open_obligation_set_hash: String,
    pub projection_outbox_batch_id: Uuid,
    pub projection_source_batch_seq: i64,
    pub projection_outbox_member_set_hash: String,
    pub post_seal_route: String,
    pub replayed: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct LockedSnapshot {
    operation_id: Uuid,
    organization_id: Uuid,
    scope_snapshot_id: Option<Uuid>,
    snapshot_status: String,
    candidate_snapshot_authority_hash: String,
    tool_truth_authority_bundle_seal_id: Uuid,
    relevant_root_set_hash: String,
    bundle_member_set_hash: String,
    semantic_authority_bundle_hash: String,
    freshness_attestation_bundle_hash: String,
    temporal_validity_bundle_hash: String,
    temporal_validity_policy_set_hash: String,
    target_state_epoch_set_hash: String,
}

fn validate_apply_envelope(input: &ApplyCandidateGatePassInput) -> Result<()> {
    let mutation_hashes = input
        .mutations
        .iter()
        .map(|mutation| mutation.mutation_hash.clone())
        .collect::<Vec<_>>();
    let transition_hashes = input
        .mutations
        .iter()
        .map(|mutation| mutation.generation_transition_hash.clone())
        .collect::<Vec<_>>();
    let unique_mutations = mutation_hashes
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let unique_proposals = input
        .mutations
        .iter()
        .map(|mutation| mutation.proposal_id)
        .collect::<std::collections::BTreeSet<_>>();
    if input.mutations.iter().any(|mutation| {
        mutation.organization_id != input.fence.organization_id
            || mutation.mutation_hash != candidate_mutation_hash(mutation)
            || mutation.proof_refs.iter().any(|source| {
                matches!(
                    source,
                    CandidateRevisionSourceRefRow::ApplicationContext(_)
                        | CandidateRevisionSourceRefRow::KnowledgeSignal(_)
                        | CandidateRevisionSourceRefRow::Gap(_)
                )
            })
            || mutation
                .refutation_refs
                .iter()
                .any(|source| matches!(source, CandidateRevisionSourceRefRow::Gap(_)))
    }) || mutation_hashes.iter().any(|hash| !is_sha256(hash))
        || transition_hashes.iter().any(|hash| !is_sha256(hash))
        || unique_mutations.len() != mutation_hashes.len()
        || unique_proposals.len() != input.mutations.len()
        || candidate_exact_set_hash("candidate_mutations.v1", &mutation_hashes)
            != input.mutation_set_hash
        || candidate_exact_set_hash("candidate_generation_transitions.v1", &transition_hashes)
            != input.expected_authority.generation_transition_set_hash
    {
        return Err(conflict(MUTATION_SET_INVALID));
    }
    Ok(())
}

async fn gate_authority_hash_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
) -> Result<String> {
    hash_json_on(
        tx,
        &json!({
            "domain":"candidate_gate_apply_authority.v1",
            "fence":&input.fence,
            "stable_compilation_request_id":input.stable_compilation_request_id,
            "expected_authority":&input.expected_authority,
            "active_analysis_attempt_id":input.active_analysis_attempt_id,
            "active_analysis_attempt_ordinal":input.active_analysis_attempt_ordinal,
            "mutations":&input.mutations,
            "mutation_set_hash":input.mutation_set_hash,
            "claim_component_set_hash":input.claim_component_set_hash,
            "verification_contract_set_hash":input.verification_contract_set_hash,
            "verification_plan_set_hash":input.verification_plan_set_hash,
            "input_dispositions":&input.input_dispositions,
            "input_relations":&input.input_relations,
            "final_submitter_worker_run_id":input.final_submitter_worker_run_id,
            "expected_source_head_version":input.expected_source_head_version,
        }),
    )
    .await
}

async fn seal_candidate_compilation_for_apply_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
) -> Result<()> {
    let material_hash: String = sqlx::query_scalar(
        r#"SELECT material_hash
             FROM candidate_analysis_host_compilation_materials
            WHERE stable_compilation_request_id=$1
              AND analysis_attempt_id=$2 AND snapshot_id=$3
              AND operation_id=$4 AND organization_id=$5
              AND final_submitter_worker_run_id=$6
              AND mutation_set_hash=$7 AND claim_component_set_hash=$8
              AND verification_contract_set_hash=$9
              AND verification_plan_set_hash=$10
              AND generation_transition_set_hash=$11
            FOR SHARE"#,
    )
    .bind(input.stable_compilation_request_id)
    .bind(input.active_analysis_attempt_id)
    .bind(input.fence.snapshot_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(input.final_submitter_worker_run_id)
    .bind(&input.mutation_set_hash)
    .bind(&input.claim_component_set_hash)
    .bind(&input.verification_contract_set_hash)
    .bind(&input.verification_plan_set_hash)
    .bind(&input.expected_authority.generation_transition_set_hash)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    let compilation_seal_id = Uuid::new_v5(
        &input.active_analysis_attempt_id,
        b"candidate_host_compilation_seal.v1",
    );
    let compiler_seal_hash = hash_json_on(
        tx,
        &json!({
            "domain":"candidate_host_compilation_seal.v1",
            "analysis_attempt_id":input.active_analysis_attempt_id,
            "snapshot_id":input.fence.snapshot_id,
            "operation_id":input.fence.operation_id,
            "organization_id":input.fence.organization_id,
            "final_submitter_worker_run_id":input.final_submitter_worker_run_id,
            "mutation_set_hash":input.mutation_set_hash,
            "claim_component_set_hash":input.claim_component_set_hash,
            "verification_contract_set_hash":input.verification_contract_set_hash,
            "verification_plan_set_hash":input.verification_plan_set_hash,
            "generation_transition_set_hash":input.expected_authority.generation_transition_set_hash,
            "compilation_material_hash":material_hash,
        }),
    )
    .await?;
    let inserted = sqlx::query(
        r#"INSERT INTO candidate_analysis_host_compilation_seals(
               compilation_seal_id,stable_compilation_request_id,
               analysis_attempt_id,snapshot_id,operation_id,organization_id,
               final_submitter_worker_run_id,mutation_set_hash,
               claim_component_set_hash,verification_contract_set_hash,
               verification_plan_set_hash,generation_transition_set_hash,
               compilation_material_hash,compiler_seal_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
           ON CONFLICT(analysis_attempt_id) DO NOTHING"#,
    )
    .bind(compilation_seal_id)
    .bind(input.stable_compilation_request_id)
    .bind(input.active_analysis_attempt_id)
    .bind(input.fence.snapshot_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(input.final_submitter_worker_run_id)
    .bind(&input.mutation_set_hash)
    .bind(&input.claim_component_set_hash)
    .bind(&input.verification_contract_set_hash)
    .bind(&input.verification_plan_set_hash)
    .bind(&input.expected_authority.generation_transition_set_hash)
    .bind(&material_hash)
    .bind(&compiler_seal_hash)
    .execute(&mut **tx)
    .await?;
    if inserted.rows_affected() == 0 {
        let persisted: Option<(Uuid, Uuid, String, String)> = sqlx::query_as(
            r#"SELECT compilation_seal_id,stable_compilation_request_id,
                      compilation_material_hash,compiler_seal_hash
                 FROM candidate_analysis_host_compilation_seals
                WHERE analysis_attempt_id=$1"#,
        )
        .bind(input.active_analysis_attempt_id)
        .fetch_optional(&mut **tx)
        .await?;
        if persisted
            != Some((
                compilation_seal_id,
                input.stable_compilation_request_id,
                material_hash,
                compiler_seal_hash,
            ))
        {
            return Err(conflict(AUTHORITY_MISMATCH));
        }
    }
    Ok(())
}

pub async fn apply_candidate_gate_pass(
    pool: &PgPool,
    input: ApplyCandidateGatePassInput,
) -> Result<CandidateGenerationSealRowView> {
    validate_apply_envelope(&input)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let isolation: String = sqlx::query_scalar("SHOW transaction_isolation")
        .fetch_one(&mut *tx)
        .await?;
    if isolation != "repeatable read" {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    lock_and_require_registry_canonical_on(&mut tx, input.fence.operation_id).await?;

    if let Some(replayed) = load_apply_replay_on(&mut tx, &input).await? {
        tx.commit().await?;
        return Ok(replayed);
    }
    validate_write_fence_on(&mut tx, &input.fence).await?;
    validate_final_submitter_fence_on(&mut tx, &input.fence).await?;
    let exact_closure = super::candidate_analysis::validate_candidate_analysis_exact_closure_on(
        &mut tx,
        input.active_analysis_attempt_id,
        input.fence.snapshot_id,
    )
    .await?;
    if !exact_closure.gate_eligible
        || exact_closure.proposal_census_hash != input.expected_authority.proposal_census_hash
        || exact_closure.critic_census_hash.as_deref()
            != Some(input.expected_authority.critic_census_hash.as_str())
        || exact_closure.coverage_subreview_census_set_hash
            != input.expected_authority.coverage_subreview_census_set_hash
        || exact_closure.coverage_checklist_set_hash
            != input.expected_authority.coverage_checklist_set_hash
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    seal_candidate_compilation_for_apply_on(&mut tx, &input).await?;
    validate_apply_authority_on(&mut tx, &input).await?;

    let generation_ordinal: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(generation_ordinal)+1,0) FROM hypothesis_generations WHERE operation_id=$1 AND organization_id=$2",
    )
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    let previous_generation_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT generation_id FROM hypothesis_generations WHERE operation_id=$1 AND organization_id=$2 ORDER BY generation_ordinal DESC LIMIT 1",
    )
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    let generation_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        format!("candidate_generation:{generation_ordinal}").as_bytes(),
    );
    let gate_decision_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        b"candidate_gate_decision.v1",
    );

    let mut pending = Vec::new();
    for (gate_ordinal, mutation) in input.mutations.iter().enumerate() {
        if mutation.organization_id != input.fence.organization_id {
            return Err(conflict(MUTATION_SET_INVALID));
        }
        if matches!(
            mutation.route,
            CandidateMutationRouteRow::AttachCurrent { .. }
                | CandidateMutationRouteRow::NoSemanticChange { .. }
        ) {
            continue;
        }
        let mut prepared = prepare_mutation_on(&mut tx, &input, mutation).await?;
        prepared.gate_ordinal = i32::try_from(gate_ordinal).unwrap_or(i32::MAX);
        pending.push(prepared);
    }
    pending.sort_by(|left, right| {
        (
            left.semantic_key_hash.as_str(),
            left.root_id,
            left.proposal_id,
        )
            .cmp(&(
                right.semantic_key_hash.as_str(),
                right.root_id,
                right.proposal_id,
            ))
    });
    let root_ordinals = pending
        .iter()
        .map(|mutation| (mutation.root_id, mutation.revision_ordinal))
        .collect::<BTreeSet<_>>();
    if root_ordinals.len() != pending.len() {
        return Err(conflict(MUTATION_SET_INVALID));
    }
    validate_compiled_exact_sets(&input, &pending)?;
    let mutation_hashes = input
        .mutations
        .iter()
        .map(|mutation| mutation.mutation_hash.clone())
        .collect::<Vec<_>>();
    let mutation_set_hash = candidate_exact_set_hash("candidate_mutations.v1", &mutation_hashes);
    if mutation_set_hash != input.mutation_set_hash {
        return Err(conflict(MUTATION_SET_INVALID));
    }
    let gate_authority_hash = gate_authority_hash_on(&mut tx, &input).await?;
    let gate_decision_hash = hash_json_on(
        &mut tx,
        &json!({"gate_authority_hash":gate_authority_hash,"mutation_set_hash":mutation_set_hash}),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_candidate_gate_decisions(
               decision_id,stable_request_id,operation_id,organization_id,
               candidate_snapshot_id,analysis_attempt_id,mutation_count,mutation_set_hash,
               generation_transition_count,generation_transition_set_hash,
               gate_authority_hash,decision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(gate_decision_id)
    .bind(input.stable_apply_request_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(input.fence.snapshot_id)
    .bind(input.active_analysis_attempt_id)
    .bind(i64::try_from(input.mutations.len()).unwrap_or(i64::MAX))
    .bind(&mutation_set_hash)
    .bind(i64::try_from(input.mutations.len()).unwrap_or(i64::MAX))
    .bind(&input.expected_authority.generation_transition_set_hash)
    .bind(&gate_authority_hash)
    .bind(&gate_decision_hash)
    .execute(&mut *tx)
    .await?;

    let mut created_revisions = Vec::with_capacity(pending.len());
    let mut revision_by_root = BTreeMap::new();
    let mut state_event_ids = Vec::with_capacity(pending.len());
    for mutation in &pending {
        let event_id = persist_mutation_compound_on(
            &mut tx,
            &input,
            gate_decision_id,
            mutation.gate_ordinal,
            mutation,
        )
        .await?;
        created_revisions.push(mutation.revision_id);
        revision_by_root.insert(mutation.root_id, mutation.revision_id);
        state_event_ids.push(event_id);
    }
    let mut generation_additions = revision_by_root.clone();
    for (gate_ordinal, mutation) in input.mutations.iter().enumerate() {
        let (root_id, revision_id, requires_current, add_to_generation, route_kind) =
            match mutation.route {
                CandidateMutationRouteRow::AttachCurrent {
                    root_id,
                    revision_id,
                } => (root_id, revision_id, true, true, "attach_current"),
                CandidateMutationRouteRow::NoSemanticChange {
                    root_id,
                    revision_id,
                } => (root_id, revision_id, false, false, "no_semantic_change"),
                _ => continue,
            };
        let valid: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM attack_hypothesis_revisions revision
                  WHERE revision.root_id=$1 AND revision.revision_id=$2
                    AND revision.operation_id=$3 AND revision.organization_id=$4
                    AND revision.semantic_key_hash=$5
                    AND revision.epistemic_state=$6
                    AND ($7=FALSE OR EXISTS(
                        SELECT 1 FROM attack_hypothesis_heads head
                         WHERE head.root_id=revision.root_id
                           AND head.head_revision_id=revision.revision_id
                           AND head.operation_id=revision.operation_id
                           AND head.organization_id=revision.organization_id
                           AND head.head_lifecycle_state='current')) )"#,
        )
        .bind(root_id)
        .bind(revision_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(&mutation.semantic_key_hash)
        .bind(mutation.state.as_str())
        .bind(requires_current)
        .fetch_one(&mut *tx)
        .await?;
        if !valid || revision_by_root.insert(root_id, revision_id).is_some() {
            return Err(conflict(MUTATION_SET_INVALID));
        }
        if add_to_generation {
            generation_additions.insert(root_id, revision_id);
        }
        sqlx::query(
            r#"INSERT INTO hypothesis_candidate_gate_decision_members(
                   mutation_id,decision_id,operation_id,organization_id,ordinal,route_kind,
                   root_id,predecessor_revision_id,successor_revision_id,semantic_key_hash,
                   successor_epistemic_state,origin_decision_hash,
                   generation_transition_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,NULL,$8,$9,$10,$11,$12,$11)"#,
        )
        .bind(Uuid::new_v5(
            &gate_decision_id,
            mutation.mutation_hash.as_bytes(),
        ))
        .bind(gate_decision_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(i32::try_from(gate_ordinal).unwrap_or(i32::MAX))
        .bind(route_kind)
        .bind(root_id)
        .bind(revision_id)
        .bind(&mutation.semantic_key_hash)
        .bind(mutation.state.as_str())
        .bind(&mutation.mutation_hash)
        .bind(&mutation.generation_transition_hash)
        .execute(&mut *tx)
        .await?;
    }

    persist_input_decisions_on(&mut tx, &input, &revision_by_root).await?;

    let previous_members: Vec<(Uuid, Uuid)> =
        if let Some(previous_generation_id) = previous_generation_id {
            sqlx::query_as(
                r#"SELECT generation_member_id,revision_id
                  FROM hypothesis_generation_members
                 WHERE generation_id=$1 ORDER BY ordinal FOR SHARE"#,
            )
            .bind(previous_generation_id)
            .fetch_all(&mut *tx)
            .await?
        } else {
            Vec::new()
        };
    let consumed_revisions = pending
        .iter()
        .filter(|mutation| matches!(mutation.route_kind, "split" | "merge"))
        .flat_map(|mutation| {
            mutation
                .relation_sources
                .iter()
                .map(|source| source.revision_id)
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut generation_revisions = previous_members
        .iter()
        .map(|(_, revision_id)| *revision_id)
        .filter(|revision_id| !consumed_revisions.contains(revision_id))
        .collect::<std::collections::BTreeSet<_>>();
    generation_revisions.extend(generation_additions.values().copied());
    let generation_revisions = generation_revisions.into_iter().collect::<Vec<_>>();

    sqlx::query(
        r#"INSERT INTO hypothesis_generations(
               generation_id,operation_id,organization_id,generation_ordinal,
               candidate_snapshot_id,candidate_gate_decision_id,
               candidate_snapshot_authority_hash,previous_generation_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(generation_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(generation_ordinal)
    .bind(input.fence.snapshot_id)
    .bind(gate_decision_id)
    .bind(&input.expected_authority.candidate_snapshot_authority_hash)
    .bind(previous_generation_id)
    .execute(&mut *tx)
    .await?;
    let mut generation_member_hashes = Vec::new();
    for (ordinal, revision_id) in generation_revisions.iter().enumerate() {
        let member_hash = hash_json_on(
            &mut tx,
            &json!({"generation_id":generation_id,"revision_id":revision_id,"ordinal":ordinal}),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO hypothesis_generation_members(
                   generation_member_id,generation_id,operation_id,organization_id,
                   revision_id,ordinal,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(Uuid::new_v5(&generation_id, member_hash.as_bytes()))
        .bind(generation_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(revision_id)
        .bind(ordinal as i32)
        .bind(&member_hash)
        .execute(&mut *tx)
        .await?;
        generation_member_hashes.push(member_hash);
    }
    persist_generation_transitions_on(
        &mut tx,
        &input,
        generation_id,
        previous_generation_id,
        &previous_members,
        &generation_revisions,
        &pending,
    )
    .await?;
    let generation_member_set_hash = investigation_set_hash_on(
        &mut tx,
        "hypothesis_generation_members.v1",
        &generation_member_hashes,
    )
    .await?;
    let event_hashes: Vec<String> = sqlx::query_scalar(
        "SELECT event_hash FROM attack_hypothesis_state_events WHERE event_id=ANY($1) ORDER BY event_hash",
    )
    .bind(&state_event_ids)
    .fetch_all(&mut *tx)
    .await?;
    let generation_event_set_hash =
        investigation_set_hash_on(&mut tx, "hypothesis_generation_events.v1", &event_hashes)
            .await?;
    let open_obligation_set_hash: String = sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(to_jsonb(COALESCE(array_agg(obligation_hash ORDER BY obligation_hash),ARRAY[]::TEXT[]))::TEXT)
             FROM candidate_analysis_enrichment_obligations WHERE snapshot_id=$1"#,
    )
    .bind(input.fence.snapshot_id)
    .fetch_one(&mut *tx)
    .await?;
    let generation_hash = hash_json_on(
        &mut tx,
        &json!({"generation":generation_id,"members":generation_member_set_hash,
                "events":generation_event_set_hash,"obligations":open_obligation_set_hash}),
    )
    .await?;
    let generation_seal_id = Uuid::new_v5(&generation_id, b"hypothesis_generation_seal.v1");
    sqlx::query(
        r#"INSERT INTO hypothesis_generation_seals(
               seal_id,generation_id,member_count,member_set_hash,event_count,event_set_hash,
               open_obligation_set_hash,controller_worker_run_id,generation_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(generation_seal_id)
    .bind(generation_id)
    .bind(generation_revisions.len() as i64)
    .bind(&generation_member_set_hash)
    .bind(state_event_ids.len() as i64)
    .bind(&generation_event_set_hash)
    .bind(&open_obligation_set_hash)
    .bind(input.final_submitter_worker_run_id)
    .bind(&generation_hash)
    .execute(&mut *tx)
    .await?;

    let rollout_mode: String = sqlx::query_scalar(
        "SELECT investigation_rollout_mode FROM operation_state WHERE operation_id=$1 FOR SHARE",
    )
    .bind(input.fence.operation_id)
    .fetch_one(&mut *tx)
    .await?;
    let residual_spec = if input.mutations.is_empty() {
        Some((
            "candidate_true_zero_proposal_closeout",
            json!({"route":"reporting","verification":"not_applicable_true_zero"}),
        ))
    } else if matches!(
        rollout_mode.as_str(),
        "registry_authoritative_legacy_projection" | "new_only"
    ) {
        // Plan C is installed for authoritative-new operations.  The sealed
        // generation/plan is the scheduler handoff; emitting the historical
        // `plan_c_verification_unavailable` residual here would permanently
        // skip Verification and falsely route straight to Reporting.
        None
    } else {
        Some((
            "plan_c_verification_unavailable",
            json!({"route":"reporting","verification":"not_available_plan_c"}),
        ))
    };
    let post_seal_route = match residual_spec.as_ref().map(|(reason, _)| *reason) {
        Some("candidate_true_zero_proposal_closeout") => "true_zero_reporting",
        Some("plan_c_verification_unavailable") => "historical_reporting_placeholder",
        Some(_) => "invalid",
        None => "verification_campaign_admission",
    };
    let residual = if let Some((residual_reason, residual_route)) = residual_spec {
        let residual_id = Uuid::new_v5(&generation_id, residual_reason.as_bytes());
        let residual_hash = hash_json_on(
            &mut tx,
            &json!({"generation":generation_id,"reason":residual_reason}),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO hypothesis_residual_risks(
                   residual_id,operation_id,organization_id,snapshot_id,reason_code,
                   owner_kind,next_action,residual_hash
               ) VALUES($1,$2,$3,$4,$5,'candidate_analysis',$6,$7)"#,
        )
        .bind(residual_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(input.fence.snapshot_id)
        .bind(residual_reason)
        .bind(residual_route)
        .bind(&residual_hash)
        .execute(&mut *tx)
        .await?;
        Some((residual_id, residual_hash, residual_reason))
    } else {
        None
    };

    let occurred_at: DateTime<Utc> = sqlx::query_scalar("SELECT statement_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    let outbox_batch_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        b"candidate_projection_batch.v1",
    );
    let outbox_members = build_projection_members(
        &mut tx,
        &input,
        gate_decision_id,
        &gate_decision_hash,
        generation_id,
        &pending,
        &state_event_ids,
        residual
            .as_ref()
            .map(|(residual_id, residual_hash, residual_reason)| {
                (residual_id, residual_hash.as_str(), *residual_reason)
            }),
        occurred_at,
    )
    .await?;
    let project_scope_id: Option<Uuid> =
        sqlx::query_scalar("SELECT project_scope_id FROM operation_state WHERE operation_id=$1")
            .bind(input.fence.operation_id)
            .fetch_one(&mut *tx)
            .await?;
    let outbox = append_projection_source_batch_on(
        &mut tx,
        AppendProjectionSourceBatchRow {
            batch_id: outbox_batch_id,
            operation_id: input.fence.operation_id,
            project_scope_id,
            stable_request_id: input.stable_apply_request_id,
            source_transaction_id: gate_decision_id,
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            members: outbox_members,
        },
    )
    .await?;
    let apply_receipt_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        b"candidate_canonical_apply_receipt.v1",
    );
    let mut apply_member_hashes = Vec::with_capacity(pending.len());
    for mutation in &pending {
        apply_member_hashes.push(
            hash_json_on(
                &mut tx,
                &json!({
                    "revision_id":mutation.revision_id,
                    "revision_hash":mutation.revision_hash,
                }),
            )
            .await?,
        );
    }
    let apply_revision_set_hash =
        candidate_exact_set_hash("candidate_apply_revisions.v1", &apply_member_hashes);
    let apply_receipt_hash = hash_json_on(
        &mut tx,
        &json!({
            "gate_decision_id":gate_decision_id,
            "gate_decision_hash":gate_decision_hash,
            "generation_id":generation_id,
            "generation_hash":generation_hash,
            "generation_seal_id":generation_seal_id,
            "projection_outbox_batch_id":outbox.batch_id,
            "projection_outbox_member_set_hash":outbox.member_set_hash,
            "revision_set_hash":apply_revision_set_hash,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO hypothesis_candidate_canonical_apply_receipts(
               apply_receipt_id,stable_request_id,operation_id,organization_id,
               analysis_attempt_id,candidate_gate_decision_id,generation_id,
               generation_seal_id,projection_outbox_batch_id,revision_count,
               revision_set_hash,receipt_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
    )
    .bind(apply_receipt_id)
    .bind(input.stable_apply_request_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(input.active_analysis_attempt_id)
    .bind(gate_decision_id)
    .bind(generation_id)
    .bind(generation_seal_id)
    .bind(outbox.batch_id)
    .bind(i64::try_from(pending.len()).unwrap_or(i64::MAX))
    .bind(&apply_revision_set_hash)
    .bind(&apply_receipt_hash)
    .execute(&mut *tx)
    .await?;
    for (ordinal, (mutation, member_hash)) in pending.iter().zip(apply_member_hashes).enumerate() {
        sqlx::query(
            r#"INSERT INTO hypothesis_candidate_canonical_apply_receipt_members(
                   apply_receipt_member_id,apply_receipt_id,operation_id,organization_id,
                   revision_id,ordinal,revision_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(Uuid::new_v5(
            &apply_receipt_id,
            mutation.revision_id.as_bytes(),
        ))
        .bind(apply_receipt_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(mutation.revision_id)
        .bind(i32::try_from(ordinal).unwrap_or(i32::MAX))
        .bind(&mutation.revision_hash)
        .bind(member_hash)
        .execute(&mut *tx)
        .await?;
    }
    let predecessor_event_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT attempt_event_id FROM candidate_analysis_attempt_state_events WHERE analysis_attempt_id=$1 ORDER BY event_ordinal DESC LIMIT 1 FOR SHARE",
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_optional(&mut *tx)
    .await?;
    let next_event_ordinal: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(event_ordinal)+1,0) FROM candidate_analysis_attempt_state_events WHERE analysis_attempt_id=$1",
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_one(&mut *tx)
    .await?;
    let attempt_event_hash = hash_json_on(
        &mut tx,
        &json!({"attempt":input.active_analysis_attempt_id,"kind":"sealed","gate":gate_decision_hash}),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO candidate_analysis_attempt_state_events(
               attempt_event_id,analysis_attempt_id,event_ordinal,event_kind,
               predecessor_event_id,event_hash
           ) VALUES($1,$2,$3,'sealed',$4,$5)"#,
    )
    .bind(Uuid::new_v5(
        &input.active_analysis_attempt_id,
        attempt_event_hash.as_bytes(),
    ))
    .bind(input.active_analysis_attempt_id)
    .bind(next_event_ordinal)
    .bind(predecessor_event_id)
    .bind(attempt_event_hash)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(CandidateGenerationSealRowView {
        operation_id: input.fence.operation_id,
        scope_snapshot_id: input.fence.scope_snapshot_id,
        organization_id: input.fence.organization_id,
        snapshot_id: input.fence.snapshot_id,
        analysis_attempt_id: input.active_analysis_attempt_id,
        generation_id,
        generation_ordinal,
        generation_seal_id,
        generation_member_count: generation_revisions.len() as i64,
        generation_member_set_hash,
        generation_event_set_hash,
        open_obligation_set_hash,
        projection_outbox_batch_id: outbox.batch_id,
        projection_source_batch_seq: outbox.source_batch_seq,
        projection_outbox_member_set_hash: outbox.member_set_hash,
        post_seal_route: post_seal_route.to_string(),
        replayed: false,
    })
}

#[derive(sqlx::FromRow)]
struct PersistedGateDecision {
    decision_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    candidate_snapshot_id: Uuid,
    analysis_attempt_id: Uuid,
    mutation_set_hash: String,
    gate_authority_hash: String,
    decision_hash: String,
}

async fn load_apply_replay_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
) -> Result<Option<CandidateGenerationSealRowView>> {
    let decision: Option<PersistedGateDecision> = sqlx::query_as(
        r#"SELECT decision_id,operation_id,organization_id,candidate_snapshot_id,
                      analysis_attempt_id,mutation_set_hash,gate_authority_hash,decision_hash
                 FROM hypothesis_candidate_gate_decisions
                WHERE stable_request_id=$1"#,
    )
    .bind(input.stable_apply_request_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(decision) = decision else {
        return Ok(None);
    };
    let expected_decision_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        b"candidate_gate_decision.v1",
    );
    let expected_gate_authority_hash = gate_authority_hash_on(tx, input).await?;
    let expected_decision_hash = hash_json_on(
        tx,
        &json!({
            "gate_authority_hash":expected_gate_authority_hash,
            "mutation_set_hash":input.mutation_set_hash,
        }),
    )
    .await?;
    if decision.decision_id != expected_decision_id
        || decision.operation_id != input.fence.operation_id
        || decision.organization_id != input.fence.organization_id
        || decision.candidate_snapshot_id != input.fence.snapshot_id
        || decision.analysis_attempt_id != input.active_analysis_attempt_id
        || decision.mutation_set_hash != input.mutation_set_hash
        || decision.gate_authority_hash != expected_gate_authority_hash
        || decision.decision_hash != expected_decision_hash
    {
        return Err(conflict(APPLY_REPLAY_DRIFT));
    }
    let batch: Option<(Uuid, i64, String, Uuid)> = sqlx::query_as(
        r#"SELECT batch_id,source_batch_seq,member_set_hash,source_transaction_id
             FROM investigation_projection_outbox_batches
            WHERE operation_id=$1 AND stable_request_id=$2
            ORDER BY source_batch_seq LIMIT 1"#,
    )
    .bind(input.fence.operation_id)
    .bind(input.stable_apply_request_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((batch_id, source_seq, outbox_hash, source_transaction_id)) = batch else {
        return Err(conflict(APPLY_REPLAY_DRIFT));
    };
    if source_transaction_id != decision.decision_id {
        return Err(conflict(APPLY_REPLAY_DRIFT));
    }
    let row: (Uuid, i32, Uuid, i64, String, String, String) = sqlx::query_as(
        r#"SELECT generation.generation_id,generation.generation_ordinal,seal.seal_id,
                  seal.member_count,seal.member_set_hash,seal.event_set_hash,
                  seal.open_obligation_set_hash
             FROM hypothesis_generations generation
             JOIN hypothesis_generation_seals seal ON seal.generation_id=generation.generation_id
            WHERE generation.candidate_gate_decision_id=$1
              AND generation.candidate_snapshot_id=$2
              AND generation.operation_id=$3
              AND generation.organization_id=$4"#,
    )
    .bind(decision.decision_id)
    .bind(input.fence.snapshot_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    let residual_reason: Option<String> = sqlx::query_scalar(
        r#"SELECT residual.reason_code
             FROM hypothesis_residual_risks residual
            WHERE residual.operation_id=$1 AND residual.organization_id=$2
              AND residual.snapshot_id=$3
              AND residual.reason_code IN (
                  'candidate_true_zero_proposal_closeout',
                  'plan_c_verification_unavailable'
              )
            ORDER BY CASE residual.reason_code
                         WHEN 'candidate_true_zero_proposal_closeout' THEN 0 ELSE 1
                     END
            LIMIT 1"#,
    )
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(input.fence.snapshot_id)
    .fetch_optional(&mut **tx)
    .await?;
    let post_seal_route = match residual_reason.as_deref() {
        Some("candidate_true_zero_proposal_closeout") => "true_zero_reporting",
        Some("plan_c_verification_unavailable") => "historical_reporting_placeholder",
        Some(_) => return Err(conflict(APPLY_REPLAY_DRIFT)),
        None => "verification_campaign_admission",
    };
    Ok(Some(CandidateGenerationSealRowView {
        operation_id: input.fence.operation_id,
        scope_snapshot_id: input.fence.scope_snapshot_id,
        organization_id: input.fence.organization_id,
        snapshot_id: input.fence.snapshot_id,
        analysis_attempt_id: input.active_analysis_attempt_id,
        generation_id: row.0,
        generation_ordinal: row.1,
        generation_seal_id: row.2,
        generation_member_count: row.3,
        generation_member_set_hash: row.4,
        generation_event_set_hash: row.5,
        open_obligation_set_hash: row.6,
        projection_outbox_batch_id: batch_id,
        projection_source_batch_seq: source_seq,
        projection_outbox_member_set_hash: outbox_hash,
        post_seal_route: post_seal_route.to_string(),
        replayed: true,
    }))
}

async fn validate_apply_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
) -> Result<()> {
    let snapshot = sqlx::query_as::<_, LockedSnapshot>(
        r#"SELECT operation_id,organization_id,scope_snapshot_id,snapshot_status,
                  candidate_snapshot_authority_hash,tool_truth_authority_bundle_seal_id,
                  relevant_root_set_hash,bundle_member_set_hash,semantic_authority_bundle_hash,
                  freshness_attestation_bundle_hash,temporal_validity_bundle_hash,
                  temporal_validity_policy_set_hash,target_state_epoch_set_hash
             FROM candidate_analysis_snapshots WHERE snapshot_id=$1 FOR UPDATE"#,
    )
    .bind(input.fence.snapshot_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::NotFound("candidate_analysis_snapshot".into()))?;
    let frozen = load_snapshot_on(tx, input.fence.snapshot_id).await?;
    let gate_reevaluation =
        reevaluate_candidate_gate_authority_on(tx, input.fence.snapshot_id).await?;
    let ready = CandidateSnapshotDispositionRow::SealedReady;
    if snapshot.snapshot_status != "sealed_ready"
        || ready != CandidateSnapshotDispositionRow::SealedReady
        || snapshot.operation_id != input.fence.operation_id
        || snapshot.organization_id != input.fence.organization_id
        || snapshot.scope_snapshot_id != Some(input.fence.scope_snapshot_id)
        || frozen.snapshot_hash != input.expected_authority.snapshot_hash
        || snapshot.candidate_snapshot_authority_hash
            != input.expected_authority.candidate_snapshot_authority_hash
        || snapshot.tool_truth_authority_bundle_seal_id
            != input.expected_authority.tool_truth_authority_bundle_seal_id
        || snapshot.relevant_root_set_hash
            != input.expected_authority.tool_truth_authority_root_set_hash
        || snapshot.bundle_member_set_hash
            != input
                .expected_authority
                .tool_truth_authority_bundle_member_set_hash
        || snapshot.semantic_authority_bundle_hash
            != input.expected_authority.semantic_authority_bundle_hash
        || snapshot.freshness_attestation_bundle_hash
            != input.expected_authority.freshness_attestation_bundle_hash
        || snapshot.temporal_validity_bundle_hash
            != input.expected_authority.temporal_validity_bundle_hash
        || snapshot.temporal_validity_policy_set_hash
            != input.expected_authority.temporal_validity_policy_set_hash
        || snapshot.target_state_epoch_set_hash
            != input.expected_authority.target_state_epoch_set_hash
        || frozen.tool_truth_authority_receipt_set_hash
            != input
                .expected_authority
                .tool_truth_authority_receipt_set_hash
        || frozen.denominator_graph_bundle_hash
            != input.expected_authority.denominator_graph_bundle_hash
        || frozen.temporal_validity_decision_set_hash
            != input.expected_authority.temporal_validity_decision_set_hash
        || gate_reevaluation.temporal_hash
            != input.expected_authority.gate_temporal_reevaluation_hash
        || frozen.knowledge_feed_catalog_policy_seal_hash
            != input
                .expected_authority
                .knowledge_feed_catalog_policy_seal_hash
        || frozen.knowledge_feed_required_member_set_hash
            != input
                .expected_authority
                .knowledge_feed_required_member_set_hash
        || frozen.knowledge_feed_signature_algorithm_set_hash
            != input
                .expected_authority
                .knowledge_feed_signature_algorithm_set_hash
        || frozen.knowledge_feed_trust_store_hash
            != input.expected_authority.knowledge_feed_trust_store_hash
        || frozen.knowledge_feed_key_revocation_epoch_hash
            != input
                .expected_authority
                .knowledge_feed_key_revocation_epoch_hash
        || frozen.knowledge_feed_snapshot_set_hash
            != input.expected_authority.knowledge_feed_snapshot_set_hash
        || frozen.product_version_census_hash
            != input.expected_authority.product_version_census_hash
        || frozen.knowledge_feed_match_census_hash
            != input.expected_authority.knowledge_feed_match_census_hash
        || gate_reevaluation.knowledge_feed_hash
            != input
                .expected_authority
                .gate_knowledge_feed_reevaluation_hash
        || frozen.stale_revalidation_obligation_set_hash
            != input
                .expected_authority
                .stale_revalidation_obligation_set_hash
        || frozen.knowledge_feed_obligation_set_hash
            != input.expected_authority.knowledge_feed_obligation_set_hash
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let attempt_valid: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM candidate_analysis_attempts attempt
                WHERE attempt.analysis_attempt_id=$1 AND attempt.snapshot_id=$2
                  AND attempt.operation_id=$3 AND attempt.organization_id=$4
                  AND attempt.attempt_ordinal=$5
                  AND NOT EXISTS(
                      SELECT 1 FROM candidate_analysis_attempt_state_events terminal
                       WHERE terminal.analysis_attempt_id=attempt.analysis_attempt_id
                         AND terminal.event_kind IN ('superseded_missed_hypothesis','sealed','blocked')
                  ))"#,
    )
    .bind(input.active_analysis_attempt_id)
    .bind(input.fence.snapshot_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(input.active_analysis_attempt_ordinal)
    .fetch_one(&mut **tx)
    .await?;
    let (proposal_hash, critic_hash): (String, String) = sqlx::query_as(
        r#"SELECT proposal.census_hash,critic.census_hash
             FROM candidate_analysis_proposal_censuses proposal
             JOIN candidate_analysis_critic_censuses critic USING(analysis_attempt_id)
            WHERE proposal.analysis_attempt_id=$1"#,
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let prior_chain_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT event.event_hash
              FROM candidate_analysis_attempts attempt
              JOIN candidate_analysis_attempt_state_events event
                ON event.analysis_attempt_id=attempt.analysis_attempt_id
             WHERE attempt.snapshot_id=$1 AND attempt.attempt_ordinal<$2
               AND event.event_kind IN ('superseded_missed_hypothesis','sealed','blocked')
             ORDER BY attempt.attempt_ordinal,event.event_ordinal"#,
    )
    .bind(input.fence.snapshot_id)
    .bind(input.active_analysis_attempt_ordinal)
    .fetch_all(&mut **tx)
    .await?;
    let prior_chain_hash = hash_text_array_on(tx, &prior_chain_hashes).await?;

    let input_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM candidate_analysis_snapshot_inputs WHERE snapshot_id=$1",
    )
    .bind(input.fence.snapshot_id)
    .fetch_one(&mut **tx)
    .await?;
    let chunk_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT census_hash FROM candidate_analysis_input_chunk_censuses
            WHERE snapshot_id=$1 ORDER BY snapshot_input_id"#,
    )
    .bind(input.fence.snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let subreview_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT census_hash
              FROM candidate_analysis_hypothesis_coverage_subreview_censuses
             WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id"#,
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if input_count == 0
        || chunk_hashes.len() as i64 != input_count
        || subreview_hashes.len() as i64 != input_count
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let input_chunk_census_set_hash = hash_text_array_on(tx, &chunk_hashes).await?;
    let coverage_subreview_census_set_hash = hash_text_array_on(tx, &subreview_hashes).await?;
    let (coverage_synthesis_census_set_hash, global_node_id): (String, Uuid) = sqlx::query_as(
        r#"SELECT census_hash,global_root_node_id
                  FROM candidate_analysis_hypothesis_coverage_synthesis_censuses
                 WHERE analysis_attempt_id=$1"#,
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    let coverage_global_semantic_root_hash: String = sqlx::query_scalar(
        r#"SELECT node_hash
              FROM candidate_analysis_hypothesis_coverage_synthesis_census_members
             WHERE synthesis_node_id=$1 AND analysis_attempt_id=$2
               AND node_kind='global_semantic_root'"#,
    )
    .bind(global_node_id)
    .bind(input.active_analysis_attempt_id)
    .fetch_one(&mut **tx)
    .await?;
    let coverage_global_review_hash: String = sqlx::query_scalar(
        r#"SELECT review_hash
              FROM candidate_analysis_hypothesis_coverage_global_reviews
             WHERE analysis_attempt_id=$1"#,
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    let coverage_review_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT review_hash
              FROM candidate_analysis_hypothesis_coverage_reviews
             WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id"#,
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if coverage_review_hashes.len() as i64 != input_count {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let coverage_review_set_hash = hash_text_array_on(tx, &coverage_review_hashes).await?;
    let coverage_checklist_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT member_hash
              FROM candidate_analysis_hypothesis_coverage_checklist_members
             WHERE analysis_attempt_id=$1 ORDER BY snapshot_input_id,ordinal"#,
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    if coverage_checklist_hashes.is_empty() {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let coverage_checklist_set_hash = hash_text_array_on(tx, &coverage_checklist_hashes).await?;
    let controller_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT artifact_hash FROM candidate_analysis_artifacts
            WHERE analysis_attempt_id=$1 AND artifact_kind='controller_decision.v1'
            ORDER BY artifact_hash"#,
    )
    .bind(input.active_analysis_attempt_id)
    .fetch_all(&mut **tx)
    .await?;
    let controller_decision_set_hash = hash_text_array_on(tx, &controller_hashes).await?;
    let compiled_seal: Option<(String, String, String, String, String)> = sqlx::query_as(
        r#"SELECT mutation_set_hash,claim_component_set_hash,
                  verification_contract_set_hash,verification_plan_set_hash,
                  generation_transition_set_hash
             FROM candidate_analysis_host_compilation_seals
            WHERE analysis_attempt_id=$1
              AND snapshot_id=$2
              AND operation_id=$3
              AND organization_id=$4
              AND final_submitter_worker_run_id=$5"#,
    )
    .bind(input.active_analysis_attempt_id)
    .bind(input.fence.snapshot_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(input.final_submitter_worker_run_id)
    .fetch_optional(&mut **tx)
    .await?;
    let source_head_version: i64 = sqlx::query_scalar(
        "SELECT last_source_batch_seq FROM investigation_projection_source_heads WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(input.fence.operation_id)
    .fetch_one(&mut **tx)
    .await?;
    if !attempt_valid
        || input.active_analysis_attempt_id != input.fence.analysis_attempt_id
        || input.active_analysis_attempt_ordinal != input.fence.analysis_attempt_ordinal
        || proposal_hash != input.expected_authority.proposal_census_hash
        || critic_hash != input.expected_authority.critic_census_hash
        || prior_chain_hash != input.expected_authority.prior_terminal_attempt_chain_hash
        || input_chunk_census_set_hash != input.expected_authority.input_chunk_census_set_hash
        || coverage_subreview_census_set_hash
            != input.expected_authority.coverage_subreview_census_set_hash
        || coverage_synthesis_census_set_hash
            != input.expected_authority.coverage_synthesis_census_set_hash
        || coverage_global_semantic_root_hash
            != input.expected_authority.coverage_global_semantic_root_hash
        || coverage_global_review_hash != input.expected_authority.coverage_global_review_hash
        || coverage_review_set_hash != input.expected_authority.coverage_review_set_hash
        || coverage_checklist_set_hash != input.expected_authority.coverage_checklist_set_hash
        || controller_decision_set_hash != input.expected_authority.controller_decision_set_hash
        || compiled_seal
            != Some((
                input.mutation_set_hash.clone(),
                input.claim_component_set_hash.clone(),
                input.verification_contract_set_hash.clone(),
                input.verification_plan_set_hash.clone(),
                input
                    .expected_authority
                    .generation_transition_set_hash
                    .clone(),
            ))
        || source_head_version != input.expected_source_head_version
        || input.final_submitter_worker_run_id != input.fence.worker_run_id
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    Ok(())
}

#[derive(Debug)]
struct PreparedMutation {
    gate_ordinal: i32,
    proposal_id: Uuid,
    route_kind: &'static str,
    root_kind: &'static str,
    root_id: Uuid,
    predecessor_revision_id: Option<Uuid>,
    revision_id: Uuid,
    revision_ordinal: i32,
    semantic_key: Value,
    semantic_key_hash: String,
    proposal: AnalysisArtifactBodyRow,
    state: CandidateMutationEpistemicState,
    proof_refs: Vec<CandidateRevisionSourceRefRow>,
    refutation_refs: Vec<CandidateRevisionSourceRefRow>,
    origin_decision_hash: String,
    revision_ingredients_hash: String,
    revision_hash: String,
    member_hash: String,
    relation_sources: Vec<PreparedRelationSource>,
}

#[derive(Debug, Clone)]
struct PreparedRelationSource {
    root_id: Uuid,
    revision_id: Uuid,
    relation_kind: &'static str,
}

async fn prepare_mutation_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
    mutation: &CandidateMutationRow,
) -> Result<PreparedMutation> {
    #[derive(Debug, Deserialize)]
    struct PersistedProposal {
        proposal_id: Uuid,
        subject_kind: String,
        subject_identity_hash: String,
        predicate_schema: String,
        predicate_version: u32,
        predicate_arguments: Vec<(String, String)>,
        trust_boundary: String,
        polarity: String,
        structured_claim: String,
        proof_refs: Vec<Value>,
    }
    let body: Value = sqlx::query_scalar(
        r#"SELECT proposal.structured_proposal
             FROM hypothesis_proposals proposal
            WHERE proposal.proposal_id=$1 AND proposal.analysis_attempt_id=$2"#,
    )
    .bind(mutation.proposal_id)
    .bind(input.active_analysis_attempt_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| DbError::NotFound("hypothesis_proposal".into()))?;
    let persisted: PersistedProposal = serde_json::from_value(body)?;
    if persisted.proposal_id != mutation.proposal_id {
        return Err(conflict(MUTATION_SET_INVALID));
    }
    let mut predicate_arguments = serde_json::Map::new();
    for (key, value) in persisted.predicate_arguments {
        if predicate_arguments
            .insert(key, Value::String(value))
            .is_some()
        {
            return Err(conflict(MUTATION_SET_INVALID));
        }
    }
    let predicate = golish_core::hypothesis_semantic_key::PredicateIdentity::new(
        persisted.predicate_schema,
        persisted.predicate_version,
        Value::Object(predicate_arguments),
    )
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let polarity =
        golish_core::hypothesis_semantic_key::ClaimPolarity::try_from(persisted.polarity.as_str())
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let evidence_refs = persisted
        .proof_refs
        .iter()
        .filter_map(|reference| reference.get("source_hash").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let proposal = AnalysisArtifactBodyRow::HypothesisProposal {
        proposal_id: persisted.proposal_id,
        subject_kind: persisted.subject_kind,
        subject_identity_hash: persisted.subject_identity_hash,
        predicate,
        trust_boundary: persisted.trust_boundary,
        polarity,
        prose: persisted.structured_claim,
        confidence: 0,
        priority: 0,
        tags: Vec::new(),
        evidence_refs,
    };
    let AnalysisArtifactBodyRow::HypothesisProposal {
        subject_kind,
        subject_identity_hash,
        predicate,
        trust_boundary,
        polarity,
        ..
    } = &proposal
    else {
        unreachable!("constructed hypothesis proposal variant")
    };
    let semantic_key = HypothesisSemanticKeyV1::new(
        input.fence.organization_id,
        AtTimeSubjectIdentity::new(subject_kind.clone(), subject_identity_hash.clone())
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
        predicate.clone(),
        trust_boundary.clone(),
        *polarity,
    )
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let semantic_key_hash = semantic_key
        .hash()
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    if semantic_key_hash != mutation.semantic_key_hash {
        return Err(conflict(MUTATION_SET_INVALID));
    }
    let semantic_key_body = serde_json::to_value(&semantic_key)?;
    let (route_kind, root_kind, root_id, predecessor_revision_id) = match &mutation.route {
        CandidateMutationRouteRow::CreateInitial { root_id }
            if *root_id
                == initial_root_id(input.fence.operation_id, &semantic_key)
                    .map_err(|error| DbError::Other(anyhow::Error::new(error)))? =>
        {
            ("create_initial", "initial", *root_id, None)
        }
        CandidateMutationRouteRow::ReopenHistorical {
            root_id,
            predecessor_revision_id,
        } => (
            "reopen_historical",
            "initial",
            *root_id,
            Some(*predecessor_revision_id),
        ),
        CandidateMutationRouteRow::Split {
            parent_root_id,
            child_root_id,
        } if *child_root_id
            == split_root_id(input.fence.operation_id, &semantic_key, *parent_root_id)
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))? =>
        {
            ("split", "split", *child_root_id, None)
        }
        CandidateMutationRouteRow::Merge {
            parent_root_ids,
            successor_root_id,
        } if parent_root_ids.len() >= 2
            && *successor_root_id
                == merge_root_id(input.fence.operation_id, &semantic_key, parent_root_ids)
                    .map_err(|error| DbError::Other(anyhow::Error::new(error)))? =>
        {
            ("merge", "merge", *successor_root_id, None)
        }
        CandidateMutationRouteRow::Derive {
            source_root_id,
            source_revision_id,
            derivation_rule_hash,
            successor_root_id,
        } if *successor_root_id
            == derive_root_id(
                input.fence.operation_id,
                &semantic_key,
                *source_root_id,
                *source_revision_id,
                derivation_rule_hash,
            )
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))? =>
        {
            ("derive", "derive", *successor_root_id, None)
        }
        CandidateMutationRouteRow::NarrowSuccessor {
            source_root_id,
            source_revision_id,
            successor_root_id,
            covered_claim_component_set_hash,
        } if *successor_root_id
            == derive_root_id(
                input.fence.operation_id,
                &semantic_key,
                *source_root_id,
                *source_revision_id,
                covered_claim_component_set_hash,
            )
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))? =>
        {
            ("narrow_successor", "derive", *successor_root_id, None)
        }
        _ => return Err(conflict(MUTATION_SET_INVALID)),
    };
    let relation_sources = match &mutation.route {
        CandidateMutationRouteRow::Split { parent_root_id, .. } => {
            vec![load_current_relation_source_on(tx, input, *parent_root_id, "split").await?]
        }
        CandidateMutationRouteRow::Merge {
            parent_root_ids, ..
        } => {
            let mut sources = Vec::with_capacity(parent_root_ids.len());
            for parent_root_id in parent_root_ids {
                sources.push(
                    load_current_relation_source_on(tx, input, *parent_root_id, "merge").await?,
                );
            }
            sources.sort_by_key(|source| (source.root_id, source.revision_id));
            sources.dedup_by_key(|source| (source.root_id, source.revision_id));
            if sources.len() < 2 {
                return Err(conflict(MUTATION_SET_INVALID));
            }
            sources
        }
        CandidateMutationRouteRow::Derive {
            source_root_id,
            source_revision_id,
            ..
        } => {
            let source =
                load_current_relation_source_on(tx, input, *source_root_id, "derive").await?;
            if source.revision_id != *source_revision_id {
                return Err(conflict(MUTATION_SET_INVALID));
            }
            vec![source]
        }
        CandidateMutationRouteRow::NarrowSuccessor {
            source_root_id,
            source_revision_id,
            ..
        } => {
            let valid: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                       SELECT 1 FROM attack_hypothesis_revisions
                        WHERE revision_id=$1 AND root_id=$2 AND operation_id=$3
                          AND organization_id=$4 AND lifecycle_state='current')"#,
            )
            .bind(source_revision_id)
            .bind(source_root_id)
            .bind(input.fence.operation_id)
            .bind(input.fence.organization_id)
            .fetch_one(&mut **tx)
            .await?;
            if !valid {
                return Err(conflict(MUTATION_SET_INVALID));
            }
            vec![PreparedRelationSource {
                root_id: *source_root_id,
                revision_id: *source_revision_id,
                relation_kind: "refine",
            }]
        }
        _ => Vec::new(),
    };
    let revision_ordinal: i32 = if let Some(predecessor) = predecessor_revision_id {
        sqlx::query_scalar(
            "SELECT revision_ordinal+1 FROM attack_hypothesis_revisions WHERE revision_id=$1 AND root_id=$2 FOR SHARE",
        )
        .bind(predecessor)
        .bind(root_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| conflict(MUTATION_SET_INVALID))?
    } else {
        0
    };
    let origin_decision_hash = hash_json_on(
        tx,
        &json!({
            "proposal_id":mutation.proposal_id,"route_kind":route_kind,"root_id":root_id,
            "predecessor_revision_id":predecessor_revision_id,"semantic_key_hash":semantic_key_hash,
            "relation_sources":relation_sources.iter().map(|source| json!({
                "root_id":source.root_id,"revision_id":source.revision_id,
                "relation_kind":source.relation_kind,
            })).collect::<Vec<_>>(),
            "generation_transition_hash":mutation.generation_transition_hash,
            "successor_state":mutation.state.as_str(),
        }),
    )
    .await?;
    let revision_id = candidate_revision_id(
        root_id,
        u32::try_from(revision_ordinal).map_err(|_| conflict(MUTATION_SET_INVALID))?,
        &semantic_key_hash,
        &origin_decision_hash,
    )
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let revision_ingredients_hash = hash_json_on(
        tx,
        &json!({"proposal":proposal,"origin_decision_hash":origin_decision_hash}),
    )
    .await?;
    let revision_hash = hash_json_on(
        tx,
        &json!({
            "revision_id":revision_id,"root_id":root_id,"ordinal":revision_ordinal,
            "semantic_key_hash":semantic_key_hash,"state":mutation.state.as_str(),
            "ingredients":revision_ingredients_hash,
        }),
    )
    .await?;
    if !is_sha256(&mutation.mutation_hash) {
        return Err(conflict(MUTATION_SET_INVALID));
    }
    let member_hash = mutation.mutation_hash.clone();
    Ok(PreparedMutation {
        gate_ordinal: -1,
        proposal_id: mutation.proposal_id,
        route_kind,
        root_kind,
        root_id,
        predecessor_revision_id,
        revision_id,
        revision_ordinal,
        semantic_key: semantic_key_body,
        semantic_key_hash,
        proposal,
        state: mutation.state,
        proof_refs: mutation.proof_refs.clone(),
        refutation_refs: mutation.refutation_refs.clone(),
        origin_decision_hash,
        revision_ingredients_hash,
        revision_hash,
        member_hash,
        relation_sources,
    })
}

async fn load_current_relation_source_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
    root_id: Uuid,
    relation_kind: &'static str,
) -> Result<PreparedRelationSource> {
    let revision_id: Uuid = sqlx::query_scalar(
        r#"SELECT head_revision_id FROM attack_hypothesis_heads
            WHERE root_id=$1 AND operation_id=$2 AND organization_id=$3
              AND head_lifecycle_state='current' FOR SHARE"#,
    )
    .bind(root_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| conflict(MUTATION_SET_INVALID))?;
    Ok(PreparedRelationSource {
        root_id,
        revision_id,
        relation_kind,
    })
}

async fn persist_input_decisions_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
    revision_by_root: &BTreeMap<Uuid, Uuid>,
) -> Result<()> {
    let expected_inputs: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT snapshot_input_id FROM candidate_analysis_snapshot_inputs
            WHERE snapshot_id=$1 ORDER BY stable_input_key"#,
    )
    .bind(input.fence.snapshot_id)
    .fetch_all(&mut **tx)
    .await?;
    let observed_inputs = input
        .input_dispositions
        .iter()
        .map(|decision| decision.input_id)
        .collect::<std::collections::BTreeSet<_>>();
    let expected_input_set = expected_inputs
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if expected_inputs.is_empty()
        || observed_inputs != expected_input_set
        || input.input_dispositions.len() != expected_inputs.len()
    {
        return Err(conflict(MUTATION_SET_INVALID));
    }
    for decision in &input.input_dispositions {
        if !matches!(
            decision.disposition.as_str(),
            "analyzed"
                | "informational"
                | "duplicate_input"
                | "not_security_relevant"
                | "gap"
                | "blocked"
        ) || decision.reason_code.trim().is_empty()
        {
            return Err(conflict(MUTATION_SET_INVALID));
        }
        let disposition_hash = hash_json_on(
            tx,
            &json!({
                "analysis_attempt_id":input.active_analysis_attempt_id,
                "snapshot_input_id":decision.input_id,
                "disposition":decision.disposition,
                "reason_code":decision.reason_code,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO input_processing_dispositions(
                   input_disposition_id,analysis_attempt_id,snapshot_input_id,
                   disposition,reason_code,disposition_hash
               ) VALUES($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(Uuid::new_v5(
            &input.active_analysis_attempt_id,
            decision.input_id.as_bytes(),
        ))
        .bind(input.active_analysis_attempt_id)
        .bind(decision.input_id)
        .bind(&decision.disposition)
        .bind(&decision.reason_code)
        .bind(disposition_hash)
        .execute(&mut **tx)
        .await?;
    }
    let mut relation_keys = std::collections::BTreeSet::new();
    for relation in &input.input_relations {
        if !expected_input_set.contains(&relation.input_id)
            || !matches!(
                relation.relation_kind.as_str(),
                "creates_hypothesis"
                    | "supports_existing"
                    | "contradicts_existing"
                    | "qualifies_existing"
            )
        {
            return Err(conflict(MUTATION_SET_INVALID));
        }
        let revision_id = revision_by_root
            .get(&relation.root_id)
            .copied()
            .ok_or_else(|| conflict(MUTATION_SET_INVALID))?;
        if !relation_keys.insert((
            relation.input_id,
            revision_id,
            relation.relation_kind.clone(),
        )) {
            return Err(conflict(MUTATION_SET_INVALID));
        }
        let relation_hash = hash_json_on(
            tx,
            &json!({
                "analysis_attempt_id":input.active_analysis_attempt_id,
                "snapshot_input_id":relation.input_id,
                "revision_id":revision_id,
                "relation_kind":relation.relation_kind,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO input_hypothesis_relations(
                   input_hypothesis_relation_id,analysis_attempt_id,snapshot_input_id,
                   revision_id,relation_kind,relation_hash
               ) VALUES($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(Uuid::new_v5(
            &input.active_analysis_attempt_id,
            relation_hash.as_bytes(),
        ))
        .bind(input.active_analysis_attempt_id)
        .bind(relation.input_id)
        .bind(revision_id)
        .bind(&relation.relation_kind)
        .bind(relation_hash)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn persist_generation_transitions_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
    generation_id: Uuid,
    previous_generation_id: Option<Uuid>,
    previous_members: &[(Uuid, Uuid)],
    generation_revisions: &[Uuid],
    pending: &[PreparedMutation],
) -> Result<()> {
    let Some(previous_generation_id) = previous_generation_id else {
        if previous_members.is_empty() {
            return Ok(());
        }
        return Err(conflict(MUTATION_SET_INVALID));
    };
    let current = generation_revisions
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for (previous_member_id, previous_revision_id) in previous_members {
        let mut successors = pending
            .iter()
            .filter(|mutation| matches!(mutation.route_kind, "split" | "merge"))
            .filter(|mutation| {
                mutation
                    .relation_sources
                    .iter()
                    .any(|source| source.revision_id == *previous_revision_id)
            })
            .map(|mutation| mutation.revision_id)
            .collect::<Vec<_>>();
        successors.sort_unstable();
        successors.dedup();
        let disposition = if successors.is_empty() && current.contains(previous_revision_id) {
            "unchanged"
        } else if !successors.is_empty() && !current.contains(previous_revision_id) {
            "successor"
        } else {
            return Err(conflict(MUTATION_SET_INVALID));
        };
        let transition_hash = hash_json_on(
            tx,
            &json!({
                "generation_id":generation_id,
                "previous_generation_id":previous_generation_id,
                "previous_generation_member_id":previous_member_id,
                "previous_revision_id":previous_revision_id,
                "disposition":disposition,
                "successor_revision_ids":successors,
            }),
        )
        .await?;
        let transition_id = Uuid::new_v5(&generation_id, previous_member_id.as_bytes());
        sqlx::query(
            r#"INSERT INTO hypothesis_generation_transitions(
                   transition_id,generation_id,operation_id,organization_id,
                   previous_generation_id,previous_generation_member_id,
                   previous_revision_id,disposition,transition_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(transition_id)
        .bind(generation_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(previous_generation_id)
        .bind(previous_member_id)
        .bind(previous_revision_id)
        .bind(disposition)
        .bind(transition_hash)
        .execute(&mut **tx)
        .await?;
        for (ordinal, successor_revision_id) in successors.iter().enumerate() {
            let member_hash = hash_json_on(
                tx,
                &json!({
                    "transition_id":transition_id,
                    "successor_revision_id":successor_revision_id,
                    "ordinal":ordinal,
                }),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO hypothesis_generation_transition_successors(
                       successor_id,transition_id,operation_id,organization_id,
                       successor_revision_id,ordinal,member_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
            )
            .bind(Uuid::new_v5(
                &transition_id,
                successor_revision_id.as_bytes(),
            ))
            .bind(transition_id)
            .bind(input.fence.operation_id)
            .bind(input.fence.organization_id)
            .bind(successor_revision_id)
            .bind(i32::try_from(ordinal).map_err(|_| conflict(MUTATION_SET_INVALID))?)
            .bind(member_hash)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

async fn persist_mutation_compound_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
    gate_decision_id: Uuid,
    gate_ordinal: i32,
    mutation: &PreparedMutation,
) -> Result<Uuid> {
    let identity_ingredients = json!({
        "root_kind":mutation.root_kind,"semantic_key_hash":mutation.semantic_key_hash,
        "route_kind":mutation.route_kind,
    });
    let identity_hash = hash_json_on(tx, &identity_ingredients).await?;
    if mutation.predecessor_revision_id.is_none() {
        sqlx::query(
            r#"INSERT INTO attack_hypotheses(
                   root_id,operation_id,organization_id,root_kind,
                   identity_ingredients,identity_ingredients_hash
               ) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(root_id) DO NOTHING"#,
        )
        .bind(mutation.root_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(mutation.root_kind)
        .bind(identity_ingredients)
        .bind(&identity_hash)
        .execute(&mut **tx)
        .await?;
        let retained: (Uuid, Uuid, String) = sqlx::query_as(
            "SELECT operation_id,organization_id,identity_ingredients_hash FROM attack_hypotheses WHERE root_id=$1 FOR SHARE",
        )
        .bind(mutation.root_id)
        .fetch_one(&mut **tx)
        .await?;
        if retained
            != (
                input.fence.operation_id,
                input.fence.organization_id,
                identity_hash,
            )
        {
            return Err(conflict("ROOT_ID_COLLISION"));
        }
    }
    let AnalysisArtifactBodyRow::HypothesisProposal {
        subject_kind,
        subject_identity_hash,
        predicate,
        trust_boundary,
        polarity,
        prose,
        confidence,
        priority,
        tags,
        evidence_refs,
        ..
    } = &mutation.proposal
    else {
        return Err(conflict(MUTATION_SET_INVALID));
    };
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,predecessor_revision_id,
               revision_ordinal,semantic_key,semantic_key_hash,subject_kind,
               subject_identity_hash,target_type_at_time,target_value_at_time,
               predicate_schema,predicate_version,normalized_arguments,trust_boundary,
               polarity,epistemic_state,lifecycle_state,planning_readiness,structured_claim,
               assumptions,missing_facts,priority,risk_impact,origin_decision_hash,
               revision_ingredients_hash,revision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'subject_identity_hash',$10,$11,$12,
                    $13,$14,$15,$16,'current','ready_for_strategy',$17,'[]','[]',$18,$19,$20,$21,$22)"#,
    )
    .bind(mutation.revision_id)
    .bind(mutation.root_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(mutation.predecessor_revision_id)
    .bind(mutation.revision_ordinal)
    .bind(&mutation.semantic_key)
    .bind(&mutation.semantic_key_hash)
    .bind(subject_kind)
    .bind(subject_identity_hash)
    .bind(predicate.schema())
    .bind(i32::try_from(predicate.version()).unwrap_or(i32::MAX))
    .bind(predicate.normalized_arguments().as_value())
    .bind(trust_boundary)
    .bind(polarity.as_str())
    .bind(mutation.state.as_str())
    .bind(json!({
        "prose":prose,
        "confidence":confidence,
        "tags":tags,
        "proposal_evidence_refs":evidence_refs,
        "proof_refs":mutation.proof_refs,
        "refutation_refs":mutation.refutation_refs,
    }))
    .bind(*priority)
    .bind(json!({"impact":"candidate_unscored"}))
    .bind(&mutation.origin_decision_hash)
    .bind(&mutation.revision_ingredients_hash)
    .bind(&mutation.revision_hash)
    .execute(&mut **tx)
    .await?;

    persist_compiled_authorities_for_revision_on(
        tx,
        &input.claim_components,
        &input.verification_contracts,
        &input.verification_plans,
        mutation.revision_id,
        &mutation.revision_hash,
        &mutation.revision_ingredients_hash,
    )
    .await?;
    if mutation.predecessor_revision_id.is_none() {
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_heads(
                   root_id,operation_id,organization_id,head_revision_id,head_revision_hash,
                   head_semantic_key_hash,head_epistemic_state,head_lifecycle_state
               ) VALUES($1,$2,$3,$4,$5,$6,$7,'current')"#,
        )
        .bind(mutation.root_id)
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(mutation.revision_id)
        .bind(&mutation.revision_hash)
        .bind(&mutation.semantic_key_hash)
        .bind(mutation.state.as_str())
        .execute(&mut **tx)
        .await?;
    } else {
        let advanced = sqlx::query(
            r#"UPDATE attack_hypothesis_heads SET head_revision_id=$2,head_revision_hash=$3,
                   head_semantic_key_hash=$4,head_epistemic_state=$5,head_lifecycle_state='current',
                   head_version=head_version+1
                WHERE root_id=$1 AND head_revision_id=$6 AND head_lifecycle_state='current'"#,
        )
        .bind(mutation.root_id)
        .bind(mutation.revision_id)
        .bind(&mutation.revision_hash)
        .bind(&mutation.semantic_key_hash)
        .bind(mutation.state.as_str())
        .bind(mutation.predecessor_revision_id)
        .execute(&mut **tx)
        .await?;
        if advanced.rows_affected() != 1 {
            return Err(conflict(MUTATION_SET_INVALID));
        }
    }
    for source in &mutation.relation_sources {
        let relation_hash = hash_json_on(
            tx,
            &json!({
                "source_root_id":source.root_id,
                "source_revision_id":source.revision_id,
                "target_root_id":mutation.root_id,
                "target_revision_id":mutation.revision_id,
                "relation_kind":source.relation_kind,
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_relations(
                   relation_id,operation_id,organization_id,source_root_id,
                   source_revision_id,target_root_id,target_revision_id,
                   relation_kind,relation_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
        )
        .bind(Uuid::new_v5(
            &mutation.revision_id,
            relation_hash.as_bytes(),
        ))
        .bind(input.fence.operation_id)
        .bind(input.fence.organization_id)
        .bind(source.root_id)
        .bind(source.revision_id)
        .bind(mutation.root_id)
        .bind(mutation.revision_id)
        .bind(source.relation_kind)
        .bind(relation_hash)
        .execute(&mut **tx)
        .await?;
    }
    let mutation_id = Uuid::new_v5(&gate_decision_id, mutation.member_hash.as_bytes());
    sqlx::query(
        r#"INSERT INTO hypothesis_candidate_gate_decision_members(
               mutation_id,decision_id,operation_id,organization_id,ordinal,route_kind,
               root_id,predecessor_revision_id,successor_revision_id,semantic_key_hash,
               successor_epistemic_state,origin_decision_hash,
               generation_transition_hash,member_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(mutation_id)
    .bind(gate_decision_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(gate_ordinal)
    .bind(mutation.route_kind)
    .bind(mutation.root_id)
    .bind(mutation.predecessor_revision_id)
    .bind(mutation.revision_id)
    .bind(&mutation.semantic_key_hash)
    .bind(mutation.state.as_str())
    .bind(&mutation.origin_decision_hash)
    .bind(
        input
            .mutations
            .get(usize::try_from(gate_ordinal).map_err(|_| conflict(MUTATION_SET_INVALID))?)
            .ok_or_else(|| conflict(MUTATION_SET_INVALID))?
            .generation_transition_hash
            .as_str(),
    )
    .bind(&mutation.member_hash)
    .execute(&mut **tx)
    .await?;
    let event_kind = match mutation.state {
        CandidateMutationEpistemicState::Proposed => "created",
        CandidateMutationEpistemicState::Supported => "supported",
        CandidateMutationEpistemicState::Contested => "contested",
        CandidateMutationEpistemicState::Inconclusive => "inconclusive",
    };
    let event_id = Uuid::new_v5(&mutation.revision_id, b"candidate_creating_event.v1");
    let event_hash = hash_json_on(
        tx,
        &json!({"revision":mutation.revision_id,"event_kind":event_kind,"mutation":mutation_id}),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_state_events(
               event_id,operation_id,organization_id,root_id,predecessor_revision_id,
               successor_revision_id,event_kind,origin_authority,successor_epistemic_state,
               authority_receipt_kind,authority_receipt_id,authority_receipt_hash,
               event_hash,server_decision_id,server_decision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,'candidate_analysis',$8,
                    'candidate_gate_decision',$9,$10,$11,$9,$10)"#,
    )
    .bind(event_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .bind(mutation.root_id)
    .bind(mutation.predecessor_revision_id)
    .bind(mutation.revision_id)
    .bind(event_kind)
    .bind(mutation.state.as_str())
    .bind(mutation_id)
    .bind(&mutation.origin_decision_hash)
    .bind(event_hash)
    .execute(&mut **tx)
    .await?;
    Ok(event_id)
}

pub(super) async fn persist_compiled_authorities_for_revision_on(
    tx: &mut Transaction<'_, Postgres>,
    claim_components: &[HypothesisClaimComponentV1],
    verification_contracts: &[VerificationContractV1],
    verification_plans: &[HypothesisVerificationPlanV1],
    revision_id: Uuid,
    revision_hash: &str,
    revision_ingredients_hash: &str,
) -> Result<()> {
    let components = claim_components
        .iter()
        .filter(|component| component.revision_id() == revision_id)
        .collect::<Vec<_>>();
    let contracts = verification_contracts
        .iter()
        .filter(|contract| contract.revision_id() == revision_id)
        .collect::<Vec<_>>();
    let plan = verification_plans
        .iter()
        .find(|plan| plan.revision_id() == revision_id)
        .ok_or_else(|| conflict(COMPILED_AUTHORITY_INCOMPLETE))?;
    if components.is_empty()
        || contracts.is_empty()
        || plan.revision_hash() != revision_hash
        || plan.revision_ingredients_hash() != revision_ingredients_hash
    {
        return Err(conflict(COMPILED_AUTHORITY_INCOMPLETE));
    }
    let mut component_ids = BTreeMap::new();
    for component in components {
        if component.revision_hash() != revision_hash {
            return Err(conflict(COMPILED_AUTHORITY_INCOMPLETE));
        }
        let component_id = Uuid::new_v5(&revision_id, component.member_hash().as_bytes());
        component_ids.insert(component.member_hash().to_owned(), component_id);
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_claim_components(
                   component_id,revision_id,revision_hash,component_ordinal,component_key,kind,
                   canonical_fragment_hash,canonical_condition_hash,required,
                   derivation_contract_version,derivation_contract_digest,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
        )
        .bind(component_id)
        .bind(revision_id)
        .bind(revision_hash)
        .bind(component.component_ordinal() as i32)
        .bind(component.component_key())
        .bind(component.kind().as_str())
        .bind(component.canonical_fragment_hash())
        .bind(component.canonical_condition_hash())
        .bind(component.required())
        .bind(component.derivation_contract_version() as i32)
        .bind(component.derivation_contract_digest())
        .bind(component.member_hash())
        .execute(&mut **tx)
        .await?;
    }

    let mut plan_objective_by_hash = BTreeMap::new();
    for plan_objective in plan.objectives() {
        let contract = contracts
            .iter()
            .find(|contract| contract.contract_id() == plan_objective.verification_contract_id())
            .ok_or_else(|| conflict(COMPILED_AUTHORITY_INCOMPLETE))?;
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_objectives(
                   objective_id,revision_id,objective_ordinal,objective_intent,
                   stopping_criteria,stopping_criteria_hash,objective_hash
               ) VALUES($1,$2,$3,'{}','{}',$4,$5)"#,
        )
        .bind(contract.objective_id())
        .bind(revision_id)
        .bind(plan_objective_by_hash.len() as i32)
        .bind(contract.stopping_criteria_hash())
        .bind(plan_objective.objective_hash())
        .execute(&mut **tx)
        .await?;
        persist_contract_on(tx, contract).await?;
        for (ordinal, component_hash) in plan_objective
            .claim_component_member_hashes()
            .iter()
            .enumerate()
        {
            let component_id = component_ids
                .get(component_hash)
                .copied()
                .ok_or_else(|| conflict(COMPILED_AUTHORITY_INCOMPLETE))?;
            let binding_hash = hash_json_on(
                tx,
                &json!({"contract":contract.contract_id(),"component":component_hash,"ordinal":ordinal}),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO attack_hypothesis_verification_objective_claim_components(
                       binding_id,contract_id,revision_id,objective_id,claim_component_id,
                       ordinal,component_member_hash,binding_member_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
            )
            .bind(Uuid::new_v5(
                &contract.contract_id(),
                binding_hash.as_bytes(),
            ))
            .bind(contract.contract_id())
            .bind(revision_id)
            .bind(contract.objective_id())
            .bind(component_id)
            .bind(ordinal as i32)
            .bind(component_hash)
            .bind(binding_hash)
            .execute(&mut **tx)
            .await?;
        }
        let plan_objective_id =
            Uuid::new_v5(&plan.plan_id(), plan_objective.member_hash().as_bytes());
        plan_objective_by_hash.insert(plan_objective.member_hash().to_owned(), plan_objective_id);
    }

    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_plans(
               plan_id,revision_id,revision_hash,revision_ingredients_hash,
               required_claim_component_count,required_claim_component_set_hash,
               objective_count,objective_set_hash,proof_path_count,proof_path_set_hash,
               outer_aggregation_policy_version,outer_aggregation_policy_digest,
               plan_hash,sealed_at
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,statement_timestamp())"#,
    )
    .bind(plan.plan_id())
    .bind(revision_id)
    .bind(plan.revision_hash())
    .bind(plan.revision_ingredients_hash())
    .bind(plan.required_claim_component_count() as i64)
    .bind(plan.required_claim_component_set_hash())
    .bind(plan.objective_count() as i64)
    .bind(plan.objective_set_hash())
    .bind(plan.proof_path_count() as i64)
    .bind(plan.proof_path_set_hash())
    .bind(plan.outer_aggregation_policy_version() as i32)
    .bind(plan.outer_aggregation_policy_digest())
    .bind(plan.plan_hash())
    .execute(&mut **tx)
    .await?;
    for (ordinal, objective) in plan.objectives().iter().enumerate() {
        let plan_objective_id = plan_objective_by_hash[objective.member_hash()];
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_plan_objectives(
                   plan_objective_id,plan_id,revision_id,objective_id,
                   verification_contract_id,ordinal,objective_hash,
                   verification_contract_version,verification_contract_hash,
                   claim_component_count,claim_component_set_hash,stopping_criteria_hash,
                   outcome_requirement,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
        )
        .bind(plan_objective_id)
        .bind(plan.plan_id())
        .bind(revision_id)
        .bind(objective.objective_id())
        .bind(objective.verification_contract_id())
        .bind(ordinal as i32)
        .bind(objective.objective_hash())
        .bind(objective.verification_contract_version() as i32)
        .bind(objective.verification_contract_hash())
        .bind(objective.claim_component_count() as i64)
        .bind(objective.claim_component_set_hash())
        .bind(objective.stopping_criteria_hash())
        .bind(objective.outcome_requirement().as_str())
        .bind(objective.member_hash())
        .execute(&mut **tx)
        .await?;
    }
    for path in plan.proof_paths() {
        let path_id = Uuid::new_v5(&plan.plan_id(), path.path_hash().as_bytes());
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_plan_paths(
                   path_id,plan_id,path_ordinal,path_key,member_count,member_set_hash,path_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(path_id)
        .bind(plan.plan_id())
        .bind(path.path_ordinal() as i32)
        .bind(path.path_key())
        .bind(path.member_count() as i64)
        .bind(path.member_set_hash())
        .bind(path.path_hash())
        .execute(&mut **tx)
        .await?;
        for member in path.members() {
            let plan_objective_id = plan_objective_by_hash
                .get(member.plan_objective_member_hash())
                .copied()
                .ok_or_else(|| conflict(COMPILED_AUTHORITY_INCOMPLETE))?;
            sqlx::query(
                r#"INSERT INTO attack_hypothesis_verification_plan_path_members(
                       path_member_id,path_id,plan_id,plan_objective_id,
                       plan_objective_member_hash,revision_id,member_ordinal,
                       verification_contract_hash,claim_component_set_hash,role,
                       falsifier_claim_component_member_hashes,falsifier_claim_component_count,
                       falsifier_claim_component_set_hash,member_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
            )
            .bind(Uuid::new_v5(&path_id, member.member_hash().as_bytes()))
            .bind(path_id)
            .bind(plan.plan_id())
            .bind(plan_objective_id)
            .bind(member.plan_objective_member_hash())
            .bind(revision_id)
            .bind(member.member_ordinal() as i32)
            .bind(member.verification_contract_hash())
            .bind(member.claim_component_set_hash())
            .bind(match member.role() {
                HypothesisVerificationPlanPathMemberRoleV1::RequiredProof => "required_proof",
                HypothesisVerificationPlanPathMemberRoleV1::RequiredProofAndPathFalsifier => {
                    "required_proof_and_path_falsifier"
                }
            })
            .bind(member.falsifier_claim_component_member_hashes())
            .bind(member.falsifier_claim_component_member_hashes().len() as i64)
            .bind(member.falsifier_claim_component_set_hash())
            .bind(member.member_hash())
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

async fn persist_contract_on(
    tx: &mut Transaction<'_, Postgres>,
    contract: &VerificationContractV1,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_verification_contracts(
               contract_id,revision_id,revision_hash,objective_id,contract_schema,
               contract_version,combinator,predicate_count,predicate_set_hash,
               required_control_count,required_control_set_hash,explicit_no_required_control,
               paired_differential_count,paired_differential_set_hash,ordered_step_count,
               ordered_step_set_hash,stopping_criteria_hash,compiler_digest,rule_digest,
               policy_snapshot_hash,contract_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)"#,
    )
    .bind(contract.contract_id())
    .bind(contract.revision_id())
    .bind(contract.revision_hash())
    .bind(contract.objective_id())
    .bind(contract.contract_schema())
    .bind(contract.contract_version() as i32)
    .bind(contract.combinator().as_str())
    .bind(contract.predicate_count() as i64)
    .bind(contract.predicate_set_hash())
    .bind(contract.required_control_count() as i64)
    .bind(contract.required_control_set_hash())
    .bind(contract.explicit_no_required_control())
    .bind(contract.paired_differential_count() as i64)
    .bind(contract.paired_differential_set_hash())
    .bind(contract.ordered_step_count() as i64)
    .bind(contract.ordered_step_set_hash())
    .bind(contract.stopping_criteria_hash())
    .bind(contract.compiler_digest())
    .bind(contract.rule_digest())
    .bind(contract.policy_snapshot_hash())
    .bind(contract.contract_hash())
    .execute(&mut **tx)
    .await?;
    let mut predicate_ids = BTreeMap::new();
    for component in contract.predicate_components() {
        let id = Uuid::new_v5(&contract.contract_id(), component.member_hash().as_bytes());
        predicate_ids.insert(component.semantic_key().to_owned(), id);
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_predicate_components(
                   predicate_component_id,contract_id,ordinal,semantic_key,predicate_schema,
                   predicate_version,normalized_arguments,normalized_arguments_hash,
                   expected_polarity,prerequisite_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(id)
        .bind(contract.contract_id())
        .bind(component.ordinal() as i32)
        .bind(component.semantic_key())
        .bind(component.predicate_schema())
        .bind(component.predicate_version() as i32)
        .bind(component.normalized_arguments().as_value())
        .bind(hash_json_on(tx, component.normalized_arguments().as_value()).await?)
        .bind(component.expected_polarity().as_str())
        .bind(component.prerequisite_hash())
        .bind(component.member_hash())
        .execute(&mut **tx)
        .await?;
    }
    let mut control_ids = BTreeMap::new();
    for control in contract.required_controls() {
        let id = Uuid::new_v5(&contract.contract_id(), control.member_hash().as_bytes());
        control_ids.insert(
            (control.control_id().to_owned(), control.control_version()),
            id,
        );
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_required_controls(
                   required_control_id,contract_id,ordinal,control_id,control_version,
                   control_contract_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(id)
        .bind(contract.contract_id())
        .bind(control.ordinal() as i32)
        .bind(control.control_id())
        .bind(control.control_version() as i32)
        .bind(control.control_contract_hash())
        .bind(control.member_hash())
        .execute(&mut **tx)
        .await?;
    }
    for pair in contract.paired_differential_bindings() {
        let baseline_id = predicate_ids[pair.baseline_component_key()];
        let variant_id = predicate_ids[pair.variant_component_key()];
        let control_id = control_ids[&(
            pair.required_control_id().to_owned(),
            pair.required_control_version(),
        )];
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_pair_bindings(
                   pair_binding_id,contract_id,ordinal,pair_key,baseline_component_id,
                   baseline_component_key,variant_component_id,variant_component_key,
                   required_control_member_id,required_control_id,required_control_version,
                   required_control_contract_hash,required_control_member_hash,
                   comparator_rule_id,comparator_rule_version,comparator_rule_digest,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
        )
        .bind(Uuid::new_v5(
            &contract.contract_id(),
            pair.member_hash().as_bytes(),
        ))
        .bind(contract.contract_id())
        .bind(pair.ordinal() as i32)
        .bind(pair.pair_key())
        .bind(baseline_id)
        .bind(pair.baseline_component_key())
        .bind(variant_id)
        .bind(pair.variant_component_key())
        .bind(control_id)
        .bind(pair.required_control_id())
        .bind(pair.required_control_version() as i32)
        .bind(pair.required_control_contract_hash())
        .bind(pair.required_control_member_hash())
        .bind(pair.comparator_rule_id())
        .bind(pair.comparator_rule_version() as i32)
        .bind(pair.comparator_rule_digest())
        .bind(pair.member_hash())
        .execute(&mut **tx)
        .await?;
    }
    for step in contract.ordered_steps() {
        let component_id = predicate_ids[step.component_key()];
        sqlx::query(
            r#"INSERT INTO attack_hypothesis_verification_ordered_steps(
                   ordered_step_id,contract_id,step_ordinal,predicate_component_id,
                   component_key,predecessor_step_ordinal,session_binding_key_schema,
                   session_binding_key_version,session_scope,interleaving_policy,
                   reset_policy,step_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
        )
        .bind(Uuid::new_v5(
            &contract.contract_id(),
            step.step_hash().as_bytes(),
        ))
        .bind(contract.contract_id())
        .bind(step.step_ordinal() as i32)
        .bind(component_id)
        .bind(step.component_key())
        .bind(step.predecessor_step_ordinal().map(|value| value as i32))
        .bind(step.session_binding_key_schema())
        .bind(step.session_binding_key_version() as i32)
        .bind(step.session_scope().as_str())
        .bind(step.interleaving_policy().as_str())
        .bind(step.reset_policy().as_str())
        .bind(step.step_hash())
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn build_projection_members(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyCandidateGatePassInput,
    gate_decision_id: Uuid,
    gate_decision_hash: &str,
    generation_id: Uuid,
    mutations: &[PreparedMutation],
    state_event_ids: &[Uuid],
    residual: Option<(&Uuid, &str, &str)>,
    occurred_at: DateTime<Utc>,
) -> Result<Vec<ProjectionOutboxSourceRow>> {
    let mut members = Vec::new();
    let (source_set_hash, observation_window_hash): (String, String) = sqlx::query_as(
        r#"SELECT source_set_hash,observation_window_hash
             FROM candidate_analysis_snapshots
            WHERE snapshot_id=$1 AND operation_id=$2 AND organization_id=$3"#,
    )
    .bind(input.fence.snapshot_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    let (
        generation_ordinal,
        generation_seal_hash,
        generation_member_set_hash,
        generation_event_set_hash,
        open_obligation_set_hash,
    ): (i32, String, String, String, String) = sqlx::query_as(
        r#"SELECT generation.generation_ordinal,seal.generation_hash,
                  seal.member_set_hash,seal.event_set_hash,seal.open_obligation_set_hash
             FROM hypothesis_generations generation
             JOIN hypothesis_generation_seals seal USING(generation_id)
            WHERE generation.generation_id=$1"#,
    )
    .bind(generation_id)
    .fetch_one(&mut **tx)
    .await?;
    let generation_ordinal =
        u32::try_from(generation_ordinal).map_err(|_| conflict(MUTATION_SET_INVALID))?;
    let generation_manifest: Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
               'generation_id',generation.generation_id,
               'generation_ordinal',generation.generation_ordinal,
               'previous_generation_id',generation.previous_generation_id,
               'candidate_snapshot_id',generation.candidate_snapshot_id,
               'candidate_gate_decision_id',generation.candidate_gate_decision_id,
               'candidate_snapshot_authority_hash',generation.candidate_snapshot_authority_hash,
               'generation_hash',seal.generation_hash,
               'member_count',seal.member_count,
               'member_set_hash',seal.member_set_hash,
               'event_count',seal.event_count,
               'event_set_hash',seal.event_set_hash,
               'open_obligation_set_hash',seal.open_obligation_set_hash,
               'members',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                    'revision_id',member.revision_id,'ordinal',member.ordinal,
                    'member_hash',member.member_hash) ORDER BY member.ordinal)
                  FROM hypothesis_generation_members member
                 WHERE member.generation_id=generation.generation_id),'[]'::JSONB),
               'transitions',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                    'transition_id',transition.transition_id,
                    'previous_generation_member_id',transition.previous_generation_member_id,
                    'previous_revision_id',transition.previous_revision_id,
                    'disposition',transition.disposition,
                    'transition_hash',transition.transition_hash) ORDER BY transition.transition_id)
                  FROM hypothesis_generation_transitions transition
                 WHERE transition.generation_id=generation.generation_id),'[]'::JSONB),
               'input_dispositions',COALESCE((SELECT jsonb_agg(to_jsonb(disposition)
                    ORDER BY disposition.snapshot_input_id)
                  FROM input_processing_dispositions disposition
                 WHERE disposition.analysis_attempt_id=$2),'[]'::JSONB),
               'input_relations',COALESCE((SELECT jsonb_agg(to_jsonb(relation)
                    ORDER BY relation.snapshot_input_id,relation.revision_id)
                  FROM input_hypothesis_relations relation
                 WHERE relation.analysis_attempt_id=$2),'[]'::JSONB),
               'gate_decision_id',$3::UUID,
               'gate_decision_hash',$4::TEXT)
          FROM hypothesis_generations generation
          JOIN hypothesis_generation_seals seal USING(generation_id)
         WHERE generation.generation_id=$1"#,
    )
    .bind(generation_id)
    .bind(input.active_analysis_attempt_id)
    .bind(gate_decision_id)
    .bind(gate_decision_hash)
    .fetch_one(&mut **tx)
    .await?;
    let body = golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(
        generation_manifest,
    )
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    members.push(ProjectionOutboxSourceRow {
        outbox_member_id: Uuid::new_v5(&generation_id, b"projection:generation"),
        change_kind: ProjectionChangeKind::Insert,
        source: ProjectionSourceSnapshotV1::Generation(
            GenerationProjectionRecordV1::try_new(generation_id.to_string(), 1, 1, body)
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
        ),
        source_occurred_at: Some(occurred_at),
        source_time_status: ProjectionSourceTimeStatusV1::Known,
        invalidation_reason: None,
        storage: ProjectionSourceStorageV1::Inline,
    });
    for (mutation, event_id) in mutations.iter().zip(state_event_ids) {
        let target_value_at_time = mutation
            .semantic_key
            .get("subject")
            .and_then(|subject| subject.get("identity_hash"))
            .and_then(Value::as_str)
            .ok_or_else(|| conflict(MUTATION_SET_INVALID))?;
        let plan = input
            .verification_plans
            .iter()
            .find(|plan| plan.revision_id() == mutation.revision_id)
            .ok_or_else(|| conflict(COMPILED_AUTHORITY_INCOMPLETE))?;
        let disposition = match mutation.state {
            CandidateMutationEpistemicState::Proposed => {
                ComparisonHypothesisDispositionV1::Proposed
            }
            CandidateMutationEpistemicState::Supported => {
                ComparisonHypothesisDispositionV1::Supported
            }
            CandidateMutationEpistemicState::Contested => {
                ComparisonHypothesisDispositionV1::Contested
            }
            CandidateMutationEpistemicState::Inconclusive => {
                ComparisonHypothesisDispositionV1::Inconclusive
            }
        };
        let comparison_record = InvestigationComparisonRecordInputV1 {
            semantic_key_hash: mutation.semantic_key_hash.clone(),
            revision_ingredients_hash: mutation.revision_ingredients_hash.clone(),
            authority_basis: ComparisonAuthorityBasisInputV1::PlanBChecked {
                authority: Box::new(PlanBCheckedComparisonAuthorityInputV1 {
                    checked_authority: CheckedAuthorityComparisonV1 {
                        bundle_seal_hash: input
                            .expected_authority
                            .candidate_snapshot_authority_hash
                            .clone(),
                        root_set_hash: input
                            .expected_authority
                            .tool_truth_authority_root_set_hash
                            .clone(),
                        bundle_member_set_hash: input
                            .expected_authority
                            .tool_truth_authority_bundle_member_set_hash
                            .clone(),
                        receipt_set_hash: input
                            .expected_authority
                            .tool_truth_authority_receipt_set_hash
                            .clone(),
                        denominator_graph_bundle_hash: input
                            .expected_authority
                            .denominator_graph_bundle_hash
                            .clone(),
                        semantic_authority_bundle_hash: input
                            .expected_authority
                            .semantic_authority_bundle_hash
                            .clone(),
                        freshness_attestation_bundle_hash: input
                            .expected_authority
                            .freshness_attestation_bundle_hash
                            .clone(),
                        temporal_validity_bundle_hash: input
                            .expected_authority
                            .temporal_validity_bundle_hash
                            .clone(),
                        temporal_validity_policy_set_hash: input
                            .expected_authority
                            .temporal_validity_policy_set_hash
                            .clone(),
                        temporal_validity_decision_set_hash: input
                            .expected_authority
                            .temporal_validity_decision_set_hash
                            .clone(),
                        target_state_epoch_set_hash: input
                            .expected_authority
                            .target_state_epoch_set_hash
                            .clone(),
                        observation_window_hash: observation_window_hash.clone(),
                        gate_temporal_reevaluation_hash: input
                            .expected_authority
                            .gate_temporal_reevaluation_hash
                            .clone(),
                    },
                    knowledge_feed: KnowledgeFeedComparisonV1 {
                        catalog_policy_seal_hash: input
                            .expected_authority
                            .knowledge_feed_catalog_policy_seal_hash
                            .clone(),
                        required_member_set_hash: input
                            .expected_authority
                            .knowledge_feed_required_member_set_hash
                            .clone(),
                        signature_algorithm_set_hash: input
                            .expected_authority
                            .knowledge_feed_signature_algorithm_set_hash
                            .clone(),
                        trust_store_hash: input
                            .expected_authority
                            .knowledge_feed_trust_store_hash
                            .clone(),
                        key_revocation_epoch_hash: input
                            .expected_authority
                            .knowledge_feed_key_revocation_epoch_hash
                            .clone(),
                        snapshot_set_hash: input
                            .expected_authority
                            .knowledge_feed_snapshot_set_hash
                            .clone(),
                        product_version_census_hash: input
                            .expected_authority
                            .product_version_census_hash
                            .clone(),
                        match_census_hash: input
                            .expected_authority
                            .knowledge_feed_match_census_hash
                            .clone(),
                        source_set_hash: source_set_hash.clone(),
                        gate_reevaluation_hash: input
                            .expected_authority
                            .gate_knowledge_feed_reevaluation_hash
                            .clone(),
                        obligation_set_hash: input
                            .expected_authority
                            .knowledge_feed_obligation_set_hash
                            .clone(),
                    },
                    claim_component_member_hashes: input
                        .claim_components
                        .iter()
                        .filter(|component| component.revision_id() == mutation.revision_id)
                        .map(|component| component.member_hash().to_owned())
                        .collect(),
                    verification_contract_member_hashes: input
                        .verification_contracts
                        .iter()
                        .filter(|contract| contract.revision_id() == mutation.revision_id)
                        .map(|contract| contract.contract_hash().to_owned())
                        .collect(),
                    verification_plan_member_hashes: vec![plan.plan_hash().to_owned()],
                    verification_plan_objective_member_hashes: plan
                        .objectives()
                        .iter()
                        .map(|objective| objective.member_hash().to_owned())
                        .collect(),
                    verification_plan_path_member_hashes: plan
                        .proof_paths()
                        .iter()
                        .map(|path| path.path_hash().to_owned())
                        .collect(),
                    coverage_subreview_member_hashes: vec![input
                        .expected_authority
                        .coverage_subreview_census_set_hash
                        .clone()],
                    coverage_synthesis_member_hashes: vec![
                        input
                            .expected_authority
                            .coverage_synthesis_census_set_hash
                            .clone(),
                        input
                            .expected_authority
                            .coverage_global_semantic_root_hash
                            .clone(),
                    ],
                    coverage_final_review_member_hashes: vec![
                        input.expected_authority.coverage_global_review_hash.clone(),
                        input.expected_authority.coverage_review_set_hash.clone(),
                    ],
                    coverage_checklist_member_hashes: vec![input
                        .expected_authority
                        .coverage_checklist_set_hash
                        .clone()],
                    sampling_degraded_residual_member_hashes: Vec::new(),
                }),
            },
            generation: GenerationComparisonV1 {
                generation_ordinal,
                generation_seal_hash: generation_seal_hash.clone(),
                generation_member_set_hash: generation_member_set_hash.clone(),
                generation_event_set_hash: generation_event_set_hash.clone(),
                open_obligation_set_hash: open_obligation_set_hash.clone(),
            },
            disposition,
            readiness: if residual.is_some() {
                ComparisonHypothesisReadinessV1::ReportingOnlyPlanCUnavailable
            } else {
                ComparisonHypothesisReadinessV1::PlanningReady
            },
            plan_c: if residual.is_some() {
                PlanCComparisonAuthorityInputV1::not_available_plan_c()
            } else {
                PlanCComparisonAuthorityInputV1::pending_campaign_admission()
            },
            finding_lineage_member_hashes: Vec::new(),
            refutation_lineage_member_hashes: Vec::new(),
            residual_member_hashes: residual
                .map(|(_, residual_hash, _)| vec![residual_hash.to_owned()])
                .unwrap_or_default(),
            coverage_member_hashes: vec![input.expected_authority.coverage_review_set_hash.clone()],
        };
        let hypothesis_body = freeze_comparison_projection_source_body_v1(
            json!({
                "source_generation_id":generation_id,
                "root_id":mutation.root_id,"revision_id":mutation.revision_id,
                "revision_ordinal":mutation.revision_ordinal,
                "predecessor_revision_id":mutation.predecessor_revision_id,
                "revision_hash":mutation.revision_hash,
                "revision_ingredients_hash":mutation.revision_ingredients_hash,
                "semantic_key":mutation.semantic_key,
                "semantic_key_hash":mutation.semantic_key_hash,
                "state":mutation.state.as_str(),
                "lifecycle_state":"current",
                "planning_readiness":"ready_for_strategy",
                "target_type_at_time":"subject_identity_hash",
                "target_value_at_time":target_value_at_time,
                "origin_decision_hash":mutation.origin_decision_hash,
                "proposal":mutation.proposal,
                "proof_refs":mutation.proof_refs,
                "refutation_refs":mutation.refutation_refs,
                "relation_sources":mutation.relation_sources.iter().map(|source|json!({
                    "root_id":source.root_id,"revision_id":source.revision_id,
                    "relation_kind":source.relation_kind,
                })).collect::<Vec<_>>(),
            }),
            None,
            Some(comparison_record),
        )?;
        let hypothesis_version = u64::try_from(mutation.revision_ordinal + 1)
            .map_err(|_| conflict(MUTATION_SET_INVALID))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(&mutation.revision_id, b"projection:hypothesis"),
            change_kind: if mutation.revision_ordinal == 0 {
                ProjectionChangeKind::Insert
            } else {
                ProjectionChangeKind::Supersede
            },
            source: ProjectionSourceSnapshotV1::Hypothesis(
                HypothesisProjectionRecordV1::try_new(
                    mutation.root_id.to_string(),
                    hypothesis_version,
                    1,
                    hypothesis_body,
                )
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            ),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        });
        let plan_body = golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(
            serde_json::to_value(plan)?,
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(&mutation.revision_id, b"projection:verification_plan"),
            change_kind: ProjectionChangeKind::Close,
            source: ProjectionSourceSnapshotV1::HypothesisVerificationPlan(
                HypothesisVerificationPlanProjectionRecordV1::try_new(
                    plan.plan_id().to_string(),
                    u64::from(plan.plan_version()),
                    1,
                    plan_body,
                )
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            ),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        });
        let event_body = golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(
            json!({"event_id":event_id,"revision_id":mutation.revision_id}),
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(event_id, b"projection:state_event"),
            change_kind: ProjectionChangeKind::Insert,
            source: ProjectionSourceSnapshotV1::HypothesisStateEvent(
                HypothesisStateEventProjectionRecordV1::try_new(
                    event_id.to_string(),
                    1,
                    1,
                    event_body,
                )
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            ),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        });
        for source in &mutation.relation_sources {
            let relation_hash = hash_json_on(
                tx,
                &json!({
                    "source_root_id":source.root_id,
                    "source_revision_id":source.revision_id,
                    "target_root_id":mutation.root_id,
                    "target_revision_id":mutation.revision_id,
                    "relation_kind":source.relation_kind,
                }),
            )
            .await?;
            let relation_id = Uuid::new_v5(&mutation.revision_id, relation_hash.as_bytes());
            let relation_body =
                golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(json!({
                    "relation_id":relation_id,
                    "relation_hash":relation_hash,
                    "source_root_id":source.root_id,
                    "source_revision_id":source.revision_id,
                    "target_root_id":mutation.root_id,
                    "target_revision_id":mutation.revision_id,
                    "relation_kind":source.relation_kind,
                }))
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
            members.push(ProjectionOutboxSourceRow {
                outbox_member_id: Uuid::new_v5(&relation_id, b"projection:relation"),
                change_kind: ProjectionChangeKind::Insert,
                source: ProjectionSourceSnapshotV1::Relation(
                    RelationProjectionRecordV1::try_new(
                        relation_id.to_string(),
                        1,
                        1,
                        relation_body,
                    )
                    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
                ),
                source_occurred_at: Some(occurred_at),
                source_time_status: ProjectionSourceTimeStatusV1::Known,
                invalidation_reason: None,
                storage: ProjectionSourceStorageV1::Inline,
            });
        }
    }
    if let Some((residual_id, residual_hash, residual_reason)) = residual {
        let residual_body=golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(json!({"residual_id":residual_id,"residual_hash":residual_hash,"reason":residual_reason}))
            .map_err(|error|DbError::Other(anyhow::Error::new(error)))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(residual_id, b"projection:residual"),
            change_kind: ProjectionChangeKind::Insert,
            source: ProjectionSourceSnapshotV1::Residual(
                ResidualProjectionRecordV1::try_new(residual_id.to_string(), 1, 1, residual_body)
                    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            ),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        });
    }
    Ok(members)
}
