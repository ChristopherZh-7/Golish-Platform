//! Production host for opening unified Investigation analysis subjects.
//!
//! Snapshot authority is selected by the existing Hypothesis Registry port.
//! This adapter then resolves the repository-created ordinal-zero attempt and
//! binds it to the exact registered unified Investigation analysis work item.

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use golish_agent_kit::db_traits::{
    CandidateAnalysisSnapshotDispositionV1, CandidateRegistryMutationDecisionV1,
    CompileSealAndAdmitInvestigationGeneration, FreezeCandidateAnalysisSnapshot,
    HypothesisRegistryError, HypothesisRegistryRepository, InvestigationAnalysisAuthorityChunkV1,
    InvestigationAnalysisAuthorityInputV1, InvestigationAnalysisHostError,
    InvestigationAnalysisHostRepository, InvestigationAnalysisHostResult,
    InvestigationAnalysisSubjectAuthorityV1, InvestigationGenerationAdmissionView,
    LoadCommittedInvestigationAnalysisPostSynthesisAdmission,
    LoadCommittedInvestigationAnalysisPrimaryPostSynthesisAdmission,
    PrepareInvestigationAnalysisSubject, PreparedInvestigationAnalysisSubject,
    ResumeInvestigationAnalysisPostSynthesis, ResumeInvestigationAnalysisPrimaryPostSynthesis,
    ResumedInvestigationAnalysisPostSynthesisView, UnifiedInvestigationSubjectKind,
};
use golish_agent_kit::harness::hypothesis_registry::RevisionSourceRef;
use golish_agent_kit::task_orchestrator::hypothesis_analysis::{
    CandidateHypothesisProposal, CandidateProofReferenceRole, CandidateProposalReadiness,
};
use golish_db::repo::investigation_asset_verification as asset_verification;
use golish_db::repo::{
    hypothesis_registry::{
        CandidateMutationRouteRow, CandidateMutationRow, CandidateRevisionSourceRefRow,
    },
    investigation_analysis_bindings::{
        BindInvestigationAnalysisAttemptInput, InvestigationAnalysisBindingStoreError,
        PgInvestigationAnalysisBindingRepository,
    },
    investigation_hypothesis_compiler::{
        apply_investigation_compilation, load_pending_discovery_compiler_proposals,
        prepare_investigation_compilation, ApplyInvestigationCompilationInput,
        InvestigationProofRefInput, InvestigationProposalInput,
        PrepareInvestigationCompilationInput,
    },
    unified_investigation_runtime::{InvestigationStageIdentity, InvestigationUnitIdentity},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::hypothesis_registry::PgHypothesisRegistryRepository;

#[derive(Clone)]
pub struct PgInvestigationAnalysisHostRepository {
    pool: Arc<PgPool>,
    registry: Arc<dyn HypothesisRegistryRepository>,
    bindings: PgInvestigationAnalysisBindingRepository,
}

#[derive(sqlx::FromRow)]
struct AnalysisPostSynthesisResumeRow {
    current_state: String,
    head_version: i64,
    latest_event_id: Option<Uuid>,
    observed_stop_epoch: i64,
    checkpoint_sha256: String,
    synthesis_event_sha256: String,
    latest_reason_code: String,
}

#[derive(sqlx::FromRow)]
struct AnalysisPostSynthesisCommittedArtifactCountsRow {
    current_state: String,
    checkpoint_sha256: String,
    decision_count: i64,
    apply_receipt_count: i64,
    admission_count: i64,
}

#[derive(sqlx::FromRow)]
struct AnalysisPostSynthesisCommittedAdmissionRow {
    compilation_decision_id: Uuid,
    apply_receipt_id: Uuid,
    generation_id: Uuid,
    generation_ordinal: i32,
    generation_seal_id: Uuid,
    generation_member_count: i64,
    admission_set_id: Uuid,
    verification_task_ids: Vec<Uuid>,
}

fn complete_committed_artifact_set(
    decision_count: i64,
    apply_receipt_count: i64,
    admission_count: i64,
) -> Result<bool, &'static str> {
    match (decision_count, apply_receipt_count, admission_count) {
        (0, 0, 0) => Ok(false),
        (1, 1, 1) => Ok(true),
        _ => Err("post-synthesis committed admission artifacts are partial or collide"),
    }
}

struct CommittedEvolutionFixedPointAuthority<'a> {
    stable_compilation_request_id: Uuid,
    stable_apply_request_id: Uuid,
    identity: &'a golish_agent_kit::db_traits::UnifiedInvestigationUnitIdentity,
    work_id: Uuid,
    prepared_subject: &'a PreparedInvestigationAnalysisSubject,
    task_plan_id: Uuid,
    delegation_census_seal_id: Uuid,
    primary_worker_run_id: Uuid,
}

async fn load_committed_evolution_fixed_point_on(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    authority: CommittedEvolutionFixedPointAuthority<'_>,
) -> InvestigationAnalysisHostResult<Option<InvestigationGenerationAdmissionView>> {
    let Some(pending_evolution_authority_id) =
        authority.prepared_subject.pending_evolution_authority_id
    else {
        return Ok(None);
    };
    let expected_decision_id = Uuid::new_v5(
        &authority.stable_compilation_request_id,
        b"investigation_hypothesis_compilation_decision.v1",
    );
    let row = sqlx::query_as::<_, (Uuid, Uuid, i32, Uuid, i64, Uuid)>(
        r#"SELECT decision.decision_id,generation.generation_id,
                  generation.generation_ordinal,seal.seal_id,seal.member_count,
                  admission.admission_set_id
             FROM investigation_evolution_fixed_point_apply_receipts fixed_apply
             JOIN investigation_hypothesis_compilation_decisions decision
               ON decision.decision_id=fixed_apply.decision_id
              AND decision.operation_id=fixed_apply.operation_id
              AND decision.organization_id=fixed_apply.organization_id
             JOIN hypothesis_pending_evolution_authorities pending
               ON pending.pending_evolution_authority_id=fixed_apply.pending_evolution_authority_id
              AND pending.consolidation_batch_id=fixed_apply.consolidation_batch_id
              AND pending.source_generation_id=fixed_apply.source_generation_id
              AND pending.operation_id=fixed_apply.operation_id
              AND pending.project_scope_id=fixed_apply.project_scope_id
              AND pending.organization_id=fixed_apply.organization_id
             JOIN candidate_analysis_snapshots snapshot
               ON snapshot.snapshot_id=decision.candidate_snapshot_id
              AND snapshot.operation_id=decision.operation_id
              AND snapshot.organization_id=decision.organization_id
             JOIN hypothesis_generation_seals source_snapshot_seal
               ON source_snapshot_seal.seal_id=snapshot.previous_generation_seal_id
              AND source_snapshot_seal.generation_id=pending.source_generation_id
             JOIN hypothesis_generations generation
               ON generation.generation_id=pending.source_generation_id
              AND generation.operation_id=pending.operation_id
              AND generation.organization_id=pending.organization_id
             JOIN hypothesis_generation_seals seal
               ON seal.generation_id=generation.generation_id
             JOIN verification_admission_sets admission
               ON admission.generation_id=generation.generation_id
              AND admission.status='sealed'
             JOIN investigation_hypothesis_canonical_apply_receipts source_apply
               ON source_apply.generation_id=generation.generation_id
            WHERE fixed_apply.stable_request_id=$1
              AND fixed_apply.pending_evolution_authority_id=$2
              AND decision.decision_id=$3 AND decision.stable_request_id=$4
              AND decision.binding_id=$5 AND decision.authority_id=$6
              AND decision.operation_id=$7 AND decision.stage_execution_id=$8
              AND decision.stage_run_unit_id=$9 AND decision.organization_id=$10
              AND decision.work_id=$11 AND decision.candidate_snapshot_id=$12
              AND decision.analysis_attempt_id=$13 AND decision.task_plan_id=$14
              AND decision.delegation_census_seal_id=$15
              AND decision.primary_worker_run_id=$16
              AND snapshot.scope_snapshot_id=$17
              AND decision.cognitive_output_schema='investigation_cognitive_output.v1'
              AND decision.mutation_count=(SELECT COUNT(*)
                    FROM investigation_hypothesis_compilation_members member
                   WHERE member.decision_id=decision.decision_id)
              AND decision.proof_member_count=(SELECT COUNT(*)
                    FROM investigation_hypothesis_compilation_proof_members proof
                   WHERE proof.decision_id=decision.decision_id)
              AND NOT EXISTS(SELECT 1
                    FROM investigation_hypothesis_compilation_members member
                   WHERE member.decision_id=decision.decision_id
                     AND member.route_kind='create_initial')
              AND NOT EXISTS(SELECT 1
                    FROM investigation_hypothesis_canonical_apply_receipts current_apply
                   WHERE current_apply.decision_id=decision.decision_id)
            FOR SHARE OF fixed_apply,decision,pending,snapshot,generation,seal,admission"#,
    )
    .bind(authority.stable_apply_request_id)
    .bind(pending_evolution_authority_id)
    .bind(expected_decision_id)
    .bind(authority.stable_compilation_request_id)
    .bind(authority.prepared_subject.binding_id)
    .bind(authority.identity.stage.authority_id)
    .bind(authority.identity.stage.operation_id)
    .bind(authority.identity.stage.stage_execution_id)
    .bind(authority.identity.stage_run_unit_id)
    .bind(authority.identity.organization_id)
    .bind(authority.work_id)
    .bind(authority.prepared_subject.candidate_snapshot_id)
    .bind(authority.prepared_subject.analysis_attempt_id)
    .bind(authority.task_plan_id)
    .bind(authority.delegation_census_seal_id)
    .bind(authority.primary_worker_run_id)
    .bind(authority.identity.stage.scope_snapshot_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(map_sqlx_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(InvestigationGenerationAdmissionView {
        compilation_decision_id: row.0,
        generation_id: row.1,
        generation_ordinal: u32::try_from(row.2).map_err(|_| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "fixed-point source generation ordinal is negative".to_owned(),
            }
        })?,
        generation_seal_id: row.3,
        generation_member_count: u32::try_from(row.4).map_err(|_| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "fixed-point source generation member count overflowed".to_owned(),
            }
        })?,
        verification_task_ids: Vec::new(),
        evolution_fixed_point: true,
        replayed: true,
    }))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("serialize JSON string"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize JSON key"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256_json(value: &Value) -> String {
    let digest = Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{digest}")
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

impl PgInvestigationAnalysisHostRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self::with_registry(
            pool.clone(),
            Arc::new(PgHypothesisRegistryRepository::new(pool)),
        )
    }

    pub fn with_registry(
        pool: Arc<PgPool>,
        registry: Arc<dyn HypothesisRegistryRepository>,
    ) -> Self {
        Self {
            pool: pool.clone(),
            registry,
            bindings: PgInvestigationAnalysisBindingRepository::new(pool),
        }
    }

    async fn load_analysis_authority_inputs(
        &self,
        snapshot_id: Uuid,
    ) -> InvestigationAnalysisHostResult<Vec<InvestigationAnalysisAuthorityInputV1>> {
        const MAX_INPUTS: usize = 256;
        const MAX_CHUNKS: usize = 1_024;
        const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;

        let rows = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                String,
                String,
                i64,
                Uuid,
                i32,
                String,
                serde_json::Value,
            ),
        >(
            r#"SELECT input.snapshot_input_id,input.stable_input_key,input.source_kind,
                      input.source_content_hash,input.source_byte_count,chunk.chunk_id,
                      chunk.ordinal,chunk.chunk_hash,chunk.immutable_redacted_body
                 FROM candidate_analysis_snapshot_inputs input
                 JOIN candidate_analysis_input_chunk_censuses census
                   ON census.snapshot_input_id=input.snapshot_input_id
                  AND census.snapshot_id=input.snapshot_id
                 JOIN candidate_analysis_input_chunk_census_members chunk
                   ON chunk.chunk_census_id=census.chunk_census_id
                  AND chunk.snapshot_input_id=input.snapshot_input_id
                  AND chunk.snapshot_id=input.snapshot_id
                WHERE input.snapshot_id=$1 AND census.disposition='complete'
                ORDER BY input.stable_input_key,input.snapshot_input_id,chunk.ordinal"#,
        )
        .bind(snapshot_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|error| InvestigationAnalysisHostError::Infrastructure {
            detail: error.to_string(),
        })?;
        if rows.len() > MAX_CHUNKS {
            return Err(InvestigationAnalysisHostError::SnapshotBlocked {
                detail: "Candidate proof authority exceeds the bounded chunk count".to_owned(),
            });
        }

        let mut inputs = Vec::<InvestigationAnalysisAuthorityInputV1>::new();
        let mut expected_byte_count = Vec::<usize>::new();
        let mut body_bytes = Vec::<Vec<u8>>::new();
        for (
            input_id,
            stable_input_key,
            source_kind,
            source_sha256,
            source_byte_count,
            chunk_id,
            chunk_ordinal,
            chunk_sha256,
            immutable_body,
        ) in rows
        {
            let encoded = immutable_body
                .get("canonical_source_fragment")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "Candidate proof chunk has no canonical source fragment".to_owned(),
                })?;
            let decoded = decode_lower_hex(encoded).ok_or_else(|| {
                InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "Candidate proof chunk has invalid canonical hex".to_owned(),
                }
            })?;
            let needs_new = inputs.last().is_none_or(|input| input.input_id != input_id);
            if needs_new {
                if inputs.len() >= MAX_INPUTS {
                    return Err(InvestigationAnalysisHostError::SnapshotBlocked {
                        detail: "Candidate proof authority exceeds the bounded input count"
                            .to_owned(),
                    });
                }
                let expected = usize::try_from(source_byte_count).map_err(|_| {
                    InvestigationAnalysisHostError::AuthorityMismatch {
                        detail: "Candidate proof input has an invalid byte count".to_owned(),
                    }
                })?;
                inputs.push(InvestigationAnalysisAuthorityInputV1 {
                    input_id,
                    stable_input_key,
                    source_kind,
                    source_sha256,
                    body: String::new(),
                    chunks: Vec::new(),
                });
                expected_byte_count.push(expected);
                body_bytes.push(Vec::with_capacity(expected));
            }
            let input = inputs.last_mut().expect("authority input was initialized");
            let expected_ordinal = u32::try_from(input.chunks.len()).unwrap_or(u32::MAX);
            let ordinal = u32::try_from(chunk_ordinal).map_err(|_| {
                InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "Candidate proof chunk has a negative ordinal".to_owned(),
                }
            })?;
            if input.input_id != input_id || ordinal != expected_ordinal {
                return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "Candidate proof chunk census is not contiguous".to_owned(),
                });
            }
            input.chunks.push(InvestigationAnalysisAuthorityChunkV1 {
                chunk_id,
                chunk_ordinal: ordinal,
                chunk_sha256,
            });
            body_bytes
                .last_mut()
                .expect("authority body was initialized")
                .extend_from_slice(&decoded);
        }

        let total_bytes = body_bytes.iter().map(Vec::len).sum::<usize>();
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(InvestigationAnalysisHostError::SnapshotBlocked {
                detail: "Candidate proof authority exceeds the bounded prompt byte count"
                    .to_owned(),
            });
        }
        for ((input, bytes), expected) in inputs.iter_mut().zip(body_bytes).zip(expected_byte_count)
        {
            if bytes.len() != expected {
                return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "Candidate proof input byte count differs from its sealed census"
                        .to_owned(),
                });
            }
            input.body = String::from_utf8(bytes).map_err(|_| {
                InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "Candidate proof input is not canonical UTF-8 JSON".to_owned(),
                }
            })?;
        }
        Ok(inputs)
    }
}

