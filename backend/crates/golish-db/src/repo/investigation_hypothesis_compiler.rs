//! Investigation-owned canonical Hypothesis compiler transaction.
//!
//! This is deliberately independent of the legacy Candidate finalizer fence.
//! Cognitive output crosses this boundary only as a canonical advisory JSON
//! proposal.  The repository locks the unified Investigation authority,
//! resolves proof references against the frozen snapshot and recomputes every
//! route, identity and exact-set seal before writing canonical state.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use golish_core::hypothesis_semantic_key::{
    candidate_revision_id, initial_root_id, AtTimeSubjectIdentity, CandidateMutationEpistemicState,
    ClaimPolarity, HypothesisSemanticKeyV1, PredicateIdentity,
};
use golish_core::hypothesis_verification::{
    HypothesisClaimComponentV1, HypothesisVerificationPlanV1,
};
use golish_core::hypothesis_verification_task::{
    HypothesisVerificationTaskHeaderV1, NewHypothesisVerificationTaskV1,
};
use golish_core::investigation_projection::{
    GenerationProjectionRecordV1, HypothesisProjectionRecordV1,
    HypothesisStateEventProjectionRecordV1, HypothesisVerificationPlanProjectionRecordV1,
    ProjectionChangeKind, ProjectionSourceSnapshotV1, ProjectionSourceTimeStatusV1,
};
use golish_core::verification_contract::VerificationContractV1;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::hypothesis_legacy_projection::{
    append_projection_source_batch_on, AppendProjectionSourceBatchRow, ProjectionOutboxSourceRow,
    ProjectionSourceStorageV1,
};
use super::hypothesis_registry::{CandidateMutationRouteRow, CandidateMutationRow};
use crate::{DbError, Result};

const AUTHORITY_MISMATCH: &str = "INVESTIGATION_HYPOTHESIS_COMPILER_AUTHORITY_MISMATCH";
const REPLAY_DRIFT: &str = "INVESTIGATION_HYPOTHESIS_COMPILER_REPLAY_DRIFT";
const COMPILED_INVALID: &str = "INVESTIGATION_HYPOTHESIS_COMPILER_COMPILED_INVALID";
const TYPED_RESIDUAL_REQUIRED: &str = "INVESTIGATION_HYPOTHESIS_COMPILER_TYPED_RESIDUAL_REQUIRED";