fn decode_lower_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let mut decoded = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let high = (pair[0] as char).to_digit(16)?;
        let low = (pair[1] as char).to_digit(16)?;
        decoded.push(u8::try_from((high << 4) | low).ok()?);
    }
    Some(decoded)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrozenToolTruthPrerequisiteMember {
    root_family: String,
    member_status: String,
    root_operation_id: Uuid,
    fork_source_operation_id: Option<Uuid>,
    revalidation_obligation_ids: Vec<Uuid>,
}

fn stage_rank(stage: &str) -> Option<u8> {
    match stage {
        "target_intel" => Some(0),
        "external_attack_surface" => Some(1),
        "enumeration" => Some(2),
        "vuln_triage" => Some(3),
        _ => None,
    }
}

fn root_family_stage(root_family: &str) -> Option<&'static str> {
    match root_family {
        "ti" => Some("target_intel"),
        "eas" => Some("external_attack_surface"),
        "enum" => Some("enumeration"),
        "vuln" => Some("vuln_triage"),
        _ => None,
    }
}

/// Adopted fork predecessors are immutable historical Analysis context. They
/// never authorize a dynamic verification tool invocation: that later path
/// has its own exact lane/target/JIT/budget authority. Consequently an expired
/// adopted root must remain visible to the roles as potentially stale evidence,
/// but must not force the CLI to rerun already sealed predecessor stages.
/// Same-operation stale roots still require their exact frozen revalidation
/// obligations because they represent mutable work in the current operation.
fn stale_tool_truth_prerequisite(
    operation_id: Uuid,
    members: &[FrozenToolTruthPrerequisiteMember],
) -> InvestigationAnalysisHostResult<Option<InvestigationAnalysisHostError>> {
    let stale_predecessors = members
        .iter()
        .filter(|member| {
            member.root_operation_id != operation_id && member.member_status != "consistent_fresh"
        })
        .collect::<Vec<_>>();
    if stale_predecessors.is_empty() {
        let mut same_operation_stale = members
            .iter()
            .filter(|member| {
                member.root_operation_id == operation_id
                    && member.member_status != "consistent_fresh"
            })
            .collect::<Vec<_>>();
        if same_operation_stale.is_empty() {
            return Ok(None);
        }
        same_operation_stale.sort_by_key(|member| {
            root_family_stage(&member.root_family)
                .and_then(stage_rank)
                .unwrap_or(u8::MAX)
        });
        let mut stale_roots = Vec::with_capacity(same_operation_stale.len());
        let mut revalidation_obligation_ids = Vec::new();
        for member in same_operation_stale {
            let stage = root_family_stage(&member.root_family).ok_or_else(|| {
                InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: format!(
                        "stale same-operation root family is outside the closed catalog: {}",
                        member.root_family
                    ),
                }
            })?;
            if !matches!(
                member.member_status.as_str(),
                "expired" | "semantic_invalid" | "mixed_epoch" | "skew_exceeded"
            ) {
                return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: format!(
                        "stale same-operation member status is outside the closed catalog: {}",
                        member.member_status
                    ),
                });
            }
            stale_roots.push(format!("{stage}:{}", member.member_status));
            revalidation_obligation_ids.extend_from_slice(&member.revalidation_obligation_ids);
        }
        stale_roots.sort();
        stale_roots.dedup();
        revalidation_obligation_ids.sort_unstable();
        revalidation_obligation_ids.dedup();
        if revalidation_obligation_ids.is_empty() {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "stale same-operation roots have no frozen Tool Truth revalidation obligation authority"
                    .to_owned(),
            });
        }
        return Ok(Some(InvestigationAnalysisHostError::RevalidationRequired {
            operation_id,
            revalidation_obligation_ids,
            stale_roots,
        }));
    }
    let source_operation_id = stale_predecessors[0].root_operation_id;
    if source_operation_id.is_nil()
        || stale_predecessors.iter().any(|member| {
            member.root_operation_id != source_operation_id
                || member.fork_source_operation_id != Some(source_operation_id)
        })
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "stale predecessor roots do not share the exact stage-fork source authority"
                .to_owned(),
        });
    }
    for member in stale_predecessors {
        root_family_stage(&member.root_family).ok_or_else(|| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: format!(
                    "stale predecessor root family is outside the closed catalog: {}",
                    member.root_family
                ),
            }
        })?;
        if member.member_status != "expired" {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: format!(
                    "adopted predecessor is not valid immutable historical context: {}",
                    member.member_status
                ),
            });
        }
    }
    Ok(None)
}

#[async_trait]
impl InvestigationAnalysisHostRepository for PgInvestigationAnalysisHostRepository {
    async fn prepare_analysis_subject(
        &self,
        request: PrepareInvestigationAnalysisSubject,
    ) -> InvestigationAnalysisHostResult<PreparedInvestigationAnalysisSubject> {
        if request.stable_request_id.is_nil()
            || request.work_id.is_nil()
            || request.asset_lane_id.is_nil()
        {
            return Err(InvestigationAnalysisHostError::InvalidRequest {
                detail: "stable_request_id, work_id, and asset_lane_id must be non-nil".to_owned(),
            });
        }
        let snapshot = self
            .registry
            .freeze_candidate_snapshot(FreezeCandidateAnalysisSnapshot {
                stable_consumer_request_id: request.stable_request_id,
                operation_id: request.identity.stage.operation_id,
                scope_snapshot_id: request.identity.stage.scope_snapshot_id,
                organization_id: request.identity.organization_id,
                asset_lane_id: request.asset_lane_id,
            })
            .await
            .map_err(map_registry_error)?;
        if snapshot.asset_lane_id != Some(request.asset_lane_id) {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "Candidate snapshot escaped the active asset lane".to_owned(),
            });
        }
        if !matches!(
            snapshot.disposition,
            CandidateAnalysisSnapshotDispositionV1::SealedReady
                | CandidateAnalysisSnapshotDispositionV1::SealedAnalysisReadyWithResiduals
        ) {
            return Err(InvestigationAnalysisHostError::SnapshotBlocked {
                detail: format!(
                    "Candidate snapshot {} is not authorized for unified analysis",
                    snapshot.snapshot_id
                ),
            });
        }
        let prerequisite_members =
            sqlx::query_as::<_, (String, String, Uuid, Option<Uuid>, Vec<Uuid>)>(
                r#"SELECT member.root_family,member.member_status,root.operation_id,
                      fork.source_operation_id,
                      COALESCE(ARRAY(
                          SELECT obligation.tool_truth_revalidation_obligation_id
                            FROM candidate_analysis_revalidation_obligations obligation
                           WHERE obligation.snapshot_id=$2
                             AND obligation.root_family=member.root_family
                             AND obligation.tool_truth_revalidation_obligation_id IS NOT NULL
                           ORDER BY obligation.tool_truth_revalidation_obligation_id
                      ),ARRAY[]::UUID[]) AS revalidation_obligation_ids
                 FROM tool_truth_authority_bundle_members member
                 JOIN coverage_denominators root
                   ON root.id=member.root_denominator_id
                  AND root.execution_authority_id=member.root_execution_authority_id
                 LEFT JOIN operation_stage_forks fork
                   ON fork.operation_id=member.operation_id
                WHERE member.bundle_seal_id=$1
                ORDER BY member.ordinal"#,
            )
            .bind(snapshot.tool_truth_authority_bundle_seal_id)
            .bind(snapshot.snapshot_id)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|error| InvestigationAnalysisHostError::Infrastructure {
                detail: error.to_string(),
            })?
            .into_iter()
            .map(
                |(
                    root_family,
                    member_status,
                    root_operation_id,
                    fork_source_operation_id,
                    revalidation_obligation_ids,
                )| {
                    FrozenToolTruthPrerequisiteMember {
                        root_family,
                        member_status,
                        root_operation_id,
                        fork_source_operation_id,
                        revalidation_obligation_ids,
                    }
                },
            )
            .collect::<Vec<_>>();
        if prerequisite_members.len()
            != golish_pentest_domain::tool_truth::ToolTruthRootFamilyV1::EXECUTION_RECEIPT_ROOTS
                .len()
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "Candidate Tool Truth prerequisite census is incomplete".to_owned(),
            });
        }
        if let Some(blocked) = stale_tool_truth_prerequisite(
            request.identity.stage.operation_id,
            &prerequisite_members,
        )? {
            return Err(blocked);
        }
        let open_pending_evolution_ids: Vec<Uuid> = sqlx::query_scalar(
            r#"SELECT pending.pending_evolution_authority_id
                 FROM candidate_analysis_snapshots candidate_snapshot
                 JOIN hypothesis_generation_seals source_seal
                   ON source_seal.seal_id=candidate_snapshot.previous_generation_seal_id
                 JOIN hypothesis_pending_evolution_authorities pending
                   ON pending.source_generation_id=source_seal.generation_id
                  AND pending.operation_id=candidate_snapshot.operation_id
                  AND pending.organization_id=candidate_snapshot.organization_id
                  AND pending.asset_lane_id=candidate_snapshot.asset_lane_id
                 JOIN operation_state operation
                   ON operation.operation_id=pending.operation_id
                  AND operation.project_scope_id=pending.project_scope_id
                 LEFT JOIN hypothesis_consolidation_receipts terminal
                   ON terminal.consolidation_batch_id=pending.consolidation_batch_id
                WHERE candidate_snapshot.snapshot_id=$1
                  AND candidate_snapshot.operation_id=$2
                  AND candidate_snapshot.organization_id=$3
                  AND candidate_snapshot.scope_snapshot_id=$4
                  AND candidate_snapshot.asset_lane_id=$5
                  AND (
                        terminal.consolidation_receipt_id IS NULL
                        OR EXISTS(
                            SELECT 1
                              FROM investigation_analysis_attempt_bindings replay_binding
                             WHERE replay_binding.stable_request_id=$6
                               AND replay_binding.candidate_snapshot_id=candidate_snapshot.snapshot_id
                               AND replay_binding.operation_id=pending.operation_id
                               AND replay_binding.organization_id=pending.organization_id
                        )
                  )
                ORDER BY pending.pending_evolution_authority_id"#,
        )
        .bind(snapshot.snapshot_id)
        .bind(request.identity.stage.operation_id)
        .bind(request.identity.organization_id)
        .bind(request.identity.stage.scope_snapshot_id)
        .bind(request.asset_lane_id)
        .bind(request.stable_request_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|error| InvestigationAnalysisHostError::Infrastructure {
            detail: error.to_string(),
        })?;
        if open_pending_evolution_ids.len() > 1
            || open_pending_evolution_ids.first().copied() != request.pending_evolution_authority_id
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "Candidate snapshot pending evolution authority drifted".to_owned(),
            });
        }
        let attempt = self
            .bindings
            .load_ordinal_zero_attempt(
                request.identity.stage.operation_id,
                request.identity.organization_id,
                snapshot.snapshot_id,
            )
            .await
            .map_err(map_binding_error)?
            .ok_or_else(|| InvestigationAnalysisHostError::NotFound {
                detail: "sealed Candidate snapshot has no ordinal-zero analysis attempt".to_owned(),
            })?;
        let binding = self
            .bindings
            .bind(&BindInvestigationAnalysisAttemptInput {
                binding_id: Uuid::new_v5(
                    &request.stable_request_id,
                    b"unified_investigation_analysis_binding.v1",
                ),
                stable_request_id: request.stable_request_id,
                identity: InvestigationUnitIdentity {
                    stage: InvestigationStageIdentity {
                        authority_id: request.identity.stage.authority_id,
                        operation_id: request.identity.stage.operation_id,
                        stage_execution_id: request.identity.stage.stage_execution_id,
                        owning_stage_run_request_id: request
                            .identity
                            .stage
                            .owning_stage_run_request_id,
                        scope_snapshot_id: request.identity.stage.scope_snapshot_id,
                    },
                    stage_run_unit_id: request.identity.stage_run_unit_id,
                    organization_id: request.identity.organization_id,
                },
                work_id: request.work_id,
                candidate_snapshot_id: snapshot.snapshot_id,
                analysis_attempt_id: attempt.analysis_attempt_id,
            })
            .await
            .map_err(map_binding_error)?;
        let authority_inputs = self
            .load_analysis_authority_inputs(snapshot.snapshot_id)
            .await?;
        if authority_inputs.is_empty() {
            return Err(InvestigationAnalysisHostError::SnapshotBlocked {
                detail: "sealed Candidate snapshot exposes no readable proof authority".to_owned(),
            });
        }
        let subject_authorities = sqlx::query_as::<_, (String, Uuid, String, String)>(
            r#"WITH lane AS (
                   SELECT lane.asset_lane_id,lane.target_id,lane.organization_id
                     FROM investigation_asset_lanes lane
                    WHERE lane.asset_lane_id=$1
                      AND lane.operation_id=$2
                      AND lane.stage_execution_id=$3
                      AND lane.scope_snapshot_id=$4
                      AND lane.organization_id=$5
                      AND lane.state IN('analyzing','evolving')
               )
               SELECT subject_kind,subject_id,display_value,
                      tool_truth_sha256(jsonb_build_object(
                          'domain','investigation_subject_identity.v1',
                          'subject_kind',subject_kind,
                          'subject_id',subject_id,
                          'display_value',display_value
                      )::TEXT) AS subject_identity_hash
                 FROM (
                     SELECT 'asset'::TEXT AS subject_kind,target.id AS subject_id,
                            target.value AS display_value
                       FROM lane
                       JOIN targets target ON target.id=lane.target_id
                     UNION ALL
                     SELECT 'endpoint'::TEXT AS subject_kind,endpoint.id,endpoint.url
                       FROM lane
                       JOIN api_endpoints endpoint ON endpoint.target_id=lane.target_id
                      WHERE endpoint.target_id=lane.target_id
                     UNION ALL
                     SELECT 'web_origin'::TEXT,origin.id,origin.origin
                       FROM lane
                       JOIN fingerprint_origin_observations observation
                         ON observation.target_id=lane.target_id
                       JOIN web_origins origin
                         ON origin.id=observation.web_origin_id
                        AND origin.organization_id=lane.organization_id
                 ) subjects
                ORDER BY subject_kind,subject_id
                LIMIT 256"#,
        )
        .bind(request.asset_lane_id)
        .bind(request.identity.stage.operation_id)
        .bind(request.identity.stage.stage_execution_id)
        .bind(request.identity.stage.scope_snapshot_id)
        .bind(request.identity.organization_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|error| InvestigationAnalysisHostError::Infrastructure {
            detail: error.to_string(),
        })?
        .into_iter()
        .map(
            |(subject_kind, subject_id, display_value, subject_identity_hash)| {
                InvestigationAnalysisSubjectAuthorityV1 {
                    subject_id,
                    subject_kind,
                    display_value,
                    subject_identity_hash,
                }
            },
        )
        .collect::<Vec<_>>();
        if subject_authorities.is_empty() {
            return Err(InvestigationAnalysisHostError::SnapshotBlocked {
                detail: "sealed Candidate snapshot exposes no proposal subject authority"
                    .to_owned(),
            });
        }

        let subject_fingerprint_sha256 = if let Some(pending_evolution_authority_id) =
            request.pending_evolution_authority_id
        {
            sqlx::query_scalar(
                r#"SELECT investigation_evolution_analysis_subject_fingerprint($1,$2)"#,
            )
            .bind(pending_evolution_authority_id)
            .bind(&attempt.attempt_input_hash)
            .fetch_one(self.pool.as_ref())
            .await
            .map_err(|error| InvestigationAnalysisHostError::Infrastructure {
                detail: error.to_string(),
            })?
        } else {
            attempt.attempt_input_hash
        };
        Ok(PreparedInvestigationAnalysisSubject {
            subject_kind: UnifiedInvestigationSubjectKind::AnalysisAttempt,
            analysis_attempt_id: attempt.analysis_attempt_id,
            candidate_snapshot_id: snapshot.snapshot_id,
            candidate_snapshot_sha256: snapshot.snapshot_hash,
            subject_fingerprint_sha256,
            binding_id: binding.binding.binding_id,
            asset_lane_id: request.asset_lane_id,
            pending_evolution_authority_id: request.pending_evolution_authority_id,
            authority_inputs,
            subject_authorities,
            replayed: binding.replayed,
        })
    }

    async fn compile_seal_and_admit(
        &self,
        request: CompileSealAndAdmitInvestigationGeneration,
    ) -> InvestigationAnalysisHostResult<InvestigationGenerationAdmissionView> {
        if [
            request.stable_compilation_request_id,
            request.stable_apply_request_id,
            request.stable_admission_request_id,
            request.work_id,
            request.task_plan_id,
            request.delegation_census_seal_id,
            request.primary_worker_run_id,
        ]
        .into_iter()
        .any(|id| id.is_nil())
            || if request.advisory.candidate_proposals.is_empty() {
                !request.advisory.action_intents.is_empty()
                    || request.advisory.residuals.len() != 1
                    || !request.advisory.residuals[0].is_valid_no_hypothesis_residual()
            } else {
                !request.advisory.residuals.is_empty()
            }
            || request.advisory.candidate_proposals.iter().any(|proposal| {
                proposal.proof_refs.is_empty()
                    || proposal
                        .proof_refs
                        .iter()
                        .any(|proof| proof.role == CandidateProofReferenceRole::Gap)
                    || proposal.readiness != CandidateProposalReadiness::ReadyForStrategy
            })
        {
            return Err(InvestigationAnalysisHostError::InvalidRequest {
                detail: "canonical compiler ids/proposal set are invalid".to_owned(),
            });
        }
        let binding = self
            .bindings
            .load(
                &InvestigationUnitIdentity {
                    stage: InvestigationStageIdentity {
                        authority_id: request.identity.stage.authority_id,
                        operation_id: request.identity.stage.operation_id,
                        stage_execution_id: request.identity.stage.stage_execution_id,
                        owning_stage_run_request_id: request
                            .identity
                            .stage
                            .owning_stage_run_request_id
                            .clone(),
                        scope_snapshot_id: request.identity.stage.scope_snapshot_id,
                    },
                    stage_run_unit_id: request.identity.stage_run_unit_id,
                    organization_id: request.identity.organization_id,
                },
                request.work_id,
            )
            .await
            .map_err(map_binding_error)?
            .ok_or_else(|| InvestigationAnalysisHostError::NotFound {
                detail: "Investigation analysis binding is absent".to_owned(),
            })?;
        if binding.binding_id != request.prepared_subject.binding_id
            || binding.analysis_attempt_id != request.prepared_subject.analysis_attempt_id
            || binding.candidate_snapshot_id != request.prepared_subject.candidate_snapshot_id
            || request.prepared_subject.asset_lane_id.is_nil()
            || request.advisory.subject_id != request.prepared_subject.analysis_attempt_id
            || request.advisory.candidate_snapshot_id
                != request.prepared_subject.candidate_snapshot_id
            || request.advisory.subject_fingerprint_sha256
                != request.prepared_subject.subject_fingerprint_sha256
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "canonical compiler subject/binding authority drifted".to_owned(),
            });
        }
        let mut proposals = request
            .advisory
            .candidate_proposals
            .iter()
            .map(compiler_proposal_input)
            .collect::<InvestigationAnalysisHostResult<Vec<_>>>()?;
        let pending_discovery_proposals = load_pending_discovery_compiler_proposals(
            self.pool.as_ref(),
            request.identity.stage.operation_id,
            request.prepared_subject.asset_lane_id,
            request.prepared_subject.candidate_snapshot_id,
        )
        .await
        .map_err(map_compiler_db_error)?;
        let mut proposal_ids = proposals
            .iter()
            .map(|proposal| proposal.proposal_id)
            .collect::<std::collections::BTreeSet<_>>();
        for proposal in pending_discovery_proposals {
            if !proposal_ids.insert(proposal.proposal_id) {
                return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "pending discovery proposal duplicated the cognitive proposal set"
                        .to_owned(),
                });
            }
            proposals.push(proposal);
        }
        let canonical_action_intents = request
            .advisory
            .action_intents
            .iter()
            .map(|intent| {
                serde_json::json!({
                    "intent_id": intent.intent_id,
                    "proposal_id": intent.proposal_id,
                    "capability": advisory_capability_name(intent.capability),
                    "purpose_code": intent.purpose_code,
                    "evidence_authority_refs": intent.evidence_authority_refs,
                })
            })
            .collect();
        let prepared = prepare_investigation_compilation(
            self.pool.as_ref(),
            PrepareInvestigationCompilationInput {
                stable_compilation_request_id: request.stable_compilation_request_id,
                authority_id: request.identity.stage.authority_id,
                operation_id: request.identity.stage.operation_id,
                stage_execution_id: request.identity.stage.stage_execution_id,
                stage_run_unit_id: request.identity.stage_run_unit_id,
                scope_snapshot_id: request.identity.stage.scope_snapshot_id,
                organization_id: request.identity.organization_id,
                binding_id: request.prepared_subject.binding_id,
                work_id: request.work_id,
                candidate_snapshot_id: request.prepared_subject.candidate_snapshot_id,
                analysis_attempt_id: request.prepared_subject.analysis_attempt_id,
                task_plan_id: request.task_plan_id,
                delegation_census_seal_id: request.delegation_census_seal_id,
                primary_worker_run_id: request.primary_worker_run_id,
                pending_evolution_authority_id: request
                    .prepared_subject
                    .pending_evolution_authority_id,
                proposals,
                canonical_action_intents,
            },
        )
        .await
        .map_err(map_compiler_db_error)?;
        if prepared.asset_lane_id != request.prepared_subject.asset_lane_id {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "canonical compiler escaped the prepared asset lane".to_owned(),
            });
        }
        let compiled = crate::ai::candidate_analysis_gate::compile_candidate_host_recipe(
            &prepared.server_recipe,
        )
        .map_err(map_registry_error)?;
        let routes = &compiled.mutation_routes;
        let mutations = compiled
            .mutations
            .into_iter()
            .map(|mutation| {
                let route = routes.get(&mutation.proposal_id).cloned().ok_or_else(|| {
                    InvestigationAnalysisHostError::AuthorityMismatch {
                        detail: "Investigation compiler route exact set is open".to_owned(),
                    }
                })?;
                db_mutation(mutation, route)
            })
            .collect::<InvestigationAnalysisHostResult<Vec<_>>>()?;
        let applied = apply_investigation_compilation(
            self.pool.as_ref(),
            ApplyInvestigationCompilationInput {
                prepared,
                stable_apply_request_id: request.stable_apply_request_id,
                stable_admission_request_id: request.stable_admission_request_id,
                pending_evolution_authority_id: request
                    .prepared_subject
                    .pending_evolution_authority_id,
                mutations,
                claim_components: compiled.claim_components,
                verification_contracts: compiled.verification_contracts,
                verification_plans: compiled.verification_plans,
            },
        )
        .await
        .map_err(map_compiler_db_error)?;
        for discovery in asset_verification::list_pending_hypothesis_discoveries(
            self.pool.as_ref(),
            request.identity.stage.operation_id,
            request.prepared_subject.asset_lane_id,
        )
        .await
        .map_err(map_compiler_db_error)?
        {
            asset_verification::admit_or_dismiss_pending_hypothesis_discovery(
                self.pool.as_ref(),
                &asset_verification::AdmitOrDismissPendingHypothesisDiscoveryInput {
                    stable_request_id: Uuid::new_v5(
                        &discovery.discovery_authority_id,
                        b"investigation-pending-discovery-post-compiler-consumption.v1",
                    ),
                    discovery_authority_id: discovery.discovery_authority_id,
                    expected_asset_lane_id: request.prepared_subject.asset_lane_id,
                    expected_session_id: discovery.session_id,
                },
            )
            .await
            .map_err(map_compiler_db_error)?;
        }
        Ok(InvestigationGenerationAdmissionView {
            compilation_decision_id: applied.compilation_decision_id,
            generation_id: applied.generation_id,
            generation_ordinal: u32::try_from(applied.generation_ordinal).map_err(|_| {
                InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "canonical generation ordinal is negative".to_owned(),
                }
            })?,
            generation_seal_id: applied.generation_seal_id,
            generation_member_count: u32::try_from(applied.generation_member_count).map_err(
                |_| InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "canonical generation member count overflow".to_owned(),
                },
            )?,
            verification_task_ids: applied.verification_task_ids,
            evolution_fixed_point: applied.evolution_fixed_point,
            replayed: applied.replayed,
        })
    }

    async fn load_committed_analysis_primary_post_synthesis_admission(
        &self,
        request: LoadCommittedInvestigationAnalysisPrimaryPostSynthesisAdmission,
    ) -> InvestigationAnalysisHostResult<Option<InvestigationGenerationAdmissionView>> {
        if [
            request.stable_compilation_request_id,
            request.stable_apply_request_id,
            request.stable_admission_request_id,
            request.work_id,
            request.task_plan_id,
            request.delegation_census_seal_id,
            request.primary_work_item_id,
            request.primary_worker_run_id,
        ]
        .into_iter()
        .any(|id| id.is_nil())
            || !valid_sha256(&request.primary_synthesis_event_sha256)
        {
            return Err(InvestigationAnalysisHostError::InvalidRequest {
                detail: "normal Primary committed post-synthesis ids/hash are invalid".to_owned(),
            });
        }
        let expected_decision_id = Uuid::new_v5(
            &request.stable_compilation_request_id,
            b"investigation_hypothesis_compilation_decision.v1",
        );
        let expected_apply_receipt_id = Uuid::new_v5(
            &request.stable_apply_request_id,
            b"investigation_hypothesis_canonical_apply_receipt.v1",
        );
        let expected_admission_set_id = Uuid::new_v5(
            &request.stable_admission_request_id,
            b"verification_admission_set.v1",
        );
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        let counts = sqlx::query_as::<_, AnalysisPostSynthesisCommittedArtifactCountsRow>(
            r#"SELECT work.current_state,
                      tool_truth_sha256(primary_worker.checkpoint::TEXT) AS checkpoint_sha256,
                      (SELECT COUNT(*)
                         FROM investigation_hypothesis_compilation_decisions decision
                        WHERE decision.decision_id=$20 OR decision.stable_request_id=$7
                           OR decision.binding_id=$11 OR decision.task_plan_id=$2) AS decision_count,
                      (SELECT COUNT(*)
                         FROM investigation_hypothesis_canonical_apply_receipts receipt
                        WHERE receipt.apply_receipt_id=$21 OR receipt.stable_request_id=$8
                           OR receipt.decision_id=$20) AS apply_receipt_count,
                      (SELECT COUNT(*)
                         FROM verification_admission_sets admission
                        WHERE admission.admission_set_id=$22 OR admission.stable_request_id=$9
                           OR admission.generation_id IN (
                               SELECT receipt.generation_id
                                 FROM investigation_hypothesis_canonical_apply_receipts receipt
                                WHERE receipt.decision_id=$20
                           )) AS admission_count
                 FROM investigation_run_work_items work
                 JOIN investigation_run_work_state_events latest
                   ON latest.event_id=work.latest_event_id AND latest.work_id=work.work_id
                 JOIN investigation_analysis_attempt_bindings binding
                   ON binding.work_id=work.work_id AND binding.authority_id=work.authority_id
                  AND binding.binding_id=$11 AND binding.candidate_snapshot_id=$12
                  AND binding.analysis_attempt_id=$13
                 JOIN investigation_pentagi_task_plans task_plan
                   ON task_plan.task_plan_id=$2 AND task_plan.authority_id=work.authority_id
                  AND task_plan.operation_id=work.operation_id
                  AND task_plan.stage_execution_id=work.stage_execution_id
                  AND task_plan.stage_run_unit_id=work.stage_run_unit_id
                  AND task_plan.organization_id=work.organization_id
                  AND task_plan.subject_kind='analysis_attempt'
                  AND task_plan.subject_id=binding.analysis_attempt_id
                  AND task_plan.subject_fingerprint_sha256=$14 AND task_plan.status='sealed'
                 JOIN investigation_refiner_plan_ledger_seals refiner_seal
                   ON refiner_seal.task_plan_id=task_plan.task_plan_id
                 JOIN investigation_pentagi_delegation_census_seals census
                   ON census.census_seal_id=$6 AND census.task_plan_id=task_plan.task_plan_id
                  AND census.primary_worker_run_id=$4
                 JOIN investigation_pentagi_pipeline_events synthesis
                   ON synthesis.task_plan_id=task_plan.task_plan_id
                  AND synthesis.event_kind='primary_synthesis'
                  AND synthesis.actor_worker_run_id=$4
                  AND synthesis.parent_dispatch_receipt_id=census.primary_dispatch_receipt_id
                  AND synthesis.event_sha256=$5
                 JOIN pentagi_logical_dispatch_receipts dispatch
                   ON dispatch.dispatch_receipt_id=census.primary_dispatch_receipt_id
                  AND dispatch.task_plan_id=task_plan.task_plan_id
                  AND dispatch.actor_kind='primary' AND dispatch.subtask_id IS NULL
                  AND dispatch.stage_work_item_id=$3 AND dispatch.worker_run_id=$4
                 JOIN stage_work_items primary_item
                  ON primary_item.id=$3 AND primary_item.team_plan_id=task_plan.stage_team_plan_id
                  AND primary_item.stable_key='leader:primary'
                  AND primary_item.kind='investigation_primary'
                  AND primary_item.role=(SELECT leader_role FROM stage_team_plans WHERE id=task_plan.stage_team_plan_id)
                  AND primary_item.created_by='server_seed' AND primary_item.required_for_barrier=FALSE
                  AND primary_item.status='completed' AND primary_item.terminal_at IS NOT NULL
                 JOIN stage_worker_runs primary_worker
                   ON primary_worker.id=$4 AND primary_worker.work_item_id=primary_item.id
                  AND primary_worker.status='passed' AND primary_worker.terminal_at IS NOT NULL
                  AND primary_worker.lease_token IS NULL
                  AND primary_worker.active_tool_call_id IS NULL
                  AND jsonb_typeof(primary_worker.checkpoint)='array'
                WHERE work.work_id=$1 AND work.authority_id=$10
                  AND work.operation_id=$15 AND work.stage_execution_id=$16
                  AND work.stage_run_unit_id=$17 AND work.scope_snapshot_id=$18
                  AND work.organization_id=$19 AND work.work_kind='analysis'
                  AND (
                      (work.current_state='blocked' AND latest.to_state='blocked'
                       AND latest.reason_code IN (
                           'investigation_analysis_host_infrastructure',
                           'investigation_analysis_host_authority_mismatch'
                       ))
                      OR
                      (work.current_state='running' AND latest.to_state='running'
                       AND latest.reason_code='post_synthesis_analysis_primary_recovery.v1|'
                           || synthesis.event_sha256 || '|'
                           || tool_truth_sha256(primary_worker.checkpoint::TEXT))
                      OR
                      (work.current_state IN ('completed','residual')
                       AND latest.to_state=work.current_state
                       AND latest.reason_code IN (
                           'canonical_generation_sealed_and_admitted',
                           'canonical_generation_admitted_with_residuals'
                       ))
                  )
                  AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                        WHERE all_worker.work_item_id=primary_item.id)=1
                  AND (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events event
                        WHERE event.task_plan_id=task_plan.task_plan_id
                          AND event.event_kind='primary_synthesis')=1
                  AND (SELECT COUNT(*) FROM jsonb_path_query(
                        primary_worker.checkpoint,
                        'strict $.** ? (@.name == "submit_result")'))=1
                  AND unified_investigation_submit_result_v1(primary_worker.checkpoint)
                        IS NOT NULL"#,
        )
        .bind(request.work_id)
        .bind(request.task_plan_id)
        .bind(request.primary_work_item_id)
        .bind(request.primary_worker_run_id)
        .bind(&request.primary_synthesis_event_sha256)
        .bind(request.delegation_census_seal_id)
        .bind(request.stable_compilation_request_id)
        .bind(request.stable_apply_request_id)
        .bind(request.stable_admission_request_id)
        .bind(request.identity.stage.authority_id)
        .bind(request.prepared_subject.binding_id)
        .bind(request.prepared_subject.candidate_snapshot_id)
        .bind(request.prepared_subject.analysis_attempt_id)
        .bind(&request.prepared_subject.subject_fingerprint_sha256)
        .bind(request.identity.stage.operation_id)
        .bind(request.identity.stage.stage_execution_id)
        .bind(request.identity.stage_run_unit_id)
        .bind(request.identity.stage.scope_snapshot_id)
        .bind(request.identity.organization_id)
        .bind(expected_decision_id)
        .bind(expected_apply_receipt_id)
        .bind(expected_admission_set_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "normal Primary post-synthesis witness is not exact".to_owned(),
        })?;
        if !valid_sha256(&counts.checkpoint_sha256) {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "normal Primary checkpoint hash drifted".to_owned(),
            });
        }
        if (
            counts.decision_count,
            counts.apply_receipt_count,
            counts.admission_count,
        ) == (1, 0, 0)
        {
            let fixed_point = load_committed_evolution_fixed_point_on(
                &mut tx,
                CommittedEvolutionFixedPointAuthority {
                    stable_compilation_request_id: request.stable_compilation_request_id,
                    stable_apply_request_id: request.stable_apply_request_id,
                    identity: &request.identity,
                    work_id: request.work_id,
                    prepared_subject: &request.prepared_subject,
                    task_plan_id: request.task_plan_id,
                    delegation_census_seal_id: request.delegation_census_seal_id,
                    primary_worker_run_id: request.primary_worker_run_id,
                },
            )
            .await?
            .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "normal Primary fixed-point replay authority is partial or foreign"
                    .to_owned(),
            })?;
            tx.commit().await.map_err(map_sqlx_error)?;
            return Ok(Some(fixed_point));
        }
        if !complete_committed_artifact_set(
            counts.decision_count,
            counts.apply_receipt_count,
            counts.admission_count,
        )
        .map_err(|detail| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: detail.to_owned(),
        })? {
            tx.commit().await.map_err(map_sqlx_error)?;
            return Ok(None);
        }
        let row = sqlx::query_as::<_, AnalysisPostSynthesisCommittedAdmissionRow>(
            r#"SELECT decision.decision_id AS compilation_decision_id,
                      receipt.apply_receipt_id,generation.generation_id,
                      generation.generation_ordinal,seal.seal_id AS generation_seal_id,
                      seal.member_count AS generation_member_count,admission.admission_set_id,
                      ARRAY(SELECT member.task_id FROM verification_admission_members member
                             WHERE member.admission_set_id=admission.admission_set_id
                               AND member.disposition='scheduled'
                             ORDER BY member.hypothesis_revision_id) AS verification_task_ids
                 FROM investigation_hypothesis_compilation_decisions decision
                 JOIN investigation_hypothesis_canonical_apply_receipts receipt
                   ON receipt.apply_receipt_id=$12 AND receipt.stable_request_id=$2
                  AND receipt.decision_id=decision.decision_id
                  AND receipt.operation_id=decision.operation_id
                  AND receipt.organization_id=decision.organization_id
                 JOIN hypothesis_generations generation
                   ON generation.generation_id=receipt.generation_id
                  AND generation.operation_id=decision.operation_id
                  AND generation.organization_id=decision.organization_id
                  AND generation.candidate_snapshot_id=decision.candidate_snapshot_id
                  AND generation.investigation_compilation_decision_id=decision.decision_id
                  AND generation.candidate_gate_decision_id IS NULL
                 JOIN hypothesis_generation_seals seal
                   ON seal.seal_id=receipt.generation_seal_id
                  AND seal.generation_id=generation.generation_id
                  AND seal.controller_worker_run_id=decision.primary_worker_run_id
                 JOIN verification_admission_sets admission
                   ON admission.admission_set_id=$13 AND admission.stable_request_id=$3
                  AND admission.operation_id=decision.operation_id
                  AND admission.stage_execution_id=decision.stage_execution_id
                  AND admission.stage_run_unit_id=decision.stage_run_unit_id
                  AND admission.scope_snapshot_id=$9
                  AND admission.organization_id=decision.organization_id
                  AND admission.generation_id=generation.generation_id
                  AND admission.status='sealed'
                WHERE decision.decision_id=$11 AND decision.stable_request_id=$1
                  AND decision.binding_id=$4 AND decision.authority_id=$5
                  AND decision.operation_id=$6 AND decision.stage_execution_id=$7
                  AND decision.stage_run_unit_id=$8 AND decision.organization_id=$10
                  AND decision.work_id=$14 AND decision.candidate_snapshot_id=$15
                  AND decision.analysis_attempt_id=$16 AND decision.task_plan_id=$17
                  AND decision.delegation_census_seal_id=$18
                  AND decision.primary_worker_run_id=$19
                  AND decision.cognitive_output_schema='investigation_cognitive_output.v1'
                  AND decision.mutation_count=(SELECT COUNT(*)
                        FROM investigation_hypothesis_compilation_members member
                       WHERE member.decision_id=decision.decision_id)
                  AND decision.proof_member_count=(SELECT COUNT(*)
                        FROM investigation_hypothesis_compilation_proof_members proof
                       WHERE proof.decision_id=decision.decision_id)
                  AND seal.member_count=(SELECT COUNT(*) FROM hypothesis_generation_members member
                       WHERE member.generation_id=generation.generation_id)
                  AND admission.member_count=seal.member_count
                  AND admission.member_count=(SELECT COUNT(*)
                        FROM verification_admission_members member
                       WHERE member.admission_set_id=admission.admission_set_id)
                  AND NOT EXISTS(
                        SELECT 1
                          FROM verification_admission_members member
                          JOIN hypothesis_verification_tasks task
                            ON task.task_id=member.task_id
                          LEFT JOIN hypothesis_verification_task_assignment_sets assignment
                            ON assignment.task_id=task.task_id
                         WHERE member.admission_set_id=admission.admission_set_id
                           AND member.disposition='scheduled'
                           AND (task.task_contract_version<>'hypothesis_verification_task.dynamic_v2'
                                OR assignment.assignment_set_id IS NOT NULL))"#,
        )
        .bind(request.stable_compilation_request_id)
        .bind(request.stable_apply_request_id)
        .bind(request.stable_admission_request_id)
        .bind(request.prepared_subject.binding_id)
        .bind(request.identity.stage.authority_id)
        .bind(request.identity.stage.operation_id)
        .bind(request.identity.stage.stage_execution_id)
        .bind(request.identity.stage_run_unit_id)
        .bind(request.identity.stage.scope_snapshot_id)
        .bind(request.identity.organization_id)
        .bind(expected_decision_id)
        .bind(expected_apply_receipt_id)
        .bind(expected_admission_set_id)
        .bind(request.work_id)
        .bind(request.prepared_subject.candidate_snapshot_id)
        .bind(request.prepared_subject.analysis_attempt_id)
        .bind(request.task_plan_id)
        .bind(request.delegation_census_seal_id)
        .bind(request.primary_worker_run_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "normal Primary committed admission is partial or drifted".to_owned(),
        })?;
        let generation_ordinal = u32::try_from(row.generation_ordinal).map_err(|_| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "normal Primary committed generation ordinal is negative".to_owned(),
            }
        })?;
        let generation_member_count = u32::try_from(row.generation_member_count).map_err(|_| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "normal Primary committed generation member count overflowed".to_owned(),
            }
        })?;
        let expected_generation_id = Uuid::new_v5(
            &request.stable_apply_request_id,
            format!("investigation_generation:{generation_ordinal}").as_bytes(),
        );
        if row.compilation_decision_id != expected_decision_id
            || row.apply_receipt_id != expected_apply_receipt_id
            || row.admission_set_id != expected_admission_set_id
            || row.generation_id != expected_generation_id
            || row.generation_seal_id
                != Uuid::new_v5(&expected_generation_id, b"hypothesis_generation_seal.v1")
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "normal Primary committed deterministic ids drifted".to_owned(),
            });
        }
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(Some(InvestigationGenerationAdmissionView {
            compilation_decision_id: row.compilation_decision_id,
            generation_id: row.generation_id,
            generation_ordinal,
            generation_seal_id: row.generation_seal_id,
            generation_member_count,
            verification_task_ids: row.verification_task_ids,
            evolution_fixed_point: false,
            replayed: true,
        }))
    }

    async fn load_committed_analysis_post_synthesis_admission(
        &self,
        request: LoadCommittedInvestigationAnalysisPostSynthesisAdmission,
    ) -> InvestigationAnalysisHostResult<Option<InvestigationGenerationAdmissionView>> {
        if [
            request.stable_compilation_request_id,
            request.stable_apply_request_id,
            request.stable_admission_request_id,
            request.work_id,
            request.task_plan_id,
            request.delegation_census_seal_id,
            request.recovery_work_item_id,
            request.recovery_worker_run_id,
            request.primary_worker_run_id,
        ]
        .into_iter()
        .any(|id| id.is_nil())
            || !valid_sha256(&request.primary_synthesis_event_sha256)
        {
            return Err(InvestigationAnalysisHostError::InvalidRequest {
                detail: "committed post-synthesis admission ids/hash are invalid".to_owned(),
            });
        }
        let expected_decision_id = Uuid::new_v5(
            &request.stable_compilation_request_id,
            b"investigation_hypothesis_compilation_decision.v1",
        );
        let expected_apply_receipt_id = Uuid::new_v5(
            &request.stable_apply_request_id,
            b"investigation_hypothesis_canonical_apply_receipt.v1",
        );
        let expected_admission_set_id = Uuid::new_v5(
            &request.stable_admission_request_id,
            b"verification_admission_set.v1",
        );
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx_error)?;
        let counts = sqlx::query_as::<_, AnalysisPostSynthesisCommittedArtifactCountsRow>(
            r#"SELECT work.current_state,
                      tool_truth_sha256(recovery_worker.checkpoint::TEXT) AS checkpoint_sha256,
                      (SELECT COUNT(*)
                         FROM investigation_hypothesis_compilation_decisions decision
                        WHERE decision.decision_id=$21
                           OR decision.stable_request_id=$8
                           OR decision.binding_id=$12
                           OR decision.task_plan_id=$2) AS decision_count,
                      (SELECT COUNT(*)
                         FROM investigation_hypothesis_canonical_apply_receipts receipt
                        WHERE receipt.apply_receipt_id=$22
                           OR receipt.stable_request_id=$9
                           OR receipt.decision_id=$21) AS apply_receipt_count,
                      (SELECT COUNT(*)
                         FROM verification_admission_sets admission
                        WHERE admission.admission_set_id=$23
                           OR admission.stable_request_id=$10
                           OR admission.generation_id IN (
                               SELECT receipt.generation_id
                                 FROM investigation_hypothesis_canonical_apply_receipts receipt
                                WHERE receipt.decision_id=$21
                           )) AS admission_count
                 FROM investigation_run_work_items work
                 JOIN investigation_analysis_attempt_bindings binding
                   ON binding.work_id=work.work_id
                  AND binding.authority_id=work.authority_id
                  AND binding.binding_id=$12
                  AND binding.candidate_snapshot_id=$13
                  AND binding.analysis_attempt_id=$14
                 JOIN investigation_pentagi_task_plans task_plan
                   ON task_plan.task_plan_id=$2
                  AND task_plan.authority_id=work.authority_id
                  AND task_plan.operation_id=work.operation_id
                  AND task_plan.stage_execution_id=work.stage_execution_id
                  AND task_plan.stage_run_unit_id=work.stage_run_unit_id
                  AND task_plan.organization_id=work.organization_id
                  AND task_plan.subject_kind='analysis_attempt'
                  AND task_plan.subject_id=binding.analysis_attempt_id
                  AND task_plan.subject_fingerprint_sha256=$15
                  AND task_plan.status='sealed'
                 JOIN investigation_refiner_plan_ledger_seals refiner_seal
                   ON refiner_seal.task_plan_id=task_plan.task_plan_id
                 JOIN investigation_pentagi_delegation_census_seals census
                   ON census.census_seal_id=$7
                  AND census.task_plan_id=task_plan.task_plan_id
                  AND census.primary_worker_run_id=$5
                 JOIN investigation_pentagi_pipeline_events synthesis
                   ON synthesis.task_plan_id=task_plan.task_plan_id
                  AND synthesis.event_kind='primary_synthesis'
                  AND synthesis.actor_worker_run_id=$5
                  AND synthesis.parent_dispatch_receipt_id=census.primary_dispatch_receipt_id
                  AND synthesis.event_sha256=$6
                 JOIN stage_worker_runs source_worker
                   ON source_worker.id=$5
                  AND source_worker.status='failed'
                  AND source_worker.terminal_at IS NOT NULL
                  AND source_worker.lease_token IS NULL
                  AND source_worker.active_tool_call_id IS NULL
                 JOIN stage_work_items source_item
                   ON source_item.id=source_worker.work_item_id
                  AND source_item.team_plan_id=task_plan.stage_team_plan_id
                  AND source_item.stable_key='leader:primary'
                  AND source_item.status='exhausted'
                 JOIN stage_worker_outputs source_output
                   ON source_output.team_plan_id=source_item.team_plan_id
                  AND source_output.work_item_id=source_item.id
                  AND source_output.worker_run_id=source_worker.id
                  AND source_output.business_disposition='blocked'
                  AND source_output.canonical_output->>'kind'='stage_team_attempts_exhausted'
                  AND source_output.canonical_output->>'failure_code'='stage_team_worker_lease_expired'
                  AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=ANY(source_output.blocker_codes)
                 JOIN stage_work_items recovery_v1
                   ON recovery_v1.id=uuid_generate_v5(
                          source_item.id,
                          'sealed-investigation-synthesis-recovery-primary-v1'
                      )
                  AND recovery_v1.team_plan_id=source_item.team_plan_id
                  AND recovery_v1.stable_key='leader:synthesis-recovery:' || source_item.id::TEXT
                  AND recovery_v1.kind=source_item.kind
                  AND recovery_v1.status='exhausted'
                  AND recovery_v1.terminal_at IS NOT NULL
                 JOIN stage_worker_runs recovery_v1_worker
                   ON recovery_v1_worker.work_item_id=recovery_v1.id
                  AND recovery_v1_worker.status='failed'
                  AND recovery_v1_worker.terminal_at IS NOT NULL
                  AND recovery_v1_worker.lease_token IS NULL
                  AND recovery_v1_worker.active_tool_call_id IS NULL
                  AND recovery_v1_worker.checkpoint #>>
                      '{stage_team_execution_failure,code}'='stage_team_worker_lease_expired'
                 JOIN stage_worker_outputs recovery_v1_output
                   ON recovery_v1_output.team_plan_id=source_item.team_plan_id
                  AND recovery_v1_output.work_item_id=recovery_v1.id
                  AND recovery_v1_output.worker_run_id=recovery_v1_worker.id
                  AND recovery_v1_output.business_disposition='blocked'
                  AND recovery_v1_output.canonical_output->>'kind'='stage_team_attempts_exhausted'
                  AND recovery_v1_output.canonical_output->>'failure_code'='stage_team_worker_lease_expired'
                  AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=ANY(recovery_v1_output.blocker_codes)
                 JOIN stage_work_items recovery_v2
                   ON recovery_v2.id=$3
                  AND recovery_v2.id=uuid_generate_v5(
                          recovery_v1.id,
                          'sealed-investigation-synthesis-recovery-primary-v2'
                      )
                  AND recovery_v2.team_plan_id=source_item.team_plan_id
                  AND recovery_v2.stable_key=recovery_v1.stable_key
                  AND recovery_v2.kind='investigation_primary_recovery'
                 JOIN stage_worker_runs recovery_worker
                   ON recovery_worker.id=$4
                  AND recovery_worker.work_item_id=recovery_v2.id
                  AND recovery_worker.active_tool_call_id IS NULL
                  AND jsonb_typeof(recovery_worker.checkpoint)='array'
                WHERE work.work_id=$1
                  AND work.authority_id=$11
                  AND work.operation_id=$16
                  AND work.stage_execution_id=$17
                  AND work.stage_run_unit_id=$18
                  AND work.scope_snapshot_id=$19
                  AND work.organization_id=$20
                  AND work.work_kind='analysis'
                  AND work.current_state IN ('blocked','running','completed','residual')
                  AND (
                      (recovery_v2.status='running'
                       AND recovery_v2.terminal_at IS NULL
                       AND recovery_worker.status='running'
                       AND recovery_worker.terminal_at IS NULL
                       AND recovery_worker.lease_token IS NOT NULL)
                      OR
                      (recovery_v2.status='completed'
                       AND recovery_v2.terminal_at IS NOT NULL
                       AND recovery_worker.status='passed'
                       AND recovery_worker.terminal_at IS NOT NULL
                       AND recovery_worker.lease_token IS NULL)
                  )
                  AND (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events event
                        WHERE event.task_plan_id=task_plan.task_plan_id
                          AND event.event_kind='primary_synthesis')=1
                  AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                        WHERE all_worker.work_item_id=recovery_v2.id)=1
                  AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                        WHERE all_worker.work_item_id=recovery_v1.id)=1
                  AND (SELECT COUNT(*) FROM stage_worker_outputs all_output
                        WHERE all_output.work_item_id=recovery_v1.id)=1
                  AND (SELECT COUNT(*) FROM stage_worker_outputs all_output
                        WHERE all_output.work_item_id=source_item.id)=1
                  AND (SELECT COUNT(*)
                         FROM jsonb_path_query(
                             recovery_worker.checkpoint,
                             'strict $.** ? (@.name == "submit_result")'
                         ))=1
                  AND unified_investigation_submit_result_v1(recovery_worker.checkpoint)
                        IS NOT NULL"#,
        )
        .bind(request.work_id)
        .bind(request.task_plan_id)
        .bind(request.recovery_work_item_id)
        .bind(request.recovery_worker_run_id)
        .bind(request.primary_worker_run_id)
        .bind(&request.primary_synthesis_event_sha256)
        .bind(request.delegation_census_seal_id)
        .bind(request.stable_compilation_request_id)
        .bind(request.stable_apply_request_id)
        .bind(request.stable_admission_request_id)
        .bind(request.identity.stage.authority_id)
        .bind(request.prepared_subject.binding_id)
        .bind(request.prepared_subject.candidate_snapshot_id)
        .bind(request.prepared_subject.analysis_attempt_id)
        .bind(&request.prepared_subject.subject_fingerprint_sha256)
        .bind(request.identity.stage.operation_id)
        .bind(request.identity.stage.stage_execution_id)
        .bind(request.identity.stage_run_unit_id)
        .bind(request.identity.stage.scope_snapshot_id)
        .bind(request.identity.organization_id)
        .bind(expected_decision_id)
        .bind(expected_apply_receipt_id)
        .bind(expected_admission_set_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "committed post-synthesis recovery witness is not exact".to_owned(),
        })?;
        if !matches!(
            counts.current_state.as_str(),
            "blocked" | "running" | "completed" | "residual"
        ) || !valid_sha256(&counts.checkpoint_sha256)
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "committed post-synthesis recovery state/checkpoint drifted".to_owned(),
            });
        }
        if (
            counts.decision_count,
            counts.apply_receipt_count,
            counts.admission_count,
        ) == (1, 0, 0)
        {
            let fixed_point = load_committed_evolution_fixed_point_on(
                &mut tx,
                CommittedEvolutionFixedPointAuthority {
                    stable_compilation_request_id: request.stable_compilation_request_id,
                    stable_apply_request_id: request.stable_apply_request_id,
                    identity: &request.identity,
                    work_id: request.work_id,
                    prepared_subject: &request.prepared_subject,
                    task_plan_id: request.task_plan_id,
                    delegation_census_seal_id: request.delegation_census_seal_id,
                    primary_worker_run_id: request.primary_worker_run_id,
                },
            )
            .await?
            .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "recovery fixed-point replay authority is partial or foreign".to_owned(),
            })?;
            tx.commit().await.map_err(map_sqlx_error)?;
            return Ok(Some(fixed_point));
        }
        let committed = complete_committed_artifact_set(
            counts.decision_count,
            counts.apply_receipt_count,
            counts.admission_count,
        )
        .map_err(|detail| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: detail.to_owned(),
        })?;
        if !committed {
            tx.commit().await.map_err(map_sqlx_error)?;
            return Ok(None);
        }
        let row = sqlx::query_as::<_, AnalysisPostSynthesisCommittedAdmissionRow>(
            r#"SELECT decision.decision_id AS compilation_decision_id,
                      receipt.apply_receipt_id,
                      generation.generation_id,generation.generation_ordinal,
                      seal.seal_id AS generation_seal_id,
                      seal.member_count AS generation_member_count,
                      admission.admission_set_id,
                      ARRAY(
                          SELECT member.task_id
                            FROM verification_admission_members member
                           WHERE member.admission_set_id=admission.admission_set_id
                             AND member.disposition='scheduled'
                           ORDER BY member.hypothesis_revision_id
                      ) AS verification_task_ids
                 FROM investigation_hypothesis_compilation_decisions decision
                 JOIN investigation_pentagi_delegation_census_seals census
                   ON census.census_seal_id=decision.delegation_census_seal_id
                  AND census.task_plan_id=decision.task_plan_id
                  AND census.primary_worker_run_id=decision.primary_worker_run_id
                  AND census.seal_sha256=decision.delegation_census_sha256
                 JOIN investigation_hypothesis_canonical_apply_receipts receipt
                   ON receipt.apply_receipt_id=$12
                  AND receipt.stable_request_id=$2
                  AND receipt.decision_id=decision.decision_id
                  AND receipt.operation_id=decision.operation_id
                  AND receipt.organization_id=decision.organization_id
                 JOIN hypothesis_generations generation
                   ON generation.generation_id=receipt.generation_id
                  AND generation.operation_id=decision.operation_id
                  AND generation.organization_id=decision.organization_id
                  AND generation.candidate_snapshot_id=decision.candidate_snapshot_id
                  AND generation.investigation_compilation_decision_id=decision.decision_id
                  AND generation.candidate_gate_decision_id IS NULL
                 JOIN hypothesis_generation_seals seal
                   ON seal.seal_id=receipt.generation_seal_id
                  AND seal.generation_id=generation.generation_id
                  AND seal.controller_worker_run_id=decision.primary_worker_run_id
                 JOIN verification_admission_sets admission
                   ON admission.admission_set_id=$13
                  AND admission.stable_request_id=$3
                  AND admission.operation_id=decision.operation_id
                  AND admission.stage_execution_id=decision.stage_execution_id
                  AND admission.stage_run_unit_id=decision.stage_run_unit_id
                  AND admission.scope_snapshot_id=$9
                  AND admission.organization_id=decision.organization_id
                  AND admission.generation_id=generation.generation_id
                  AND admission.status='sealed'
                WHERE decision.decision_id=$11
                  AND decision.stable_request_id=$1
                  AND decision.binding_id=$4
                  AND decision.authority_id=$5
                  AND decision.operation_id=$6
                  AND decision.stage_execution_id=$7
                  AND decision.stage_run_unit_id=$8
                  AND decision.organization_id=$10
                  AND decision.work_id=$14
                  AND decision.candidate_snapshot_id=$15
                  AND decision.analysis_attempt_id=$16
                  AND decision.task_plan_id=$17
                  AND decision.delegation_census_seal_id=$18
                  AND decision.primary_worker_run_id=$19
                  AND decision.cognitive_output_schema='investigation_cognitive_output.v1'
                  AND decision.mutation_count=(
                      SELECT COUNT(*) FROM investigation_hypothesis_compilation_members member
                       WHERE member.decision_id=decision.decision_id
                  )
                  AND decision.proof_member_count=(
                      SELECT COUNT(*) FROM investigation_hypothesis_compilation_proof_members proof
                       WHERE proof.decision_id=decision.decision_id
                  )
                  AND receipt.revision_count=(
                      SELECT COUNT(*) FROM investigation_hypothesis_compilation_members member
                       WHERE member.decision_id=decision.decision_id
                         AND member.route_kind='create_initial'
                  )
                  AND seal.member_count=(
                      SELECT COUNT(*) FROM hypothesis_generation_members member
                       WHERE member.generation_id=generation.generation_id
                  )
                  AND admission.member_count=seal.member_count
                  AND admission.member_count=(
                      SELECT COUNT(*) FROM verification_admission_members member
                       WHERE member.admission_set_id=admission.admission_set_id
                  )
                  AND NOT EXISTS(
                      SELECT 1
                        FROM verification_admission_members member
                        JOIN hypothesis_verification_tasks task
                          ON task.task_id=member.task_id
                        LEFT JOIN hypothesis_verification_task_assignment_sets assignment
                          ON assignment.task_id=task.task_id
                       WHERE member.admission_set_id=admission.admission_set_id
                         AND member.disposition='scheduled'
                         AND (task.task_contract_version<>'hypothesis_verification_task.dynamic_v2'
                              OR assignment.assignment_set_id IS NOT NULL)
                  )"#,
        )
        .bind(request.stable_compilation_request_id)
        .bind(request.stable_apply_request_id)
        .bind(request.stable_admission_request_id)
        .bind(request.prepared_subject.binding_id)
        .bind(request.identity.stage.authority_id)
        .bind(request.identity.stage.operation_id)
        .bind(request.identity.stage.stage_execution_id)
        .bind(request.identity.stage_run_unit_id)
        .bind(request.identity.stage.scope_snapshot_id)
        .bind(request.identity.organization_id)
        .bind(expected_decision_id)
        .bind(expected_apply_receipt_id)
        .bind(expected_admission_set_id)
        .bind(request.work_id)
        .bind(request.prepared_subject.candidate_snapshot_id)
        .bind(request.prepared_subject.analysis_attempt_id)
        .bind(request.task_plan_id)
        .bind(request.delegation_census_seal_id)
        .bind(request.primary_worker_run_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "committed post-synthesis admission authority is partial or drifted".to_owned(),
        })?;
        let generation_ordinal = u32::try_from(row.generation_ordinal).map_err(|_| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "committed post-synthesis generation ordinal is negative".to_owned(),
            }
        })?;
        let generation_member_count = u32::try_from(row.generation_member_count).map_err(|_| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "committed post-synthesis generation member count overflowed".to_owned(),
            }
        })?;
        let expected_generation_id = Uuid::new_v5(
            &request.stable_apply_request_id,
            format!("investigation_generation:{generation_ordinal}").as_bytes(),
        );
        let expected_generation_seal_id =
            Uuid::new_v5(&expected_generation_id, b"hypothesis_generation_seal.v1");
        if row.compilation_decision_id != expected_decision_id
            || row.apply_receipt_id != expected_apply_receipt_id
            || row.admission_set_id != expected_admission_set_id
            || row.generation_id != expected_generation_id
            || row.generation_seal_id != expected_generation_seal_id
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "committed post-synthesis deterministic ids drifted".to_owned(),
            });
        }
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(Some(InvestigationGenerationAdmissionView {
            compilation_decision_id: row.compilation_decision_id,
            generation_id: row.generation_id,
            generation_ordinal,
            generation_seal_id: row.generation_seal_id,
            generation_member_count,
            verification_task_ids: row.verification_task_ids,
            evolution_fixed_point: false,
            replayed: true,
        }))
    }

    async fn resume_analysis_primary_post_synthesis(
        &self,
        request: ResumeInvestigationAnalysisPrimaryPostSynthesis,
    ) -> InvestigationAnalysisHostResult<ResumedInvestigationAnalysisPostSynthesisView> {
        if [
            request.work_id,
            request.task_plan_id,
            request.delegation_census_seal_id,
            request.primary_work_item_id,
            request.primary_worker_run_id,
        ]
        .into_iter()
        .any(|id| id.is_nil())
            || !valid_sha256(&request.primary_synthesis_event_sha256)
        {
            return Err(InvestigationAnalysisHostError::InvalidRequest {
                detail: "normal Primary post-synthesis recovery ids/hash are invalid".to_owned(),
            });
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        let row = sqlx::query_as::<_, AnalysisPostSynthesisResumeRow>(
            r#"SELECT work.current_state,work.head_version,work.latest_event_id,
                      work.observed_stop_epoch,
                      tool_truth_sha256(primary_worker.checkpoint::TEXT) AS checkpoint_sha256,
                      synthesis.event_sha256 AS synthesis_event_sha256,
                      latest.reason_code AS latest_reason_code
                 FROM investigation_run_work_items work
                 JOIN investigation_run_work_state_events latest
                   ON latest.event_id=work.latest_event_id AND latest.work_id=work.work_id
                 JOIN investigation_analysis_attempt_bindings binding
                   ON binding.work_id=work.work_id AND binding.authority_id=work.authority_id
                  AND binding.binding_id=$8 AND binding.candidate_snapshot_id=$9
                  AND binding.analysis_attempt_id=$10
                 JOIN investigation_pentagi_task_plans task_plan
                   ON task_plan.task_plan_id=$2 AND task_plan.authority_id=work.authority_id
                  AND task_plan.operation_id=work.operation_id
                  AND task_plan.stage_execution_id=work.stage_execution_id
                  AND task_plan.stage_run_unit_id=work.stage_run_unit_id
                  AND task_plan.organization_id=work.organization_id
                  AND task_plan.subject_kind='analysis_attempt'
                  AND task_plan.subject_id=binding.analysis_attempt_id
                  AND task_plan.subject_fingerprint_sha256=$11 AND task_plan.status='sealed'
                 JOIN investigation_refiner_plan_ledger_seals refiner_seal
                   ON refiner_seal.task_plan_id=task_plan.task_plan_id
                 JOIN investigation_pentagi_delegation_census_seals census
                   ON census.census_seal_id=$6 AND census.task_plan_id=task_plan.task_plan_id
                  AND census.primary_worker_run_id=$4
                 JOIN investigation_pentagi_pipeline_events synthesis
                   ON synthesis.task_plan_id=task_plan.task_plan_id
                  AND synthesis.event_kind='primary_synthesis'
                  AND synthesis.actor_worker_run_id=$4
                  AND synthesis.parent_dispatch_receipt_id=census.primary_dispatch_receipt_id
                  AND synthesis.event_sha256=$5
                 JOIN pentagi_logical_dispatch_receipts dispatch
                   ON dispatch.dispatch_receipt_id=census.primary_dispatch_receipt_id
                  AND dispatch.task_plan_id=task_plan.task_plan_id
                  AND dispatch.actor_kind='primary' AND dispatch.subtask_id IS NULL
                  AND dispatch.stage_work_item_id=$3 AND dispatch.worker_run_id=$4
                 JOIN stage_work_items primary_item
                  ON primary_item.id=$3 AND primary_item.team_plan_id=task_plan.stage_team_plan_id
                  AND primary_item.stable_key='leader:primary'
                  AND primary_item.kind='investigation_primary'
                  AND primary_item.role=(SELECT leader_role FROM stage_team_plans WHERE id=task_plan.stage_team_plan_id)
                  AND primary_item.created_by='server_seed' AND primary_item.required_for_barrier=FALSE
                  AND primary_item.status='completed' AND primary_item.terminal_at IS NOT NULL
                 JOIN stage_worker_runs primary_worker
                   ON primary_worker.id=$4 AND primary_worker.work_item_id=primary_item.id
                  AND primary_worker.status='passed' AND primary_worker.terminal_at IS NOT NULL
                  AND primary_worker.lease_token IS NULL
                  AND primary_worker.active_tool_call_id IS NULL
                  AND jsonb_typeof(primary_worker.checkpoint)='array'
                WHERE work.work_id=$1 AND work.authority_id=$7
                  AND work.operation_id=$12 AND work.stage_execution_id=$13
                  AND work.stage_run_unit_id=$14 AND work.scope_snapshot_id=$15
                  AND work.organization_id=$16 AND work.work_kind='analysis'
                  AND work.current_state IN ('blocked','running')
                  AND NOT EXISTS(SELECT 1
                        FROM investigation_hypothesis_compilation_decisions decision
                       WHERE decision.binding_id=binding.binding_id
                          OR decision.task_plan_id=task_plan.task_plan_id)
                  AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                        WHERE all_worker.work_item_id=primary_item.id)=1
                  AND (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events event
                        WHERE event.task_plan_id=task_plan.task_plan_id
                          AND event.event_kind='primary_synthesis')=1
                  AND (SELECT COUNT(*) FROM jsonb_path_query(
                        primary_worker.checkpoint,
                        'strict $.** ? (@.name == "submit_result")'))=1
                  AND unified_investigation_submit_result_v1(primary_worker.checkpoint)
                        IS NOT NULL
                FOR UPDATE OF work"#,
        )
        .bind(request.work_id)
        .bind(request.task_plan_id)
        .bind(request.primary_work_item_id)
        .bind(request.primary_worker_run_id)
        .bind(&request.primary_synthesis_event_sha256)
        .bind(request.delegation_census_seal_id)
        .bind(request.identity.stage.authority_id)
        .bind(request.prepared_subject.binding_id)
        .bind(request.prepared_subject.candidate_snapshot_id)
        .bind(request.prepared_subject.analysis_attempt_id)
        .bind(&request.prepared_subject.subject_fingerprint_sha256)
        .bind(request.identity.stage.operation_id)
        .bind(request.identity.stage.stage_execution_id)
        .bind(request.identity.stage_run_unit_id)
        .bind(request.identity.stage.scope_snapshot_id)
        .bind(request.identity.organization_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "normal Primary post-synthesis recovery witness is not exact".to_owned(),
        })?;
        let reason_code = format!(
            "post_synthesis_analysis_primary_recovery.v1|{}|{}",
            row.synthesis_event_sha256, row.checkpoint_sha256
        );
        if row.current_state == "running" {
            if row.latest_reason_code != reason_code {
                return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "normal Primary post-synthesis rearm replay drifted".to_owned(),
                });
            }
            let latest_event_id = row.latest_event_id.ok_or_else(|| {
                InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "normal Primary post-synthesis rearm event is absent".to_owned(),
                }
            })?;
            tx.commit().await.map_err(map_sqlx_error)?;
            return Ok(ResumedInvestigationAnalysisPostSynthesisView {
                work_id: request.work_id,
                current_state: row.current_state,
                head_version: row.head_version,
                latest_event_id,
                checkpoint_sha256: row.checkpoint_sha256,
                replayed: true,
            });
        }
        if !matches!(
            row.latest_reason_code.as_str(),
            "investigation_analysis_host_infrastructure"
                | "investigation_analysis_host_authority_mismatch"
        ) {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: format!(
                    "normal Primary post-synthesis terminal reason drifted: {}",
                    row.latest_reason_code
                ),
            });
        }
        let next_head_version = row.head_version.checked_add(1).ok_or_else(|| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "normal Primary post-synthesis work version overflow".to_owned(),
            }
        })?;
        let stable_request_id = Uuid::new_v5(
            &request.work_id,
            format!(
                "normal-primary-post-synthesis-analysis-rearm:{}:{reason_code}",
                row.head_version
            )
            .as_bytes(),
        );
        let event_material = serde_json::json!({
            "authority_id": request.identity.stage.authority_id,
            "from_state": "blocked",
            "head_version": row.head_version,
            "observed_stop_epoch": row.observed_stop_epoch,
            "reason_code": reason_code,
            "to_state": "Running",
            "work_id": request.work_id,
        });
        let event_sha256 = sha256_json(&event_material);
        let event_id = Uuid::new_v5(
            &stable_request_id,
            format!("normal-primary-post-synthesis-rearm-event:{event_sha256}").as_bytes(),
        );
        sqlx::query(
            r#"INSERT INTO investigation_run_work_state_events(
                   event_id,stable_request_id,work_id,expected_head_version,event_ordinal,
                   from_state,to_state,observed_stop_epoch,reason_code,event_sha256)
               VALUES($1,$2,$3,$4,$5,'blocked','running',$6,$7,$8)"#,
        )
        .bind(event_id)
        .bind(stable_request_id)
        .bind(request.work_id)
        .bind(row.head_version)
        .bind(next_head_version)
        .bind(row.observed_stop_epoch)
        .bind(&reason_code)
        .bind(event_sha256)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(ResumedInvestigationAnalysisPostSynthesisView {
            work_id: request.work_id,
            current_state: "running".to_owned(),
            head_version: next_head_version,
            latest_event_id: event_id,
            checkpoint_sha256: row.checkpoint_sha256,
            replayed: false,
        })
    }

    async fn resume_analysis_post_synthesis(
        &self,
        request: ResumeInvestigationAnalysisPostSynthesis,
    ) -> InvestigationAnalysisHostResult<ResumedInvestigationAnalysisPostSynthesisView> {
        if [
            request.work_id,
            request.task_plan_id,
            request.recovery_work_item_id,
            request.recovery_worker_run_id,
            request.primary_worker_run_id,
        ]
        .into_iter()
        .any(|id| id.is_nil())
            || !valid_sha256(&request.primary_synthesis_event_sha256)
        {
            return Err(InvestigationAnalysisHostError::InvalidRequest {
                detail: "post-synthesis Analysis recovery ids/hash are invalid".to_owned(),
            });
        }
        let mut tx = self.pool.begin().await.map_err(map_sqlx_error)?;
        let row = sqlx::query_as::<_, AnalysisPostSynthesisResumeRow>(
            r#"SELECT work.current_state,work.head_version,work.latest_event_id,
                      work.observed_stop_epoch,
                      tool_truth_sha256(recovery_worker.checkpoint::TEXT) AS checkpoint_sha256,
                      synthesis.event_sha256 AS synthesis_event_sha256,
                      latest.reason_code AS latest_reason_code
                 FROM investigation_run_work_items work
                 JOIN investigation_run_work_state_events latest
                   ON latest.event_id=work.latest_event_id
                  AND latest.work_id=work.work_id
                 JOIN investigation_analysis_attempt_bindings binding
                   ON binding.work_id=work.work_id
                  AND binding.authority_id=work.authority_id
                  AND binding.binding_id=$8
                  AND binding.candidate_snapshot_id=$9
                  AND binding.analysis_attempt_id=$10
                 JOIN investigation_pentagi_task_plans task_plan
                   ON task_plan.task_plan_id=$2
                  AND task_plan.authority_id=work.authority_id
                  AND task_plan.stage_run_unit_id=work.stage_run_unit_id
                  AND task_plan.organization_id=work.organization_id
                  AND task_plan.subject_kind='analysis_attempt'
                  AND task_plan.subject_id=binding.analysis_attempt_id
                  AND task_plan.subject_fingerprint_sha256=$11
                  AND task_plan.status='sealed'
                 JOIN investigation_refiner_plan_ledger_seals refiner_seal
                   ON refiner_seal.task_plan_id=task_plan.task_plan_id
                 JOIN investigation_pentagi_delegation_census_seals census
                   ON census.task_plan_id=task_plan.task_plan_id
                  AND census.primary_worker_run_id=$5
                 JOIN investigation_pentagi_pipeline_events synthesis
                   ON synthesis.task_plan_id=task_plan.task_plan_id
                  AND synthesis.event_kind='primary_synthesis'
                  AND synthesis.actor_worker_run_id=$5
                  AND synthesis.parent_dispatch_receipt_id=
                      census.primary_dispatch_receipt_id
                  AND synthesis.event_sha256=$6
                 JOIN stage_worker_runs source_worker
                   ON source_worker.id=$5
                  AND source_worker.status='failed'
                  AND source_worker.terminal_at IS NOT NULL
                  AND source_worker.lease_token IS NULL
                  AND source_worker.active_tool_call_id IS NULL
                 JOIN stage_work_items source_item
                   ON source_item.id=source_worker.work_item_id
                  AND source_item.team_plan_id=task_plan.stage_team_plan_id
                  AND source_item.stable_key='leader:primary'
                  AND source_item.status='exhausted'
                 JOIN stage_worker_outputs source_output
                   ON source_output.team_plan_id=source_item.team_plan_id
                  AND source_output.work_item_id=source_item.id
                  AND source_output.worker_run_id=source_worker.id
                  AND source_output.business_disposition='blocked'
                  AND source_output.canonical_output->>'kind'=
                      'stage_team_attempts_exhausted'
                  AND source_output.canonical_output->>'failure_code'=
                      'stage_team_worker_lease_expired'
                  AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                      ANY(source_output.blocker_codes)
                 JOIN stage_work_items recovery_v1
                   ON recovery_v1.id=uuid_generate_v5(
                          source_item.id,
                          'sealed-investigation-synthesis-recovery-primary-v1'
                      )
                  AND recovery_v1.team_plan_id=source_item.team_plan_id
                  AND recovery_v1.stable_key=
                      'leader:synthesis-recovery:' || source_item.id::TEXT
                  AND recovery_v1.kind=source_item.kind
                  AND recovery_v1.status='exhausted'
                  AND recovery_v1.terminal_at IS NOT NULL
                 JOIN stage_worker_runs recovery_v1_worker
                   ON recovery_v1_worker.work_item_id=recovery_v1.id
                  AND recovery_v1_worker.status='failed'
                  AND recovery_v1_worker.terminal_at IS NOT NULL
                 JOIN stage_worker_outputs recovery_v1_output
                   ON recovery_v1_output.team_plan_id=source_item.team_plan_id
                  AND recovery_v1_output.work_item_id=recovery_v1.id
                  AND recovery_v1_output.worker_run_id=recovery_v1_worker.id
                  AND recovery_v1_output.business_disposition='blocked'
                  AND recovery_v1_output.canonical_output->>'kind'=
                      'stage_team_attempts_exhausted'
                  AND recovery_v1_output.canonical_output->>'failure_code'=
                      'stage_team_worker_lease_expired'
                  AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=
                      ANY(recovery_v1_output.blocker_codes)
                 JOIN stage_work_items recovery_v2
                   ON recovery_v2.id=$3
                  AND recovery_v2.id=uuid_generate_v5(
                          recovery_v1.id,
                          'sealed-investigation-synthesis-recovery-primary-v2'
                      )
                  AND recovery_v2.team_plan_id=source_item.team_plan_id
                  AND recovery_v2.stable_key=recovery_v1.stable_key
                  AND recovery_v2.kind='investigation_primary_recovery'
                  AND recovery_v2.status='running'
                  AND recovery_v2.terminal_at IS NULL
                 JOIN stage_worker_runs recovery_worker
                   ON recovery_worker.id=$4
                  AND recovery_worker.work_item_id=recovery_v2.id
                  AND recovery_worker.status='running'
                  AND recovery_worker.terminal_at IS NULL
                  AND recovery_worker.lease_token IS NOT NULL
                  AND recovery_worker.active_tool_call_id IS NULL
                  AND jsonb_typeof(recovery_worker.checkpoint)='array'
                WHERE work.work_id=$1
                  AND work.authority_id=$7
                  AND work.operation_id=$12
                  AND work.stage_execution_id=$13
                  AND work.stage_run_unit_id=$14
                  AND work.scope_snapshot_id=$15
                  AND work.organization_id=$16
                  AND work.work_kind='analysis'
                  AND work.current_state IN ('blocked','running')
                  AND NOT EXISTS(
                      SELECT 1
                        FROM investigation_hypothesis_compilation_decisions decision
                       WHERE decision.binding_id=binding.binding_id
                          OR decision.task_plan_id=task_plan.task_plan_id
                  )
                  AND (SELECT COUNT(*) FROM investigation_pentagi_pipeline_events event
                        WHERE event.task_plan_id=task_plan.task_plan_id
                          AND event.event_kind='primary_synthesis')=1
                  AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                        WHERE all_worker.work_item_id=recovery_v2.id)=1
                  AND (SELECT COUNT(*) FROM stage_worker_runs all_worker
                        WHERE all_worker.work_item_id=recovery_v1.id)=1
                  AND (SELECT COUNT(*) FROM stage_worker_outputs all_output
                        WHERE all_output.work_item_id=source_item.id)=1
                  AND (SELECT COUNT(*)
                         FROM jsonb_path_query(
                             recovery_worker.checkpoint,
                             'strict $.** ? (@.name == "submit_result")'
                         ))=1
                  AND unified_investigation_submit_result_v1(recovery_worker.checkpoint)
                        IS NOT NULL
                FOR UPDATE OF work"#,
        )
        .bind(request.work_id)
        .bind(request.task_plan_id)
        .bind(request.recovery_work_item_id)
        .bind(request.recovery_worker_run_id)
        .bind(request.primary_worker_run_id)
        .bind(&request.primary_synthesis_event_sha256)
        .bind(request.identity.stage.authority_id)
        .bind(request.prepared_subject.binding_id)
        .bind(request.prepared_subject.candidate_snapshot_id)
        .bind(request.prepared_subject.analysis_attempt_id)
        .bind(&request.prepared_subject.subject_fingerprint_sha256)
        .bind(request.identity.stage.operation_id)
        .bind(request.identity.stage.stage_execution_id)
        .bind(request.identity.stage_run_unit_id)
        .bind(request.identity.stage.scope_snapshot_id)
        .bind(request.identity.organization_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_sqlx_error)?
        .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "post-synthesis Analysis recovery witness is not exact".to_owned(),
        })?;
        let reason_code = format!(
            "post_synthesis_analysis_recovery.v1|{}|{}",
            row.synthesis_event_sha256, row.checkpoint_sha256
        );
        if row.current_state == "running" {
            if row.latest_reason_code != reason_code {
                return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "post-synthesis Analysis rearm replay drifted".to_owned(),
                });
            }
            let latest_event_id = row.latest_event_id.ok_or_else(|| {
                InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "post-synthesis Analysis rearm event is absent".to_owned(),
                }
            })?;
            tx.commit().await.map_err(map_sqlx_error)?;
            return Ok(ResumedInvestigationAnalysisPostSynthesisView {
                work_id: request.work_id,
                current_state: row.current_state,
                head_version: row.head_version,
                latest_event_id,
                checkpoint_sha256: row.checkpoint_sha256,
                replayed: true,
            });
        }
        if row.latest_reason_code != "investigation_analysis_host_authority_mismatch" {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: format!(
                    "post-synthesis Analysis terminal reason drifted: {}",
                    row.latest_reason_code
                ),
            });
        }
        let next_head_version = row.head_version.checked_add(1).ok_or_else(|| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "post-synthesis Analysis work version overflow".to_owned(),
            }
        })?;
        let stable_request_id = Uuid::new_v5(
            &request.work_id,
            format!(
                "post-synthesis-analysis-rearm:{}:{reason_code}",
                row.head_version
            )
            .as_bytes(),
        );
        let event_material = serde_json::json!({
            "authority_id": request.identity.stage.authority_id,
            "from_state": "blocked",
            "head_version": row.head_version,
            "observed_stop_epoch": row.observed_stop_epoch,
            "reason_code": reason_code,
            "to_state": "Running",
            "work_id": request.work_id,
        });
        let event_sha256 = sha256_json(&event_material);
        let event_id = Uuid::new_v5(
            &stable_request_id,
            format!("post-synthesis-analysis-rearm-event:{event_sha256}").as_bytes(),
        );
        sqlx::query(
            r#"INSERT INTO investigation_run_work_state_events(
                   event_id,stable_request_id,work_id,expected_head_version,event_ordinal,
                   from_state,to_state,observed_stop_epoch,reason_code,event_sha256)
               VALUES($1,$2,$3,$4,$5,'blocked','running',$6,$7,$8)"#,
        )
        .bind(event_id)
        .bind(stable_request_id)
        .bind(request.work_id)
        .bind(row.head_version)
        .bind(next_head_version)
        .bind(row.observed_stop_epoch)
        .bind(&reason_code)
        .bind(event_sha256)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx_error)?;
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(ResumedInvestigationAnalysisPostSynthesisView {
            work_id: request.work_id,
            current_state: "running".to_owned(),
            head_version: next_head_version,
            latest_event_id: event_id,
            checkpoint_sha256: row.checkpoint_sha256,
            replayed: false,
        })
    }
}