fn conflict(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationProofRefInput {
    pub input_id: Uuid,
    pub chunk_id: Uuid,
    pub source_hash: String,
    pub source_role: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationProposalInput {
    pub proposal_id: Uuid,
    pub canonical_proposal: Value,
    pub proof_refs: Vec<InvestigationProofRefInput>,
}

#[derive(Debug, Clone)]
pub struct PrepareInvestigationCompilationInput {
    pub stable_compilation_request_id: Uuid,
    pub authority_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub binding_id: Uuid,
    pub work_id: Uuid,
    pub candidate_snapshot_id: Uuid,
    pub analysis_attempt_id: Uuid,
    pub task_plan_id: Uuid,
    pub delegation_census_seal_id: Uuid,
    pub primary_worker_run_id: Uuid,
    pub proposals: Vec<InvestigationProposalInput>,
    pub canonical_action_intents: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInvestigationProofMember {
    pub proposal_id: Uuid,
    pub input_id: Uuid,
    pub chunk_id: Uuid,
    pub source_hash: String,
    pub source_role: String,
    pub member_sha256: String,
}

#[derive(Debug, Clone)]
pub struct PreparedInvestigationCompilation {
    pub input: PrepareInvestigationCompilationInput,
    pub delegation_census_sha256: String,
    pub candidate_snapshot_authority_sha256: String,
    pub proposal_set_sha256: String,
    pub action_intent_set_sha256: String,
    pub proof_member_set_sha256: String,
    pub resolved_proofs: Vec<ResolvedInvestigationProofMember>,
    /// Server-owned route/revision recipe accepted by the existing pure app
    /// compiler. It contains no execution authority.
    pub server_recipe: Value,
    pub preparation_sha256: String,
}

#[derive(Debug, Clone)]
pub struct ApplyInvestigationCompilationInput {
    pub prepared: PreparedInvestigationCompilation,
    pub stable_apply_request_id: Uuid,
    pub stable_admission_request_id: Uuid,
    pub mutations: Vec<CandidateMutationRow>,
    pub claim_components: Vec<HypothesisClaimComponentV1>,
    pub verification_contracts: Vec<VerificationContractV1>,
    pub verification_plans: Vec<HypothesisVerificationPlanV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvestigationCanonicalApplyView {
    pub compilation_decision_id: Uuid,
    pub generation_id: Uuid,
    pub generation_ordinal: i32,
    pub generation_seal_id: Uuid,
    pub generation_member_count: i64,
    pub admission_set_id: Uuid,
    pub verification_task_ids: Vec<Uuid>,
    pub campaign_reservation_ids: Vec<Uuid>,
    pub projection_outbox_batch_id: Uuid,
    pub replayed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalProposal {
    proposal_id: Uuid,
    subject_kind: String,
    subject_identity_hash: String,
    predicate_schema: String,
    predicate_version: u32,
    predicate_arguments: Vec<(String, String)>,
    trust_boundary: String,
    polarity: String,
    structured_claim: String,
    preconditions: Vec<String>,
    impact: String,
    #[serde(default)]
    proof_refs: Vec<InvestigationProofRefInput>,
    #[serde(default)]
    knowledge_signals: Vec<Value>,
    readiness: Value,
}

#[derive(Debug)]
struct PreparedRevision {
    proposal_id: Uuid,
    canonical_proposal: Value,
    root_id: Uuid,
    revision_id: Uuid,
    semantic_key: Value,
    semantic_key_sha256: String,
    revision_ingredients_sha256: String,
    revision_sha256: String,
    origin_decision_sha256: String,
    generation_transition_sha256: String,
    state: CandidateMutationEpistemicState,
    member_sha256: String,
}

#[derive(sqlx::FromRow)]
struct ApplyReplayRow {
    decision_id: Uuid,
    binding_id: Uuid,
    task_plan_id: Uuid,
    proposal_set_sha256: String,
    action_intent_set_sha256: String,
    proof_member_set_sha256: String,
    mutation_set_sha256: String,
    claim_component_set_sha256: String,
    verification_contract_set_sha256: String,
    verification_plan_set_sha256: String,
    generation_transition_set_sha256: String,
    generation_id: Uuid,
    generation_seal_id: Uuid,
    generation_ordinal: i32,
    outbox_id: Uuid,
    member_count: i64,
    admission_set_id: Uuid,
}

async fn json_hash_on(tx: &mut Transaction<'_, Postgres>, value: &Value) -> Result<String> {
    Ok(
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(value)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn exact_set_hash_on(
    tx: &mut Transaction<'_, Postgres>,
    domain: &str,
    hashes: &[String],
) -> Result<String> {
    let mut canonical_hashes = hashes.to_vec();
    canonical_hashes.sort();
    Ok(
        sqlx::query_scalar("SELECT unified_investigation_exact_set_hash($1,$2::TEXT[])")
            .bind(domain)
            .bind(&canonical_hashes)
            .fetch_one(&mut **tx)
            .await?,
    )
}

async fn unordered_exact_set_hash_on(
    tx: &mut Transaction<'_, Postgres>,
    domain: &str,
    hashes: &[String],
) -> Result<String> {
    let mut canonical_hashes = hashes.to_vec();
    canonical_hashes.sort();
    exact_set_hash_on(tx, domain, &canonical_hashes).await
}

fn rust_exact_set_hash(domain: &str, hashes: &[String]) -> String {
    fn field(hasher: &mut Sha256, value: &[u8]) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    let mut hashes = hashes.to_vec();
    hashes.sort();
    let mut hasher = Sha256::new();
    field(&mut hasher, domain.as_bytes());
    for hash in hashes {
        field(&mut hasher, hash.as_bytes());
    }
    encode_digest(hasher.finalize())
}

fn encode_digest(digest: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest.as_ref() {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn mutation_hash(mutation: &CandidateMutationRow) -> String {
    fn field(hasher: &mut Sha256, value: &[u8]) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    let proof = mutation
        .proof_refs
        .iter()
        .map(|source| source.canonical_key())
        .collect::<Vec<_>>()
        .join("\u{1f}");
    let refutations = mutation
        .refutation_refs
        .iter()
        .map(|source| serde_json::to_string(source).unwrap_or_default())
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
    encode_digest(hasher.finalize())
}

/// Recomputes the transport hash for host-compiled material. This does not
/// grant route authority; `apply_investigation_compilation` derives and checks
/// the route again under the transaction lock.
pub fn reseal_investigation_mutation(mut mutation: CandidateMutationRow) -> CandidateMutationRow {
    mutation.mutation_hash = mutation_hash(&mutation);
    mutation
}

fn sha256_json(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("serializable compiler hash material");
    encode_digest(Sha256::digest(bytes))
}

fn event_kind(state: CandidateMutationEpistemicState) -> &'static str {
    match state {
        CandidateMutationEpistemicState::Proposed => "created",
        CandidateMutationEpistemicState::Supported => "supported",
        CandidateMutationEpistemicState::Contested => "contested",
        CandidateMutationEpistemicState::Inconclusive => "inconclusive",
    }
}

pub async fn prepare_investigation_compilation(
    pool: &PgPool,
    input: PrepareInvestigationCompilationInput,
) -> Result<PreparedInvestigationCompilation> {
    let ids = [
        input.stable_compilation_request_id,
        input.authority_id,
        input.operation_id,
        input.stage_execution_id,
        input.stage_run_unit_id,
        input.scope_snapshot_id,
        input.organization_id,
        input.binding_id,
        input.work_id,
        input.candidate_snapshot_id,
        input.analysis_attempt_id,
        input.task_plan_id,
        input.delegation_census_seal_id,
        input.primary_worker_run_id,
    ];
    if ids.iter().any(Uuid::is_nil) {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    if input.proposals.is_empty() {
        return Err(conflict(TYPED_RESIDUAL_REQUIRED));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let authority: Option<(String, String)> = sqlx::query_as(
        r#"SELECT census.seal_sha256,snapshot.candidate_snapshot_authority_hash
             FROM investigation_analysis_attempt_bindings binding
             JOIN candidate_analysis_snapshots snapshot
               ON snapshot.snapshot_id=binding.candidate_snapshot_id
              AND snapshot.operation_id=binding.operation_id
              AND snapshot.organization_id=binding.organization_id
              AND snapshot.scope_snapshot_id=binding.scope_snapshot_id
              AND snapshot.snapshot_status IN (
                  'sealed_ready','sealed_analysis_ready_with_residuals'
              )
             JOIN investigation_pentagi_task_plans plan
               ON plan.task_plan_id=$11 AND plan.authority_id=binding.authority_id
              AND plan.operation_id=binding.operation_id
              AND plan.stage_execution_id=binding.stage_execution_id
              AND plan.stage_run_unit_id=binding.stage_run_unit_id
              AND plan.organization_id=binding.organization_id
              AND plan.subject_kind='analysis_attempt'
              AND plan.subject_id=binding.analysis_attempt_id
              AND plan.status='sealed'
             JOIN investigation_pentagi_delegation_census_seals census
               ON census.census_seal_id=$12 AND census.task_plan_id=plan.task_plan_id
              AND census.primary_worker_run_id=$13
            WHERE binding.binding_id=$7 AND binding.authority_id=$1
              AND binding.operation_id=$2 AND binding.stage_execution_id=$3
              AND binding.stage_run_unit_id=$4 AND binding.scope_snapshot_id=$5
              AND binding.organization_id=$6 AND binding.work_id=$8
              AND binding.candidate_snapshot_id=$9
              AND binding.analysis_attempt_id=$10
              AND EXISTS(
                  SELECT 1 FROM investigation_pentagi_pipeline_events event
                   WHERE event.task_plan_id=plan.task_plan_id
                     AND event.event_kind='primary_synthesis'
                     AND event.actor_worker_run_id=$13)
            FOR SHARE OF binding,snapshot,plan,census"#,
    )
    .bind(input.authority_id)
    .bind(input.operation_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.scope_snapshot_id)
    .bind(input.organization_id)
    .bind(input.binding_id)
    .bind(input.work_id)
    .bind(input.candidate_snapshot_id)
    .bind(input.analysis_attempt_id)
    .bind(input.task_plan_id)
    .bind(input.delegation_census_seal_id)
    .bind(input.primary_worker_run_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((delegation_census_sha256, candidate_snapshot_authority_sha256)) = authority else {
        return Err(conflict(AUTHORITY_MISMATCH));
    };

    let mut proposal_ids = BTreeSet::new();
    let mut proposal_hashes = Vec::with_capacity(input.proposals.len());
    let mut proof_hashes = Vec::new();
    let mut resolved_proofs = Vec::new();
    let mut recipe_items = Vec::with_capacity(input.proposals.len());
    for (proposal_ordinal, proposal_input) in input.proposals.iter().enumerate() {
        if proposal_input.proposal_id.is_nil() || !proposal_ids.insert(proposal_input.proposal_id) {
            return Err(conflict(COMPILED_INVALID));
        }
        let proposal: CanonicalProposal =
            serde_json::from_value(proposal_input.canonical_proposal.clone())?;
        if proposal.proposal_id != proposal_input.proposal_id
            || proposal.subject_kind.trim().is_empty()
            || !valid_sha256(&proposal.subject_identity_hash)
            || proposal.predicate_schema.trim().is_empty()
            || proposal.predicate_version == 0
            || proposal.trust_boundary.trim().is_empty()
            || proposal.structured_claim.trim().is_empty()
        {
            return Err(conflict(COMPILED_INVALID));
        }
        let mut arguments = serde_json::Map::new();
        for (key, value) in &proposal.predicate_arguments {
            if key.trim().is_empty()
                || arguments
                    .insert(key.clone(), Value::String(value.clone()))
                    .is_some()
            {
                return Err(conflict(COMPILED_INVALID));
            }
        }
        let predicate = PredicateIdentity::new(
            proposal.predicate_schema.clone(),
            proposal.predicate_version,
            Value::Object(arguments.clone()),
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let polarity = ClaimPolarity::try_from(proposal.polarity.as_str())
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let semantic_key = HypothesisSemanticKeyV1::new(
            input.organization_id,
            AtTimeSubjectIdentity::new(
                proposal.subject_kind.clone(),
                proposal.subject_identity_hash.clone(),
            )
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            predicate,
            proposal.trust_boundary.clone(),
            polarity,
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let semantic_key_sha256 = semantic_key
            .hash()
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let current: Option<(Uuid, Uuid, String)> = sqlx::query_as(
            r#"SELECT head.root_id,head.head_revision_id,revision.revision_hash
                 FROM attack_hypothesis_heads head
                 JOIN attack_hypothesis_revisions revision
                   ON revision.revision_id=head.head_revision_id
                WHERE head.operation_id=$1 AND head.organization_id=$2
                  AND head.head_semantic_key_hash=$3
                  AND head.head_lifecycle_state='current' FOR SHARE OF head,revision"#,
        )
        .bind(input.operation_id)
        .bind(input.organization_id)
        .bind(&semantic_key_sha256)
        .fetch_optional(&mut *tx)
        .await?;
        let (route_kind, root_id, current_revision_id, current_revision_hash) =
            if let Some((root_id, revision_id, revision_hash)) = current {
                (
                    "attach_current",
                    root_id,
                    Some(revision_id),
                    Some(revision_hash),
                )
            } else {
                (
                    "create_initial",
                    initial_root_id(input.operation_id, &semantic_key)
                        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
                    None,
                    None,
                )
            };
        let generation_transition_sha256 = json_hash_on(
            &mut tx,
            &json!({
                "domain":"investigation_generation_transition.v1",
                "proposal_id":proposal.proposal_id,
                "route_kind":route_kind,
                "root_id":root_id,
                "revision_id":current_revision_id,
                "semantic_key_sha256":semantic_key_sha256,
            }),
        )
        .await?;
        let origin_decision_sha256 = json_hash_on(
            &mut tx,
            &json!({
                "proposal_id":proposal.proposal_id,
                "route_kind":route_kind,
                "root_id":root_id,
                "predecessor_revision_id":Value::Null,
                "semantic_key_hash":semantic_key_sha256,
                "relation_sources":[],
                "generation_transition_hash":generation_transition_sha256,
                "successor_state":"proposed",
            }),
        )
        .await?;
        let proposal_sha256 = json_hash_on(&mut tx, &proposal_input.canonical_proposal).await?;
        proposal_hashes.push(proposal_sha256.clone());

        let mut proof_refs_json = Vec::new();
        let mut refutation_refs_json = Vec::new();
        if proposal_input.proof_refs.is_empty() || proposal.proof_refs != proposal_input.proof_refs
        {
            return Err(conflict(COMPILED_INVALID));
        }
        for proof in &proposal_input.proof_refs {
            if !valid_sha256(&proof.source_hash)
                || !matches!(
                    proof.source_role.as_str(),
                    "support" | "contradiction" | "authorization_use"
                )
            {
                return Err(conflict(COMPILED_INVALID));
            }
            let valid: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                       SELECT 1 FROM candidate_analysis_snapshot_inputs input
                       JOIN candidate_analysis_input_chunk_census_members chunk
                         ON chunk.snapshot_input_id=input.snapshot_input_id
                        AND chunk.snapshot_id=input.snapshot_id
                      WHERE input.snapshot_id=$1 AND input.snapshot_input_id=$2
                        AND chunk.chunk_id=$3 AND input.source_content_hash=$4)"#,
            )
            .bind(input.candidate_snapshot_id)
            .bind(proof.input_id)
            .bind(proof.chunk_id)
            .bind(&proof.source_hash)
            .fetch_one(&mut *tx)
            .await?;
            if !valid {
                return Err(conflict(AUTHORITY_MISMATCH));
            }
            let member_sha256 = json_hash_on(
                &mut tx,
                &json!({
                    "proposal_id":proposal.proposal_id,
                    "input_id":proof.input_id,
                    "chunk_id":proof.chunk_id,
                    "source_hash":proof.source_hash,
                    "source_role":proof.source_role,
                }),
            )
            .await?;
            proof_hashes.push(member_sha256.clone());
            resolved_proofs.push(ResolvedInvestigationProofMember {
                proposal_id: proposal.proposal_id,
                input_id: proof.input_id,
                chunk_id: proof.chunk_id,
                source_hash: proof.source_hash.clone(),
                source_role: proof.source_role.clone(),
                member_sha256,
            });
            let source = json!({"kind":"tool_truth_evidence","id":proof.source_hash});
            if proof.source_role == "contradiction" {
                refutation_refs_json.push(source);
            } else {
                proof_refs_json.push(source);
            }
        }
        let revision_ingredients_sha256 = json_hash_on(
            &mut tx,
            &json!({
                "proposal":proposal_input.canonical_proposal,
                "origin_decision_hash":origin_decision_sha256,
            }),
        )
        .await?;
        let revision_id = if route_kind == "create_initial" {
            candidate_revision_id(root_id, 0, &semantic_key_sha256, &origin_decision_sha256)
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?
        } else {
            current_revision_id.ok_or_else(|| conflict(COMPILED_INVALID))?
        };
        let revision_sha256 = if route_kind == "create_initial" {
            json_hash_on(
                &mut tx,
                &json!({
                    "revision_id":revision_id,
                    "root_id":root_id,
                    "ordinal":0,
                    "semantic_key_hash":semantic_key_sha256,
                    "state":"proposed",
                    "ingredients":revision_ingredients_sha256,
                }),
            )
            .await?
        } else {
            current_revision_hash.ok_or_else(|| conflict(COMPILED_INVALID))?
        };
        let route = if route_kind == "create_initial" {
            json!({"kind":"create_initial","root_id":root_id})
        } else {
            json!({
                "kind":"attach_current",
                "root_id":root_id,
                "revision_id":revision_id,
            })
        };
        let compiler_seed = json_hash_on(
            &mut tx,
            &json!({
                "domain":"investigation_hypothesis_compiler_seed.v1",
                "proposal_sha256":proposal_sha256,
                "snapshot_sha256":candidate_snapshot_authority_sha256,
                "semantic_key_sha256":semantic_key_sha256,
            }),
        )
        .await?;
        recipe_items.push(json!({
            "proposal_id":proposal.proposal_id,
            "semantic_key_hash":semantic_key_sha256,
            "state":"proposed",
            "route":route,
            "generation_transition_hash":generation_transition_sha256,
            "proof_refs":proof_refs_json,
            "refutation_refs":refutation_refs_json,
            "polarity":proposal.polarity,
            "predicate_schema":proposal.predicate_schema,
            "predicate_version":proposal.predicate_version,
            "predicate_arguments":Value::Object(arguments),
            "revision":{
                "revision_id":revision_id,
                "revision_hash":revision_sha256,
                "revision_ingredients_hash":revision_ingredients_sha256,
                "derivation_digest":compiler_seed,
                "claim_clause_hash":json_hash_on(&mut tx,&json!({"claim":proposal.structured_claim})).await?,
                "impact_hash":json_hash_on(&mut tx,&json!({"impact":proposal.impact})).await?,
                "trust_boundary_hash":json_hash_on(&mut tx,&json!({"trust_boundary":proposal.trust_boundary})).await?,
                "identity_hash":proposal.subject_identity_hash,
                "objective_id":Uuid::new_v5(&revision_id,b"investigation:objective:v1"),
                "objective_hash":json_hash_on(&mut tx,&json!({"revision_id":revision_id,"objective":"verify"})).await?,
                "stopping_criteria_hash":json_hash_on(&mut tx,&json!({"stop":"typed_oracle_terminal"})).await?,
                "compiler_digest":compiler_seed,
                "rule_digest":json_hash_on(&mut tx,&json!({"rule":"investigation_compiler.v1"})).await?,
                "policy_snapshot_hash":candidate_snapshot_authority_sha256,
                "outer_policy_digest":json_hash_on(&mut tx,&json!({"policy":"all_required_paths.v1"})).await?,
            },
            "ordinal":proposal_ordinal,
        }));
    }
    recipe_items.sort_by(|left, right| {
        (
            left["semantic_key_hash"].as_str(),
            left["route"]["root_id"].as_str(),
            left["proposal_id"].as_str(),
        )
            .cmp(&(
                right["semantic_key_hash"].as_str(),
                right["route"]["root_id"].as_str(),
                right["proposal_id"].as_str(),
            ))
    });
    for (ordinal, item) in recipe_items.iter_mut().enumerate() {
        item["ordinal"] = json!(ordinal);
    }
    resolved_proofs.sort_by_key(|proof| {
        (
            proof.member_sha256.clone(),
            proof.proposal_id,
            proof.input_id,
            proof.chunk_id,
        )
    });
    let action_hashes = {
        let mut hashes = Vec::with_capacity(input.canonical_action_intents.len());
        for intent in &input.canonical_action_intents {
            if !intent.is_object() {
                return Err(conflict(COMPILED_INVALID));
            }
            hashes.push(json_hash_on(&mut tx, intent).await?);
        }
        hashes
    };
    let proposal_set_sha256 = unordered_exact_set_hash_on(
        &mut tx,
        "investigation_candidate_proposals.v1",
        &proposal_hashes,
    )
    .await?;
    let action_intent_set_sha256 = unordered_exact_set_hash_on(
        &mut tx,
        "investigation_advisory_action_intents.v1",
        &action_hashes,
    )
    .await?;
    let proof_member_set_sha256 = unordered_exact_set_hash_on(
        &mut tx,
        "investigation_hypothesis_compilation_proofs.v1",
        &proof_hashes,
    )
    .await?;
    let server_recipe = json!({
        "schema":"investigation_server_compiler_recipe.v1",
        "organization_id":input.organization_id,
        "items":recipe_items,
    });
    let preparation_sha256 = json_hash_on(
        &mut tx,
        &json!({
            "stable_compilation_request_id":input.stable_compilation_request_id,
            "binding_id":input.binding_id,
            "task_plan_id":input.task_plan_id,
            "delegation_census_seal_id":input.delegation_census_seal_id,
            "primary_worker_run_id":input.primary_worker_run_id,
            "proposal_set_sha256":proposal_set_sha256,
            "action_intent_set_sha256":action_intent_set_sha256,
            "proof_member_set_sha256":proof_member_set_sha256,
            "server_recipe":server_recipe,
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(PreparedInvestigationCompilation {
        input,
        delegation_census_sha256,
        candidate_snapshot_authority_sha256,
        proposal_set_sha256,
        action_intent_set_sha256,
        proof_member_set_sha256,
        resolved_proofs,
        server_recipe,
        preparation_sha256,
    })
}

pub async fn apply_investigation_compilation(
    pool: &PgPool,
    input: ApplyInvestigationCompilationInput,
) -> Result<InvestigationCanonicalApplyView> {
    if input.stable_apply_request_id.is_nil()
        || input.stable_admission_request_id.is_nil()
        || input.mutations.is_empty()
        || input.prepared.input.proposals.len() != input.mutations.len()
    {
        return Err(conflict(COMPILED_INVALID));
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await?;
    let p = &input.prepared;
    let owner = &p.input;
    sqlx::query("SELECT 1 FROM operation_state WHERE operation_id=$1 FOR UPDATE")
        .bind(owner.operation_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;

    validate_prepared_authority_on(&mut tx, p).await?;
    if let Some(replay) = load_apply_replay_on(&mut tx, &input).await? {
        tx.commit().await?;
        return Ok(replay);
    }
    validate_prepared_authority_on(&mut tx, p).await?;

    let mut mutation_by_proposal = BTreeMap::new();
    for mutation in &input.mutations {
        if mutation.organization_id != owner.organization_id
            || mutation.operator_rank != 0
            || mutation.mutation_hash != mutation_hash(mutation)
            || !valid_sha256(&mutation.generation_transition_hash)
            || mutation_by_proposal
                .insert(mutation.proposal_id, mutation)
                .is_some()
        {
            return Err(conflict(COMPILED_INVALID));
        }
    }
    let expected_proposal_ids = owner
        .proposals
        .iter()
        .map(|proposal| proposal.proposal_id)
        .collect::<BTreeSet<_>>();
    if mutation_by_proposal
        .keys()
        .copied()
        .collect::<BTreeSet<_>>()
        != expected_proposal_ids
    {
        return Err(conflict(COMPILED_INVALID));
    }

    let decision_id = Uuid::new_v5(
        &owner.stable_compilation_request_id,
        b"investigation_hypothesis_compilation_decision.v1",
    );
    let generation_ordinal: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(generation_ordinal)+1,0) FROM hypothesis_generations WHERE operation_id=$1 AND organization_id=$2",
    )
    .bind(owner.operation_id)
    .bind(owner.organization_id)
    .fetch_one(&mut *tx)
    .await?;
    let previous_generation_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT generation_id FROM hypothesis_generations WHERE operation_id=$1 AND organization_id=$2 ORDER BY generation_ordinal DESC LIMIT 1 FOR SHARE",
    )
    .bind(owner.operation_id)
    .bind(owner.organization_id)
    .fetch_optional(&mut *tx)
    .await?;
    let generation_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        format!("investigation_generation:{generation_ordinal}").as_bytes(),
    );

    let mut prepared_revisions = Vec::new();
    let mut compilation_member_hashes = Vec::new();
    let mut transition_hashes = Vec::new();
    for proposal_input in &owner.proposals {
        let mutation = mutation_by_proposal[&proposal_input.proposal_id];
        let prepared = prepare_revision_on(&mut tx, owner, proposal_input, mutation).await?;
        compilation_member_hashes.push(mutation.mutation_hash.clone());
        transition_hashes.push(mutation.generation_transition_hash.clone());
        prepared_revisions.push(prepared);
    }
    prepared_revisions.sort_by_key(|revision| {
        (
            revision.semantic_key_sha256.clone(),
            revision.root_id,
            revision.proposal_id,
        )
    });
    if prepared_revisions
        .iter()
        .map(|revision| revision.root_id)
        .collect::<BTreeSet<_>>()
        .len()
        != prepared_revisions.len()
    {
        return Err(conflict(COMPILED_INVALID));
    }
    let new_revision_ids = prepared_revisions
        .iter()
        .filter(|revision| {
            matches!(
                mutation_by_proposal[&revision.proposal_id].route,
                CandidateMutationRouteRow::CreateInitial { .. }
            )
        })
        .map(|revision| revision.revision_id)
        .collect::<BTreeSet<_>>();

    validate_compiled_authority_exact_sets(&input, &prepared_revisions, &new_revision_ids)?;
    let mutation_set_sha256 = unordered_exact_set_hash_on(
        &mut tx,
        "investigation_hypothesis_compilation_members.v1",
        &compilation_member_hashes,
    )
    .await?;
    let claim_component_set_sha256 = rust_exact_set_hash(
        "candidate_claim_components.v1",
        &input
            .claim_components
            .iter()
            .map(|item| item.member_hash().to_owned())
            .collect::<Vec<_>>(),
    );
    let verification_contract_set_sha256 = rust_exact_set_hash(
        "candidate_contracts.v1",
        &input
            .verification_contracts
            .iter()
            .map(|item| item.contract_hash().to_owned())
            .collect::<Vec<_>>(),
    );
    let verification_plan_set_sha256 = rust_exact_set_hash(
        "candidate_plans.v1",
        &input
            .verification_plans
            .iter()
            .map(|item| item.plan_hash().to_owned())
            .collect::<Vec<_>>(),
    );
    let generation_transition_set_sha256 =
        rust_exact_set_hash("candidate_generation_transitions.v1", &transition_hashes);
    let decision_sha256 = json_hash_on(
        &mut tx,
        &json!({
            "domain":"investigation_hypothesis_compilation_decision.v1",
            "preparation_sha256":p.preparation_sha256,
            "mutation_set_sha256":mutation_set_sha256,
            "claim_component_set_sha256":claim_component_set_sha256,
            "verification_contract_set_sha256":verification_contract_set_sha256,
            "verification_plan_set_sha256":verification_plan_set_sha256,
            "generation_transition_set_sha256":generation_transition_set_sha256,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_hypothesis_compilation_decisions(
               decision_id,stable_request_id,binding_id,authority_id,operation_id,
               stage_execution_id,stage_run_unit_id,organization_id,work_id,
               candidate_snapshot_id,analysis_attempt_id,task_plan_id,
               delegation_census_seal_id,primary_worker_run_id,delegation_census_sha256,
               cognitive_output_schema,proposal_count,proposal_set_sha256,
               action_intent_count,action_intent_set_sha256,proof_member_count,
               proof_member_set_sha256,mutation_count,mutation_set_sha256,
               claim_component_set_sha256,verification_contract_set_sha256,
               verification_plan_set_sha256,generation_transition_set_sha256,decision_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,
                  'investigation_cognitive_output.v1',$16,$17,$18,$19,$20,$21,
                  $22,$23,$24,$25,$26,$27,$28)"#,
    )
    .bind(decision_id)
    .bind(owner.stable_compilation_request_id)
    .bind(owner.binding_id)
    .bind(owner.authority_id)
    .bind(owner.operation_id)
    .bind(owner.stage_execution_id)
    .bind(owner.stage_run_unit_id)
    .bind(owner.organization_id)
    .bind(owner.work_id)
    .bind(owner.candidate_snapshot_id)
    .bind(owner.analysis_attempt_id)
    .bind(owner.task_plan_id)
    .bind(owner.delegation_census_seal_id)
    .bind(owner.primary_worker_run_id)
    .bind(&p.delegation_census_sha256)
    .bind(i64::try_from(owner.proposals.len()).map_err(|_| conflict(COMPILED_INVALID))?)
    .bind(&p.proposal_set_sha256)
    .bind(
        i64::try_from(owner.canonical_action_intents.len())
            .map_err(|_| conflict(COMPILED_INVALID))?,
    )
    .bind(&p.action_intent_set_sha256)
    .bind(i64::try_from(p.resolved_proofs.len()).map_err(|_| conflict(COMPILED_INVALID))?)
    .bind(&p.proof_member_set_sha256)
    .bind(i64::try_from(prepared_revisions.len()).map_err(|_| conflict(COMPILED_INVALID))?)
    .bind(&mutation_set_sha256)
    .bind(&claim_component_set_sha256)
    .bind(&verification_contract_set_sha256)
    .bind(&verification_plan_set_sha256)
    .bind(&generation_transition_set_sha256)
    .bind(&decision_sha256)
    .execute(&mut *tx)
    .await?;

    let mut state_event_ids = Vec::new();
    let mut revision_by_root = BTreeMap::new();
    let mut compilation_member_by_proposal = BTreeMap::new();
    for (ordinal, revision) in prepared_revisions.iter().enumerate() {
        let mutation = mutation_by_proposal[&revision.proposal_id];
        let compilation_member_id = Uuid::new_v5(&decision_id, mutation.mutation_hash.as_bytes());
        if matches!(
            mutation.route,
            CandidateMutationRouteRow::CreateInitial { .. }
        ) {
            persist_new_revision_on(
                &mut tx,
                owner,
                revision,
                &input.claim_components,
                &input.verification_contracts,
                &input.verification_plans,
            )
            .await?;
        }
        sqlx::query(
            r#"INSERT INTO investigation_hypothesis_compilation_members(
                   compilation_member_id,decision_id,operation_id,organization_id,ordinal,
                   proposal_id,canonical_proposal,proposal_sha256,route_kind,root_id,
                   predecessor_revision_id,successor_revision_id,semantic_key_sha256,
                   successor_epistemic_state,origin_decision_sha256,
                   generation_transition_sha256,member_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,NULL,$11,$12,$13,$14,$15,$16)"#,
        )
        .bind(compilation_member_id)
        .bind(decision_id)
        .bind(owner.operation_id)
        .bind(owner.organization_id)
        .bind(i32::try_from(ordinal).map_err(|_| conflict(COMPILED_INVALID))?)
        .bind(revision.proposal_id)
        .bind(&revision.canonical_proposal)
        .bind(json_hash_on(&mut tx, &revision.canonical_proposal).await?)
        .bind(match mutation.route {
            CandidateMutationRouteRow::CreateInitial { .. } => "create_initial",
            CandidateMutationRouteRow::AttachCurrent { .. } => "attach_current",
            _ => return Err(conflict(COMPILED_INVALID)),
        })
        .bind(revision.root_id)
        .bind(revision.revision_id)
        .bind(&revision.semantic_key_sha256)
        .bind(revision.state.as_str())
        .bind(&revision.origin_decision_sha256)
        .bind(&revision.generation_transition_sha256)
        .bind(&revision.member_sha256)
        .execute(&mut *tx)
        .await?;
        compilation_member_by_proposal.insert(revision.proposal_id, compilation_member_id);
        revision_by_root.insert(revision.root_id, revision.revision_id);
        if matches!(
            mutation.route,
            CandidateMutationRouteRow::CreateInitial { .. }
        ) {
            let event_id = Uuid::new_v5(&revision.revision_id, b"investigation_creating_event.v1");
            let event_sha256 = json_hash_on(
                &mut tx,
                &json!({
                    "revision_id":revision.revision_id,
                    "event_kind":event_kind(revision.state),
                    "compilation_member_id":compilation_member_id,
                }),
            )
            .await?;
            sqlx::query(
                r#"INSERT INTO attack_hypothesis_state_events(
                       event_id,operation_id,organization_id,root_id,predecessor_revision_id,
                       successor_revision_id,event_kind,origin_authority,successor_epistemic_state,
                       authority_receipt_kind,authority_receipt_id,authority_receipt_hash,
                       event_hash,server_decision_id,server_decision_hash)
                   VALUES($1,$2,$3,$4,NULL,$5,$6,'investigation_compiler',$7,
                          'investigation_compilation_decision',$8,$9,$10,$8,$9)"#,
            )
            .bind(event_id)
            .bind(owner.operation_id)
            .bind(owner.organization_id)
            .bind(revision.root_id)
            .bind(revision.revision_id)
            .bind(event_kind(revision.state))
            .bind(revision.state.as_str())
            .bind(compilation_member_id)
            .bind(&revision.origin_decision_sha256)
            .bind(event_sha256)
            .execute(&mut *tx)
            .await?;
            state_event_ids.push(event_id);
        }
    }
    persist_proof_members_on(
        &mut tx,
        owner,
        decision_id,
        &p.resolved_proofs,
        &compilation_member_by_proposal,
        &prepared_revisions,
    )
    .await?;

    let previous_members: Vec<(Uuid, Uuid)> = if let Some(previous) = previous_generation_id {
        sqlx::query_as(
            "SELECT generation_member_id,revision_id FROM hypothesis_generation_members WHERE generation_id=$1 ORDER BY ordinal FOR SHARE",
        )
        .bind(previous)
        .fetch_all(&mut *tx)
        .await?
    } else {
        Vec::new()
    };
    let mut generation_revisions = previous_members
        .iter()
        .map(|(_, revision)| *revision)
        .collect::<BTreeSet<_>>();
    generation_revisions.extend(revision_by_root.values().copied());
    let generation_revisions = generation_revisions.into_iter().collect::<Vec<_>>();
    sqlx::query(
        r#"INSERT INTO hypothesis_generations(
               generation_id,operation_id,organization_id,generation_ordinal,
               candidate_snapshot_id,candidate_gate_decision_id,
               investigation_compilation_decision_id,candidate_snapshot_authority_hash,
               previous_generation_id)
           VALUES($1,$2,$3,$4,$5,NULL,$6,$7,$8)"#,
    )
    .bind(generation_id)
    .bind(owner.operation_id)
    .bind(owner.organization_id)
    .bind(generation_ordinal)
    .bind(owner.candidate_snapshot_id)
    .bind(decision_id)
    .bind(&p.candidate_snapshot_authority_sha256)
    .bind(previous_generation_id)
    .execute(&mut *tx)
    .await?;
    let mut generation_member_hashes = Vec::new();
    let mut generation_members = Vec::new();
    for (ordinal, revision_id) in generation_revisions.iter().enumerate() {
        let member_sha256 = json_hash_on(
            &mut tx,
            &json!({"generation_id":generation_id,"revision_id":revision_id,"ordinal":ordinal}),
        )
        .await?;
        let member_id = Uuid::new_v5(&generation_id, member_sha256.as_bytes());
        sqlx::query(
            r#"INSERT INTO hypothesis_generation_members(
                   generation_member_id,generation_id,operation_id,organization_id,
                   revision_id,ordinal,member_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(member_id)
        .bind(generation_id)
        .bind(owner.operation_id)
        .bind(owner.organization_id)
        .bind(revision_id)
        .bind(i32::try_from(ordinal).map_err(|_| conflict(COMPILED_INVALID))?)
        .bind(&member_sha256)
        .execute(&mut *tx)
        .await?;
        generation_member_hashes.push(member_sha256);
        generation_members.push((member_id, *revision_id));
    }
    persist_unchanged_transitions_on(
        &mut tx,
        owner,
        generation_id,
        previous_generation_id,
        &previous_members,
    )
    .await?;
    let generation_member_set_sha256 = exact_set_hash_on(
        &mut tx,
        "hypothesis_generation_members.v1",
        &generation_member_hashes,
    )
    .await?;
    let event_hashes: Vec<String> = if state_event_ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(
            "SELECT event_hash FROM attack_hypothesis_state_events WHERE event_id=ANY($1) ORDER BY event_hash",
        )
        .bind(&state_event_ids)
        .fetch_all(&mut *tx)
        .await?
    };
    let event_set_sha256 =
        exact_set_hash_on(&mut tx, "hypothesis_generation_events.v1", &event_hashes).await?;
    let open_obligation_set_sha256: String = sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(to_jsonb(COALESCE(array_agg(obligation_hash ORDER BY obligation_hash),ARRAY[]::TEXT[]))::TEXT)
             FROM candidate_analysis_enrichment_obligations WHERE snapshot_id=$1"#,
    )
    .bind(owner.candidate_snapshot_id)
    .fetch_one(&mut *tx)
    .await?;
    let generation_sha256 = json_hash_on(
        &mut tx,
        &json!({
            "generation":generation_id,
            "members":generation_member_set_sha256,
            "events":event_set_sha256,
            "obligations":open_obligation_set_sha256,
        }),
    )
    .await?;
    let generation_seal_id = Uuid::new_v5(&generation_id, b"hypothesis_generation_seal.v1");
    sqlx::query(
        r#"INSERT INTO hypothesis_generation_seals(
               seal_id,generation_id,member_count,member_set_hash,event_count,event_set_hash,
               open_obligation_set_hash,controller_worker_run_id,generation_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)"#,
    )
    .bind(generation_seal_id)
    .bind(generation_id)
    .bind(i64::try_from(generation_members.len()).map_err(|_| conflict(COMPILED_INVALID))?)
    .bind(&generation_member_set_sha256)
    .bind(i64::try_from(state_event_ids.len()).map_err(|_| conflict(COMPILED_INVALID))?)
    .bind(&event_set_sha256)
    .bind(&open_obligation_set_sha256)
    .bind(owner.primary_worker_run_id)
    .bind(&generation_sha256)
    .execute(&mut *tx)
    .await?;

    let (admission_set_id, verification_task_ids, campaign_reservation_ids) =
        persist_tasks_and_admission_on(
            &mut tx,
            &input,
            generation_id,
            &generation_members,
            &new_revision_ids,
            &open_obligation_set_sha256,
        )
        .await?;
    let occurred_at: DateTime<Utc> = sqlx::query_scalar("SELECT statement_timestamp()")
        .fetch_one(&mut *tx)
        .await?;
    let outbox_batch_id = persist_projection_on(
        &mut tx,
        &input,
        decision_id,
        &decision_sha256,
        generation_id,
        &prepared_revisions,
        &state_event_ids,
        occurred_at,
    )
    .await?;
    let revision_hashes = prepared_revisions
        .iter()
        .filter(|revision| new_revision_ids.contains(&revision.revision_id))
        .map(|revision| revision.revision_sha256.clone())
        .collect::<Vec<_>>();
    let revision_set_sha256 = exact_set_hash_on(
        &mut tx,
        "investigation_canonical_apply_revisions.v1",
        &revision_hashes,
    )
    .await?;
    let apply_receipt_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        b"investigation_hypothesis_canonical_apply_receipt.v1",
    );
    let apply_receipt_sha256 = json_hash_on(
        &mut tx,
        &json!({
            "decision_id":decision_id,
            "decision_sha256":decision_sha256,
            "generation_id":generation_id,
            "generation_seal_id":generation_seal_id,
            "admission_set_id":admission_set_id,
            "verification_task_ids":verification_task_ids,
            "campaign_reservation_ids":campaign_reservation_ids,
        }),
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO investigation_hypothesis_canonical_apply_receipts(
               apply_receipt_id,stable_request_id,decision_id,operation_id,organization_id,
               generation_id,generation_seal_id,projection_outbox_batch_id,
               revision_count,revision_set_sha256,receipt_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(apply_receipt_id)
    .bind(input.stable_apply_request_id)
    .bind(decision_id)
    .bind(owner.operation_id)
    .bind(owner.organization_id)
    .bind(generation_id)
    .bind(generation_seal_id)
    .bind(outbox_batch_id)
    .bind(i64::try_from(revision_hashes.len()).map_err(|_| conflict(COMPILED_INVALID))?)
    .bind(&revision_set_sha256)
    .bind(&apply_receipt_sha256)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(InvestigationCanonicalApplyView {
        compilation_decision_id: decision_id,
        generation_id,
        generation_ordinal,
        generation_seal_id,
        generation_member_count: i64::try_from(generation_members.len())
            .map_err(|_| conflict(COMPILED_INVALID))?,
        admission_set_id,
        verification_task_ids,
        campaign_reservation_ids,
        projection_outbox_batch_id: outbox_batch_id,
        replayed: false,
    })
}

async fn validate_prepared_authority_on(
    tx: &mut Transaction<'_, Postgres>,
    prepared: &PreparedInvestigationCompilation,
) -> Result<()> {
    let owner = &prepared.input;
    let current: Option<(String, String)> = sqlx::query_as(
        r#"SELECT census.seal_sha256,snapshot.candidate_snapshot_authority_hash
             FROM investigation_analysis_attempt_bindings binding
             JOIN candidate_analysis_snapshots snapshot
               ON snapshot.snapshot_id=binding.candidate_snapshot_id
              AND snapshot.operation_id=binding.operation_id
              AND snapshot.organization_id=binding.organization_id
              AND snapshot.scope_snapshot_id=binding.scope_snapshot_id
              AND snapshot.snapshot_status IN (
                  'sealed_ready','sealed_analysis_ready_with_residuals'
              )
             JOIN investigation_pentagi_task_plans plan
               ON plan.task_plan_id=$11 AND plan.authority_id=binding.authority_id
              AND plan.operation_id=binding.operation_id
              AND plan.stage_execution_id=binding.stage_execution_id
              AND plan.stage_run_unit_id=binding.stage_run_unit_id
              AND plan.organization_id=binding.organization_id
              AND plan.subject_kind='analysis_attempt'
              AND plan.subject_id=binding.analysis_attempt_id AND plan.status='sealed'
             JOIN investigation_pentagi_delegation_census_seals census
               ON census.census_seal_id=$12 AND census.task_plan_id=plan.task_plan_id
              AND census.primary_worker_run_id=$13
            WHERE binding.binding_id=$1 AND binding.authority_id=$2
              AND binding.operation_id=$3 AND binding.stage_execution_id=$4
              AND binding.stage_run_unit_id=$5 AND binding.scope_snapshot_id=$6
              AND binding.organization_id=$7 AND binding.work_id=$8
              AND binding.candidate_snapshot_id=$9 AND binding.analysis_attempt_id=$10
              AND EXISTS(SELECT 1 FROM investigation_pentagi_pipeline_events event
                   WHERE event.task_plan_id=plan.task_plan_id
                     AND event.event_kind='primary_synthesis'
                     AND event.actor_worker_run_id=$13)
            FOR SHARE OF binding,snapshot,plan,census"#,
    )
    .bind(owner.binding_id)
    .bind(owner.authority_id)
    .bind(owner.operation_id)
    .bind(owner.stage_execution_id)
    .bind(owner.stage_run_unit_id)
    .bind(owner.scope_snapshot_id)
    .bind(owner.organization_id)
    .bind(owner.work_id)
    .bind(owner.candidate_snapshot_id)
    .bind(owner.analysis_attempt_id)
    .bind(owner.task_plan_id)
    .bind(owner.delegation_census_seal_id)
    .bind(owner.primary_worker_run_id)
    .fetch_optional(&mut **tx)
    .await?;
    if current
        != Some((
            prepared.delegation_census_sha256.clone(),
            prepared.candidate_snapshot_authority_sha256.clone(),
        ))
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    let mut proposal_hashes = Vec::new();
    let mut proof_hashes = Vec::new();
    let mut proof_keys = BTreeSet::new();
    for proposal in &owner.proposals {
        proposal_hashes.push(json_hash_on(tx, &proposal.canonical_proposal).await?);
        for proof in &proposal.proof_refs {
            if !proof_keys.insert((
                proposal.proposal_id,
                proof.input_id,
                proof.chunk_id,
                proof.source_role.clone(),
            )) {
                return Err(conflict(COMPILED_INVALID));
            }
            let valid: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(SELECT 1
                     FROM candidate_analysis_snapshot_inputs input
                     JOIN candidate_analysis_input_chunk_census_members chunk
                       ON chunk.snapshot_input_id=input.snapshot_input_id
                      AND chunk.snapshot_id=input.snapshot_id
                    WHERE input.snapshot_id=$1 AND input.snapshot_input_id=$2
                      AND chunk.chunk_id=$3 AND input.source_content_hash=$4)"#,
            )
            .bind(owner.candidate_snapshot_id)
            .bind(proof.input_id)
            .bind(proof.chunk_id)
            .bind(&proof.source_hash)
            .fetch_one(&mut **tx)
            .await?;
            if !valid {
                return Err(conflict(AUTHORITY_MISMATCH));
            }
            proof_hashes.push(
                json_hash_on(
                    tx,
                    &json!({
                        "proposal_id":proposal.proposal_id,
                        "input_id":proof.input_id,
                        "chunk_id":proof.chunk_id,
                        "source_hash":proof.source_hash,
                        "source_role":proof.source_role,
                    }),
                )
                .await?,
            );
        }
    }
    let mut action_hashes = Vec::new();
    for action in &owner.canonical_action_intents {
        action_hashes.push(json_hash_on(tx, action).await?);
    }
    let expected_proposals =
        exact_set_hash_on(tx, "investigation_candidate_proposals.v1", &proposal_hashes).await?;
    let expected_actions = exact_set_hash_on(
        tx,
        "investigation_advisory_action_intents.v1",
        &action_hashes,
    )
    .await?;
    let expected_proofs = unordered_exact_set_hash_on(
        tx,
        "investigation_hypothesis_compilation_proofs.v1",
        &proof_hashes,
    )
    .await?;
    let expected_preparation = json_hash_on(
        tx,
        &json!({
            "stable_compilation_request_id":owner.stable_compilation_request_id,
            "binding_id":owner.binding_id,
            "task_plan_id":owner.task_plan_id,
            "delegation_census_seal_id":owner.delegation_census_seal_id,
            "primary_worker_run_id":owner.primary_worker_run_id,
            "proposal_set_sha256":expected_proposals,
            "action_intent_set_sha256":expected_actions,
            "proof_member_set_sha256":expected_proofs,
            "server_recipe":prepared.server_recipe,
        }),
    )
    .await?;
    if prepared.proposal_set_sha256 != expected_proposals
        || prepared.action_intent_set_sha256 != expected_actions
        || prepared.proof_member_set_sha256 != expected_proofs
        || prepared.preparation_sha256 != expected_preparation
        || prepared.resolved_proofs.len() != proof_keys.len()
    {
        return Err(conflict(AUTHORITY_MISMATCH));
    }
    Ok(())
}

async fn prepare_revision_on(
    tx: &mut Transaction<'_, Postgres>,
    owner: &PrepareInvestigationCompilationInput,
    proposal_input: &InvestigationProposalInput,
    mutation: &CandidateMutationRow,
) -> Result<PreparedRevision> {
    let proposal: CanonicalProposal =
        serde_json::from_value(proposal_input.canonical_proposal.clone())?;
    let mut arguments = serde_json::Map::new();
    for (key, value) in &proposal.predicate_arguments {
        if arguments
            .insert(key.clone(), Value::String(value.clone()))
            .is_some()
        {
            return Err(conflict(COMPILED_INVALID));
        }
    }
    let semantic_key = HypothesisSemanticKeyV1::new(
        owner.organization_id,
        AtTimeSubjectIdentity::new(
            proposal.subject_kind.clone(),
            proposal.subject_identity_hash.clone(),
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
        PredicateIdentity::new(
            proposal.predicate_schema.clone(),
            proposal.predicate_version,
            Value::Object(arguments),
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
        proposal.trust_boundary.clone(),
        ClaimPolarity::try_from(proposal.polarity.as_str())
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
    )
    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let semantic_key_sha256 = semantic_key
        .hash()
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    if mutation.semantic_key_hash != semantic_key_sha256 {
        return Err(conflict(COMPILED_INVALID));
    }
    let (route_kind, root_id, revision_id, revision_sha256) = match mutation.route {
        CandidateMutationRouteRow::CreateInitial { root_id } => {
            let expected_root = initial_root_id(owner.operation_id, &semantic_key)
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
            if root_id != expected_root
                || sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM attack_hypotheses WHERE root_id=$1)",
                )
                .bind(root_id)
                .fetch_one(&mut **tx)
                .await?
            {
                return Err(conflict(COMPILED_INVALID));
            }
            ("create_initial", root_id, Uuid::nil(), String::new())
        }
        CandidateMutationRouteRow::AttachCurrent {
            root_id,
            revision_id,
        } => {
            let row: Option<String> = sqlx::query_scalar(
                r#"SELECT revision.revision_hash
                     FROM attack_hypothesis_heads head
                     JOIN attack_hypothesis_revisions revision
                       ON revision.revision_id=head.head_revision_id
                    WHERE head.root_id=$1 AND head.head_revision_id=$2
                      AND head.operation_id=$3 AND head.organization_id=$4
                      AND head.head_semantic_key_hash=$5
                      AND head.head_lifecycle_state='current' FOR SHARE OF head,revision"#,
            )
            .bind(root_id)
            .bind(revision_id)
            .bind(owner.operation_id)
            .bind(owner.organization_id)
            .bind(&semantic_key_sha256)
            .fetch_optional(&mut **tx)
            .await?;
            (
                "attach_current",
                root_id,
                revision_id,
                row.ok_or_else(|| conflict(COMPILED_INVALID))?,
            )
        }
        _ => return Err(conflict(COMPILED_INVALID)),
    };
    let expected_transition = json_hash_on(
        tx,
        &json!({
            "domain":"investigation_generation_transition.v1",
            "proposal_id":proposal.proposal_id,
            "route_kind":route_kind,
            "root_id":root_id,
            "revision_id":if route_kind=="attach_current" { Some(revision_id) } else { None },
            "semantic_key_sha256":semantic_key_sha256,
        }),
    )
    .await?;
    if expected_transition != mutation.generation_transition_hash {
        return Err(conflict(COMPILED_INVALID));
    }
    let origin_decision_sha256 = json_hash_on(
        tx,
        &json!({
            "proposal_id":proposal.proposal_id,"route_kind":route_kind,"root_id":root_id,
            "predecessor_revision_id":Value::Null,"semantic_key_hash":semantic_key_sha256,
            "relation_sources":[],"generation_transition_hash":expected_transition,
            "successor_state":mutation.state.as_str(),
        }),
    )
    .await?;
    let revision_ingredients_sha256 = json_hash_on(
        tx,
        &json!({
            "proposal":proposal_input.canonical_proposal,
            "origin_decision_hash":origin_decision_sha256,
        }),
    )
    .await?;
    let (revision_id, revision_sha256) = if route_kind == "create_initial" {
        let id = candidate_revision_id(root_id, 0, &semantic_key_sha256, &origin_decision_sha256)
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        let hash = json_hash_on(
            tx,
            &json!({
                "revision_id":id,"root_id":root_id,"ordinal":0,
                "semantic_key_hash":semantic_key_sha256,"state":mutation.state.as_str(),
                "ingredients":revision_ingredients_sha256,
            }),
        )
        .await?;
        (id, hash)
    } else {
        (revision_id, revision_sha256)
    };
    Ok(PreparedRevision {
        proposal_id: proposal.proposal_id,
        canonical_proposal: proposal_input.canonical_proposal.clone(),
        root_id,
        revision_id,
        semantic_key: serde_json::to_value(&semantic_key)?,
        semantic_key_sha256,
        revision_ingredients_sha256,
        revision_sha256,
        origin_decision_sha256,
        generation_transition_sha256: expected_transition,
        state: mutation.state,
        member_sha256: mutation.mutation_hash.clone(),
    })
}

fn validate_compiled_authority_exact_sets(
    input: &ApplyInvestigationCompilationInput,
    revisions: &[PreparedRevision],
    new_revision_ids: &BTreeSet<Uuid>,
) -> Result<()> {
    let component_revisions = input
        .claim_components
        .iter()
        .map(HypothesisClaimComponentV1::revision_id)
        .collect::<BTreeSet<_>>();
    let contract_revisions = input
        .verification_contracts
        .iter()
        .map(VerificationContractV1::revision_id)
        .collect::<BTreeSet<_>>();
    let plan_revisions = input
        .verification_plans
        .iter()
        .map(HypothesisVerificationPlanV1::revision_id)
        .collect::<BTreeSet<_>>();
    if component_revisions != *new_revision_ids
        || contract_revisions != *new_revision_ids
        || plan_revisions != *new_revision_ids
        || revisions.iter().any(|revision| {
            new_revision_ids.contains(&revision.revision_id)
                && (!input.claim_components.iter().any(|component| {
                    component.revision_id() == revision.revision_id
                        && component.revision_hash() == revision.revision_sha256
                }) || !input.verification_contracts.iter().any(|contract| {
                    contract.revision_id() == revision.revision_id
                        && contract.revision_hash() == revision.revision_sha256
                }) || !input.verification_plans.iter().any(|plan| {
                    plan.revision_id() == revision.revision_id
                        && plan.revision_hash() == revision.revision_sha256
                        && plan.revision_ingredients_hash() == revision.revision_ingredients_sha256
                }))
        })
    {
        return Err(conflict(COMPILED_INVALID));
    }
    Ok(())
}

async fn persist_new_revision_on(
    tx: &mut Transaction<'_, Postgres>,
    owner: &PrepareInvestigationCompilationInput,
    revision: &PreparedRevision,
    claim_components: &[HypothesisClaimComponentV1],
    verification_contracts: &[VerificationContractV1],
    verification_plans: &[HypothesisVerificationPlanV1],
) -> Result<()> {
    let proposal: CanonicalProposal = serde_json::from_value(revision.canonical_proposal.clone())?;
    let identity_ingredients = json!({
        "root_kind":"initial",
        "semantic_key_hash":revision.semantic_key_sha256,
        "route_kind":"create_initial",
    });
    let identity_sha256 = json_hash_on(tx, &identity_ingredients).await?;
    sqlx::query(
        r#"INSERT INTO attack_hypotheses(
               root_id,operation_id,organization_id,root_kind,
               identity_ingredients,identity_ingredients_hash)
           VALUES($1,$2,$3,'initial',$4,$5)"#,
    )
    .bind(revision.root_id)
    .bind(owner.operation_id)
    .bind(owner.organization_id)
    .bind(identity_ingredients)
    .bind(identity_sha256)
    .execute(&mut **tx)
    .await?;
    let normalized_arguments = Value::Object(
        proposal
            .predicate_arguments
            .iter()
            .map(|(key, value)| (key.clone(), Value::String(value.clone())))
            .collect(),
    );
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_revisions(
               revision_id,root_id,operation_id,organization_id,predecessor_revision_id,
               revision_ordinal,semantic_key,semantic_key_hash,subject_kind,
               subject_identity_hash,target_type_at_time,target_value_at_time,
               predicate_schema,predicate_version,normalized_arguments,trust_boundary,
               polarity,epistemic_state,lifecycle_state,planning_readiness,structured_claim,
               assumptions,missing_facts,priority,risk_impact,origin_decision_hash,
               revision_ingredients_hash,revision_hash)
           VALUES($1,$2,$3,$4,NULL,0,$5,$6,$7,$8,'subject_identity_hash',$8,$9,$10,
                  $11,$12,$13,$14,'current','ready_for_strategy',$15,'[]','[]',0,$16,$17,$18,$19)"#,
    )
    .bind(revision.revision_id)
    .bind(revision.root_id)
    .bind(owner.operation_id)
    .bind(owner.organization_id)
    .bind(&revision.semantic_key)
    .bind(&revision.semantic_key_sha256)
    .bind(&proposal.subject_kind)
    .bind(&proposal.subject_identity_hash)
    .bind(&proposal.predicate_schema)
    .bind(i32::try_from(proposal.predicate_version).map_err(|_| conflict(COMPILED_INVALID))?)
    .bind(normalized_arguments)
    .bind(&proposal.trust_boundary)
    .bind(&proposal.polarity)
    .bind(revision.state.as_str())
    .bind(json!({
        "prose":proposal.structured_claim,
        "preconditions":proposal.preconditions,
        "proof_refs":proposal.proof_refs,
        "knowledge_signals":proposal.knowledge_signals,
        "readiness":proposal.readiness,
    }))
    .bind(json!({"impact":proposal.impact}))
    .bind(&revision.origin_decision_sha256)
    .bind(&revision.revision_ingredients_sha256)
    .bind(&revision.revision_sha256)
    .execute(&mut **tx)
    .await?;
    super::hypothesis_registry::persist_compiled_authorities_for_revision_on(
        tx,
        claim_components,
        verification_contracts,
        verification_plans,
        revision.revision_id,
        &revision.revision_sha256,
        &revision.revision_ingredients_sha256,
    )
    .await?;
    sqlx::query(
        r#"INSERT INTO attack_hypothesis_heads(
               root_id,operation_id,organization_id,head_revision_id,head_revision_hash,
               head_semantic_key_hash,head_epistemic_state,head_lifecycle_state)
           VALUES($1,$2,$3,$4,$5,$6,$7,'current')"#,
    )
    .bind(revision.root_id)
    .bind(owner.operation_id)
    .bind(owner.organization_id)
    .bind(revision.revision_id)
    .bind(&revision.revision_sha256)
    .bind(&revision.semantic_key_sha256)
    .bind(revision.state.as_str())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn persist_proof_members_on(
    tx: &mut Transaction<'_, Postgres>,
    owner: &PrepareInvestigationCompilationInput,
    decision_id: Uuid,
    proofs: &[ResolvedInvestigationProofMember],
    member_by_proposal: &BTreeMap<Uuid, Uuid>,
    revisions: &[PreparedRevision],
) -> Result<()> {
    let revision_by_proposal = revisions
        .iter()
        .map(|revision| (revision.proposal_id, revision.revision_id))
        .collect::<BTreeMap<_, _>>();
    for (ordinal, proof) in proofs.iter().enumerate() {
        let compilation_member_id = member_by_proposal
            .get(&proof.proposal_id)
            .copied()
            .ok_or_else(|| conflict(COMPILED_INVALID))?;
        let revision_id = revision_by_proposal[&proof.proposal_id];
        sqlx::query(
            r#"INSERT INTO investigation_hypothesis_compilation_proof_members(
                   proof_member_id,decision_id,compilation_member_id,successor_revision_id,
                   operation_id,organization_id,candidate_snapshot_id,ordinal,
                   snapshot_input_id,chunk_id,source_role,source_sha256,member_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)"#,
        )
        .bind(Uuid::new_v5(&decision_id, proof.member_sha256.as_bytes()))
        .bind(decision_id)
        .bind(compilation_member_id)
        .bind(revision_id)
        .bind(owner.operation_id)
        .bind(owner.organization_id)
        .bind(owner.candidate_snapshot_id)
        .bind(i32::try_from(ordinal).map_err(|_| conflict(COMPILED_INVALID))?)
        .bind(proof.input_id)
        .bind(proof.chunk_id)
        .bind(&proof.source_role)
        .bind(&proof.source_hash)
        .bind(&proof.member_sha256)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn persist_unchanged_transitions_on(
    tx: &mut Transaction<'_, Postgres>,
    owner: &PrepareInvestigationCompilationInput,
    generation_id: Uuid,
    previous_generation_id: Option<Uuid>,
    previous_members: &[(Uuid, Uuid)],
) -> Result<()> {
    let Some(previous_generation_id) = previous_generation_id else {
        return if previous_members.is_empty() {
            Ok(())
        } else {
            Err(conflict(COMPILED_INVALID))
        };
    };
    for (member_id, revision_id) in previous_members {
        let transition_sha256 = json_hash_on(
            tx,
            &json!({
                "generation_id":generation_id,
                "previous_generation_id":previous_generation_id,
                "previous_generation_member_id":member_id,
                "previous_revision_id":revision_id,
                "disposition":"unchanged",
                "successor_revision_ids":[],
            }),
        )
        .await?;
        sqlx::query(
            r#"INSERT INTO hypothesis_generation_transitions(
                   transition_id,generation_id,operation_id,organization_id,
                   previous_generation_id,previous_generation_member_id,
                   previous_revision_id,disposition,transition_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,'unchanged',$8)"#,
        )
        .bind(Uuid::new_v5(&generation_id, member_id.as_bytes()))
        .bind(generation_id)
        .bind(owner.operation_id)
        .bind(owner.organization_id)
        .bind(previous_generation_id)
        .bind(member_id)
        .bind(revision_id)
        .bind(transition_sha256)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn persist_tasks_and_admission_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyInvestigationCompilationInput,
    generation_id: Uuid,
    generation_members: &[(Uuid, Uuid)],
    new_revision_ids: &BTreeSet<Uuid>,
    open_obligation_set_sha256: &str,
) -> Result<(Uuid, Vec<Uuid>, Vec<Uuid>)> {
    let owner = &input.prepared.input;
    let project_scope_id: Uuid =
        sqlx::query_scalar("SELECT project_scope_id FROM operation_state WHERE operation_id=$1")
            .bind(owner.operation_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten()
            .ok_or_else(|| conflict(AUTHORITY_MISMATCH))?;
    let admission_set_id = Uuid::new_v5(
        &input.stable_admission_request_id,
        b"verification_admission_set.v1",
    );
    sqlx::query(
        r#"INSERT INTO verification_admission_sets(
               admission_set_id,stable_request_id,operation_id,stage_execution_id,
               stage_run_unit_id,scope_snapshot_id,organization_id,generation_id)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
    )
    .bind(admission_set_id)
    .bind(input.stable_admission_request_id)
    .bind(owner.operation_id)
    .bind(owner.stage_execution_id)
    .bind(owner.stage_run_unit_id)
    .bind(owner.scope_snapshot_id)
    .bind(owner.organization_id)
    .bind(generation_id)
    .execute(&mut **tx)
    .await?;

    let proof_hashes_by_revision = {
        let proposal_to_revision = input
            .mutations
            .iter()
            .filter_map(|mutation| match mutation.route {
                CandidateMutationRouteRow::CreateInitial { root_id }
                | CandidateMutationRouteRow::AttachCurrent { root_id, .. } => input
                    .prepared
                    .input
                    .proposals
                    .iter()
                    .find(|proposal| proposal.proposal_id == mutation.proposal_id)
                    .map(|proposal| (root_id, proposal)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut by_revision = BTreeMap::new();
        for (root_id, proposal) in proposal_to_revision {
            let revision_id: Uuid = sqlx::query_scalar(
                "SELECT head_revision_id FROM attack_hypothesis_heads WHERE root_id=$1",
            )
            .bind(root_id)
            .fetch_one(&mut **tx)
            .await?;
            let hashes = input
                .prepared
                .resolved_proofs
                .iter()
                .filter(|proof| proof.proposal_id == proposal.proposal_id)
                .map(|proof| proof.member_sha256.clone())
                .collect::<Vec<_>>();
            by_revision.insert(
                revision_id,
                exact_set_hash_on(tx, "hypothesis_verification_task_evidence.v1", &hashes).await?,
            );
        }
        by_revision
    };
    let global_evidence_sha256 = exact_set_hash_on(
        tx,
        "hypothesis_verification_task_evidence.v1",
        &input
            .prepared
            .resolved_proofs
            .iter()
            .map(|proof| proof.member_sha256.clone())
            .collect::<Vec<_>>(),
    )
    .await?;
    let mut task_ids = Vec::with_capacity(generation_members.len());
    let mut campaign_ids = Vec::new();
    for (generation_member_id, revision_id) in generation_members {
        let (revision_sha256, plan_id, plan_sha256): (String, Uuid, String) = sqlx::query_as(
            r#"SELECT revision.revision_hash,plan.plan_id,plan.plan_hash
                 FROM attack_hypothesis_revisions revision
                 JOIN attack_hypothesis_verification_plans plan
                   ON plan.revision_id=revision.revision_id AND plan.sealed_at IS NOT NULL
                WHERE revision.revision_id=$1 AND revision.operation_id=$2
                  AND revision.organization_id=$3 FOR SHARE OF revision,plan"#,
        )
        .bind(revision_id)
        .bind(owner.operation_id)
        .bind(owner.organization_id)
        .fetch_one(&mut **tx)
        .await?;
        let semantic_evidence_sha256 = proof_hashes_by_revision
            .get(revision_id)
            .cloned()
            .unwrap_or_else(|| global_evidence_sha256.clone());
        let semantic_attempt_fingerprint = json_hash_on(
            tx,
            &json!({
                "revision_id":revision_id,
                "revision_sha256":revision_sha256,
                "plan_sha256":plan_sha256,
                "semantic_evidence_sha256":semantic_evidence_sha256,
                "open_obligation_set_sha256":open_obligation_set_sha256,
            }),
        )
        .await?;
        if !new_revision_ids.contains(revision_id) {
            let admission_member_sha256 = sha256_json(&json!({
                "admission_set_id":admission_set_id,
                "generation_member_id":generation_member_id,
                "hypothesis_revision_id":revision_id,
                "disposition":"no_new_obligation",
                "reason_code":"retained_or_attached_current_revision",
                "semantic_attempt_fingerprint":semantic_attempt_fingerprint,
                "task_id":Value::Null,
            }));
            sqlx::query(
                r#"INSERT INTO verification_admission_members(
                       admission_member_id,admission_set_id,operation_id,stage_execution_id,
                       stage_run_unit_id,scope_snapshot_id,organization_id,generation_member_id,
                       hypothesis_revision_id,disposition,reason_code,
                       semantic_attempt_fingerprint,task_id,member_sha256)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'no_new_obligation',
                          'retained_or_attached_current_revision',$10,NULL,$11)"#,
            )
            .bind(Uuid::new_v5(
                &admission_set_id,
                generation_member_id.as_bytes(),
            ))
            .bind(admission_set_id)
            .bind(owner.operation_id)
            .bind(owner.stage_execution_id)
            .bind(owner.stage_run_unit_id)
            .bind(owner.scope_snapshot_id)
            .bind(owner.organization_id)
            .bind(generation_member_id)
            .bind(revision_id)
            .bind(&semantic_attempt_fingerprint)
            .bind(&admission_member_sha256)
            .execute(&mut **tx)
            .await?;
            continue;
        }
        let header =
            HypothesisVerificationTaskHeaderV1::host_create(NewHypothesisVerificationTaskV1 {
                operation_id: owner.operation_id,
                stage_execution_id: owner.stage_execution_id,
                stage_run_unit_id: owner.stage_run_unit_id,
                organization_id: owner.organization_id,
                scope_snapshot_id: owner.scope_snapshot_id,
                hypothesis_revision_id: *revision_id,
                hypothesis_revision_sha256: revision_sha256.clone(),
                verification_plan_sha256: plan_sha256.clone(),
                relevant_evidence_snapshot_id: owner.candidate_snapshot_id,
                semantic_evidence_set_sha256: semantic_evidence_sha256,
                open_obligation_set_sha256: open_obligation_set_sha256.to_owned(),
                semantic_attempt_fingerprint: semantic_attempt_fingerprint.clone(),
                first_admission_generation_id: generation_id,
                host_rerun_receipt_id: None,
                host_rerun_receipt_sha256: None,
                rerun_contract_version: None,
            })
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        sqlx::query(
            r#"INSERT INTO hypothesis_verification_tasks(
                   task_id,stable_task_key_sha256,operation_id,project_scope_id,
                   stage_execution_id,stage_run_unit_id,scope_snapshot_id,organization_id,
                   hypothesis_revision_id,hypothesis_revision_sha256,verification_plan_id,
                   verification_plan_sha256,relevant_evidence_snapshot_id,
                   semantic_evidence_set_sha256,open_obligation_set_sha256,
                   semantic_attempt_fingerprint,task_contract_version,
                   first_admission_generation_id)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
        )
        .bind(header.task_id)
        .bind(&header.stable_task_key_sha256)
        .bind(header.operation_id)
        .bind(project_scope_id)
        .bind(header.stage_execution_id)
        .bind(header.stage_run_unit_id)
        .bind(header.scope_snapshot_id)
        .bind(header.organization_id)
        .bind(header.hypothesis_revision_id)
        .bind(&header.hypothesis_revision_sha256)
        .bind(plan_id)
        .bind(&header.verification_plan_sha256)
        .bind(header.relevant_evidence_snapshot_id)
        .bind(&header.semantic_evidence_set_sha256)
        .bind(&header.open_obligation_set_sha256)
        .bind(&header.semantic_attempt_fingerprint)
        .bind(&header.task_contract_version)
        .bind(header.first_admission_generation_id)
        .execute(&mut **tx)
        .await?;
        let task_event_id = Uuid::new_v5(&header.task_id, b"state:admitted:event");
        let task_event_sha256 = sha256_json(&json!({
            "task_id":header.task_id,"event_ordinal":0,"from_state":Value::Null,
            "to_state":"admitted","reason_code":"automatic_admission",
        }));
        sqlx::query(
            r#"INSERT INTO hypothesis_verification_task_state_events(
                   event_id,stable_request_id,task_id,event_ordinal,expected_head_version,
                   from_state,to_state,reason_code,event_sha256)
               VALUES($1,$2,$3,0,0,NULL,'admitted','automatic_admission',$4)"#,
        )
        .bind(task_event_id)
        .bind(Uuid::new_v5(&header.task_id, b"state:admitted:request"))
        .bind(header.task_id)
        .bind(task_event_sha256)
        .execute(&mut **tx)
        .await?;
        let assignment_set_id = Uuid::new_v5(&header.task_id, b"objective_assignments.v1");
        sqlx::query(
            r#"INSERT INTO hypothesis_verification_task_assignment_sets(
                   assignment_set_id,stable_request_id,task_id,hypothesis_revision_id,
                   verification_plan_id)
               VALUES($1,$2,$3,$4,$5)"#,
        )
        .bind(assignment_set_id)
        .bind(Uuid::new_v5(
            &input.stable_admission_request_id,
            format!("assignment:{revision_id}").as_bytes(),
        ))
        .bind(header.task_id)
        .bind(revision_id)
        .bind(plan_id)
        .execute(&mut **tx)
        .await?;
        let objectives: Vec<(Uuid, Uuid)> = sqlx::query_as(
            r#"SELECT plan_objective_id,objective_id
                 FROM attack_hypothesis_verification_plan_objectives
                WHERE plan_id=$1 ORDER BY ordinal,plan_objective_id"#,
        )
        .bind(plan_id)
        .fetch_all(&mut **tx)
        .await?;
        if objectives.is_empty() {
            return Err(conflict(COMPILED_INVALID));
        }
        let mut assignment_hashes = Vec::new();
        for (ordinal, (plan_objective_id, objective_id)) in objectives.iter().enumerate() {
            let campaign_id = Uuid::new_v5(
                &header.task_id,
                format!("campaign:{plan_objective_id}").as_bytes(),
            );
            let reservation_sha256 = sha256_json(&json!({
                "assignment_set_id":assignment_set_id,"task_id":header.task_id,
                "plan_objective_id":plan_objective_id,"objective_id":objective_id,
                "contract":"task_campaign_reservation.v1",
            }));
            sqlx::query(
                r#"INSERT INTO hypothesis_verification_task_campaigns(
                       campaign_id,assignment_set_id,task_id,hypothesis_revision_id,
                       verification_plan_id,plan_objective_id,verification_objective_id,
                       reservation_sha256)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
            )
            .bind(campaign_id)
            .bind(assignment_set_id)
            .bind(header.task_id)
            .bind(revision_id)
            .bind(plan_id)
            .bind(plan_objective_id)
            .bind(objective_id)
            .bind(&reservation_sha256)
            .execute(&mut **tx)
            .await?;
            let assignment_sha256 = sha256_json(&json!({
                "assignment_set_id":assignment_set_id,"task_id":header.task_id,
                "plan_objective_id":plan_objective_id,"objective_id":objective_id,
                "assignment_kind":"campaign","campaign_id":campaign_id,
            }));
            sqlx::query(
                r#"INSERT INTO hypothesis_verification_task_assignment_members(
                       assignment_member_id,assignment_set_id,task_id,hypothesis_revision_id,
                       verification_plan_id,plan_objective_id,verification_objective_id,
                       assignment_kind,campaign_id,member_sha256)
                   VALUES($1,$2,$3,$4,$5,$6,$7,'campaign',$8,$9)"#,
            )
            .bind(Uuid::new_v5(
                &assignment_set_id,
                format!("member:{ordinal}:{plan_objective_id}").as_bytes(),
            ))
            .bind(assignment_set_id)
            .bind(header.task_id)
            .bind(revision_id)
            .bind(plan_id)
            .bind(plan_objective_id)
            .bind(objective_id)
            .bind(campaign_id)
            .bind(&assignment_sha256)
            .execute(&mut **tx)
            .await?;
            assignment_hashes.push(assignment_sha256);
            campaign_ids.push(campaign_id);
        }
        let assignment_set_sha256 = exact_set_hash_on(
            tx,
            "hypothesis_verification_task_assignments.v1",
            &assignment_hashes,
        )
        .await?;
        sqlx::query(
            r#"UPDATE hypothesis_verification_task_assignment_sets
                  SET status='sealed',member_count=$2,member_set_sha256=$3,
                      row_version=1,sealed_at=statement_timestamp()
                WHERE assignment_set_id=$1 AND status='open' AND row_version=0"#,
        )
        .bind(assignment_set_id)
        .bind(i64::try_from(assignment_hashes.len()).map_err(|_| conflict(COMPILED_INVALID))?)
        .bind(assignment_set_sha256)
        .execute(&mut **tx)
        .await?;
        let admission_member_sha256 = sha256_json(&json!({
            "admission_set_id":admission_set_id,
            "generation_member_id":generation_member_id,
            "hypothesis_revision_id":revision_id,
            "disposition":"scheduled",
            "reason_code":"automatic_investigation_admission",
            "semantic_attempt_fingerprint":semantic_attempt_fingerprint,
            "task_id":header.task_id,
        }));
        sqlx::query(
            r#"INSERT INTO verification_admission_members(
                   admission_member_id,admission_set_id,operation_id,stage_execution_id,
                   stage_run_unit_id,scope_snapshot_id,organization_id,generation_member_id,
                   hypothesis_revision_id,disposition,reason_code,
                   semantic_attempt_fingerprint,task_id,member_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'scheduled',
                      'automatic_investigation_admission',$10,$11,$12)"#,
        )
        .bind(Uuid::new_v5(
            admission_set_id.as_ref(),
            generation_member_id.as_bytes(),
        ))
        .bind(admission_set_id)
        .bind(owner.operation_id)
        .bind(owner.stage_execution_id)
        .bind(owner.stage_run_unit_id)
        .bind(owner.scope_snapshot_id)
        .bind(owner.organization_id)
        .bind(generation_member_id)
        .bind(revision_id)
        .bind(&semantic_attempt_fingerprint)
        .bind(header.task_id)
        .bind(&admission_member_sha256)
        .execute(&mut **tx)
        .await?;
        task_ids.push(header.task_id);
    }
    // The admission seal trigger and closure validator define membership order
    // by hypothesis_revision_id.  Re-read the persisted members under this
    // transaction instead of sorting their hashes independently: hash order is
    // not member identity order once a generation contains multiple revisions.
    let (admission_member_count, admission_set_sha256): (i64, String) = sqlx::query_as(
        r#"SELECT COUNT(*),unified_investigation_exact_set_hash(
                   'verification_admission_members.v1',
                   COALESCE(array_agg(member_sha256 ORDER BY hypothesis_revision_id),
                            ARRAY[]::TEXT[])
               )
             FROM verification_admission_members
            WHERE admission_set_id=$1"#,
    )
    .bind(admission_set_id)
    .fetch_one(&mut **tx)
    .await?;
    sqlx::query(
        r#"UPDATE verification_admission_sets
              SET status='sealed',member_count=$2,member_set_sha256=$3,
                  row_version=1,sealed_at=statement_timestamp()
            WHERE admission_set_id=$1 AND status='open' AND row_version=0"#,
    )
    .bind(admission_set_id)
    .bind(admission_member_count)
    .bind(admission_set_sha256)
    .execute(&mut **tx)
    .await?;
    Ok((admission_set_id, task_ids, campaign_ids))
}

async fn persist_projection_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyInvestigationCompilationInput,
    decision_id: Uuid,
    decision_sha256: &str,
    generation_id: Uuid,
    revisions: &[PreparedRevision],
    event_ids: &[Uuid],
    occurred_at: DateTime<Utc>,
) -> Result<Uuid> {
    let owner = &input.prepared.input;
    let manifest: Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
               'generation_id',generation.generation_id,
               'generation_ordinal',generation.generation_ordinal,
               'previous_generation_id',generation.previous_generation_id,
               'candidate_snapshot_id',generation.candidate_snapshot_id,
               'investigation_compilation_decision_id',generation.investigation_compilation_decision_id,
               'candidate_snapshot_authority_hash',generation.candidate_snapshot_authority_hash,
               'generation_hash',seal.generation_hash,
               'member_count',seal.member_count,'member_set_hash',seal.member_set_hash,
               'event_count',seal.event_count,'event_set_hash',seal.event_set_hash,
               'open_obligation_set_hash',seal.open_obligation_set_hash,
               'members',COALESCE((SELECT jsonb_agg(jsonb_build_object(
                    'revision_id',member.revision_id,'ordinal',member.ordinal,
                    'member_hash',member.member_hash) ORDER BY member.ordinal)
                  FROM hypothesis_generation_members member
                 WHERE member.generation_id=generation.generation_id),'[]'::JSONB),
               'compilation_decision_id',$2::UUID,'compilation_decision_sha256',$3::TEXT)
          FROM hypothesis_generations generation
          JOIN hypothesis_generation_seals seal USING(generation_id)
         WHERE generation.generation_id=$1"#,
    )
    .bind(generation_id)
    .bind(decision_id)
    .bind(decision_sha256)
    .fetch_one(&mut **tx)
    .await?;
    let generation_body =
        golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(manifest)
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let mut members = vec![ProjectionOutboxSourceRow {
        outbox_member_id: Uuid::new_v5(&generation_id, b"projection:generation"),
        change_kind: ProjectionChangeKind::Insert,
        source: ProjectionSourceSnapshotV1::Generation(
            GenerationProjectionRecordV1::try_new(generation_id.to_string(), 1, 1, generation_body)
                .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
        ),
        source_occurred_at: Some(occurred_at),
        source_time_status: ProjectionSourceTimeStatusV1::Known,
        invalidation_reason: None,
        storage: ProjectionSourceStorageV1::Inline,
    }];
    for revision in revisions {
        let mutation = input
            .mutations
            .iter()
            .find(|mutation| mutation.proposal_id == revision.proposal_id)
            .ok_or_else(|| conflict(COMPILED_INVALID))?;
        if !matches!(
            mutation.route,
            CandidateMutationRouteRow::CreateInitial { .. }
        ) {
            continue;
        }
        let body =
            golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(json!({
                "source_generation_id":generation_id,
                "root_id":revision.root_id,"revision_id":revision.revision_id,
                "revision_ordinal":0,"predecessor_revision_id":Value::Null,
                "revision_hash":revision.revision_sha256,
                "revision_ingredients_hash":revision.revision_ingredients_sha256,
                "semantic_key":revision.semantic_key,
                "semantic_key_hash":revision.semantic_key_sha256,
                "state":revision.state.as_str(),"lifecycle_state":"current",
                "planning_readiness":"ready_for_strategy",
                "origin_decision_hash":revision.origin_decision_sha256,
                "proposal":revision.canonical_proposal,
                "origin_authority":"investigation_compiler",
            }))
            .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(&revision.revision_id, b"projection:hypothesis"),
            change_kind: ProjectionChangeKind::Insert,
            source: ProjectionSourceSnapshotV1::Hypothesis(
                HypothesisProjectionRecordV1::try_new(revision.root_id.to_string(), 1, 1, body)
                    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            ),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        });
        let plan = input
            .verification_plans
            .iter()
            .find(|plan| plan.revision_id() == revision.revision_id)
            .ok_or_else(|| conflict(COMPILED_INVALID))?;
        let plan_body = golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(
            serde_json::to_value(plan)?,
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(&revision.revision_id, b"projection:verification_plan"),
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
    }
    for event_id in event_ids {
        let revision_id: Uuid = sqlx::query_scalar(
            "SELECT successor_revision_id FROM attack_hypothesis_state_events WHERE event_id=$1",
        )
        .bind(event_id)
        .fetch_one(&mut **tx)
        .await?;
        let body = golish_core::hypothesis_semantic_key::CanonicalJsonObject::try_from_value(
            json!({"event_id":event_id,"revision_id":revision_id}),
        )
        .map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
        members.push(ProjectionOutboxSourceRow {
            outbox_member_id: Uuid::new_v5(event_id, b"projection:state_event"),
            change_kind: ProjectionChangeKind::Insert,
            source: ProjectionSourceSnapshotV1::HypothesisStateEvent(
                HypothesisStateEventProjectionRecordV1::try_new(event_id.to_string(), 1, 1, body)
                    .map_err(|error| DbError::Other(anyhow::Error::new(error)))?,
            ),
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            invalidation_reason: None,
            storage: ProjectionSourceStorageV1::Inline,
        });
    }
    let batch_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        b"investigation_projection_batch.v1",
    );
    let project_scope_id: Uuid =
        sqlx::query_scalar("SELECT project_scope_id FROM operation_state WHERE operation_id=$1")
            .bind(owner.operation_id)
            .fetch_one(&mut **tx)
            .await?;
    append_projection_source_batch_on(
        tx,
        AppendProjectionSourceBatchRow {
            batch_id,
            operation_id: owner.operation_id,
            project_scope_id: Some(project_scope_id),
            stable_request_id: input.stable_apply_request_id,
            source_transaction_id: decision_id,
            source_occurred_at: Some(occurred_at),
            source_time_status: ProjectionSourceTimeStatusV1::Known,
            members,
        },
    )
    .await?;
    Ok(batch_id)
}