#[cfg(test)]
mod stale_prerequisite_tests {
    use super::*;

    fn member(
        root_family: &str,
        member_status: &str,
        root_operation_id: Uuid,
        fork_source_operation_id: Option<Uuid>,
        revalidation_obligation_ids: Vec<Uuid>,
    ) -> FrozenToolTruthPrerequisiteMember {
        FrozenToolTruthPrerequisiteMember {
            root_family: root_family.to_owned(),
            member_status: member_status.to_owned(),
            root_operation_id,
            fork_source_operation_id,
            revalidation_obligation_ids,
        }
    }

    #[test]
    fn expired_adopted_predecessor_remains_historical_analysis_context() {
        let operation_id = Uuid::new_v4();
        let source_operation_id = Uuid::new_v4();
        let result = stale_tool_truth_prerequisite(
            operation_id,
            &[
                member(
                    "eas",
                    "expired",
                    source_operation_id,
                    Some(source_operation_id),
                    Vec::new(),
                ),
                member(
                    "enum",
                    "consistent_fresh",
                    source_operation_id,
                    Some(source_operation_id),
                    Vec::new(),
                ),
                member(
                    "vuln",
                    "consistent_fresh",
                    source_operation_id,
                    Some(source_operation_id),
                    Vec::new(),
                ),
            ],
        )
        .expect("classify exact predecessor");
        assert!(
            result.is_none(),
            "adopted sealed context must not rerun EAS"
        );
    }

    #[test]
    fn several_expired_adopted_predecessors_do_not_restart_the_stage_chain() {
        let operation_id = Uuid::new_v4();
        let source_operation_id = Uuid::new_v4();
        let result = stale_tool_truth_prerequisite(
            operation_id,
            &[
                member(
                    "eas",
                    "consistent_fresh",
                    source_operation_id,
                    Some(source_operation_id),
                    Vec::new(),
                ),
                member(
                    "enum",
                    "expired",
                    source_operation_id,
                    Some(source_operation_id),
                    Vec::new(),
                ),
                member(
                    "vuln",
                    "expired",
                    source_operation_id,
                    Some(source_operation_id),
                    Vec::new(),
                ),
            ],
        )
        .expect("classify exact predecessor");
        assert!(
            result.is_none(),
            "adopted sealed context must not restart Enumeration or Vuln"
        );
    }

    #[test]
    fn same_operation_stale_root_remains_on_revalidation_path() {
        let operation_id = Uuid::new_v4();
        let obligation_id = Uuid::new_v4();
        let error = stale_tool_truth_prerequisite(
            operation_id,
            &[
                member("eas", "expired", operation_id, None, vec![obligation_id]),
                member("enum", "consistent_fresh", operation_id, None, Vec::new()),
                member("vuln", "consistent_fresh", operation_id, None, Vec::new()),
            ],
        )
        .expect("classify same-operation root")
        .expect("stale same-operation root requires revalidation");
        assert!(matches!(
            error,
            InvestigationAnalysisHostError::RevalidationRequired {
                operation_id: actual_operation_id,
                ref revalidation_obligation_ids,
                ref stale_roots,
            } if actual_operation_id == operation_id
                && revalidation_obligation_ids == &[obligation_id]
                && stale_roots == &["external_attack_surface:expired"]
        ));
        assert_eq!(
            error.code(),
            "investigation_analysis_host_revalidation_required"
        );
        assert!(error
            .to_string()
            .contains("retry_mode=tool_truth_revalidation"));
    }
}