async fn load_apply_replay_on(
    tx: &mut Transaction<'_, Postgres>,
    input: &ApplyInvestigationCompilationInput,
) -> Result<Option<InvestigationCanonicalApplyView>> {
    let row: Option<ApplyReplayRow> = sqlx::query_as(
        r#"SELECT receipt.decision_id,decision.binding_id,decision.task_plan_id,
                  decision.proposal_set_sha256,decision.action_intent_set_sha256,
                  decision.proof_member_set_sha256,decision.mutation_set_sha256,
                  decision.claim_component_set_sha256,
                  decision.verification_contract_set_sha256,
                  decision.verification_plan_set_sha256,
                  decision.generation_transition_set_sha256,
                  receipt.generation_id,receipt.generation_seal_id,
                  generation.generation_ordinal,
                  receipt.projection_outbox_batch_id AS outbox_id,
                  seal.member_count,admission.admission_set_id
             FROM investigation_hypothesis_canonical_apply_receipts receipt
             JOIN investigation_hypothesis_compilation_decisions decision
               ON decision.decision_id=receipt.decision_id
             JOIN hypothesis_generations generation
               ON generation.generation_id=receipt.generation_id
             JOIN hypothesis_generation_seals seal
               ON seal.seal_id=receipt.generation_seal_id
             JOIN verification_admission_sets admission
               ON admission.generation_id=generation.generation_id AND admission.status='sealed'
            WHERE receipt.stable_request_id=$1 FOR SHARE OF receipt,decision,generation,seal,admission"#,
    )
    .bind(input.stable_apply_request_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let expected_decision_id = Uuid::new_v5(
        &input.prepared.input.stable_compilation_request_id,
        b"investigation_hypothesis_compilation_decision.v1",
    );
    let expected_admission_id = Uuid::new_v5(
        &input.stable_admission_request_id,
        b"verification_admission_set.v1",
    );
    let expected_generation_id = Uuid::new_v5(
        &input.stable_apply_request_id,
        format!("investigation_generation:{}", row.generation_ordinal).as_bytes(),
    );
    let mutation_hashes = input
        .mutations
        .iter()
        .map(|mutation| {
            if mutation.mutation_hash != mutation_hash(mutation) {
                Err(conflict(REPLAY_DRIFT))
            } else {
                Ok(mutation.mutation_hash.clone())
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let transition_hashes = input
        .mutations
        .iter()
        .map(|mutation| mutation.generation_transition_hash.clone())
        .collect::<Vec<_>>();
    let expected_mutations = exact_set_hash_on(
        tx,
        "investigation_hypothesis_compilation_members.v1",
        &mutation_hashes,
    )
    .await?;
    let expected_claims = rust_exact_set_hash(
        "candidate_claim_components.v1",
        &input
            .claim_components
            .iter()
            .map(|item| item.member_hash().to_owned())
            .collect::<Vec<_>>(),
    );
    let expected_contracts = rust_exact_set_hash(
        "candidate_contracts.v1",
        &input
            .verification_contracts
            .iter()
            .map(|item| item.contract_hash().to_owned())
            .collect::<Vec<_>>(),
    );
    let expected_plans = rust_exact_set_hash(
        "candidate_plans.v1",
        &input
            .verification_plans
            .iter()
            .map(|item| item.plan_hash().to_owned())
            .collect::<Vec<_>>(),
    );
    let expected_transitions =
        rust_exact_set_hash("candidate_generation_transitions.v1", &transition_hashes);
    if row.decision_id != expected_decision_id
        || row.binding_id != input.prepared.input.binding_id
        || row.task_plan_id != input.prepared.input.task_plan_id
        || row.proposal_set_sha256 != input.prepared.proposal_set_sha256
        || row.action_intent_set_sha256 != input.prepared.action_intent_set_sha256
        || row.proof_member_set_sha256 != input.prepared.proof_member_set_sha256
        || row.mutation_set_sha256 != expected_mutations
        || row.claim_component_set_sha256 != expected_claims
        || row.verification_contract_set_sha256 != expected_contracts
        || row.verification_plan_set_sha256 != expected_plans
        || row.generation_transition_set_sha256 != expected_transitions
        || row.generation_id != expected_generation_id
        || row.admission_set_id != expected_admission_id
    {
        return Err(conflict(REPLAY_DRIFT));
    }
    let task_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT task_id FROM verification_admission_members WHERE admission_set_id=$1 AND disposition='scheduled' ORDER BY hypothesis_revision_id",
    )
    .bind(row.admission_set_id)
    .fetch_all(&mut **tx)
    .await?;
    let campaign_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT reservation.campaign_id FROM hypothesis_verification_task_campaigns reservation WHERE reservation.task_id=ANY($1) ORDER BY reservation.task_id,reservation.plan_objective_id",
    )
    .bind(&task_ids)
    .fetch_all(&mut **tx)
    .await?;
    Ok(Some(InvestigationCanonicalApplyView {
        compilation_decision_id: row.decision_id,
        generation_id: row.generation_id,
        generation_ordinal: row.generation_ordinal,
        generation_seal_id: row.generation_seal_id,
        generation_member_count: row.member_count,
        admission_set_id: row.admission_set_id,
        verification_task_ids: task_ids,
        campaign_reservation_ids: campaign_ids,
        projection_outbox_batch_id: row.outbox_id,
        replayed: true,
    }))
}