fn map_registry_error(error: HypothesisRegistryError) -> InvestigationAnalysisHostError {
    let detail = error.to_string();
    match error {
        HypothesisRegistryError::Unavailable(_) => InvestigationAnalysisHostError::Unavailable {
            operation: "freeze_candidate_snapshot",
        },
        HypothesisRegistryError::InvalidRequest(_) => {
            InvestigationAnalysisHostError::InvalidRequest { detail }
        }
        HypothesisRegistryError::NotFound(_) => InvestigationAnalysisHostError::NotFound { detail },
        HypothesisRegistryError::Conflict(_) => InvestigationAnalysisHostError::Conflict { detail },
        HypothesisRegistryError::AuthorityMismatch(_)
        | HypothesisRegistryError::ArtifactKindForbidden(_) => {
            InvestigationAnalysisHostError::AuthorityMismatch { detail }
        }
        HypothesisRegistryError::Storage(_) => {
            InvestigationAnalysisHostError::Infrastructure { detail }
        }
    }
}

fn compiler_proposal_input(
    proposal: &CandidateHypothesisProposal,
) -> InvestigationAnalysisHostResult<InvestigationProposalInput> {
    let proof_refs = proposal
        .proof_refs
        .iter()
        .map(|proof| InvestigationProofRefInput {
            input_id: proof.input_id,
            chunk_id: proof.chunk_id,
            source_hash: proof.source_hash.clone(),
            source_role: match proof.role {
                CandidateProofReferenceRole::Support => "support",
                CandidateProofReferenceRole::Contradiction => "contradiction",
                CandidateProofReferenceRole::AuthorizationUse => "authorization_use",
                CandidateProofReferenceRole::Gap => "gap",
            }
            .to_owned(),
        })
        .collect::<Vec<_>>();
    let predicate_arguments = proposal
        .predicate_arguments
        .iter()
        .cloned()
        .collect::<BTreeMap<_, _>>();
    if predicate_arguments.len() != proposal.predicate_arguments.len() {
        return Err(InvestigationAnalysisHostError::InvalidRequest {
            detail: "canonical proposal predicate argument keys must be unique".to_owned(),
        });
    }
    Ok(InvestigationProposalInput {
        proposal_id: proposal.proposal_id,
        canonical_proposal: serde_json::json!({
            "proposal_id":proposal.proposal_id,
            "subject_kind":proposal.subject_kind,
            "subject_identity_hash":proposal.subject_identity_hash,
            "predicate_schema":proposal.predicate_schema,
            "predicate_version":proposal.predicate_version,
            "predicate_arguments":predicate_arguments,
            "trust_boundary":proposal.trust_boundary,
            "polarity":proposal.polarity,
            "structured_claim":proposal.structured_claim,
            "preconditions":proposal.preconditions,
            "impact":proposal.impact,
            "proof_refs":proof_refs,
            "knowledge_signals":proposal.knowledge_signals,
            "readiness":proposal.readiness,
        }),
        proof_refs,
    })
}

fn map_binding_error(
    error: InvestigationAnalysisBindingStoreError,
) -> InvestigationAnalysisHostError {
    let detail = error.to_string();
    match error {
        InvestigationAnalysisBindingStoreError::InvalidInput(_) => {
            InvestigationAnalysisHostError::InvalidRequest { detail }
        }
        InvestigationAnalysisBindingStoreError::IdentityConflict(_) => {
            InvestigationAnalysisHostError::AuthorityMismatch { detail }
        }
        InvestigationAnalysisBindingStoreError::Sqlx(_) => {
            InvestigationAnalysisHostError::Infrastructure { detail }
        }
    }
}

fn map_compiler_db_error(error: golish_db::DbError) -> InvestigationAnalysisHostError {
    let detail = error.to_string();
    if detail.contains("AUTHORITY_MISMATCH")
        || detail.contains("INVESTIGATION_ADMISSION_EXACT_SET_INCOMPLETE")
    {
        InvestigationAnalysisHostError::AuthorityMismatch { detail }
    } else if detail.contains("REPLAY") || detail.contains("CONFLICT") {
        InvestigationAnalysisHostError::Conflict { detail }
    } else if detail.contains("INVALID") {
        InvestigationAnalysisHostError::InvalidRequest { detail }
    } else {
        InvestigationAnalysisHostError::Infrastructure { detail }
    }
}

pub(super) fn advisory_capability_name(
    capability: golish_agent_kit::db_traits::InvestigationAdvisoryCapabilityV1,
) -> &'static str {
    use golish_agent_kit::db_traits::InvestigationAdvisoryCapabilityV1;
    match capability {
        InvestigationAdvisoryCapabilityV1::HttpObservation => "http_observation",
        InvestigationAdvisoryCapabilityV1::BrowserObservation => "browser_observation",
        InvestigationAdvisoryCapabilityV1::CliObservation => "cli_observation",
        InvestigationAdvisoryCapabilityV1::CredentialedObservation => "credentialed_observation",
    }
}

fn db_source_ref(source: RevisionSourceRef) -> CandidateRevisionSourceRefRow {
    match source {
        RevisionSourceRef::ToolTruthEvidence(value) => {
            CandidateRevisionSourceRefRow::ToolTruthEvidence(value)
        }
        RevisionSourceRef::Finding(value) => CandidateRevisionSourceRefRow::Finding(value),
        RevisionSourceRef::VerificationReceipt(value) => {
            CandidateRevisionSourceRefRow::VerificationReceipt(value)
        }
        RevisionSourceRef::ApplicationContext(value) => {
            CandidateRevisionSourceRefRow::ApplicationContext(value)
        }
        RevisionSourceRef::KnowledgeSignal(value) => {
            CandidateRevisionSourceRefRow::KnowledgeSignal(value)
        }
        RevisionSourceRef::Gap(value) => CandidateRevisionSourceRefRow::Gap(value),
    }
}

fn db_mutation(
    mutation: golish_agent_kit::harness::hypothesis_registry::CandidateHypothesisMutation,
    route: CandidateRegistryMutationDecisionV1,
) -> InvestigationAnalysisHostResult<CandidateMutationRow> {
    let route = match route {
        CandidateRegistryMutationDecisionV1::AttachCurrent {
            root_id,
            revision_id,
        } => CandidateMutationRouteRow::AttachCurrent {
            root_id,
            revision_id,
        },
        CandidateRegistryMutationDecisionV1::CreateInitial { root_id } => {
            CandidateMutationRouteRow::CreateInitial { root_id }
        }
        _ => {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "Investigation compiler emitted an unsupported canonical route".to_owned(),
            });
        }
    };
    Ok(CandidateMutationRow {
        proposal_id: mutation.proposal_id,
        organization_id: mutation.organization_id,
        semantic_key_hash: mutation.semantic_key_hash,
        operator_rank: mutation.operator_rank,
        state: mutation.state,
        proof_refs: mutation.proof_refs.into_iter().map(db_source_ref).collect(),
        refutation_refs: mutation
            .refutation_refs
            .into_iter()
            .map(db_source_ref)
            .collect(),
        generation_transition_hash: mutation.generation_transition_hash,
        mutation_hash: mutation.mutation_hash,
        route,
    })
}

fn map_sqlx_error(error: sqlx::Error) -> InvestigationAnalysisHostError {
    InvestigationAnalysisHostError::Infrastructure {
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golish_agent_kit::task_orchestrator::hypothesis_analysis::CandidateProofReference;

    #[test]
    fn post_synthesis_committed_artifacts_fail_closed_on_partial_or_collision() {
        assert_eq!(complete_committed_artifact_set(0, 0, 0), Ok(false));
        assert_eq!(complete_committed_artifact_set(1, 1, 1), Ok(true));
        for counts in [(1, 0, 0), (1, 1, 0), (0, 0, 1), (2, 1, 1)] {
            assert!(
                complete_committed_artifact_set(counts.0, counts.1, counts.2).is_err(),
                "partial/colliding artifact set {counts:?} must fail closed"
            );
        }
    }

    #[test]
    fn compiler_proposal_rebuilds_proof_role_as_canonical_source_role() {
        let proposal = CandidateHypothesisProposal {
            proposal_id: Uuid::new_v4(),
            subject_kind: "web_origin".to_owned(),
            subject_identity_hash: format!("sha256:{}", "1".repeat(64)),
            predicate_schema: "http.exposure".to_owned(),
            predicate_version: 1,
            predicate_arguments: vec![("origin".to_owned(), "redacted".to_owned())],
            trust_boundary: "internet_to_web".to_owned(),
            polarity: "positive".to_owned(),
            structured_claim: "typed claim".to_owned(),
            preconditions: Vec::new(),
            impact: "bounded".to_owned(),
            proof_refs: vec![CandidateProofReference {
                input_id: Uuid::new_v4(),
                chunk_id: Uuid::new_v4(),
                source_hash: format!("sha256:{}", "2".repeat(64)),
                role: CandidateProofReferenceRole::AuthorizationUse,
            }],
            knowledge_signals: Vec::new(),
            readiness: CandidateProposalReadiness::ReadyForStrategy,
        };
        let input = compiler_proposal_input(&proposal).expect("canonical proposal");
        assert_eq!(input.proof_refs[0].source_role, "authorization_use");
        assert_eq!(
            input.canonical_proposal["proof_refs"][0]["source_role"],
            "authorization_use"
        );
        assert!(input.canonical_proposal["proof_refs"][0]
            .get("role")
            .is_none());
        assert_eq!(
            input.canonical_proposal["predicate_arguments"],
            serde_json::json!({"origin":"redacted"})
        );
    }
}
