//! Application-owned forwarding boundary for Plan C Campaign persistence.
//!
//! The bridge preserves server-owned identifiers and typed repository error
//! classes.  It never exposes a SQL transaction or performs provider/network
//! work.  The Pg adapter below is the sole place where domain requests are
//! translated to `golish-db` short transaction compounds.

use std::sync::Arc;

use async_trait::async_trait;
use golish_agent_kit::db_traits::*;
use sqlx::PgPool;

pub(crate) fn compiler_disposition_to_repository(
    disposition: golish_pentest_app::pentest_bridge::CapabilityAssessmentDisposition,
) -> CapabilityAssessmentDispositionV1 {
    use golish_pentest_app::pentest_bridge::CapabilityAssessmentDisposition as Compiler;
    match disposition {
        Compiler::Available => CapabilityAssessmentDispositionV1::Available,
        Compiler::AdapterMissing => CapabilityAssessmentDispositionV1::AdapterMissing,
        Compiler::PolicyDenied => CapabilityAssessmentDispositionV1::PolicyDenied,
        Compiler::PrerequisiteMissing => CapabilityAssessmentDispositionV1::PrerequisiteMissing,
        Compiler::Unassessed => CapabilityAssessmentDispositionV1::Unassessed,
    }
}

fn repository_disposition_to_compiler(
    disposition: CapabilityAssessmentDispositionV1,
) -> golish_pentest_app::pentest_bridge::CapabilityAssessmentDisposition {
    use golish_pentest_app::pentest_bridge::CapabilityAssessmentDisposition as Compiler;
    match disposition {
        CapabilityAssessmentDispositionV1::Available => Compiler::Available,
        CapabilityAssessmentDispositionV1::AdapterMissing => Compiler::AdapterMissing,
        CapabilityAssessmentDispositionV1::PolicyDenied => Compiler::PolicyDenied,
        CapabilityAssessmentDispositionV1::PrerequisiteMissing => Compiler::PrerequisiteMissing,
        CapabilityAssessmentDispositionV1::Unassessed => Compiler::Unassessed,
    }
}

pub(crate) fn map_storage_error(error: anyhow::Error) -> VerificationCampaignRepositoryError {
    let detail = error.to_string();
    if detail.contains("repository_unavailable") || detail.contains("REPOSITORY_UNAVAILABLE") {
        VerificationCampaignRepositoryError::Unavailable {
            operation: "pg_verification_campaign_repository",
        }
    } else if detail.contains("OWNERSHIP")
        || detail.contains("AUTHORITY_MISMATCH")
        || detail.contains("SCOPE_MISMATCH")
    {
        VerificationCampaignRepositoryError::AuthorityMismatch { detail }
    } else if detail.contains("NOT_FOUND") || detail.contains("not found") {
        VerificationCampaignRepositoryError::NotFound { detail }
    } else if detail.contains("CONFLICT")
        || detail.contains("CAS")
        || detail.contains("STALE")
        || detail.contains("FENCE")
        || detail.contains("REPLAY")
    {
        VerificationCampaignRepositoryError::Conflict { detail }
    } else if detail.contains("INVALID") || detail.contains("OUT_OF_RANGE") {
        VerificationCampaignRepositoryError::InvalidRequest { detail }
    } else {
        VerificationCampaignRepositoryError::Infrastructure { detail }
    }
}

#[derive(Clone)]
pub struct VerificationCampaignBridge {
    repository: Arc<dyn VerificationCampaignRepository>,
}

impl VerificationCampaignBridge {
    pub fn new(repository: Arc<dyn VerificationCampaignRepository>) -> Self {
        Self { repository }
    }

    pub fn repository(&self) -> Arc<dyn VerificationCampaignRepository> {
        self.repository.clone()
    }
}

#[async_trait]
impl VerificationCampaignRepository for VerificationCampaignBridge {
    async fn seal_wave_coverage_denominator(
        &self,
        request: SealWaveCoverage,
    ) -> RepoResult<WaveCoverageSeal> {
        self.repository
            .seal_wave_coverage_denominator(request)
            .await
    }

    async fn record_capability_assessment(
        &self,
        request: RecordCapabilityAssessment,
    ) -> RepoResult<CapabilityAssessment> {
        let registry = golish_pentest_app::pentest_bridge::verification_action_compiler::VerificationCapabilityRegistry::authoritative_v1();
        let _registry_assessment =
            registry.assessment(&request.capability_id).ok_or_else(|| {
                VerificationCampaignRepositoryError::InvalidRequest {
                    detail: "capability is absent from the closed verification registry".to_owned(),
                }
            })?;
        self.repository.record_capability_assessment(request).await
    }

    async fn seal_capability_assessment_set(
        &self,
        request: SealCapabilityAssessmentSet,
    ) -> RepoResult<CapabilityAssessmentSetSeal> {
        self.repository
            .seal_capability_assessment_set(request)
            .await
    }

    async fn admit_campaign_with_fresh_tool_truth(
        &self,
        request: AdmitCampaignRequest,
    ) -> RepoResult<CampaignLease> {
        self.repository
            .admit_campaign_with_fresh_tool_truth(request)
            .await
    }

    async fn open_round(&self, request: OpenCampaignRound) -> RepoResult<CampaignRound> {
        self.repository.open_round(request).await
    }

    async fn persist_strategy_decision(&self, request: PersistStrategyDecision) -> RepoResult<()> {
        self.repository.persist_strategy_decision(request).await
    }

    async fn seal_coverage_denominator(
        &self,
        request: SealCampaignCoverageDenominator,
    ) -> RepoResult<CampaignCoverageDenominatorSeal> {
        self.repository.seal_coverage_denominator(request).await
    }

    async fn propose_prepared_action(
        &self,
        request: ProposePreparedAction,
    ) -> RepoResult<PreparedActionProposal> {
        self.repository.propose_prepared_action(request).await
    }

    async fn begin_action(&self, request: BeginPreparedAction) -> RepoResult<ActionBeginReceipt> {
        self.repository.begin_action(request).await
    }

    async fn record_action_subexecution(
        &self,
        request: RecordActionSubexecution,
    ) -> RepoResult<ActionSubexecutionReceipt> {
        self.repository.record_action_subexecution(request).await
    }

    async fn closeout_action(&self, request: CloseoutPreparedAction) -> RepoResult<ActionCloseout> {
        self.repository.closeout_action(request).await
    }

    async fn recover_unknown_action(
        &self,
        request: RecoverUnknownPreparedAction,
    ) -> RepoResult<ActionRecoveryCloseout> {
        self.repository.recover_unknown_action(request).await
    }

    async fn seal_oracle_census(&self, request: SealOracleCensus) -> RepoResult<OracleCensusSeal> {
        self.repository.seal_oracle_census(request).await
    }

    async fn close_campaign_objective(
        &self,
        request: CloseCampaignObjective,
    ) -> RepoResult<ObjectiveOutcomeReceipt> {
        self.repository.close_campaign_objective(request).await
    }

    async fn adjudicate_hypothesis_revision_with_fresh_tool_truth(
        &self,
        request: AdjudicateHypothesisRevision,
    ) -> RepoResult<HypothesisRevisionAdjudicationReceipt> {
        self.repository
            .adjudicate_hypothesis_revision_with_fresh_tool_truth(request)
            .await
    }

    async fn quarantine_campaign_authority(
        &self,
        request: QuarantineCampaignAuthority,
    ) -> RepoResult<AuthorityQuarantineReceipt> {
        self.repository.quarantine_campaign_authority(request).await
    }
}

#[derive(Clone)]
pub struct VerificationCampaignShadowBridge {
    repository: Arc<dyn VerificationCampaignShadowRepository>,
}

impl VerificationCampaignShadowBridge {
    pub fn new(repository: Arc<dyn VerificationCampaignShadowRepository>) -> Self {
        Self { repository }
    }

    pub fn repository(&self) -> Arc<dyn VerificationCampaignShadowRepository> {
        self.repository.clone()
    }
}

#[async_trait]
impl VerificationCampaignShadowRepository for VerificationCampaignShadowBridge {
    async fn open_evaluation(&self, request: OpenShadowEvaluation) -> RepoResult<ShadowEvaluation> {
        self.repository.open_evaluation(request).await
    }

    async fn record_receipt_replay_and_compare_v1(
        &self,
        request: RecordShadowReceiptReplay,
    ) -> RepoResult<ComparisonId> {
        self.repository
            .record_receipt_replay_and_compare_v1(request)
            .await
    }

    async fn close_evaluation(
        &self,
        request: CloseShadowEvaluation,
    ) -> RepoResult<ShadowEvaluationReceipt> {
        self.repository.close_evaluation(request).await
    }
}

/// Concrete production adapter.  Task 3 owns every SQL compound; this type
/// owns only the pool capability and does not expose it through the trait.
#[derive(Clone)]
pub struct PgVerificationCampaignRepository {
    pool: Arc<PgPool>,
}

impl PgVerificationCampaignRepository {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

fn campaign_coverage_epistemic_outcome<'a>(
    disposition: &str,
    verdict: Option<&'a str>,
) -> Option<&'a str> {
    match (disposition, verdict) {
        ("assessed", Some(verdict @ ("proof" | "refutation" | "inconclusive"))) => Some(verdict),
        ("blocked" | "untested", None) => Some("not_assessed"),
        _ => None,
    }
}

#[allow(dead_code)]
fn verification_budget_axes(
    requests: i64,
    response_bytes: i64,
    wall_clock_ms: i64,
    retries: i64,
) -> Vec<golish_db::repo::verification_prepared_actions::BudgetContractAxis> {
    [
        ("requests", requests),
        ("response_bytes", response_bytes),
        ("wall_clock_ms", wall_clock_ms),
        ("retries", retries),
        ("browser_steps", 1),
        ("oast_tokens", 1),
    ]
    .into_iter()
    .map(|(axis_kind, axis_limit)| {
        golish_db::repo::verification_prepared_actions::BudgetContractAxis {
            axis_kind: axis_kind.to_owned(),
            axis_limit,
        }
    })
    .collect()
}

#[allow(dead_code)]
async fn verification_budget_contract_material(
    pool: &PgPool,
    operation_id: uuid::Uuid,
    project_scope_id: uuid::Uuid,
    organization_id: uuid::Uuid,
    scope_kind: &str,
    scope_id: uuid::Uuid,
    parent_contract_id: Option<uuid::Uuid>,
    stable_request_id: uuid::Uuid,
    default_axes: Vec<golish_db::repo::verification_prepared_actions::BudgetContractAxis>,
) -> RepoResult<(
    uuid::Uuid,
    String,
    Vec<golish_pentest_app::pentest_bridge::BudgetLimit>,
)> {
    let existing: Option<(uuid::Uuid, String, Option<uuid::Uuid>)> = sqlx::query_as(
        r#"SELECT budget_contract_id,contract_hash,parent_contract_id
             FROM verification_budget_contracts
            WHERE scope_kind=$1 AND scope_id=$2 AND operation_id=$3
              AND project_scope_id=$4 AND organization_id=$5
              AND sealed_at IS NOT NULL"#,
    )
    .bind(scope_kind)
    .bind(scope_id)
    .bind(operation_id)
    .bind(project_scope_id)
    .bind(organization_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
    let contract_id = if let Some((contract_id, _, persisted_parent)) = existing.as_ref() {
        if *persisted_parent != parent_contract_id {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "verification budget parent contract drift".to_owned(),
            });
        }
        *contract_id
    } else {
        golish_db::repo::verification_prepared_actions::seal_budget_contract(
            pool,
            &golish_db::repo::verification_prepared_actions::SealBudgetContract {
                stable_request_id,
                operation_id,
                project_scope_id,
                organization_id,
                scope_kind: scope_kind.to_owned(),
                scope_id,
                parent_contract_id,
                contract_version: "verification-budget-policy.v1".to_owned(),
                axes: default_axes,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
    };
    let contract_hash: String = sqlx::query_scalar(
        "SELECT contract_hash FROM verification_budget_contracts WHERE budget_contract_id=$1 AND sealed_at IS NOT NULL",
    )
    .bind(contract_id)
    .fetch_one(pool)
    .await
    .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
    let axes: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT axis_kind,axis_limit FROM verification_budget_contract_axes
            WHERE budget_contract_id=$1 ORDER BY axis_ordinal"#,
    )
    .bind(contract_id)
    .fetch_all(pool)
    .await
    .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
    let limits = axes
        .into_iter()
        .map(|(axis, limit)| {
            use golish_pentest_app::pentest_bridge::{BudgetLimit, VerificationBudgetAxis};
            let axis = match axis.as_str() {
                "requests" => VerificationBudgetAxis::Requests,
                "response_bytes" => VerificationBudgetAxis::ResponseBytes,
                "wall_clock_ms" => VerificationBudgetAxis::WallClockMs,
                "retries" => VerificationBudgetAxis::Retries,
                "browser_steps" => VerificationBudgetAxis::BrowserSteps,
                "oast_tokens" => VerificationBudgetAxis::OastTokens,
                _ => {
                    return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                        detail: "unknown verification budget axis".to_owned(),
                    })
                }
            };
            Ok(BudgetLimit {
                axis,
                limit: u64::try_from(limit).map_err(|_| {
                    VerificationCampaignRepositoryError::AuthorityMismatch {
                        detail: "negative verification budget axis".to_owned(),
                    }
                })?,
            })
        })
        .collect::<RepoResult<Vec<_>>>()?;
    if limits.len() != 6 {
        return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "verification budget contract is not a six-axis exact set".to_owned(),
        });
    }
    Ok((contract_id, contract_hash, limits))
}

#[allow(dead_code)]
async fn preview_verification_budget_contract_hash(
    pool: &PgPool,
    operation_id: uuid::Uuid,
    project_scope_id: uuid::Uuid,
    organization_id: uuid::Uuid,
    scope_kind: &str,
    scope_id: uuid::Uuid,
    parent_contract_id: uuid::Uuid,
    axes: &[golish_db::repo::verification_prepared_actions::BudgetContractAxis],
) -> RepoResult<String> {
    let mut ordered = axes.iter().collect::<Vec<_>>();
    let order = [
        "requests",
        "response_bytes",
        "wall_clock_ms",
        "retries",
        "browser_steps",
        "oast_tokens",
    ];
    ordered.sort_by_key(|axis| {
        order
            .iter()
            .position(|candidate| *candidate == axis.axis_kind)
            .unwrap_or(order.len())
    });
    let mut member_hashes = Vec::with_capacity(ordered.len());
    for (ordinal, axis) in ordered.iter().enumerate() {
        member_hashes.push(
            sqlx::query_scalar::<_, String>("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(serde_json::json!({
                    "axis_ordinal": ordinal,
                    "axis_kind": axis.axis_kind,
                    "axis_limit": axis.axis_limit,
                }))
                .fetch_one(pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?,
        );
    }
    let member_set_hash: String = sqlx::query_scalar(
        "SELECT investigation_exact_member_set_hash('verification_budget_contract.v1',$1::TEXT[])",
    )
    .bind(&member_hashes)
    .fetch_one(pool)
    .await
    .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
    sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
        .bind(serde_json::json!({
            "operation_id": operation_id,
            "project_scope_id": project_scope_id,
            "organization_id": organization_id,
            "scope_kind": scope_kind,
            "scope_id": scope_id,
            "parent_contract_id": parent_contract_id,
            "contract_version": "verification-budget-policy.v1",
            "member_set_hash": member_set_hash,
        }))
        .fetch_one(pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))
}

// Until a given Task 3 compound exists, the production adapter fails closed
// with the same typed error as a legacy repository.  Methods are overridden as
// their short-transaction counterparts land; no generic SQL fallback exists.
#[async_trait]
impl VerificationCampaignRepository for PgVerificationCampaignRepository {
    async fn seal_wave_coverage_denominator(
        &self,
        request: SealWaveCoverage,
    ) -> RepoResult<WaveCoverageSeal> {
        if let Some((seal_id, member_count, member_set_hash)) =
            sqlx::query_as::<_, (uuid::Uuid, i64, String)>(
                r#"SELECT wave_denominator_id,member_count,member_set_hash
                     FROM verification_wave_coverage_denominators
                    WHERE stable_request_id=$1 AND operation_id=$2
                      AND organization_id=$3 AND generation_seal_id=$4"#,
            )
            .bind(request.stable_request_id)
            .bind(request.operation_id)
            .bind(request.organization_id)
            .bind(request.generation_seal_id)
            .fetch_optional(&*self.pool)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        {
            return Ok(WaveCoverageSeal {
                seal_id,
                operation_id: request.operation_id,
                generation_seal_id: request.generation_seal_id,
                member_count: u32::try_from(member_count).map_err(|_| {
                    VerificationCampaignRepositoryError::AuthorityMismatch {
                        detail: "wave member count overflow".to_owned(),
                    }
                })?,
                member_set_hash,
                replayed: true,
            });
        }
        let (project_scope_id, source_snapshot_hash): (uuid::Uuid, String) = sqlx::query_as(
            r#"SELECT operation.project_scope_id,seal.generation_hash
                 FROM operation_state operation
                 JOIN hypothesis_generation_seals seal ON seal.seal_id=$4
                 JOIN hypothesis_generations generation ON generation.generation_id=seal.generation_id
                WHERE operation.operation_id=$1 AND generation.operation_id=$1
                  AND generation.organization_id=$3
                  AND EXISTS(SELECT 1 FROM operation_org_scope_snapshots scope
                              JOIN operation_org_scope_units unit ON unit.snapshot_id=scope.id
                             WHERE scope.id=$2 AND scope.operation_id=$1
                               AND scope.project_scope_id=operation.project_scope_id
                               AND unit.organization_id=$3 AND scope.sealed_at IS NOT NULL)"#,
        )
        .bind(request.operation_id)
        .bind(request.scope_snapshot_id)
        .bind(request.organization_id)
        .bind(request.generation_seal_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "wave generation authority is not current".to_owned(),
        })?;
        #[derive(sqlx::FromRow)]
        struct WaveMemberAuthority {
            input_ref_id: uuid::Uuid,
            input_identity_hash: String,
            hypothesis_revision_id: uuid::Uuid,
            claim_component_id: uuid::Uuid,
            claim_component_key: String,
            claim_component_hash: String,
            verification_objective_id: uuid::Uuid,
            predicate_component_id: uuid::Uuid,
            predicate_semantic_key: String,
            required_control_id: Option<uuid::Uuid>,
            required_control_hash: Option<String>,
            no_control_marker_hash: Option<String>,
            capability_assessment_id: uuid::Uuid,
            capability_key: String,
        }
        let rows = sqlx::query_as::<_, WaveMemberAuthority>(
            r#"SELECT plan_objective.plan_objective_id AS input_ref_id,
                      plan_objective.member_hash AS input_identity_hash,
                      plan_objective.revision_id AS hypothesis_revision_id,
                      component.component_id AS claim_component_id,
                      component.component_key AS claim_component_key,
                      component.member_hash AS claim_component_hash,
                      plan_objective.objective_id AS verification_objective_id,
                      predicate.predicate_component_id,
                      predicate.semantic_key AS predicate_semantic_key,
                      control.required_control_id,
                      control.member_hash AS required_control_hash,
                      CASE WHEN contract.explicit_no_required_control
                           THEN contract.required_control_set_hash ELSE NULL END
                           AS no_control_marker_hash,
                      assessment.assessment_id AS capability_assessment_id,
                      assessment.capability_key
                 FROM attack_hypothesis_verification_plans plan
                 JOIN attack_hypothesis_verification_plan_objectives plan_objective
                   ON plan_objective.plan_id=plan.plan_id
                 JOIN hypothesis_generation_members generation_member
                   ON generation_member.revision_id=plan.revision_id
                 JOIN hypothesis_generation_seals generation_seal
                   ON generation_seal.generation_id=generation_member.generation_id
                  AND generation_seal.seal_id=$2
                 JOIN attack_hypothesis_verification_contracts contract
                   ON contract.contract_id=plan_objective.verification_contract_id
                 JOIN attack_hypothesis_verification_objective_claim_components binding
                   ON binding.contract_id=contract.contract_id
                 JOIN attack_hypothesis_claim_components component
                   ON component.component_id=binding.claim_component_id
                 JOIN attack_hypothesis_verification_predicate_components predicate
                   ON predicate.contract_id=contract.contract_id
                 LEFT JOIN attack_hypothesis_verification_required_controls control
                   ON control.contract_id=contract.contract_id
                 JOIN verification_capability_assessment_set_seals assessment_set
                   ON assessment_set.hypothesis_revision_id=plan.revision_id
                  AND assessment_set.verification_objective_id=plan_objective.objective_id
                  AND assessment_set.verification_contract_hash=plan_objective.verification_contract_hash
                  AND assessment_set.sealed_at IS NOT NULL
                 JOIN verification_capability_assessment_set_members assessment_member
                   ON assessment_member.assessment_set_seal_id=assessment_set.assessment_set_seal_id
                 JOIN verification_capability_assessments assessment
                   ON assessment.assessment_id=assessment_member.assessment_id
                WHERE generation_seal.seal_id=$2 AND plan.sealed_at IS NOT NULL
                  AND EXISTS (
                      SELECT 1
                        FROM attack_hypothesis_verification_plans selected_plan
                        JOIN hypothesis_generation_members selected_member
                          ON selected_member.revision_id=selected_plan.revision_id
                       WHERE selected_plan.plan_id=$1
                         AND selected_plan.sealed_at IS NOT NULL
                         AND selected_member.generation_id=generation_seal.generation_id
                  )
                ORDER BY generation_member.ordinal,plan_objective.ordinal,
                         binding.ordinal,predicate.ordinal,
                         control.ordinal NULLS FIRST,assessment_member.member_ordinal"#,
        )
        .bind(request.verification_plan_id)
        .bind(request.generation_seal_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let members = rows
            .into_iter()
            .map(|row| {
                let (expected_action_kind, expected_oracle_kind) = match row.capability_key.as_str()
                {
                    "verify.anonymous_authenticated_differential.v1" => (
                        "trusted_http_differential.v1",
                        "anonymous_access_differential.v1",
                    ),
                    "verify.directory_fingerprint.v1" => (
                        "trusted_http_directory_fingerprint.v1",
                        "directory_soft404_fingerprint.v1",
                    ),
                    "verify.nuclei_exact_replay.v1" => {
                        ("unavailable_cli_egress.v1", "nuclei_cli_signal.v1")
                    }
                    "verify.concurrent_race_differential.v1" => (
                        "host_pinned_group_transport_required.v1",
                        "concurrent_relation.v1",
                    ),
                    _ => ("unregistered_action", "unregistered_oracle"),
                };
                let control_key = row
                    .required_control_id
                    .map_or_else(|| "no_control".to_owned(), |id| id.to_string());
                golish_db::repo::verification_campaign_coverage::WaveCoverageMember {
                    semantic_key: format!(
                        "{}:{}:{}:{}:{}",
                        row.verification_objective_id,
                        row.claim_component_key,
                        row.predicate_semantic_key,
                        control_key,
                        row.capability_key
                    ),
                    input_ref_kind: "plan_objective".to_owned(),
                    input_ref_id: row.input_ref_id,
                    input_identity_hash: row.input_identity_hash,
                    hypothesis_revision_id: row.hypothesis_revision_id,
                    claim_component_id: row.claim_component_id,
                    claim_component_hash: row.claim_component_hash,
                    verification_objective_id: row.verification_objective_id,
                    predicate_component_id: row.predicate_component_id,
                    control_binding_kind: if row.required_control_id.is_some() {
                        "required".to_owned()
                    } else {
                        "explicit_no_control".to_owned()
                    },
                    required_control_id: row.required_control_id,
                    required_control_hash: row.required_control_hash,
                    no_control_marker_hash: row.no_control_marker_hash,
                    capability_assessment_id: row.capability_assessment_id,
                    expected_capability_kind: row.capability_key,
                    expected_action_kind: expected_action_kind.to_owned(),
                    expected_oracle_kind: expected_oracle_kind.to_owned(),
                }
            })
            .collect::<Vec<_>>();
        let seal_id =
            golish_db::repo::verification_campaign_coverage::seal_wave_coverage_denominator(
                &self.pool,
                &golish_db::repo::verification_campaign_coverage::SealWaveCoverageDenominator {
                    stable_request_id: request.stable_request_id,
                    operation_id: request.operation_id,
                    project_scope_id,
                    organization_id: request.organization_id,
                    generation_seal_id: request.generation_seal_id,
                    contract_version: "verification-wave-coverage-denominator.v1".to_owned(),
                    source_snapshot_hash,
                    members,
                },
            )
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let (member_count, member_set_hash): (i64, String) = sqlx::query_as(
            "SELECT member_count,member_set_hash FROM verification_wave_coverage_denominators WHERE wave_denominator_id=$1",
        )
        .bind(seal_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(WaveCoverageSeal {
            seal_id,
            operation_id: request.operation_id,
            generation_seal_id: request.generation_seal_id,
            member_count: u32::try_from(member_count).map_err(|_| {
                VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "wave member count overflow".to_owned(),
                }
            })?,
            member_set_hash,
            replayed: false,
        })
    }

    async fn record_capability_assessment(
        &self,
        request: RecordCapabilityAssessment,
    ) -> RepoResult<CapabilityAssessment> {
        let registry =
            golish_pentest_app::pentest_bridge::VerificationCapabilityRegistry::authoritative_v1();
        let registry_assessment = registry.assessment(&request.capability_id).ok_or_else(|| {
            VerificationCampaignRepositoryError::InvalidRequest {
                detail: "capability is absent from the closed verification registry".to_owned(),
            }
        })?;
        #[derive(sqlx::FromRow)]
        struct AssessmentAuthority {
            project_scope_id: uuid::Uuid,
            revision_id: uuid::Uuid,
            verification_contract_id: uuid::Uuid,
            verification_contract_hash: String,
            policy_snapshot_hash: String,
            source_snapshot_hash: String,
        }
        let authority = sqlx::query_as::<_, AssessmentAuthority>(
            r#"SELECT operation.project_scope_id,plan_objective.revision_id,
                      plan_objective.verification_contract_id,
                      plan_objective.verification_contract_hash,
                      contract.policy_snapshot_hash,seal.generation_hash AS source_snapshot_hash
                 FROM operation_state operation
                 JOIN attack_hypothesis_verification_plan_objectives plan_objective
                   ON plan_objective.objective_id=$3
                 JOIN attack_hypothesis_revisions revision
                   ON revision.revision_id=plan_objective.revision_id
                  AND revision.operation_id=operation.operation_id
                  AND revision.organization_id=$2
                 JOIN attack_hypothesis_verification_contracts contract
                   ON contract.contract_id=plan_objective.verification_contract_id
                 JOIN hypothesis_generation_members generation_member
                   ON generation_member.revision_id=revision.revision_id
                  AND generation_member.operation_id=operation.operation_id
                  AND generation_member.organization_id=$2
                 JOIN hypothesis_generation_seals seal
                   ON seal.generation_id=generation_member.generation_id
                WHERE operation.operation_id=$1
                ORDER BY seal.sealed_at DESC LIMIT 1"#,
        )
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(request.objective_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "capability assessment authority is not current".to_owned(),
        })?;
        let (status, default_reason_code) = match request.disposition {
            CapabilityAssessmentDispositionV1::Available => ("available", None),
            CapabilityAssessmentDispositionV1::AdapterMissing => {
                ("adapter_missing", Some("adapter_missing"))
            }
            CapabilityAssessmentDispositionV1::PolicyDenied => {
                ("policy_denied", Some("policy_denied"))
            }
            CapabilityAssessmentDispositionV1::PrerequisiteMissing => {
                ("prerequisite_missing", Some("prerequisite_missing"))
            }
            CapabilityAssessmentDispositionV1::Unassessed => ("unassessed", Some("unassessed")),
        };
        let available = status == "available";
        if available
            != (request.adapter_contract_version.is_some()
                && request.adapter_contract_digest.is_some())
            || (available
                && (registry_assessment.disposition
                    != repository_disposition_to_compiler(
                        CapabilityAssessmentDispositionV1::Available,
                    )
                    || request.adapter_contract_version.as_deref()
                        != registry_assessment.adapter_contract_version.as_deref()
                    || request.adapter_contract_digest.as_deref()
                        != registry_assessment.adapter_contract_digest.as_deref()))
        {
            return Err(VerificationCampaignRepositoryError::InvalidRequest {
                detail: "capability disposition/adapter does not match the closed host registry"
                    .to_owned(),
            });
        }
        if let Some(existing) = sqlx::query_as::<
            _,
            (
                uuid::Uuid,
                uuid::Uuid,
                uuid::Uuid,
                String,
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                String,
            ),
        >(
            r#"SELECT assessment_id,operation_id,verification_objective_id,
                      capability_key,status,reason_code,adapter_contract_version,
                      adapter_contract_digest,assessment_hash
                 FROM verification_capability_assessments
                WHERE stable_request_id=$1"#,
        )
        .bind(request.stable_request_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        {
            let existing_disposition = match existing.4.as_str() {
                "available" => CapabilityAssessmentDispositionV1::Available,
                "adapter_missing" => CapabilityAssessmentDispositionV1::AdapterMissing,
                "policy_denied" => CapabilityAssessmentDispositionV1::PolicyDenied,
                "prerequisite_missing" => CapabilityAssessmentDispositionV1::PrerequisiteMissing,
                "unassessed" => CapabilityAssessmentDispositionV1::Unassessed,
                _ => {
                    return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                        detail: "persisted capability assessment has an invalid disposition"
                            .to_owned(),
                    });
                }
            };
            let expected_reason = (!available).then(|| {
                request
                    .residual_reason_code
                    .clone()
                    .unwrap_or_else(|| default_reason_code.unwrap_or("unassessed").to_owned())
            });
            if existing.1 != request.operation_id
                || existing.2 != request.objective_id
                || existing.3 != request.capability_id
                || existing_disposition != request.disposition
                || existing.5 != expected_reason
                || existing.6 != request.adapter_contract_version
                || existing.7 != request.adapter_contract_digest
            {
                return Err(VerificationCampaignRepositoryError::Conflict {
                    detail: "capability assessment replay request drifted".to_owned(),
                });
            }
            return Ok(CapabilityAssessment {
                assessment_id: existing.0,
                operation_id: existing.1,
                objective_id: existing.2,
                capability_id: existing.3,
                disposition: existing_disposition,
                assessment_hash: existing.8,
                replayed: true,
            });
        }
        let reason_code = if available {
            None
        } else {
            let reason = request
                .residual_reason_code
                .as_deref()
                .or(default_reason_code)
                .unwrap_or_default()
                .trim();
            if reason.is_empty() {
                return Err(VerificationCampaignRepositoryError::InvalidRequest {
                    detail: "unavailable capability requires a typed residual reason".to_owned(),
                });
            }
            Some(reason.to_owned())
        };
        let residual_id = if available {
            None
        } else {
            let residual_id = uuid::Uuid::new_v5(
                &request.stable_request_id,
                b"verification-capability-residual.v1",
            );
            let affected_inputs = serde_json::json!([{
                "objective_id": request.objective_id,
                "capability_id": &request.capability_id,
            }]);
            let next_action = serde_json::json!({
                "kind": "capability_contract_required",
                "capability_id": &request.capability_id,
            });
            let residual_hash: String =
                sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                    .bind(serde_json::json!({
                        "operation_id": request.operation_id,
                        "organization_id": request.organization_id,
                                "revision_id": authority.revision_id,
                                "reason_code": &reason_code,
                                "affected_inputs": &affected_inputs,
                        "next_action": &next_action,
                    }))
                    .fetch_one(&*self.pool)
                    .await
                    .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
            sqlx::query(
                r#"INSERT INTO hypothesis_residual_risks(
                       residual_id,operation_id,organization_id,revision_id,reason_code,
                       owner_kind,affected_inputs,next_action,residual_hash
                   ) VALUES($1,$2,$3,$4,$5,'plan_c',$6,$7,$8)
                   ON CONFLICT(residual_id) DO NOTHING"#,
            )
            .bind(residual_id)
            .bind(request.operation_id)
            .bind(request.organization_id)
            .bind(authority.revision_id)
            .bind(&reason_code)
            .bind(&affected_inputs)
            .bind(&next_action)
            .bind(&residual_hash)
            .execute(&*self.pool)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
            Some(residual_id)
        };
        let capability_contract_version = request
            .adapter_contract_version
            .clone()
            .unwrap_or_else(|| "unavailable.v1".to_owned());
        let capability_contract_hash: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(serde_json::json!({
                    "capability_id": &request.capability_id,
                    "capability_contract_version": &capability_contract_version,
                    "adapter_contract_digest": &request.adapter_contract_digest,
                    "status": status,
                }))
                .fetch_one(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let predecessor: Option<(uuid::Uuid, i64)> = sqlx::query_as(
            r#"SELECT assessment_id,assessment_ordinal
                 FROM verification_capability_assessments
                WHERE hypothesis_revision_id=$1 AND verification_objective_id=$2
                  AND capability_key=$3
                ORDER BY assessment_ordinal DESC LIMIT 1"#,
        )
        .bind(authority.revision_id)
        .bind(request.objective_id)
        .bind(&request.capability_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let replayed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM verification_capability_assessments WHERE stable_request_id=$1)",
        )
        .bind(request.stable_request_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let row =
            golish_db::repo::verification_capability_assessments::record_capability_assessment(
                &self.pool,
                &golish_db::repo::verification_capability_assessments::RecordCapabilityAssessment {
                    stable_request_id: request.stable_request_id,
                    operation_id: request.operation_id,
                    project_scope_id: authority.project_scope_id,
                    organization_id: request.organization_id,
                    hypothesis_revision_id: authority.revision_id,
                    verification_objective_id: request.objective_id,
                    verification_contract_id: authority.verification_contract_id,
                    verification_contract_hash: authority.verification_contract_hash,
                    capability_key: request.capability_id,
                    capability_contract_version,
                    capability_contract_hash,
                    policy_snapshot_id: authority.verification_contract_id,
                    policy_snapshot_hash: authority.policy_snapshot_hash,
                    assessment_ordinal: predecessor.map_or(0, |item| item.1 + 1),
                    supersedes_assessment_id: predecessor.map(|item| item.0),
                    status: status.to_owned(),
                    reason_code,
                    residual_id,
                    adapter_contract_version: request.adapter_contract_version,
                    adapter_contract_digest: request.adapter_contract_digest,
                    source_snapshot_hash: authority.source_snapshot_hash,
                },
            )
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(CapabilityAssessment {
            assessment_id: row.assessment_id,
            operation_id: row.operation_id,
            objective_id: row.verification_objective_id,
            capability_id: row.capability_key,
            disposition: request.disposition,
            assessment_hash: row.assessment_hash,
            replayed,
        })
    }

    async fn seal_capability_assessment_set(
        &self,
        request: SealCapabilityAssessmentSet,
    ) -> RepoResult<CapabilityAssessmentSetSeal> {
        let registry =
            golish_pentest_app::pentest_bridge::VerificationCapabilityRegistry::authoritative_v1();
        if let Some((seal_id, member_count, member_set_hash, registry_contract_hash)) =
            sqlx::query_as::<_, (uuid::Uuid, i64, String, String)>(
                r#"SELECT assessment_set_seal_id,member_count,member_set_hash,
                          registry_contract_hash
                     FROM verification_capability_assessment_set_seals
                    WHERE stable_request_id=$1 AND operation_id=$2
                      AND organization_id=$3 AND verification_objective_id=$4"#,
            )
            .bind(request.stable_request_id)
            .bind(request.operation_id)
            .bind(request.organization_id)
            .bind(request.objective_id)
            .fetch_optional(&*self.pool)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        {
            if registry_contract_hash != registry.registry_contract_hash() {
                return Err(VerificationCampaignRepositoryError::Conflict {
                    detail: "assessment-set replay uses a different compiler registry contract"
                        .to_owned(),
                });
            }
            return Ok(CapabilityAssessmentSetSeal {
                seal_id,
                operation_id: request.operation_id,
                member_count: u32::try_from(member_count).map_err(|_| {
                    VerificationCampaignRepositoryError::AuthorityMismatch {
                        detail: "assessment-set member count overflow".to_owned(),
                    }
                })?,
                member_set_hash,
                replayed: true,
            });
        }
        #[derive(sqlx::FromRow)]
        struct SetAuthority {
            project_scope_id: uuid::Uuid,
            revision_id: uuid::Uuid,
            objective_id: uuid::Uuid,
            verification_contract_hash: String,
            policy_snapshot_hash: String,
            source_snapshot_hash: String,
        }
        let authority = sqlx::query_as::<_, SetAuthority>(
            r#"SELECT assessment.project_scope_id,assessment.hypothesis_revision_id AS revision_id,
                      assessment.verification_objective_id AS objective_id,
                      assessment.verification_contract_hash,assessment.policy_snapshot_hash,
                      assessment.source_snapshot_hash
                 FROM verification_capability_assessments assessment
                WHERE assessment.operation_id=$1 AND assessment.organization_id=$2
                  AND assessment.verification_objective_id=$3
                ORDER BY assessment.assessed_at DESC LIMIT 1"#,
        )
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(request.objective_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::NotFound {
            detail: "no capability assessments for objective".to_owned(),
        })?;
        let latest_assessments: Vec<(uuid::Uuid, String)> = sqlx::query_as(
            r#"SELECT DISTINCT ON (capability_key) assessment_id,capability_key
                 FROM verification_capability_assessments
                WHERE operation_id=$1 AND organization_id=$2
                  AND hypothesis_revision_id=$3 AND verification_objective_id=$4
                  AND verification_contract_hash=$5 AND policy_snapshot_hash=$6
                ORDER BY capability_key,assessment_ordinal DESC"#,
        )
        .bind(request.operation_id)
        .bind(request.organization_id)
        .bind(authority.revision_id)
        .bind(authority.objective_id)
        .bind(&authority.verification_contract_hash)
        .bind(&authority.policy_snapshot_hash)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let observed_keys = latest_assessments
            .iter()
            .map(|(_, capability_key)| capability_key.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let expected_keys = registry
            .capability_ids()
            .collect::<std::collections::BTreeSet<_>>();
        if observed_keys != expected_keys {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "latest capability assessment set is not exact-equal to the host registry"
                    .to_owned(),
            });
        }
        let assessment_ids = latest_assessments
            .into_iter()
            .map(|(assessment_id, _)| assessment_id)
            .collect();
        let seal_id = golish_db::repo::verification_capability_assessments::seal_capability_assessment_set(
            &self.pool,
            &golish_db::repo::verification_capability_assessments::SealCapabilityAssessmentSet {
                stable_request_id: request.stable_request_id,
                operation_id: request.operation_id,
                project_scope_id: authority.project_scope_id,
                organization_id: request.organization_id,
                hypothesis_revision_id: authority.revision_id,
                verification_objective_id: authority.objective_id,
                verification_contract_hash: authority.verification_contract_hash,
                policy_snapshot_hash: authority.policy_snapshot_hash,
                source_snapshot_hash: authority.source_snapshot_hash,
                registry_contract_hash: registry.registry_contract_hash().to_owned(),
                assessment_ids,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let (member_count, member_set_hash): (i64, String) = sqlx::query_as(
            "SELECT member_count,member_set_hash FROM verification_capability_assessment_set_seals WHERE assessment_set_seal_id=$1",
        )
        .bind(seal_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(CapabilityAssessmentSetSeal {
            seal_id,
            operation_id: request.operation_id,
            member_count: u32::try_from(member_count).map_err(|_| {
                VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "assessment-set member count overflow".to_owned(),
                }
            })?,
            member_set_hash,
            replayed: false,
        })
    }

    async fn admit_campaign_with_fresh_tool_truth(
        &self,
        request: AdmitCampaignRequest,
    ) -> RepoResult<CampaignLease> {
        let admitted =
            golish_db::repo::verification_campaigns::admit_campaign_with_fresh_tool_truth(
                &self.pool,
                golish_db::repo::verification_campaigns::AdmitCampaignFromAuthority {
                    stable_consumer_request_id: request.stable_consumer_request_id,
                    operation_id: request.operation_id,
                    scope_snapshot_id: request.scope_snapshot_id,
                    organization_id: request.organization_id,
                    generation_seal_id: request.generation_seal_id,
                    verification_plan_id: request.verification_plan_id,
                    objective_id: request.objective_id,
                    wave_coverage_seal_id: request.wave_coverage_seal_id,
                    capability_assessment_set_seal_id: request.capability_assessment_set_seal_id,
                    expected_campaign_id: request.expected_campaign_id,
                },
            )
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(CampaignLease {
            campaign_id: admitted.campaign.campaign_id,
            operation_id: admitted.campaign.operation_id,
            objective_id: request.objective_id,
            campaign_dispatch_generation: admitted.campaign_dispatch_generation,
            row_version: admitted.campaign.row_version,
            replayed: admitted.replayed,
        })
    }

    async fn open_round(&self, request: OpenCampaignRound) -> RepoResult<CampaignRound> {
        if let Some(existing) = sqlx::query_as::<_, (uuid::Uuid, i32, i64, i64, String)>(
            r#"SELECT round_id,round_ordinal,expected_campaign_row_version,
                      consult_member_count,consult_member_set_hash
                 FROM verification_campaign_rounds
                WHERE stable_request_id=$1 AND campaign_id=$2 AND operation_id=$3"#,
        )
        .bind(request.stable_request_id)
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        {
            if existing.2 != request.expected_campaign_row_version {
                return Err(VerificationCampaignRepositoryError::Conflict {
                    detail: "round replay expected version drift".to_owned(),
                });
            }
            return Ok(CampaignRound {
                round_id: existing.0,
                campaign_id: request.campaign_id,
                ordinal: u32::try_from(existing.1).map_err(|_| {
                    VerificationCampaignRepositoryError::AuthorityMismatch {
                        detail: "negative round ordinal".to_owned(),
                    }
                })?,
                consult_census_id: uuid::Uuid::new_v5(
                    &existing.0,
                    b"verification-consult-census.v1",
                ),
                consult_member_count: u32::try_from(existing.3).map_err(|_| {
                    VerificationCampaignRepositoryError::AuthorityMismatch {
                        detail: "consult count overflow".to_owned(),
                    }
                })?,
                consult_member_set_hash: existing.4,
                row_version: request.expected_campaign_row_version + 1,
                replayed: true,
            });
        }
        #[derive(sqlx::FromRow)]
        struct RoundAuthority {
            project_scope_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            verification_objective_id: uuid::Uuid,
            verification_contract_hash: String,
            capability_assessment_set_seal_id: uuid::Uuid,
            wave_denominator_id: uuid::Uuid,
            source_snapshot_hash: String,
        }
        let selected = sqlx::query_as::<_, RoundAuthority>(
            r#"SELECT project_scope_id,organization_id,verification_objective_id,
                      verification_contract_hash,capability_assessment_set_seal_id,
                      wave_denominator_id,source_snapshot_hash
                 FROM verification_campaigns
                WHERE campaign_id=$1 AND operation_id=$2 AND row_version=$3
                  AND state IN ('admitted','running')"#,
        )
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(request.expected_campaign_row_version)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "campaign round authority is not current".to_owned(),
        })?;
        #[derive(sqlx::FromRow)]
        struct RoundConsultAuthorityMember {
            wave_coverage_member_id: uuid::Uuid,
            member_hash: String,
            expected_capability_kind: String,
            expected_action_kind: String,
            expected_oracle_kind: String,
            assessment_status: String,
        }
        let authority_members = sqlx::query_as::<_, RoundConsultAuthorityMember>(
            r#"SELECT member.wave_coverage_member_id,member.member_hash,
                      member.expected_capability_kind,member.expected_action_kind,
                      member.expected_oracle_kind,assessment.status AS assessment_status
                 FROM verification_wave_coverage_members member
                 JOIN verification_capability_assessments assessment
                   ON assessment.assessment_id=member.capability_assessment_id
                  AND assessment.hypothesis_revision_id=member.hypothesis_revision_id
                  AND assessment.verification_objective_id=member.verification_objective_id
                WHERE member.wave_denominator_id=$1
                  AND member.verification_objective_id=$2
                ORDER BY member.member_ordinal"#,
        )
        .bind(selected.wave_denominator_id)
        .bind(selected.verification_objective_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if authority_members.is_empty() {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "campaign round has no sealed objective authority members".to_owned(),
            });
        }
        let specialist_role = authority_members
            .iter()
            .flat_map(|member| {
                [
                    member.expected_capability_kind.as_str(),
                    member.expected_action_kind.as_str(),
                    member.expected_oracle_kind.as_str(),
                ]
            })
            .find_map(|value| {
                let value = value.to_ascii_lowercase();
                if value.contains("auth") || value.contains("credential") {
                    Some("verification_auth_specialist")
                } else if value.contains("api") || value.contains("graphql") {
                    Some("verification_api_specialist")
                } else if value.contains("inject") || value.contains("sql") || value.contains("xss")
                {
                    Some("verification_injection_specialist")
                } else if value.contains("business")
                    || value.contains("state")
                    || value.contains("race")
                    || value.contains("idempot")
                {
                    Some("verification_business_logic_specialist")
                } else {
                    None
                }
            })
            .unwrap_or("verification_pentester");
        let roles = [
            "verification_lead",
            specialist_role,
            "verification_independent_critic",
        ];
        if !roles.iter().all(|role| {
            golish_sub_agents::executor::verification_campaign::is_verification_campaign_role(role)
        }) {
            return Err(map_storage_error(anyhow::anyhow!(
                "host Campaign selected a role outside the closed verification role registry"
            )));
        }
        let round_id = uuid::Uuid::new_v5(&request.stable_request_id, b"verification-round.v1");
        let round_input = serde_json::json!({
            "contract_version": "verification-campaign-round-input.v1",
            "campaign_id": request.campaign_id,
            "objective_id": selected.verification_objective_id,
            "verification_contract_hash": selected.verification_contract_hash,
            "capability_assessment_set_seal_id": selected.capability_assessment_set_seal_id,
            "wave_denominator_id": selected.wave_denominator_id,
            "source_snapshot_hash": selected.source_snapshot_hash,
        });
        let input_projection_hash: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(&round_input)
                .fetch_one(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let mut obligation_ids = authority_members
            .iter()
            .map(|member| member.wave_coverage_member_id.to_string())
            .collect::<Vec<_>>();
        obligation_ids.sort();
        let mut coverage_member_hashes = authority_members
            .iter()
            .map(|member| member.member_hash.clone())
            .collect::<Vec<_>>();
        coverage_member_hashes.sort();
        let mut residual_codes = authority_members
            .iter()
            .filter(|member| member.assessment_status != "available")
            .map(|member| format!("capability_{}", member.assessment_status))
            .collect::<Vec<_>>();
        residual_codes.sort();
        residual_codes.dedup();
        let mut consults = Vec::with_capacity(roles.len());
        for (ordinal, role_kind) in roles.into_iter().enumerate() {
            let request_packet = serde_json::json!({
                "contract_version": "verification-host-consult-request.v1",
                "campaign_id": request.campaign_id,
                "round_id": round_id,
                "objective_id": selected.verification_objective_id,
                "role_id": role_kind,
                "input_projection_hash": input_projection_hash,
                "artifact_kind": match role_kind {
                    "verification_lead" => "strategy_decision_or_terminal_intent",
                    "verification_evidence_analyst" => "evidence_analysis",
                    "verification_independent_critic" => "independent_critique",
                    "verification_refiner" => "typed_plan_delta",
                    "verification_adviser" | "verification_reflector" => "bounded_recovery_advice",
                    _ => "consult_proposal",
                },
                "obligation_ids": obligation_ids,
                "coverage_member_hashes": coverage_member_hashes,
                "residual_codes": residual_codes,
            });
            let request_member_hash: String =
                sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                    .bind(serde_json::json!({
                        "ordinal": ordinal,
                        "role_kind": role_kind,
                        "request_packet": request_packet,
                    }))
                    .fetch_one(&*self.pool)
                    .await
                    .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
            let consult_lane_id = uuid::Uuid::new_v5(&round_id, request_member_hash.as_bytes());
            let artifact_kind = match role_kind {
                "verification_lead" => "strategy_decision_or_terminal_intent",
                "verification_evidence_analyst" => "evidence_analysis",
                "verification_independent_critic" => "independent_critique",
                "verification_refiner" => "typed_plan_delta",
                "verification_adviser" | "verification_reflector" => "bounded_recovery_advice",
                _ => "consult_proposal",
            };
            let bounded_observations = match role_kind {
                "verification_lead" => {
                    vec!["host_policy_selected_only_sealed_campaign_obligations".to_owned()]
                }
                "verification_independent_critic" if residual_codes.is_empty() => {
                    vec!["closed_capability_projection_contains_no_unavailable_member".to_owned()]
                }
                "verification_independent_critic" => {
                    vec!["unavailable_capability_members_are_preserved_as_residuals".to_owned()]
                }
                _ => {
                    vec!["bounded_method_proposal_uses_only_current_campaign_authority".to_owned()]
                }
            };
            let artifact = golish_sub_agents::executor::verification_campaign::VerificationCampaignArtifactV1 {
                schema: "verification_campaign_artifact.v1".to_owned(),
                campaign_id: request.campaign_id,
                round_id,
                consult_lane_id,
                objective_id: selected.verification_objective_id,
                role_id: role_kind.to_owned(),
                input_projection_hash: input_projection_hash.clone(),
                artifact_kind: artifact_kind.to_owned(),
                disposition: "proposed".to_owned(),
                obligation_ids: obligation_ids.clone(),
                coverage_member_hashes: coverage_member_hashes.clone(),
                evidence_refs: Vec::new(),
                residual_codes: residual_codes.clone(),
                bounded_observations,
            };
            let response_artifact = serde_json::to_value(&artifact).map_err(|error| {
                VerificationCampaignRepositoryError::Infrastructure {
                    detail: format!("failed to serialize host Campaign consult artifact: {error}"),
                }
            })?;
            golish_sub_agents::executor::verification_campaign::parse_campaign_artifact(
                role_kind,
                &input_projection_hash,
                &serde_json::to_vec(&response_artifact).map_err(|error| {
                    VerificationCampaignRepositoryError::Infrastructure {
                        detail: format!("failed to encode host Campaign consult artifact: {error}"),
                    }
                })?,
            )
            .map_err(|error| {
                VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: format!("host Campaign consult artifact was rejected: {error}"),
                }
            })?;
            consults.push(
                golish_db::repo::verification_campaigns::ConsultCensusMember {
                    role_kind: role_kind.to_owned(),
                    request_packet,
                    response_artifact: None,
                    residual_id: None,
                },
            );
        }
        let row = golish_db::repo::verification_campaigns::open_round_with_consult_census(
            &self.pool,
            &golish_db::repo::verification_campaigns::OpenRoundWithConsultCensus {
                stable_request_id: request.stable_request_id,
                campaign_id: request.campaign_id,
                operation_id: request.operation_id,
                project_scope_id: selected.project_scope_id,
                organization_id: selected.organization_id,
                expected_campaign_row_version: request.expected_campaign_row_version,
                round_input,
                consults,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(CampaignRound {
            round_id: row.round_id,
            campaign_id: row.campaign_id,
            ordinal: u32::try_from(row.round_ordinal).map_err(|_| {
                VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "negative round ordinal".to_owned(),
                }
            })?,
            consult_census_id: uuid::Uuid::new_v5(&row.round_id, b"verification-consult-census.v1"),
            consult_member_count: u32::try_from(row.consult_member_count).map_err(|_| {
                VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "consult count overflow".to_owned(),
                }
            })?,
            consult_member_set_hash: row.consult_member_set_hash,
            row_version: request.expected_campaign_row_version + 1,
            replayed: false,
        })
    }

    async fn persist_strategy_decision(&self, request: PersistStrategyDecision) -> RepoResult<()> {
        let expected_schema = format!("{}.v{}", request.strategy_schema, request.strategy_version);
        let strategy = request.typed_strategy.as_object().ok_or_else(|| {
            VerificationCampaignRepositoryError::InvalidRequest {
                detail: "typed strategy must be an object".to_owned(),
            }
        })?;
        let capability_id = strategy
            .get("capability")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if request.strategy_schema.trim().is_empty()
            || request.strategy_version == 0
            || !request.strategy_hash.starts_with("sha256:")
            || !request.obligation_set_hash.starts_with("sha256:")
            || strategy.get("schema").and_then(serde_json::Value::as_str)
                != Some(expected_schema.as_str())
            || strategy
                .get("strategy_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| uuid::Uuid::parse_str(value).ok())
                != Some(request.strategy_decision_id)
            || strategy
                .get("campaign_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| uuid::Uuid::parse_str(value).ok())
                != Some(request.campaign_id)
            || !matches!(
                capability_id,
                "verify.anonymous_authenticated_differential.v1"
                    | "verify.directory_fingerprint.v1"
                    | "verify.nuclei_exact_replay.v1"
                    | "verify.concurrent_race_differential.v1"
            )
        {
            return Err(VerificationCampaignRepositoryError::InvalidRequest {
                detail: "invalid sealed strategy envelope".to_owned(),
            });
        }
        let server_strategy_hash: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(&request.typed_strategy)
                .fetch_one(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if server_strategy_hash != request.strategy_hash {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "strategy body hash differs from the sealed advisory".to_owned(),
            });
        }

        #[derive(sqlx::FromRow)]
        struct StrategyAuthority {
            project_scope_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            verification_objective_id: uuid::Uuid,
            wave_denominator_id: uuid::Uuid,
        }
        let authority = sqlx::query_as::<_, StrategyAuthority>(
            r#"SELECT campaign.project_scope_id,campaign.organization_id,
                      campaign.verification_objective_id,campaign.wave_denominator_id
                 FROM verification_campaign_rounds round
                 JOIN verification_campaigns campaign
                   ON campaign.campaign_id=round.campaign_id
                  AND campaign.operation_id=round.operation_id
                WHERE round.round_id=$1 AND round.campaign_id=$2
                  AND round.operation_id=$3 AND campaign.row_version=$4
                  AND (round.closed_at IS NULL OR EXISTS(
                      SELECT 1 FROM verification_strategy_artifacts replay_strategy
                       WHERE replay_strategy.stable_request_id=$5
                         AND replay_strategy.round_id=round.round_id
                  ))"#,
        )
        .bind(request.round_id)
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(request.expected_round_row_version)
        .bind(request.stable_request_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "strategy round authority is not current".to_owned(),
        })?;
        if strategy
            .get("objective_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            != Some(authority.verification_objective_id)
        {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "strategy objective differs from Campaign authority".to_owned(),
            });
        }
        let obligation_rows = sqlx::query_as::<_, (String, String, String)>(
            r#"SELECT semantic_key,expected_capability_kind,
                      tool_truth_sha256(jsonb_build_object(
                          'semantic_key',semantic_key,
                          'expected_capability_kind',expected_capability_kind
                      )::TEXT)
                 FROM verification_wave_coverage_members
                WHERE wave_denominator_id=$1 AND verification_objective_id=$2
                  AND expected_capability_kind=$3
                ORDER BY member_ordinal"#,
        )
        .bind(authority.wave_denominator_id)
        .bind(authority.verification_objective_id)
        .bind(capability_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if obligation_rows.is_empty() {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "strategy capability has no exact Campaign obligation".to_owned(),
            });
        }
        let obligation_hashes = obligation_rows
            .iter()
            .map(|row| row.2.clone())
            .collect::<Vec<_>>();
        let server_obligation_set_hash: String = sqlx::query_scalar(
            "SELECT unified_investigation_exact_set_hash('investigation_verification_strategy_obligations.v1',$1::TEXT[])",
        )
        .bind(&obligation_hashes)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if server_obligation_set_hash != request.obligation_set_hash {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "strategy obligation exact set differs from Campaign authority".to_owned(),
            });
        }
        let obligations = obligation_rows
            .into_iter()
            .map(|(semantic_key, obligation_kind, _)| {
                golish_db::repo::verification_campaigns::StrategyObligation {
                    obligation_kind,
                    semantic_key,
                    disposition: "planned".to_owned(),
                    residual_id: None,
                }
            })
            .collect::<Vec<_>>();
        let artifact_id =
            uuid::Uuid::new_v5(&request.stable_request_id, b"verification-strategy.v1");
        let replay = sqlx::query_as::<
            _,
            (
                uuid::Uuid,
                uuid::Uuid,
                uuid::Uuid,
                uuid::Uuid,
                uuid::Uuid,
                uuid::Uuid,
                serde_json::Value,
                String,
            ),
        >(
            r#"SELECT strategy_artifact_id,round_id,campaign_id,operation_id,
                      project_scope_id,organization_id,typed_strategy,strategy_hash
                 FROM verification_strategy_artifacts
                WHERE stable_request_id=$1"#,
        )
        .bind(request.stable_request_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if let Some(existing) = replay {
            let existing_obligations = sqlx::query_as::<_, (String, String, String)>(
                r#"SELECT semantic_key,obligation_kind,disposition
                     FROM verification_strategy_obligations
                    WHERE strategy_artifact_id=$1 ORDER BY obligation_ordinal"#,
            )
            .bind(existing.0)
            .fetch_all(&*self.pool)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
            let expected_obligations = obligations
                .iter()
                .map(|obligation| {
                    (
                        obligation.semantic_key.clone(),
                        obligation.obligation_kind.clone(),
                        obligation.disposition.clone(),
                    )
                })
                .collect::<Vec<_>>();
            if existing.0 != artifact_id
                || existing.1 != request.round_id
                || existing.2 != request.campaign_id
                || existing.3 != request.operation_id
                || existing.4 != authority.project_scope_id
                || existing.5 != authority.organization_id
                || existing.6 != request.typed_strategy
                || existing.7 != request.strategy_hash
                || existing_obligations != expected_obligations
            {
                return Err(VerificationCampaignRepositoryError::Conflict {
                    detail: "strategy replay identity drift".to_owned(),
                });
            }
            return Ok(());
        }
        golish_db::repo::verification_campaigns::record_strategy_decision(
            &self.pool,
            &golish_db::repo::verification_campaigns::RecordStrategyDecision {
                stable_request_id: request.stable_request_id,
                round_id: request.round_id,
                campaign_id: request.campaign_id,
                operation_id: request.operation_id,
                project_scope_id: authority.project_scope_id,
                organization_id: authority.organization_id,
                decision_kind: "compile_action".to_owned(),
                typed_strategy: request.typed_strategy,
                reason_code: "host_validated_strategy".to_owned(),
                residual_id: None,
                obligations,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(())
    }

    async fn seal_coverage_denominator(
        &self,
        request: SealCampaignCoverageDenominator,
    ) -> RepoResult<CampaignCoverageDenominatorSeal> {
        #[derive(sqlx::FromRow)]
        struct CoverageAuthority {
            project_scope_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            hypothesis_revision_id: uuid::Uuid,
            wave_denominator_id: uuid::Uuid,
            verification_contract_id: uuid::Uuid,
            source_snapshot_hash: String,
        }
        let authority = sqlx::query_as::<_, CoverageAuthority>(
            r#"SELECT project_scope_id,organization_id,hypothesis_revision_id,
                      wave_denominator_id,verification_contract_id,source_snapshot_hash
                 FROM verification_campaigns
                WHERE campaign_id=$1 AND operation_id=$2
                  AND verification_objective_id=$3
                  AND verification_contract_id=$4
                  AND row_version=$5 AND state='running'"#,
        )
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(request.objective_id)
        .bind(request.verification_contract_id)
        .bind(request.expected_campaign_row_version)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "campaign coverage authority is not current".to_owned(),
        })?;
        if authority.verification_contract_id != request.verification_contract_id {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "campaign verification contract identity drifted".to_owned(),
            });
        }
        let members = sqlx::query_as::<
            _,
            (
                uuid::Uuid,
                String,
                uuid::Uuid,
                String,
                String,
                String,
                uuid::Uuid,
                String,
                String,
                String,
            ),
        >(
            r#"SELECT wave_coverage_member_id,semantic_key,claim_component_id,
                      claim_component_hash,expected_capability_kind,
                      control_binding_kind,capability_assessment_id,
                      expected_capability_kind,expected_action_kind,expected_oracle_kind
                 FROM verification_wave_coverage_members
                WHERE wave_denominator_id=$1 AND verification_objective_id=$2
                ORDER BY member_ordinal"#,
        )
        .bind(authority.wave_denominator_id)
        .bind(request.objective_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .into_iter()
        .map(
            |row| golish_db::repo::verification_campaign_coverage::CampaignCoverageMember {
                wave_coverage_member_id: row.0,
                semantic_key: row.1,
                claim_component_id: row.2,
                claim_component_hash: row.3,
                obligation_kind: row.4,
                control_binding_kind: row.5,
                capability_assessment_id: row.6,
                expected_capability_kind: row.7,
                expected_action_kind: row.8,
                expected_oracle_kind: row.9,
            },
        )
        .collect::<Vec<_>>();
        let replayed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM verification_campaign_coverage_denominators WHERE stable_request_id=$1)",
        )
        .bind(request.stable_request_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if replayed {
            let (seal_id, member_count, member_set_hash): (uuid::Uuid, i64, String) =
                sqlx::query_as(
                    "SELECT campaign_denominator_id,member_count,member_set_hash FROM verification_campaign_coverage_denominators WHERE stable_request_id=$1 AND campaign_id=$2",
                )
                .bind(request.stable_request_id)
                .bind(request.campaign_id)
                .fetch_optional(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
                .ok_or_else(|| VerificationCampaignRepositoryError::Conflict {
                    detail: "coverage replay identity drift".to_owned(),
                })?;
            return Ok(CampaignCoverageDenominatorSeal {
                seal_id,
                campaign_id: request.campaign_id,
                member_count: u32::try_from(member_count).map_err(|_| {
                    VerificationCampaignRepositoryError::AuthorityMismatch {
                        detail: "coverage member count overflow".to_owned(),
                    }
                })?,
                member_set_hash,
                replayed: true,
            });
        }
        let seal_id =
            golish_db::repo::verification_campaign_coverage::seal_campaign_coverage_denominator(
                &self.pool,
                &golish_db::repo::verification_campaign_coverage::SealCampaignCoverageDenominator {
                    stable_request_id: request.stable_request_id,
                    operation_id: request.operation_id,
                    project_scope_id: authority.project_scope_id,
                    organization_id: authority.organization_id,
                    campaign_id: request.campaign_id,
                    hypothesis_revision_id: authority.hypothesis_revision_id,
                    wave_denominator_id: authority.wave_denominator_id,
                    contract_version: "verification-campaign-coverage-denominator.v1".to_owned(),
                    source_snapshot_hash: authority.source_snapshot_hash,
                    members,
                },
            )
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let (member_count, member_set_hash): (i64, String) = sqlx::query_as(
            "SELECT member_count,member_set_hash FROM verification_campaign_coverage_denominators WHERE campaign_denominator_id=$1",
        )
        .bind(seal_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(CampaignCoverageDenominatorSeal {
            seal_id,
            campaign_id: request.campaign_id,
            member_count: u32::try_from(member_count).map_err(|_| {
                VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "coverage member count overflow".to_owned(),
                }
            })?,
            member_set_hash,
            replayed,
        })
    }

    async fn propose_prepared_action(
        &self,
        request: ProposePreparedAction,
    ) -> RepoResult<PreparedActionProposal> {
        use golish_pentest_app::pentest_bridge::{
            compile_prepared_action, BudgetLimit, FrozenBudgetEnvelope, FrozenCampaignAuthority,
            FrozenCapabilityAssessmentAuthority, FrozenTargetAuthority, PreparedActionContractV1,
            SealedStrategyDecision, VerificationBudgetAxis, VerificationCapabilityRegistry,
        };

        #[derive(sqlx::FromRow)]
        struct CompileAuthority {
            project_scope_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            scope_snapshot_id: uuid::Uuid,
            scope_snapshot_hash: String,
            verification_objective_id: uuid::Uuid,
            wave_denominator_id: uuid::Uuid,
            campaign_denominator_id: uuid::Uuid,
            strategy_decision_id: uuid::Uuid,
            strategy_hash: String,
            wave_member_hash: String,
            campaign_member_hash: String,
            capability_assessment_id: uuid::Uuid,
            capability_key: String,
            capability_contract_hash: String,
            assessment_hash: String,
            assessment_status: String,
            adapter_contract_version: Option<String>,
            adapter_contract_digest: Option<String>,
            assessment_set_member_hash: String,
            registry_contract_hash: String,
            policy_snapshot_hash: String,
            semantic_authority_bundle_hash: String,
            target_live_id: uuid::Uuid,
            target_type_at_time: String,
            target_value_at_time: String,
            expected_oracle_kind: String,
        }
        let authority = sqlx::query_as::<_, CompileAuthority>(
            r#"SELECT campaign.project_scope_id,campaign.organization_id,
                      scope_snapshot.id AS scope_snapshot_id,
                      scope_snapshot.scope_hash AS scope_snapshot_hash,
                      campaign.verification_objective_id,campaign.wave_denominator_id,
                      denominator.campaign_denominator_id,
                      (strategy.typed_strategy->>'strategy_id')::UUID AS strategy_decision_id,
                      strategy.strategy_hash,
                      wave_member.member_hash AS wave_member_hash,
                      campaign_member.member_hash AS campaign_member_hash,
                      assessment.assessment_id AS capability_assessment_id,
                      assessment.capability_key,assessment.capability_contract_hash,
                      assessment.assessment_hash,assessment.status AS assessment_status,
                      assessment.adapter_contract_version,assessment.adapter_contract_digest,
                      assessment_set.member_set_hash AS assessment_set_member_hash,
                      assessment_set.registry_contract_hash,
                      assessment.policy_snapshot_hash,campaign.semantic_authority_bundle_hash,
                      resolved_target.target_live_id,resolved_target.target_type_at_time,
                      resolved_target.target_value_at_time,campaign_member.expected_oracle_kind
                 FROM verification_campaigns campaign
                 JOIN verification_campaign_rounds round
                   ON round.round_id=$3 AND round.campaign_id=campaign.campaign_id
                  AND round.operation_id=campaign.operation_id
                  AND (round.closed_at IS NULL OR (
                       round.disposition='action_compiled' AND EXISTS(
                           SELECT 1 FROM verification_prepared_actions replay
                            WHERE replay.stable_request_id=$6
                              AND replay.round_id=round.round_id
                       )))
                 JOIN verification_strategy_artifacts strategy
                   ON strategy.strategy_artifact_id=$4 AND strategy.round_id=round.round_id
                  AND strategy.campaign_id=campaign.campaign_id
                  AND strategy.decision_kind='compile_action'
                 JOIN verification_strategy_obligations obligation
                   ON obligation.strategy_artifact_id=strategy.strategy_artifact_id
                  AND obligation.obligation_id=$5 AND obligation.disposition='planned'
                 JOIN verification_campaign_coverage_denominators denominator
                   ON denominator.campaign_id=campaign.campaign_id
                  AND denominator.sealed_at IS NOT NULL
                 JOIN verification_campaign_coverage_members campaign_member
                   ON campaign_member.campaign_denominator_id=denominator.campaign_denominator_id
                  AND campaign_member.semantic_key=obligation.semantic_key
                  AND campaign_member.expected_capability_kind=obligation.obligation_kind
                 JOIN verification_wave_coverage_members wave_member
                   ON wave_member.wave_coverage_member_id=campaign_member.wave_coverage_member_id
                  AND wave_member.wave_denominator_id=campaign.wave_denominator_id
                 JOIN verification_capability_assessments assessment
                   ON assessment.assessment_id=campaign_member.capability_assessment_id
                  AND assessment.status='available'
                 JOIN verification_capability_assessment_set_seals assessment_set
                   ON assessment_set.assessment_set_seal_id=campaign.capability_assessment_set_seal_id
                  AND assessment_set.sealed_at IS NOT NULL
                 JOIN verification_capability_assessment_set_members assessment_member
                   ON assessment_member.assessment_set_seal_id=assessment_set.assessment_set_seal_id
                  AND assessment_member.assessment_id=assessment.assessment_id
                 JOIN attack_hypothesis_revisions revision
                   ON revision.revision_id=campaign.hypothesis_revision_id
                  AND revision.operation_id=campaign.operation_id
                  AND revision.organization_id=campaign.organization_id
                 JOIN project_scopes scope ON scope.project_scope_id=campaign.project_scope_id
                  AND scope.retired_at IS NULL
                 JOIN operation_org_scope_snapshots scope_snapshot
                   ON scope_snapshot.operation_id=campaign.operation_id
                  AND scope_snapshot.project_scope_id=campaign.project_scope_id
                  AND scope_snapshot.project_path_at_freeze=scope.canonical_project_path
                  AND scope_snapshot.sealed_at IS NOT NULL
                 JOIN operation_org_scope_units scope_unit
                   ON scope_unit.snapshot_id=scope_snapshot.id
                  AND scope_unit.organization_id=campaign.organization_id
                 JOIN LATERAL (
                     SELECT (ARRAY_AGG(candidate.target_live_id
                                       ORDER BY candidate.target_live_id))[1]
                                AS target_live_id,
                            (ARRAY_AGG(candidate.target_type_at_time
                                       ORDER BY candidate.target_live_id))[1]
                                AS target_type_at_time,
                            (ARRAY_AGG(candidate.target_value_at_time
                                       ORDER BY candidate.target_live_id))[1]
                                AS target_value_at_time
                       FROM (
                           SELECT DISTINCT target.id AS target_live_id,
                                  revision.target_type_at_time,
                                  revision.target_value_at_time
                             FROM targets target
                            WHERE revision.target_live_id IS NOT NULL
                              AND target.id=revision.target_live_id
                           UNION ALL
                           SELECT DISTINCT target.id,target.target_type::TEXT,target.value
                             FROM targets target
                            WHERE revision.target_live_id IS NULL
                              AND revision.subject_kind='asset'
                              AND tool_truth_sha256(jsonb_build_object(
                                  'domain','investigation_subject_identity.v1',
                                  'subject_kind','asset',
                                  'subject_id',target.id,
                                  'display_value',target.value
                              )::TEXT)=revision.subject_identity_hash
                           UNION ALL
                           SELECT DISTINCT target.id,'url'::TEXT,endpoint.url
                             FROM api_endpoints endpoint
                             JOIN targets target ON target.id=endpoint.target_id
                            WHERE revision.target_live_id IS NULL
                              AND revision.subject_kind='endpoint'
                              AND endpoint.project_path=scope.canonical_project_path
                              AND tool_truth_sha256(jsonb_build_object(
                                  'domain','investigation_subject_identity.v1',
                                  'subject_kind','endpoint',
                                  'subject_id',endpoint.id,
                                  'display_value',endpoint.url
                              )::TEXT)=revision.subject_identity_hash
                           UNION ALL
                           (
                           SELECT DISTINCT ON (origin.id)
                                  target.id,'url'::TEXT,origin.origin
                             FROM web_origins origin
                             JOIN web_origin_observations observation
                               ON observation.web_origin_id=origin.id
                              AND observation.organization_id=campaign.organization_id
                             JOIN targets target ON target.id=observation.target_id
                              AND target.organization_id=campaign.organization_id
                              AND target.project_path=scope.canonical_project_path
                              AND target.scope='in'
                            WHERE revision.target_live_id IS NULL
                              AND revision.subject_kind='web_origin'
                              AND origin.project_path=scope.canonical_project_path
                              AND observation.project_path=scope.canonical_project_path
                              AND tool_truth_sha256(jsonb_build_object(
                                  'domain','investigation_subject_identity.v1',
                                  'subject_kind','web_origin',
                                  'subject_id',origin.id,
                                  'display_value',origin.origin
                              )::TEXT)=revision.subject_identity_hash
                            ORDER BY origin.id,
                                     CASE target.target_type::TEXT
                                         WHEN 'url' THEN 0
                                         WHEN 'domain' THEN 1
                                         ELSE 2
                                     END,
                                     target.id
                           )
                       ) candidate
                     HAVING COUNT(*)=1
                 ) resolved_target ON TRUE
                 JOIN targets target ON target.id=resolved_target.target_live_id
                  AND target.organization_id=campaign.organization_id
                  AND target.project_path=scope.canonical_project_path
                  AND target.scope='in'
                WHERE campaign.campaign_id=$1 AND campaign.operation_id=$2
                  AND campaign.state='running' AND campaign.terminal_at IS NULL
                  AND campaign.superseded_at IS NULL"#,
        )
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(request.round_id)
        .bind(request.strategy_artifact_id)
        .bind(request.strategy_obligation_id)
        .bind(request.stable_request_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "prepared-action compiler authority is not current and exact".to_owned(),
        })?;

        let wave_member_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member_hash FROM verification_wave_coverage_members
                WHERE wave_denominator_id=$1 ORDER BY member_ordinal"#,
        )
        .bind(authority.wave_denominator_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let campaign_member_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT member_hash FROM verification_campaign_coverage_members
                WHERE campaign_denominator_id=$1 ORDER BY member_ordinal"#,
        )
        .bind(authority.campaign_denominator_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;

        let target_url = match authority.target_type_at_time.as_str() {
            "url" => authority.target_value_at_time.clone(),
            "domain" => format!(
                "https://{}/",
                authority.target_value_at_time.trim().trim_end_matches('.')
            ),
            "wildcard" => format!(
                "https://{}/",
                authority
                    .target_value_at_time
                    .trim()
                    .trim_start_matches("*.")
                    .trim_end_matches('.')
            ),
            "ip" => {
                let parsed = authority
                    .target_value_at_time
                    .trim()
                    .parse::<std::net::IpAddr>()
                    .map_err(|_| VerificationCampaignRepositoryError::InvalidRequest {
                        detail: "frozen IP target is not parseable".to_owned(),
                    })?;
                match parsed {
                    std::net::IpAddr::V4(value) => format!("https://{value}/"),
                    std::net::IpAddr::V6(value) => format!("https://[{value}]/"),
                }
            }
            _ => {
                return Err(VerificationCampaignRepositoryError::InvalidRequest {
                    detail: "CIDR targets cannot be compiled into an exact HTTP action".to_owned(),
                })
            }
        };
        let parsed_target = url::Url::parse(&target_url).map_err(|_| {
            VerificationCampaignRepositoryError::InvalidRequest {
                detail: "frozen target cannot be normalized into an exact URL".to_owned(),
            }
        })?;
        let normalized_host = parsed_target.host_str().ok_or_else(|| {
            VerificationCampaignRepositoryError::InvalidRequest {
                detail: "frozen target URL has no host".to_owned(),
            }
        })?;
        let port = parsed_target.port_or_known_default().ok_or_else(|| {
            VerificationCampaignRepositoryError::InvalidRequest {
                detail: "frozen target URL has no effective port".to_owned(),
            }
        })?;
        let scope_exception_hash: Option<String> = sqlx::query_scalar(
            r#"SELECT member.exact_scope_exception_hash
                 FROM capability_execution_destination_policy_members member
                 JOIN capability_execution_destination_policies policy
                   ON policy.id=member.policy_id AND policy.execution_authority_id=member.execution_authority_id
                  AND policy.sealed_at IS NOT NULL AND policy.governance_status='enforced'
                 JOIN tool_truth_execution_authorities authority
                   ON authority.id=policy.execution_authority_id
                  AND authority.operation_id=$1 AND authority.organization_id=$2
                  AND authority.project_scope_id=$6
                WHERE member.destination_role='authorized_target'
                  AND member.scheme=$3 AND member.normalized_host=$4 AND member.port=$5
                  AND member.exact_scope_exception_hash IS NOT NULL
                ORDER BY policy.created_at DESC LIMIT 1"#,
        )
        .bind(request.operation_id)
        .bind(authority.organization_id)
        .bind(parsed_target.scheme())
        .bind(normalized_host.to_ascii_lowercase())
        .bind(i32::from(port))
        .bind(authority.project_scope_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;

        let target_identity_hash: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(serde_json::json!({
                    "target_live_id": authority.target_live_id,
                    "project_scope_id": authority.project_scope_id,
                    "organization_id": authority.organization_id,
                    "target_type": &authority.target_type_at_time,
                    "target_value": &authority.target_value_at_time,
                }))
                .fetch_one(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;

        let operation_budget_request = uuid::Uuid::new_v5(
            &request.operation_id,
            format!(
                "verification-budget-operation:{}",
                authority.organization_id
            )
            .as_bytes(),
        );
        let (operation_budget_id, _, operation_limits) = verification_budget_contract_material(
            &self.pool,
            request.operation_id,
            authority.project_scope_id,
            authority.organization_id,
            "operation",
            request.operation_id,
            None,
            operation_budget_request,
            verification_budget_axes(100, 64 * 1024 * 1024, 3_600_000, 10),
        )
        .await?;
        let (wave_budget_id, _, wave_limits) = verification_budget_contract_material(
            &self.pool,
            request.operation_id,
            authority.project_scope_id,
            authority.organization_id,
            "wave",
            authority.wave_denominator_id,
            Some(operation_budget_id),
            uuid::Uuid::new_v5(
                &authority.wave_denominator_id,
                b"verification-budget-wave.v1",
            ),
            verification_budget_axes(50, 32 * 1024 * 1024, 1_800_000, 8),
        )
        .await?;
        let (campaign_budget_id, _, campaign_limits) = verification_budget_contract_material(
            &self.pool,
            request.operation_id,
            authority.project_scope_id,
            authority.organization_id,
            "campaign",
            request.campaign_id,
            Some(wave_budget_id),
            uuid::Uuid::new_v5(&request.campaign_id, b"verification-budget-campaign.v1"),
            verification_budget_axes(10, 8 * 1024 * 1024, 600_000, 4),
        )
        .await?;
        let action_axes = verification_budget_axes(2, 2 * 1024 * 1024, 120_000, 1);
        let action_limits = action_axes
            .iter()
            .map(|axis| {
                let axis_kind = match axis.axis_kind.as_str() {
                    "requests" => VerificationBudgetAxis::Requests,
                    "response_bytes" => VerificationBudgetAxis::ResponseBytes,
                    "wall_clock_ms" => VerificationBudgetAxis::WallClockMs,
                    "retries" => VerificationBudgetAxis::Retries,
                    "browser_steps" => VerificationBudgetAxis::BrowserSteps,
                    "oast_tokens" => VerificationBudgetAxis::OastTokens,
                    _ => unreachable!("host budget policy has a closed axis set"),
                };
                BudgetLimit {
                    axis: axis_kind,
                    limit: u64::try_from(axis.axis_limit)
                        .expect("host budget policy limits are positive"),
                }
            })
            .collect();
        let registry = VerificationCapabilityRegistry::authoritative_v1();
        let disposition = match authority.assessment_status.as_str() {
            "available" => CapabilityAssessmentDispositionV1::Available,
            "adapter_missing" => CapabilityAssessmentDispositionV1::AdapterMissing,
            "policy_denied" => CapabilityAssessmentDispositionV1::PolicyDenied,
            "prerequisite_missing" => CapabilityAssessmentDispositionV1::PrerequisiteMissing,
            "unassessed" => CapabilityAssessmentDispositionV1::Unassessed,
            _ => {
                return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "unknown durable capability assessment disposition".to_owned(),
                })
            }
        };
        let strategy = SealedStrategyDecision {
            strategy_decision_id: authority.strategy_decision_id,
            strategy_decision_hash: authority.strategy_hash.clone(),
            strategy_obligation_id: request.strategy_obligation_id.to_string(),
            wave_coverage_member_hash: authority.wave_member_hash.clone(),
            coverage_member_hash: authority.campaign_member_hash.clone(),
            capability_id: authority.capability_key.clone(),
        };
        let oracle_contract_digest: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(serde_json::json!({
                    "oracle_rule": authority.expected_oracle_kind,
                    "contract_version": "verification-oracle.v1",
                }))
                .fetch_one(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let exact_scope_exception_hash: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(serde_json::json!({
                    "domain": "verification_exact_scope_exception.v1",
                    "scope_snapshot_id": authority.scope_snapshot_id,
                    "scope_snapshot_hash": &authority.scope_snapshot_hash,
                    "operation_id": request.operation_id,
                    "project_scope_id": authority.project_scope_id,
                    "organization_id": authority.organization_id,
                    "target_live_id": authority.target_live_id,
                    "target_type": &authority.target_type_at_time,
                    "target_value": &authority.target_value_at_time,
                }))
                .fetch_one(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let mut compiler_authority = FrozenCampaignAuthority {
            operation_id: request.operation_id,
            organization_id: authority.organization_id,
            campaign_id: request.campaign_id,
            objective_id: authority.verification_objective_id,
            wave_denominator_member_hashes: wave_member_hashes,
            campaign_denominator_member_hashes: campaign_member_hashes,
            capability_assessment_set_hash: authority.assessment_set_member_hash.clone(),
            capability_registry_contract_hash: authority.registry_contract_hash.clone(),
            capability_assessment: FrozenCapabilityAssessmentAuthority {
                assessment_id: authority.capability_assessment_id,
                assessment_hash: authority.assessment_hash.clone(),
                capability_id: authority.capability_key.clone(),
                capability_contract_hash: authority.capability_contract_hash.clone(),
                disposition: repository_disposition_to_compiler(disposition),
                adapter_contract_version: authority.adapter_contract_version.clone(),
                adapter_contract_digest: authority.adapter_contract_digest.clone(),
            },
            oracle_contract_version: "verification-oracle.v1".to_owned(),
            oracle_contract_digest: oracle_contract_digest.clone(),
            target: FrozenTargetAuthority {
                target_id: authority.target_live_id,
                exact_url: target_url,
                scope_authority_hash: authority.semantic_authority_bundle_hash.clone(),
                target_authority_hash: target_identity_hash.clone(),
                allow_non_public_destination: scope_exception_hash.is_some(),
                non_public_destination_exception_hash: scope_exception_hash.clone(),
                credential: None,
            },
            budgets: FrozenBudgetEnvelope {
                operation: operation_limits,
                wave: wave_limits,
                campaign: campaign_limits,
                action: action_limits,
            },
        };
        let compiled = match compile_prepared_action(&strategy, &compiler_authority, &registry) {
            Err(error)
                if error.reason_code == "destination_non_public"
                    && scope_exception_hash.is_none() =>
            {
                compiler_authority.target.allow_non_public_destination = true;
                compiler_authority
                    .target
                    .non_public_destination_exception_hash = Some(exact_scope_exception_hash);
                compile_prepared_action(&strategy, &compiler_authority, &registry).map_err(
                    |error| VerificationCampaignRepositoryError::InvalidRequest {
                        detail: format!("{}: {}", error.reason_code, error.residual),
                    },
                )?
            }
            Err(error) => {
                return Err(VerificationCampaignRepositoryError::InvalidRequest {
                    detail: format!("{}: {}", error.reason_code, error.residual),
                })
            }
            Ok(compiled) => compiled,
        };
        let prepared_action_id = uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"verification-prepared-action.v1",
        );
        let action_budget_hash = preview_verification_budget_contract_hash(
            &self.pool,
            request.operation_id,
            authority.project_scope_id,
            authority.organization_id,
            "action",
            prepared_action_id,
            campaign_budget_id,
            &action_axes,
        )
        .await?;
        let action_ordinal: i32 = sqlx::query_scalar(
            r#"SELECT COALESCE(
                   (SELECT action_ordinal FROM verification_prepared_actions
                     WHERE stable_request_id=$2),
                   (SELECT COALESCE(MAX(action_ordinal),-1)::INT+1
                      FROM verification_prepared_actions WHERE campaign_id=$1)
               )"#,
        )
        .bind(request.campaign_id)
        .bind(request.stable_request_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let replayed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM verification_prepared_actions WHERE stable_request_id=$1)",
        )
        .bind(request.stable_request_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let action_contract_kind = match compiled.contract() {
            PreparedActionContractV1::SingleActionV1 { .. } => "single_action_v1",
            PreparedActionContractV1::ConcurrentActionGroupV1 { .. } => {
                "concurrent_action_group_v1"
            }
        };
        let row = golish_db::repo::verification_prepared_actions::persist_compiled_prepared_action(
            &self.pool,
            &golish_db::repo::verification_prepared_actions::PersistPreparedAction {
                stable_request_id: request.stable_request_id,
                campaign_id: request.campaign_id,
                round_id: request.round_id,
                strategy_artifact_id: request.strategy_artifact_id,
                operation_id: request.operation_id,
                project_scope_id: authority.project_scope_id,
                organization_id: authority.organization_id,
                capability_assessment_id: authority.capability_assessment_id,
                action_ordinal,
                action_contract_kind: action_contract_kind.to_owned(),
                action_kind: compiled.capability_id().to_owned(),
                canonical_request_hash: compiled.semantic_signature().to_owned(),
                display_projection: serde_json::to_value(compiled.display_projection())
                    .expect("typed redacted action display must serialize"),
                renderer_version: compiled.renderer_version().to_owned(),
                private_manifest: compiled.private_manifest_for_persistence(),
                private_manifest_hash: compiled.private_manifest_hash().to_owned(),
                review_expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
                target_live_id: Some(authority.target_live_id),
                target_type_at_time: authority.target_type_at_time.clone(),
                target_value_at_time: authority.target_value_at_time.clone(),
                target_identity_hash,
                credential_binding_hash: None,
                policy_snapshot_hash: authority.policy_snapshot_hash,
                upper_budget_set_hash: action_budget_hash.clone(),
                oracle_contract_hash: oracle_contract_digest,
                risk_tier: format!("{:?}", compiled.risk_tier()),
                compile_rejection: None,
                group_members: Vec::new(),
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let (_, persisted_action_budget_hash, _) = verification_budget_contract_material(
            &self.pool,
            request.operation_id,
            authority.project_scope_id,
            authority.organization_id,
            "action",
            row.prepared_action_id,
            Some(campaign_budget_id),
            uuid::Uuid::new_v5(&request.stable_request_id, b"verification-budget-action.v1"),
            action_axes,
        )
        .await?;
        if persisted_action_budget_hash != action_budget_hash {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "sealed action budget differs from compiler preview".to_owned(),
            });
        }
        let mut conflict_members = Vec::new();
        for key in compiled.conflict_keys() {
            let key_identity_hash: String = sqlx::query_scalar("SELECT tool_truth_sha256($1)")
                .bind(key)
                .fetch_one(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
            conflict_members.push(
                golish_db::repo::verification_prepared_actions::ConflictKeyMember {
                    key_kind: if key.starts_with("credential:") {
                        "credential_session"
                    } else {
                        "target_rate_limit"
                    }
                    .to_owned(),
                    key_identity_hash,
                    adapter_commutativity_authority_hash: None,
                },
            );
        }
        let conflict_set_request_id = uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"verification-action-conflict-set.v1",
        );
        let existing_conflict_set: Option<uuid::Uuid> = sqlx::query_scalar(
            r#"SELECT conflict_set_id FROM verification_action_conflict_sets
                WHERE stable_request_id=$1 AND prepared_action_id=$2
                  AND campaign_id=$3 AND operation_id=$4
                  AND project_scope_id=$5 AND organization_id=$6
                  AND sealed_at IS NOT NULL"#,
        )
        .bind(conflict_set_request_id)
        .bind(row.prepared_action_id)
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(authority.project_scope_id)
        .bind(authority.organization_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if let Some(conflict_set_id) = existing_conflict_set {
            let persisted_members = sqlx::query_as::<_, (String, String, Option<String>)>(
                r#"SELECT key_kind,key_identity_hash,adapter_commutativity_authority_hash
                     FROM verification_action_conflict_set_members
                    WHERE conflict_set_id=$1 ORDER BY member_ordinal"#,
            )
            .bind(conflict_set_id)
            .fetch_all(&*self.pool)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
            let mut expected_members = conflict_members
                .iter()
                .map(|member| {
                    (
                        member.key_kind.clone(),
                        member.key_identity_hash.clone(),
                        member.adapter_commutativity_authority_hash.clone(),
                    )
                })
                .collect::<Vec<_>>();
            expected_members.sort();
            if persisted_members != expected_members {
                return Err(VerificationCampaignRepositoryError::Conflict {
                    detail: "prepared-action conflict-set replay identity drift".to_owned(),
                });
            }
        } else {
            golish_db::repo::verification_prepared_actions::seal_action_conflict_set(
                &self.pool,
                &golish_db::repo::verification_prepared_actions::SealActionConflictSet {
                    stable_request_id: conflict_set_request_id,
                    prepared_action_id: row.prepared_action_id,
                    campaign_id: request.campaign_id,
                    operation_id: request.operation_id,
                    project_scope_id: authority.project_scope_id,
                    organization_id: authority.organization_id,
                    members: conflict_members,
                },
            )
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        }
        sqlx::query(
            r#"UPDATE verification_campaign_rounds
                  SET disposition='action_compiled',
                      disposition_reason_code='host_compiler_persisted_prepared_action',
                      closed_at=statement_timestamp()
                WHERE round_id=$1 AND campaign_id=$2 AND operation_id=$3
                  AND closed_at IS NULL"#,
        )
        .bind(request.round_id)
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .execute(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(PreparedActionProposal {
            prepared_action_id: row.prepared_action_id,
            campaign_id: row.campaign_id,
            capability_id: compiled.capability_id().to_owned(),
            coverage_member_hash: compiled.coverage_member_hash().to_owned(),
            private_manifest_hash: compiled.private_manifest_hash().to_owned(),
            row_version: row.row_version,
            replayed,
        })
    }

    async fn begin_action(&self, request: BeginPreparedAction) -> RepoResult<ActionBeginReceipt> {
        let stable_request_id = uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"verification-authorized-action-begin.v1",
        );
        let replayed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM verification_action_executions WHERE stable_request_id=$1)",
        )
        .bind(stable_request_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let begin = golish_db::repo::verification_prepared_actions::begin_authorized_action_with_fresh_tool_truth(
            &self.pool,
            golish_db::repo::verification_prepared_actions::BeginAuthorizedActionFromAuthority {
                stable_consumer_request_id: request.stable_request_id,
                operation_id: request.operation_id,
                campaign_id: request.campaign_id,
                prepared_action_id: request.prepared_action_id,
                authorization_receipt_id: request.authorization_receipt_id,
                expected_action_row_version: request.expected_action_row_version,
                expected_campaign_dispatch_generation: request.expected_campaign_dispatch_generation,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let authority: (i32, String, String, i64) = sqlx::query_as(
            r#"SELECT execution.execution_ordinal,reservation.upper_bound_membership_hash,
                      conflict_set.member_set_hash,execution.row_version
                 FROM verification_action_executions execution
                 JOIN verification_budget_reservations reservation
                   ON reservation.budget_reservation_id=execution.budget_reservation_id
                 JOIN verification_action_conflict_sets conflict_set
                   ON conflict_set.conflict_set_id=execution.conflict_set_id
                WHERE execution.action_execution_id=$1"#,
        )
        .bind(begin.action_execution_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(ActionBeginReceipt {
            execution_id: begin.action_execution_id,
            prepared_action_id: request.prepared_action_id,
            execution_ordinal: u32::try_from(authority.0).map_err(|_| {
                VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "negative execution ordinal".to_owned(),
                }
            })?,
            budget_reservation_set_hash: authority.1,
            conflict_lease_set_hash: authority.2,
            row_version: authority.3,
            replayed,
        })
    }

    async fn record_action_subexecution(
        &self,
        request: RecordActionSubexecution,
    ) -> RepoResult<ActionSubexecutionReceipt> {
        #[derive(sqlx::FromRow)]
        struct SubexecutionAuthority {
            group_member_id: uuid::Uuid,
            member_hash: String,
            attempt_state: String,
            receipt_hash: String,
            started_at: chrono::DateTime<chrono::Utc>,
            completed_at: chrono::DateTime<chrono::Utc>,
        }
        let authority = sqlx::query_as::<_, SubexecutionAuthority>(
            r#"SELECT member.group_member_id,member.member_hash,receipt.attempt_state,
                      receipt.receipt_authority_hash AS receipt_hash,
                      receipt.observation_started_at AS started_at,
                      receipt.observation_completed_at AS completed_at
                 FROM verification_action_executions execution
                 JOIN verification_prepared_actions action
                   ON action.prepared_action_id=execution.prepared_action_id
                  AND action.operation_id=$1 AND action.campaign_id=$2
                 JOIN verification_prepared_action_group_members member
                   ON member.prepared_action_id=action.prepared_action_id
                  AND member.member_ordinal=$6
                 JOIN capability_execution_receipts receipt
                   ON receipt.id=$7 AND receipt.finalized_at IS NOT NULL
                WHERE execution.action_execution_id=$4
                  AND execution.prepared_action_id=$3
                  AND execution.row_version=$5 AND execution.state='started'"#,
        )
        .bind(request.operation_id)
        .bind(request.campaign_id)
        .bind(request.prepared_action_id)
        .bind(request.execution_id)
        .bind(request.expected_execution_row_version)
        .bind(i32::try_from(request.subexecution_ordinal).map_err(|_| {
            VerificationCampaignRepositoryError::InvalidRequest {
                detail: "subexecution ordinal overflow".to_owned(),
            }
        })?)
        .bind(request.capability_execution_receipt_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "subexecution authority is not current".to_owned(),
        })?;
        let state = match authority.attempt_state.as_str() {
            "succeeded" => "succeeded",
            "failed" | "exhausted" => "failed",
            "outcome_unknown" => "outcome_unknown",
            _ => {
                return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "capability receipt is not terminal".to_owned(),
                })
            }
        };
        let subexecution_id = uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"verification-action-subexecution.v1",
        );
        let replayed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM verification_action_subexecutions WHERE action_subexecution_id=$1)",
        )
        .bind(subexecution_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        golish_db::repo::verification_prepared_actions::record_action_subexecution(
            &self.pool,
            &golish_db::repo::verification_prepared_actions::RecordActionSubexecution {
                action_subexecution_id: subexecution_id,
                action_execution_id: request.execution_id,
                prepared_action_id: request.prepared_action_id,
                group_member_id: authority.group_member_id,
                subexecution_ordinal: i32::try_from(request.subexecution_ordinal).map_err(
                    |_| VerificationCampaignRepositoryError::InvalidRequest {
                        detail: "subexecution ordinal overflow".to_owned(),
                    },
                )?,
                state: state.to_owned(),
                capability_execution_receipt_id: request.capability_execution_receipt_id,
                barrier_released_at: authority.started_at,
                started_at: authority.started_at,
                completed_at: authority.completed_at,
                member_hash: authority.member_hash,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(ActionSubexecutionReceipt {
            subexecution_id,
            execution_id: request.execution_id,
            subexecution_ordinal: request.subexecution_ordinal,
            receipt_hash: authority.receipt_hash,
            replayed,
        })
    }

    async fn closeout_action(&self, request: CloseoutPreparedAction) -> RepoResult<ActionCloseout> {
        #[derive(sqlx::FromRow)]
        struct CloseoutAuthority {
            attempt_state: String,
            receipt_authority_hash: String,
            typed_landing: serde_json::Value,
            existing_state: String,
            budget_reservation_id: uuid::Uuid,
        }
        let authority = sqlx::query_as::<_, CloseoutAuthority>(
            r#"SELECT receipt.attempt_state,receipt.receipt_authority_hash,
                      receipt.typed_landing,execution.state AS existing_state,
                      execution.budget_reservation_id
                 FROM verification_action_executions execution
                 JOIN verification_prepared_actions action
                   ON action.prepared_action_id=execution.prepared_action_id
                  AND action.operation_id=$1 AND action.campaign_id=$2
                 JOIN capability_execution_receipts receipt
                   ON receipt.id=$5 AND receipt.finalized_at IS NOT NULL
                WHERE execution.action_execution_id=$4
                  AND execution.prepared_action_id=$3
                  AND execution.row_version=$6"#,
        )
        .bind(request.operation_id)
        .bind(request.campaign_id)
        .bind(request.prepared_action_id)
        .bind(request.execution_id)
        .bind(request.capability_execution_receipt_id)
        .bind(request.expected_execution_row_version)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "closeout authority is not current".to_owned(),
        })?;
        let (state, disposition) = match authority.attempt_state.as_str() {
            "succeeded" => (
                "succeeded",
                golish_agent_kit::harness::verification_campaign::PreparedActionDisposition::Succeeded,
            ),
            "failed" | "exhausted" => (
                "failed",
                golish_agent_kit::harness::verification_campaign::PreparedActionDisposition::Failed,
            ),
            "outcome_unknown" => (
                "outcome_unknown",
                golish_agent_kit::harness::verification_campaign::PreparedActionDisposition::OutcomeUnknown,
            ),
            _ => {
                return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "capability receipt is not terminal".to_owned(),
                })
            }
        };
        let budget_actuals = sqlx::query_as::<_, (uuid::Uuid, String, i64)>(
            r#"SELECT reserve.ancestor_contract_id,reserve.axis_kind,
                      COALESCE(SUM(consumed.delta),0)::BIGINT AS actual
                 FROM verification_budget_ledger_entries reserve
                 LEFT JOIN verification_budget_ledger_entries consumed
                   ON consumed.budget_reservation_id=reserve.budget_reservation_id
                  AND consumed.ancestor_contract_id=reserve.ancestor_contract_id
                  AND consumed.axis_kind=reserve.axis_kind
                  AND consumed.entry_kind='consume'
                WHERE reserve.budget_reservation_id=$1 AND reserve.entry_kind='reserve'
                GROUP BY reserve.ancestor_contract_id,reserve.axis_kind
                ORDER BY reserve.ancestor_contract_id,reserve.axis_kind"#,
        )
        .bind(authority.budget_reservation_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .into_iter()
        .map(|(ancestor_contract_id, axis_kind, actual)| {
            golish_db::repo::verification_prepared_actions::BudgetActualAxis {
                ancestor_contract_id,
                axis_kind,
                actual,
            }
        })
        .collect::<Vec<_>>();
        let replayed = authority.existing_state != "started";
        let closeout_hash =
            golish_db::repo::verification_prepared_actions::closeout_action_execution(
                &self.pool,
                &golish_db::repo::verification_prepared_actions::CloseoutActionExecution {
                    action_execution_id: request.execution_id,
                    prepared_action_id: request.prepared_action_id,
                    capability_execution_receipt_id: request.capability_execution_receipt_id,
                    state: state.to_owned(),
                    closeout_body: serde_json::json!({
                        "contract_version": "verification-action-closeout.v1",
                        "action_execution_id": request.execution_id,
                        "capability_execution_receipt_id": request.capability_execution_receipt_id,
                        "receipt_authority_hash": authority.receipt_authority_hash,
                        "typed_landing": authority.typed_landing,
                    }),
                    residual_id: None,
                    cleanup_complete: true,
                    budget_actuals,
                },
            )
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let row_version: i64 = sqlx::query_scalar(
            "SELECT row_version FROM verification_action_executions WHERE action_execution_id=$1",
        )
        .bind(request.execution_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(ActionCloseout {
            execution_id: request.execution_id,
            prepared_action_id: request.prepared_action_id,
            terminal_disposition: disposition,
            capability_execution_receipt_id: request.capability_execution_receipt_id,
            closeout_hash,
            row_version,
            replayed,
        })
    }

    async fn recover_unknown_action(
        &self,
        request: RecoverUnknownPreparedAction,
    ) -> RepoResult<ActionRecoveryCloseout> {
        use golish_db::repo::verification_prepared_actions::{
            RecoverUnknownAction, RecoverUnknownActionDisposition,
        };
        let disposition = match request.disposition {
            UnknownActionRecoveryDispositionV1::OutcomeUnknown => {
                RecoverUnknownActionDisposition::OutcomeUnknown
            }
            UnknownActionRecoveryDispositionV1::ReconciledSucceeded => {
                RecoverUnknownActionDisposition::ReconciledSucceeded
            }
            UnknownActionRecoveryDispositionV1::ReconciledFailed => {
                RecoverUnknownActionDisposition::ReconciledFailed
            }
            UnknownActionRecoveryDispositionV1::ManuallyBlocked => {
                RecoverUnknownActionDisposition::ManuallyBlocked
            }
        };
        let receipt = golish_db::repo::verification_prepared_actions::recover_unknown_action(
            &self.pool,
            &RecoverUnknownAction {
                stable_request_id: request.stable_request_id,
                operation_id: request.operation_id,
                campaign_id: request.campaign_id,
                prepared_action_id: request.prepared_action_id,
                action_execution_id: request.execution_id,
                disposition,
                expected_execution_row_version: request.expected_execution_row_version,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(ActionRecoveryCloseout {
            execution_id: request.execution_id,
            disposition: request.disposition,
            recovery_receipt_id: receipt.recovery_receipt_id,
            closeout_hash: receipt.recovery_hash,
            row_version: receipt.execution_row_version,
            replayed: receipt.replayed,
        })
    }

    async fn seal_oracle_census(&self, request: SealOracleCensus) -> RepoResult<OracleCensusSeal> {
        #[derive(sqlx::FromRow)]
        struct CensusAuthority {
            project_scope_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            hypothesis_revision_id: uuid::Uuid,
            verification_contract_hash: String,
            denominator_hash: String,
        }
        let authority = sqlx::query_as::<_, CensusAuthority>(
            r#"SELECT campaign.project_scope_id,campaign.organization_id,
                      campaign.hypothesis_revision_id,
                      campaign.verification_contract_hash,
                      denominator.member_set_hash AS denominator_hash
                 FROM verification_campaigns campaign
                 JOIN verification_campaign_coverage_denominators denominator
                   ON denominator.campaign_denominator_id=$3
                  AND denominator.campaign_id=campaign.campaign_id
                  AND denominator.sealed_at IS NOT NULL
                WHERE campaign.campaign_id=$1 AND campaign.operation_id=$2
                  AND campaign.row_version=$4 AND campaign.state='running'"#,
        )
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(request.coverage_denominator_seal_id)
        .bind(request.expected_campaign_row_version)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "oracle census authority is not current".to_owned(),
        })?;
        #[derive(sqlx::FromRow)]
        struct CensusMemberRow {
            campaign_coverage_member_id: uuid::Uuid,
            predicate_component_id: uuid::Uuid,
            control_binding_kind: String,
            required_control_id: Option<uuid::Uuid>,
            required_control_hash: Option<String>,
            no_control_marker_hash: Option<String>,
            oracle_assessment_id: Option<uuid::Uuid>,
            oracle_assessment_hash: Option<String>,
            existing_residual_id: Option<uuid::Uuid>,
            existing_residual_hash: Option<String>,
            advisory_residual_id: Option<uuid::Uuid>,
            advisory_residual_hash: Option<String>,
        }
        let rows = sqlx::query_as::<_, CensusMemberRow>(
            r#"SELECT member.campaign_coverage_member_id,wave.predicate_component_id,
                      wave.control_binding_kind,wave.required_control_id,
                      wave.required_control_hash,wave.no_control_marker_hash,
                      oracle.oracle_assessment_id,oracle.assessment_hash AS oracle_assessment_hash,
                      assessment.residual_id AS existing_residual_id,
                      residual.residual_hash AS existing_residual_hash,
                      advisory_residual.residual_id AS advisory_residual_id,
                      advisory_residual.residual_hash AS advisory_residual_hash
                 FROM verification_campaign_coverage_members member
                 JOIN verification_wave_coverage_members wave
                   ON wave.wave_coverage_member_id=member.wave_coverage_member_id
                 JOIN verification_capability_assessments assessment
                   ON assessment.assessment_id=wave.capability_assessment_id
                 LEFT JOIN hypothesis_residual_risks residual
                   ON residual.residual_id=assessment.residual_id
                  AND residual.operation_id=$3
                  AND residual.organization_id=$4
                 LEFT JOIN LATERAL (
                     SELECT oracle_assessment_id,assessment_hash
                       FROM verification_oracle_assessments oracle
                      WHERE oracle.campaign_id=$2
                        AND oracle.campaign_coverage_member_id=member.campaign_coverage_member_id
                      ORDER BY oracle.oracle_revision_ordinal DESC LIMIT 1
                 ) oracle ON TRUE
                 LEFT JOIN LATERAL (
                     SELECT compile_residual.residual_id,
                            compile_residual.residual_hash
                       FROM investigation_verification_advisory_campaign_applies apply
                       JOIN investigation_verification_task_advisory_receipts advisory
                         ON advisory.advisory_receipt_id=apply.advisory_receipt_id
                        AND advisory.operation_id=$3
                        AND advisory.organization_id=$4
                        AND advisory.status='applied'
                       JOIN investigation_verification_task_advisory_seals advisory_seal
                         ON advisory_seal.advisory_receipt_id=advisory.advisory_receipt_id
                        AND advisory_seal.verification_task_id=advisory.verification_task_id
                       JOIN hypothesis_residual_risks compile_residual
                         ON compile_residual.residual_id=apply.result_id
                        AND compile_residual.operation_id=advisory.operation_id
                        AND compile_residual.organization_id=advisory.organization_id
                        AND compile_residual.revision_id=advisory.hypothesis_revision_id
                        AND compile_residual.reason_code=
                            'investigation_verification_action_not_compilable'
                        AND compile_residual.owner_kind='plan_c'
                        AND compile_residual.residual_hash=apply.result_sha256
                        AND compile_residual.closed_at IS NULL
                      WHERE apply.campaign_id=$2
                        AND apply.campaign_coverage_member_id=
                            member.campaign_coverage_member_id
                        AND apply.result_kind='residual'
                 ) advisory_residual ON TRUE
                WHERE member.campaign_denominator_id=$1
                ORDER BY member.member_ordinal"#,
        )
        .bind(request.coverage_denominator_seal_id)
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(authority.organization_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if rows.is_empty() {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "oracle census denominator has no exact members".to_owned(),
            });
        }
        let mut members = Vec::with_capacity(rows.len());
        let mut result_hashes = Vec::with_capacity(rows.len());
        for row in rows {
            let (disposition, residual_id, result_hash) = if let (
                Some(_oracle_id),
                Some(oracle_hash),
            ) = (
                row.oracle_assessment_id,
                row.oracle_assessment_hash.as_ref(),
            ) {
                ("assessed", None, oracle_hash.clone())
            } else if row.oracle_assessment_id.is_some() || row.oracle_assessment_hash.is_some() {
                return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "oracle assessment identity/hash pair is incomplete".to_owned(),
                });
            } else if let (Some(residual_id), Some(residual_hash)) = (
                row.existing_residual_id,
                row.existing_residual_hash.as_ref(),
            ) {
                ("blocked", Some(residual_id), residual_hash.clone())
            } else if row.existing_residual_id.is_some() || row.existing_residual_hash.is_some() {
                return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "capability residual identity/hash pair is incomplete".to_owned(),
                });
            } else if let (Some(residual_id), Some(residual_hash)) = (
                row.advisory_residual_id,
                row.advisory_residual_hash.as_ref(),
            ) {
                ("blocked", Some(residual_id), residual_hash.clone())
            } else if row.advisory_residual_id.is_some() || row.advisory_residual_hash.is_some() {
                return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "advisory compile residual identity/hash pair is incomplete".to_owned(),
                });
            } else {
                let residual_id = uuid::Uuid::new_v5(
                    &request.stable_request_id,
                    row.campaign_coverage_member_id.as_bytes(),
                );
                let residual_hash: String =
                    sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                        .bind(serde_json::json!({
                            "campaign_coverage_member_id": row.campaign_coverage_member_id,
                            "reason_code": "oracle_missing",
                        }))
                        .fetch_one(&*self.pool)
                        .await
                        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
                sqlx::query(
                    r#"INSERT INTO hypothesis_residual_risks(
                               residual_id,operation_id,organization_id,revision_id,
                               reason_code,owner_kind,affected_inputs,next_action,residual_hash
                           ) VALUES($1,$2,$3,$4,'oracle_missing','plan_c',$5,$6,$7)
                           ON CONFLICT(residual_id) DO NOTHING"#,
                )
                .bind(residual_id)
                .bind(request.operation_id)
                .bind(authority.organization_id)
                .bind(authority.hypothesis_revision_id)
                .bind(serde_json::json!([row.campaign_coverage_member_id]))
                .bind(serde_json::json!({"kind": "oracle_assessment_required"}))
                .bind(&residual_hash)
                .execute(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
                let persisted_hash: Option<String> = sqlx::query_scalar(
                    r#"SELECT residual_hash FROM hypothesis_residual_risks
                            WHERE residual_id=$1 AND operation_id=$2
                              AND organization_id=$3 AND revision_id=$4
                              AND reason_code='oracle_missing'"#,
                )
                .bind(residual_id)
                .bind(request.operation_id)
                .bind(authority.organization_id)
                .bind(authority.hypothesis_revision_id)
                .fetch_optional(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
                if persisted_hash.as_deref() != Some(residual_hash.as_str()) {
                    return Err(VerificationCampaignRepositoryError::Conflict {
                        detail: "oracle residual replay identity drift".to_owned(),
                    });
                }
                ("untested", Some(residual_id), residual_hash)
            };
            result_hashes.push(result_hash);
            members.push(golish_db::repo::verification_oracles::OracleCensusMember {
                campaign_coverage_member_id: row.campaign_coverage_member_id,
                predicate_component_id: row.predicate_component_id,
                control_binding_kind: row.control_binding_kind,
                required_control_id: row.required_control_id,
                required_control_hash: row.required_control_hash,
                no_control_marker_hash: row.no_control_marker_hash,
                disposition: disposition.to_owned(),
                oracle_assessment_id: row.oracle_assessment_id,
                residual_id,
            });
        }
        let result_set_hash: String = sqlx::query_scalar(
            "SELECT investigation_exact_member_set_hash('verification_oracle_result_set.v1',$1::TEXT[])",
        )
        .bind(&result_hashes)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let replayed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM verification_oracle_census_seals WHERE stable_request_id=$1)",
        )
        .bind(request.stable_request_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let seal_id = golish_db::repo::verification_oracles::seal_oracle_census(
            &self.pool,
            &golish_db::repo::verification_oracles::SealOracleCensus {
                stable_request_id: request.stable_request_id,
                campaign_id: request.campaign_id,
                campaign_denominator_id: request.coverage_denominator_seal_id,
                operation_id: request.operation_id,
                project_scope_id: authority.project_scope_id,
                organization_id: authority.organization_id,
                verification_contract_hash: authority.verification_contract_hash,
                denominator_hash: authority.denominator_hash,
                result_set_hash,
                members,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let (member_count, member_set_hash): (i64, String) = sqlx::query_as(
            "SELECT member_count,member_set_hash FROM verification_oracle_census_seals WHERE oracle_census_seal_id=$1",
        )
        .bind(seal_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(OracleCensusSeal {
            seal_id,
            campaign_id: request.campaign_id,
            member_count: u32::try_from(member_count).map_err(|_| {
                VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "oracle member count overflow".to_owned(),
                }
            })?,
            member_set_hash,
            replayed,
        })
    }

    async fn close_campaign_objective(
        &self,
        request: CloseCampaignObjective,
    ) -> RepoResult<ObjectiveOutcomeReceipt> {
        #[derive(sqlx::FromRow)]
        struct ObjectiveAuthority {
            project_scope_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            hypothesis_revision_id: uuid::Uuid,
            verification_plan_id: uuid::Uuid,
            verification_contract_hash: String,
            census_hash: String,
            result_set_hash: String,
        }
        let authority = sqlx::query_as::<_, ObjectiveAuthority>(
            r#"SELECT campaign.project_scope_id,campaign.organization_id,
                      campaign.hypothesis_revision_id,campaign.verification_plan_id,
                      campaign.verification_contract_hash,census.census_hash,
                      census.result_set_hash
                 FROM verification_campaigns campaign
                 JOIN verification_oracle_census_seals census
                   ON census.oracle_census_seal_id=$4
                  AND census.campaign_id=campaign.campaign_id
                  AND census.campaign_denominator_id=$5 AND census.sealed_at IS NOT NULL
                WHERE campaign.campaign_id=$1 AND campaign.operation_id=$2
                  AND campaign.verification_objective_id=$3
                  AND campaign.row_version=$6 AND campaign.state='running'"#,
        )
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(request.objective_id)
        .bind(request.oracle_census_seal_id)
        .bind(request.coverage_denominator_seal_id)
        .bind(request.expected_campaign_row_version)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "objective closeout authority is not current".to_owned(),
        })?;
        #[derive(sqlx::FromRow)]
        struct OutcomeSource {
            oracle_census_member_id: uuid::Uuid,
            campaign_coverage_member_id: uuid::Uuid,
            claim_component_id: uuid::Uuid,
            claim_component_hash: String,
            predicate_component_id: uuid::Uuid,
            control_binding_kind: String,
            disposition: String,
            oracle_assessment_id: Option<uuid::Uuid>,
            verdict: Option<String>,
            control_validity: Option<String>,
            prepared_action_id: Option<uuid::Uuid>,
            capability_execution_receipt_id: Option<uuid::Uuid>,
            residual_id: Option<uuid::Uuid>,
        }
        let sources = sqlx::query_as::<_, OutcomeSource>(
            r#"SELECT census_member.oracle_census_member_id,
                      census_member.campaign_coverage_member_id,
                      wave.claim_component_id,wave.claim_component_hash,
                      wave.predicate_component_id,wave.control_binding_kind,
                      census_member.disposition,census_member.oracle_assessment_id,
                      oracle.verdict,oracle.control_validity,oracle.prepared_action_id,
                      execution.capability_execution_receipt_id,
                      COALESCE(census_member.residual_id,oracle.residual_id) AS residual_id
                 FROM verification_oracle_census_members census_member
                 JOIN verification_campaign_coverage_members coverage
                   ON coverage.campaign_coverage_member_id=census_member.campaign_coverage_member_id
                 JOIN verification_wave_coverage_members wave
                   ON wave.wave_coverage_member_id=coverage.wave_coverage_member_id
                 LEFT JOIN verification_oracle_assessments oracle
                   ON oracle.oracle_assessment_id=census_member.oracle_assessment_id
                 LEFT JOIN verification_action_executions execution
                   ON execution.action_execution_id=oracle.action_execution_id
                WHERE census_member.oracle_census_seal_id=$1
                ORDER BY census_member.member_ordinal"#,
        )
        .bind(request.oracle_census_seal_id)
        .fetch_all(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let mut by_component = std::collections::BTreeMap::<uuid::Uuid, Vec<&OutcomeSource>>::new();
        for source in &sources {
            by_component
                .entry(source.claim_component_id)
                .or_default()
                .push(source);
        }
        let mut component_members = Vec::with_capacity(by_component.len());
        let mut component_outcomes = Vec::with_capacity(by_component.len());
        for component_sources in by_component.values() {
            let representative = component_sources[0];
            let proof = component_sources
                .iter()
                .find(|source| source.verdict.as_deref() == Some("proof"));
            let all_refutation = component_sources.iter().all(|source| {
                source.disposition == "assessed" && source.verdict.as_deref() == Some("refutation")
            });
            let selected = proof.copied().unwrap_or(representative);
            let (component_outcome, residual_id) = if proof.is_some() {
                ("proof", None)
            } else if all_refutation {
                ("refutation", None)
            } else {
                (
                    if component_sources
                        .iter()
                        .any(|source| source.disposition == "blocked")
                    {
                        "blocked"
                    } else {
                        "inconclusive"
                    },
                    component_sources
                        .iter()
                        .find_map(|source| source.residual_id),
                )
            };
            let residual_id = if component_outcome == "proof" || component_outcome == "refutation" {
                None
            } else if residual_id.is_some() {
                residual_id
            } else {
                let residual_id = uuid::Uuid::new_v5(
                    &request.stable_request_id,
                    representative.claim_component_id.as_bytes(),
                );
                let affected_inputs = serde_json::json!([representative.claim_component_id]);
                let next_action = serde_json::json!({"kind": "claim_component_reassessment"});
                let residual_hash: String =
                    sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                        .bind(serde_json::json!({
                            "claim_component_id": representative.claim_component_id,
                            "reason_code": "component_inconclusive",
                        }))
                        .fetch_one(&*self.pool)
                        .await
                        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
                sqlx::query(
                    r#"INSERT INTO hypothesis_residual_risks(
                           residual_id,operation_id,organization_id,revision_id,reason_code,
                           owner_kind,affected_inputs,next_action,residual_hash
                       ) VALUES($1,$2,$3,$4,'component_inconclusive','plan_c',$5,$6,$7)
                       ON CONFLICT(residual_id) DO NOTHING"#,
                )
                .bind(residual_id)
                .bind(request.operation_id)
                .bind(authority.organization_id)
                .bind(authority.hypothesis_revision_id)
                .bind(affected_inputs)
                .bind(next_action)
                .bind(residual_hash)
                .execute(&*self.pool)
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
                Some(residual_id)
            };
            component_outcomes.push(component_outcome);
            component_members.push(
                golish_db::repo::verification_fact_delta_bundles::ClaimComponentOutcomeMember {
                    claim_component_id: representative.claim_component_id,
                    hypothesis_revision_id: authority.hypothesis_revision_id,
                    claim_component_hash: representative.claim_component_hash.clone(),
                    predicate_component_id: selected.predicate_component_id,
                    oracle_census_member_id: if residual_id.is_none() {
                        Some(selected.oracle_census_member_id)
                    } else {
                        None
                    },
                    campaign_coverage_member_id: if residual_id.is_none() {
                        Some(selected.campaign_coverage_member_id)
                    } else {
                        None
                    },
                    component_outcome: component_outcome.to_owned(),
                    residual_id,
                },
            );
        }
        let component_seal_request = uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"verification-claim-component-outcomes.v1",
        );
        let claim_component_outcome_seal_id =
            if let Some(id) = sqlx::query_scalar::<_, uuid::Uuid>(
                "SELECT claim_component_outcome_seal_id FROM hypothesis_objective_claim_component_outcome_seals WHERE stable_request_id=$1",
            )
            .bind(component_seal_request)
            .fetch_optional(&*self.pool)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
            {
                id
            } else {
                golish_db::repo::verification_fact_delta_bundles::seal_objective_claim_component_outcomes(
                    &self.pool,
                    &golish_db::repo::verification_fact_delta_bundles::SealObjectiveClaimComponentOutcomes {
                        stable_request_id: component_seal_request,
                        verification_plan_id: authority.verification_plan_id,
                        hypothesis_revision_id: authority.hypothesis_revision_id,
                        verification_objective_id: request.objective_id,
                        campaign_id: Some(request.campaign_id),
                        members: component_members.clone(),
                    },
                )
                .await
                .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
            };
        let all_proof = component_outcomes.iter().all(|outcome| *outcome == "proof");
        let all_terminal = component_outcomes
            .iter()
            .all(|outcome| matches!(*outcome, "proof" | "refutation"));
        let any_refutation = component_outcomes.contains(&"refutation");
        let outcome = if all_proof {
            "proof"
        } else if all_terminal && any_refutation {
            "refutation"
        } else if component_outcomes.contains(&"blocked") {
            "blocked"
        } else {
            "inconclusive"
        };
        let aggregate_residual_id = if matches!(outcome, "proof" | "refutation") {
            None
        } else {
            sources
                .iter()
                .find_map(|source| source.residual_id)
                .or_else(|| {
                    component_members
                        .iter()
                        .find_map(|member| member.residual_id)
                })
        };
        let unresolved_member_set_hash = if matches!(outcome, "proof" | "refutation") {
            None
        } else {
            Some(authority.result_set_hash.clone())
        };
        let mut coverage_results = Vec::with_capacity(sources.len());
        for source in &sources {
            let assessed = source.disposition == "assessed";
            let Some(epistemic_outcome) =
                campaign_coverage_epistemic_outcome(&source.disposition, source.verdict.as_deref())
            else {
                return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "oracle census disposition/verdict shape is invalid".to_owned(),
                });
            };
            coverage_results.push(
                golish_db::repo::verification_fact_delta_bundles::CampaignCoverageResult {
                    campaign_coverage_member_id: source.campaign_coverage_member_id,
                    coverage_disposition: if assessed {
                        if source.verdict.as_deref() == Some("inconclusive") {
                            "tested_degraded"
                        } else {
                            "tested_complete"
                        }
                    } else if source.disposition == "blocked" {
                        "blocked"
                    } else {
                        "untested"
                    }
                    .to_owned(),
                    epistemic_outcome: epistemic_outcome.to_owned(),
                    control_binding_kind: source.control_binding_kind.clone(),
                    control_validity: source.control_validity.clone().unwrap_or_else(|| {
                        if source.control_binding_kind == "explicit_no_control" {
                            "not_required".to_owned()
                        } else {
                            "not_assessed".to_owned()
                        }
                    }),
                    prepared_action_id: source.prepared_action_id,
                    capability_execution_receipt_id: source.capability_execution_receipt_id,
                    oracle_assessment_id: source.oracle_assessment_id,
                    residual_id: source.residual_id,
                },
            );
        }
        let predecessor: Option<(uuid::Uuid, i64)> = sqlx::query_as(
            r#"SELECT current_outcome_id,current_ordinal
                 FROM hypothesis_objective_outcome_heads
                WHERE verification_plan_id=$1 AND verification_objective_id=$2"#,
        )
        .bind(authority.verification_plan_id)
        .bind(request.objective_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let replayed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM hypothesis_objective_outcome_receipts WHERE stable_request_id=$1)",
        )
        .bind(uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"objective-outcome-request.v1",
        ))
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let closeout = golish_db::repo::verification_fact_delta_bundles::close_campaign_objective_with_fact_delta(
            &self.pool,
            &golish_db::repo::verification_fact_delta_bundles::CloseCampaignObjective {
                stable_request_id: request.stable_request_id,
                campaign_id: request.campaign_id,
                operation_id: request.operation_id,
                project_scope_id: authority.project_scope_id,
                organization_id: authority.organization_id,
                hypothesis_revision_id: authority.hypothesis_revision_id,
                verification_plan_id: authority.verification_plan_id,
                verification_objective_id: request.objective_id,
                verification_contract_hash: authority.verification_contract_hash,
                expected_campaign_row_version: request.expected_campaign_row_version,
                oracle_census_seal_id: request.oracle_census_seal_id,
                campaign_denominator_id: request.coverage_denominator_seal_id,
                claim_component_outcome_seal_id,
                outcome: outcome.to_owned(),
                unresolved_member_set_hash,
                residual_id: aggregate_residual_id,
                coverage_results,
                fact_delta_kind: match outcome {
                    "proof" => "support",
                    "refutation" => "contradiction",
                    _ => "inconclusive",
                }
                .to_owned(),
                typed_fact_delta: serde_json::json!({
                    "contract_version": "verification-fact-delta.v1",
                    "objective_id": request.objective_id,
                    "outcome": outcome,
                }),
                evidence_ref_set_hash: authority.result_set_hash,
                source_authority_hash: authority.census_hash,
                expected_predecessor_outcome_id: predecessor.map(|item| item.0),
                expected_outcome_ordinal: predecessor.map_or(1, |item| item.1 + 1),
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let outcome_hash: String = sqlx::query_scalar(
            r#"SELECT outcome_hash FROM hypothesis_objective_outcome_receipts
                WHERE objective_outcome_receipt_id=$1"#,
        )
        .bind(closeout.objective_outcome_receipt_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(ObjectiveOutcomeReceipt {
            outcome_receipt_id: closeout.objective_outcome_receipt_id,
            campaign_id: request.campaign_id,
            objective_id: request.objective_id,
            outcome: match outcome {
                "proof" => golish_agent_kit::harness::verification_campaign::ObjectiveCampaignOutcome::Proof,
                "refutation" => golish_agent_kit::harness::verification_campaign::ObjectiveCampaignOutcome::Refutation,
                "blocked" => golish_agent_kit::harness::verification_campaign::ObjectiveCampaignOutcome::Blocked,
                _ => golish_agent_kit::harness::verification_campaign::ObjectiveCampaignOutcome::Inconclusive,
            },
            fact_delta_bundle_id: closeout.fact_delta_bundle_id,
            outcome_hash,
            replayed,
        })
    }

    async fn adjudicate_hypothesis_revision_with_fresh_tool_truth(
        &self,
        request: AdjudicateHypothesisRevision,
    ) -> RepoResult<HypothesisRevisionAdjudicationReceipt> {
        let receipt = golish_db::repo::hypothesis_revision_adjudications::adjudicate_revision_from_current_authority(
            &self.pool,
            golish_db::repo::hypothesis_revision_adjudications::AdjudicateRevisionFromAuthority {
                stable_consumer_request_id: request.stable_consumer_request_id,
                operation_id: request.operation_id,
                scope_snapshot_id: request.scope_snapshot_id,
                organization_id: request.organization_id,
                generation_seal_id: request.generation_seal_id,
                hypothesis_revision_id: request.hypothesis_revision_id,
                verification_plan_id: request.verification_plan_id,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let outcome = match receipt.outcome.as_str() {
            "verified" => golish_core::hypothesis_verification::HypothesisRevisionAdjudicationVerdictV1::Verified,
            "refuted" => golish_core::hypothesis_verification::HypothesisRevisionAdjudicationVerdictV1::Refuted,
            "nonterminal" => golish_core::hypothesis_verification::HypothesisRevisionAdjudicationVerdictV1::NonTerminal,
            _ => {
                return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                    detail: "unknown revision adjudication outcome".to_owned(),
                })
            }
        };
        Ok(HypothesisRevisionAdjudicationReceipt {
            adjudication_receipt_id: receipt.revision_adjudication_id,
            hypothesis_revision_id: request.hypothesis_revision_id,
            outcome,
            objective_outcome_set_seal_id: receipt.objective_outcome_set_seal_id,
            authority_bundle_seal_id: receipt.tool_truth_authority_bundle_seal_id,
            adjudication_hash: receipt.adjudication_hash,
            replayed: receipt.replayed,
        })
    }

    async fn quarantine_campaign_authority(
        &self,
        request: QuarantineCampaignAuthority,
    ) -> RepoResult<AuthorityQuarantineReceipt> {
        if let Some(existing) = sqlx::query_as::<_, (uuid::Uuid, i64, String, i64)>(
            r#"SELECT event.quarantine_event_id,event.member_count,event.member_set_hash,
                      campaign.row_version
                 FROM verification_authority_quarantine_events event
                 JOIN verification_campaign_terminal_decisions terminal
                   ON terminal.campaign_terminal_decision_id=event.campaign_terminal_decision_id
                 JOIN verification_campaigns campaign
                   ON campaign.campaign_id=terminal.campaign_id
                WHERE event.stable_request_id=$1 AND event.operation_id=$2
                  AND campaign.campaign_id=$3"#,
        )
        .bind(request.stable_request_id)
        .bind(request.operation_id)
        .bind(request.campaign_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        {
            return Ok(AuthorityQuarantineReceipt {
                quarantine_receipt_id: existing.0,
                campaign_id: request.campaign_id,
                member_count: existing.1 as u32,
                member_set_hash: existing.2,
                row_version: existing.3,
                replayed: true,
            });
        }
        #[derive(sqlx::FromRow)]
        struct QuarantineAuthority {
            project_scope_id: uuid::Uuid,
            organization_id: uuid::Uuid,
            campaign_terminal_decision_id: uuid::Uuid,
            terminal_hash: String,
            objective_outcome_receipt_id: uuid::Uuid,
            outcome_hash: String,
            campaign_coverage_receipt_id: uuid::Uuid,
            coverage_hash: String,
            oracle_census_seal_id: uuid::Uuid,
            census_hash: String,
            fact_delta_bundle_id: uuid::Uuid,
            fact_delta_hash: String,
            invalid_reconciliation_id: uuid::Uuid,
            invalid_reconciliation_hash: String,
        }
        let authority = sqlx::query_as::<_, QuarantineAuthority>(
            r#"SELECT campaign.project_scope_id,campaign.organization_id,
                      terminal.campaign_terminal_decision_id,terminal.terminal_hash,
                      outcome.objective_outcome_receipt_id,outcome.outcome_hash,
                      coverage.campaign_coverage_receipt_id,
                      coverage.receipt_hash AS coverage_hash,
                      census.oracle_census_seal_id,census.census_hash,
                      delta.fact_delta_bundle_id,delta.fact_delta_hash,
                      reconciliation.id AS invalid_reconciliation_id,
                      reconciliation.semantic_reconciliation_hash AS invalid_reconciliation_hash
                 FROM verification_campaigns campaign
                 JOIN verification_campaign_terminal_decisions terminal
                   ON terminal.campaign_id=campaign.campaign_id
                 JOIN verification_campaign_coverage_receipts coverage
                   ON coverage.campaign_id=campaign.campaign_id
                  AND coverage.campaign_terminal_decision_id=terminal.campaign_terminal_decision_id
                 JOIN hypothesis_objective_outcome_receipts outcome
                   ON outcome.campaign_terminal_decision_id=terminal.campaign_terminal_decision_id
                  AND outcome.campaign_coverage_receipt_id=coverage.campaign_coverage_receipt_id
                 JOIN verification_oracle_census_seals census
                   ON census.oracle_census_seal_id=outcome.oracle_census_seal_id
                 JOIN verification_fact_delta_bundles delta
                   ON delta.fact_delta_bundle_id=outcome.fact_delta_bundle_id
                  AND delta.campaign_id=campaign.campaign_id
                 JOIN capability_execution_reconciliations reconciliation
                   ON reconciliation.receipt_id=$4
                  AND reconciliation.reconciliation_state IN ('orphaned','superseded')
                  AND reconciliation.sealed_at IS NOT NULL
                WHERE campaign.campaign_id=$1 AND campaign.operation_id=$2
                  AND campaign.row_version=$3 AND campaign.state='terminal'
                  AND EXISTS(
                      SELECT 1 FROM verification_campaign_coverage_results result
                       WHERE result.campaign_coverage_receipt_id=coverage.campaign_coverage_receipt_id
                         AND result.capability_execution_receipt_id=$4
                  )
                ORDER BY reconciliation.semantic_authority_version DESC
                LIMIT 1"#,
        )
        .bind(request.campaign_id)
        .bind(request.operation_id)
        .bind(request.expected_campaign_row_version)
        .bind(request.source_receipt_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "campaign quarantine authority is not current or the source receipt is not invalid"
                .to_owned(),
        })?;
        let reason_code = match request.reason {
            CampaignAuthorityQuarantineReasonV1::SemanticAuthorityChanged => {
                "semantic_authority_changed"
            }
            CampaignAuthorityQuarantineReasonV1::TemporalAuthorityExpired => {
                "temporal_authority_expired"
            }
            CampaignAuthorityQuarantineReasonV1::TargetStateChanged => "target_state_changed",
            CampaignAuthorityQuarantineReasonV1::ProjectionDiverged => "projection_diverged",
            CampaignAuthorityQuarantineReasonV1::ManualSafetyHold => "manual_safety_hold",
        };
        let members = vec![
            golish_db::repo::hypothesis_consolidations::QuarantineAuthorityMember {
                authority_ref_kind: "campaign_terminal".to_owned(),
                authority_ref_id: authority.campaign_terminal_decision_id,
                authority_ref_hash: authority.terminal_hash,
            },
            golish_db::repo::hypothesis_consolidations::QuarantineAuthorityMember {
                authority_ref_kind: "objective_outcome".to_owned(),
                authority_ref_id: authority.objective_outcome_receipt_id,
                authority_ref_hash: authority.outcome_hash,
            },
            golish_db::repo::hypothesis_consolidations::QuarantineAuthorityMember {
                authority_ref_kind: "campaign_coverage".to_owned(),
                authority_ref_id: authority.campaign_coverage_receipt_id,
                authority_ref_hash: authority.coverage_hash,
            },
            golish_db::repo::hypothesis_consolidations::QuarantineAuthorityMember {
                authority_ref_kind: "oracle_census".to_owned(),
                authority_ref_id: authority.oracle_census_seal_id,
                authority_ref_hash: authority.census_hash,
            },
            golish_db::repo::hypothesis_consolidations::QuarantineAuthorityMember {
                authority_ref_kind: "fact_delta".to_owned(),
                authority_ref_id: authority.fact_delta_bundle_id,
                authority_ref_hash: authority.fact_delta_hash,
            },
        ];
        let receipt = golish_db::repo::hypothesis_consolidations::quarantine_campaign_authority(
            &self.pool,
            &golish_db::repo::hypothesis_consolidations::QuarantineCampaignAuthority {
                stable_request_id: request.stable_request_id,
                operation_id: request.operation_id,
                project_scope_id: authority.project_scope_id,
                organization_id: authority.organization_id,
                campaign_terminal_decision_id: authority.campaign_terminal_decision_id,
                objective_outcome_receipt_id: authority.objective_outcome_receipt_id,
                campaign_coverage_receipt_id: authority.campaign_coverage_receipt_id,
                oracle_census_seal_id: authority.oracle_census_seal_id,
                fact_delta_bundle_id: authority.fact_delta_bundle_id,
                invalid_semantic_reconciliation_id: authority.invalid_reconciliation_id,
                invalid_semantic_reconciliation_hash: authority.invalid_reconciliation_hash,
                residual_reason_code: reason_code.to_owned(),
                members,
                typed_correction_delta: serde_json::json!({
                    "contract_version": "verification-authority-correction.v1",
                    "kind": "retract_fact_delta",
                    "fact_delta_bundle_id": authority.fact_delta_bundle_id,
                    "reason_code": reason_code,
                    "source_receipt_id": request.source_receipt_id,
                }),
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let (member_count, member_set_hash): (i64, String) = sqlx::query_as(
            r#"SELECT member_count,member_set_hash
                 FROM verification_authority_quarantine_events
                WHERE quarantine_event_id=$1"#,
        )
        .bind(receipt.quarantine_event_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(AuthorityQuarantineReceipt {
            quarantine_receipt_id: receipt.quarantine_event_id,
            campaign_id: request.campaign_id,
            member_count: member_count as u32,
            member_set_hash,
            row_version: request.expected_campaign_row_version,
            replayed: false,
        })
    }
}

#[async_trait]
impl VerificationCampaignShadowRepository for PgVerificationCampaignRepository {
    async fn open_evaluation(&self, request: OpenShadowEvaluation) -> RepoResult<ShadowEvaluation> {
        if let Some(existing) = sqlx::query_as::<_, (uuid::Uuid, i64, String, i64)>(
            r#"SELECT shadow_evaluation_id,obligation_member_count,
                      obligation_member_set_hash,row_version
                 FROM verification_campaign_shadow_evaluations
                WHERE stable_request_id=$1 AND operation_id=$2
                  AND organization_id=$3"#,
        )
        .bind(request.stable_request_id)
        .bind(request.operation_id)
        .bind(request.organization_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        {
            return Ok(ShadowEvaluation {
                evaluation_id: existing.0,
                operation_id: request.operation_id,
                frozen_snapshot_id: request.frozen_snapshot_id,
                item_count: existing.1 as u32,
                item_set_hash: existing.2,
                row_version: existing.3,
                replayed: true,
            });
        }
        #[derive(sqlx::FromRow)]
        struct ObligationRow {
            plan_objective_id: uuid::Uuid,
            ordinal: i32,
            member_hash: String,
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let authority: (uuid::Uuid, i64) = sqlx::query_as(
            r#"SELECT operation.project_scope_id,projection.change_seq
                 FROM operation_state operation
                 JOIN operation_org_scope_snapshots scope
                   ON scope.id=$2 AND scope.operation_id=operation.operation_id
                  AND scope.project_scope_id=operation.project_scope_id
                  AND scope.sealed_at IS NOT NULL
                 JOIN operation_org_scope_units unit
                   ON unit.snapshot_id=scope.id AND unit.organization_id=$3
                 JOIN attack_hypothesis_revisions revision
                   ON revision.revision_id=$4 AND revision.operation_id=operation.operation_id
                  AND revision.organization_id=$3
                 JOIN attack_hypothesis_verification_plans plan
                   ON plan.plan_id=$5 AND plan.revision_id=revision.revision_id
                  AND plan.sealed_at IS NOT NULL
                 JOIN investigation_projection_heads projection
                   ON projection.operation_id=operation.operation_id
                WHERE operation.operation_id=$1
                  AND operation.investigation_rollout_mode IN ('shadow_registry','dual_read_compare')
                  AND operation.tool_truth_contract IN ('shadow_v1','receipt_v1')
                FOR SHARE OF operation,scope,revision,plan,projection"#,
        )
        .bind(request.operation_id)
        .bind(request.scope_snapshot_id)
        .bind(request.organization_id)
        .bind(request.hypothesis_revision_id)
        .bind(request.verification_plan_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "shadow evaluation authority is not frozen in a compare-enabled mode"
                .to_owned(),
        })?;
        let obligations = sqlx::query_as::<_, ObligationRow>(
            r#"SELECT plan_objective_id,ordinal,
                      tool_truth_sha256(jsonb_build_object(
                          'contract_version','verification-shadow-obligation.v1',
                          'plan_objective_id',plan_objective_id,
                          'plan_objective_member_hash',member_hash,
                          'frozen_snapshot_id',$2,
                          'frozen_snapshot_hash',$3,
                          'obligation_census_hash',$4
                      )) AS member_hash
                 FROM attack_hypothesis_verification_plan_objectives
                WHERE plan_id=$1 ORDER BY ordinal"#,
        )
        .bind(request.verification_plan_id)
        .bind(request.frozen_snapshot_id)
        .bind(&request.frozen_snapshot_hash)
        .bind(&request.obligation_census_hash)
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if obligations.is_empty() {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "shadow evaluation has no sealed plan obligations".to_owned(),
            });
        }
        let hashes = obligations
            .iter()
            .map(|item| item.member_hash.clone())
            .collect::<Vec<_>>();
        let item_set_hash: String = sqlx::query_scalar(
            "SELECT investigation_exact_member_set_hash('verification_campaign_shadow_evaluation.v1',$1::TEXT[])",
        )
        .bind(&hashes)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let evaluation_hash: String = sqlx::query_scalar(
            r#"SELECT tool_truth_sha256(jsonb_build_object(
                   'operation_id',$1,'hypothesis_revision_id',$2,
                   'verification_plan_id',$3,'frozen_snapshot_id',$4,
                   'frozen_snapshot_hash',$5,'obligation_census_hash',$6,
                   'obligation_member_set_hash',$7,'as_of_change_seq',$8
               ))"#,
        )
        .bind(request.operation_id)
        .bind(request.hypothesis_revision_id)
        .bind(request.verification_plan_id)
        .bind(request.frozen_snapshot_id)
        .bind(&request.frozen_snapshot_hash)
        .bind(&request.obligation_census_hash)
        .bind(&item_set_hash)
        .bind(authority.1)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let evaluation_id = uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"verification-campaign-shadow-evaluation.v1",
        );
        sqlx::query(
            r#"INSERT INTO verification_campaign_shadow_evaluations(
                   shadow_evaluation_id,stable_request_id,operation_id,project_scope_id,
                   organization_id,hypothesis_revision_id,verification_plan_id,
                   frozen_snapshot_id,frozen_snapshot_hash,obligation_census_hash,
                   as_of_change_seq,source_snapshot_hash,obligation_member_count,
                   obligation_member_set_hash,evaluation_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$9,$12,$13,$14)"#,
        )
        .bind(evaluation_id)
        .bind(request.stable_request_id)
        .bind(request.operation_id)
        .bind(authority.0)
        .bind(request.organization_id)
        .bind(request.hypothesis_revision_id)
        .bind(request.verification_plan_id)
        .bind(request.frozen_snapshot_id)
        .bind(&request.frozen_snapshot_hash)
        .bind(&request.obligation_census_hash)
        .bind(authority.1)
        .bind(obligations.len() as i64)
        .bind(&item_set_hash)
        .bind(evaluation_hash)
        .execute(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        for obligation in &obligations {
            let obligation_id =
                uuid::Uuid::new_v5(&evaluation_id, obligation.plan_objective_id.as_bytes());
            let plan_member_hash: String = sqlx::query_scalar(
                "SELECT member_hash FROM attack_hypothesis_verification_plan_objectives WHERE plan_objective_id=$1",
            )
            .bind(obligation.plan_objective_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
            sqlx::query(
                r#"INSERT INTO verification_campaign_shadow_evaluation_obligations(
                       shadow_evaluation_obligation_id,shadow_evaluation_id,operation_id,
                       project_scope_id,organization_id,obligation_ordinal,
                       plan_objective_id,plan_objective_member_hash,frozen_target_hash,member_hash
                   ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
            )
            .bind(obligation_id)
            .bind(evaluation_id)
            .bind(request.operation_id)
            .bind(authority.0)
            .bind(request.organization_id)
            .bind(obligation.ordinal)
            .bind(obligation.plan_objective_id)
            .bind(plan_member_hash)
            .bind(&request.frozen_snapshot_hash)
            .bind(&obligation.member_hash)
            .execute(&mut *tx)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        }
        tx.commit()
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(ShadowEvaluation {
            evaluation_id,
            operation_id: request.operation_id,
            frozen_snapshot_id: request.frozen_snapshot_id,
            item_count: obligations.len() as u32,
            item_set_hash,
            row_version: 0,
            replayed: false,
        })
    }

    async fn record_receipt_replay_and_compare_v1(
        &self,
        request: RecordShadowReceiptReplay,
    ) -> RepoResult<ComparisonId> {
        #[derive(sqlx::FromRow)]
        struct ReplayAuthority {
            organization_id: uuid::Uuid,
            project_scope_id: uuid::Uuid,
            plan_objective_id: uuid::Uuid,
            obligation_ordinal: i32,
            frozen_target_hash: String,
            as_of_change_seq: i64,
        }
        let authority = sqlx::query_as::<_, ReplayAuthority>(
            r#"SELECT evaluation.organization_id,evaluation.project_scope_id,
                      obligation.plan_objective_id,obligation.obligation_ordinal,
                      obligation.frozen_target_hash,evaluation.as_of_change_seq
                 FROM verification_campaign_shadow_evaluations evaluation
                 JOIN verification_campaign_shadow_evaluation_obligations obligation
                   ON obligation.shadow_evaluation_id=evaluation.shadow_evaluation_id
                  AND obligation.shadow_evaluation_obligation_id=$3
                WHERE evaluation.shadow_evaluation_id=$1
                  AND evaluation.operation_id=$2 AND evaluation.state='open'"#,
        )
        .bind(request.evaluation_id)
        .bind(request.operation_id)
        .bind(request.evaluation_item_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::AuthorityMismatch {
            detail: "shadow replay obligation is not open/current".to_owned(),
        })?;
        if authority.frozen_target_hash != request.frozen_target_hash
            || request.adapter_contract_version.trim().is_empty()
            || request.oracle_contract_version.trim().is_empty()
            || !request.compiled_semantic_signature.starts_with("sha256:")
        {
            return Err(VerificationCampaignRepositoryError::InvalidRequest {
                detail: "shadow replay semantic signature/target/version is invalid".to_owned(),
            });
        }
        let receipt_valid: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM capability_execution_receipts receipt
                    WHERE receipt.id=$1 AND receipt.finalized_at IS NOT NULL
                      AND receipt.reconciliation_state='consistent'
                      AND receipt.current_semantic_reconciliation_id IS NOT NULL
               )"#,
        )
        .bind(request.legacy_capability_receipt_id)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        // The shadow port intentionally lacks enough fields to fabricate a
        // complete canonical comparison record.  A validated legacy receipt
        // is therefore recorded honestly as an incomplete whole-record
        // sample until both independently frozen complete records exist.
        let sample = golish_db::repo::investigation_projection::compare_and_record_v1(
            &self.pool,
            golish_db::repo::investigation_projection::CompareAndRecordV1Input {
                operation_id: request.operation_id,
                organization_id: Some(authority.organization_id),
                as_of_change_seq: authority.as_of_change_seq,
                record_kind: "verification_campaign_shadow_replay".to_owned(),
                record_key: format!(
                    "{}:{}:{}:{}:{}",
                    request.evaluation_id,
                    request.evaluation_item_id,
                    request.compiled_semantic_signature,
                    request.adapter_contract_version,
                    request.oracle_contract_version
                ),
                legacy: None,
                registry: None,
            },
        )
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if !receipt_valid && sample.comparison_state != "incomplete" {
            return Err(VerificationCampaignRepositoryError::AuthorityMismatch {
                detail: "invalid legacy receipt cannot yield a complete shadow comparison"
                    .to_owned(),
            });
        }
        let member_hash: String = sqlx::query_scalar(
            r#"SELECT tool_truth_sha256(jsonb_build_object(
                   'contract_version','verification-shadow-replay.v1',
                   'evaluation_id',$1,'obligation_id',$2,'comparison_id',$3,
                   'compiled_semantic_signature_hash',$4,
                   'legacy_capability_execution_receipt_id',$5,
                   'adapter_contract_version',$6,'oracle_contract_version',$7
               ))"#,
        )
        .bind(request.evaluation_id)
        .bind(request.evaluation_item_id)
        .bind(sample.comparison_id)
        .bind(&request.compiled_semantic_signature)
        .bind(request.legacy_capability_receipt_id)
        .bind(&request.adapter_contract_version)
        .bind(&request.oracle_contract_version)
        .fetch_one(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let item_id = uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"verification-shadow-replay-item.v1",
        );
        let inserted = sqlx::query(
            r#"INSERT INTO verification_campaign_shadow_evaluation_items(
                   shadow_evaluation_item_id,shadow_evaluation_id,operation_id,
                   project_scope_id,organization_id,item_ordinal,
                   shadow_evaluation_obligation_id,plan_objective_id,
                   compiled_semantic_signature_hash,legacy_capability_execution_receipt_id,
                   deterministic_oracle_replay_ref,comparison_id,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
               ON CONFLICT(shadow_evaluation_item_id) DO NOTHING"#,
        )
        .bind(item_id)
        .bind(request.evaluation_id)
        .bind(request.operation_id)
        .bind(authority.project_scope_id)
        .bind(authority.organization_id)
        .bind(authority.obligation_ordinal)
        .bind(request.evaluation_item_id)
        .bind(authority.plan_objective_id)
        .bind(&request.compiled_semantic_signature)
        .bind(request.legacy_capability_receipt_id)
        .bind(uuid::Uuid::new_v5(
            &request.stable_request_id,
            b"deterministic-oracle-replay.v1",
        ))
        .bind(sample.comparison_id)
        .bind(member_hash)
        .execute(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if inserted.rows_affected() == 0 {
            let existing: uuid::Uuid = sqlx::query_scalar(
                "SELECT comparison_id FROM verification_campaign_shadow_evaluation_items WHERE shadow_evaluation_item_id=$1",
            )
            .bind(item_id)
            .fetch_one(&*self.pool)
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
            if existing != sample.comparison_id {
                return Err(VerificationCampaignRepositoryError::Conflict {
                    detail: "shadow replay response-loss identity drift".to_owned(),
                });
            }
        }
        Ok(ComparisonId(sample.comparison_id))
    }

    async fn close_evaluation(
        &self,
        request: CloseShadowEvaluation,
    ) -> RepoResult<ShadowEvaluationReceipt> {
        if let Some(closed) = sqlx::query_as::<_, (i64, String, String, i64)>(
            r#"SELECT comparison_count,comparison_id_set_hash,receipt_hash,row_version
                 FROM verification_campaign_shadow_evaluations
                WHERE shadow_evaluation_id=$1 AND operation_id=$2 AND state='closed'"#,
        )
        .bind(request.evaluation_id)
        .bind(request.operation_id)
        .fetch_optional(&*self.pool)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        {
            return Ok(ShadowEvaluationReceipt {
                evaluation_id: request.evaluation_id,
                comparison_count: closed.0 as u32,
                comparison_id_set_hash: closed.1,
                receipt_hash: closed.2,
                row_version: closed.3,
                replayed: true,
            });
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let obligation_count: i64 = sqlx::query_scalar(
            r#"SELECT obligation_member_count
                 FROM verification_campaign_shadow_evaluations
                WHERE shadow_evaluation_id=$1 AND operation_id=$2 AND state='open'
                  AND row_version=$3 FOR UPDATE"#,
        )
        .bind(request.evaluation_id)
        .bind(request.operation_id)
        .bind(request.expected_row_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?
        .ok_or_else(|| VerificationCampaignRepositoryError::Conflict {
            detail: "shadow evaluation close CAS is stale".to_owned(),
        })?;
        let comparison_ids: Vec<uuid::Uuid> = sqlx::query_scalar(
            r#"SELECT comparison_id FROM verification_campaign_shadow_evaluation_items
                WHERE shadow_evaluation_id=$1 ORDER BY item_ordinal"#,
        )
        .bind(request.evaluation_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        if comparison_ids.len() as i64 != obligation_count {
            return Err(VerificationCampaignRepositoryError::Conflict {
                detail: "shadow evaluation cannot close with missing replay comparisons".to_owned(),
            });
        }
        let comparison_text = comparison_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let comparison_id_set_hash: String = sqlx::query_scalar(
            "SELECT investigation_exact_member_set_hash('verification_campaign_shadow_comparisons.v1',$1::TEXT[])",
        )
        .bind(&comparison_text)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        let receipt_hash: String = sqlx::query_scalar(
            r#"SELECT tool_truth_sha256(jsonb_build_object(
                   'contract_version','verification-shadow-evaluation-receipt.v1',
                   'shadow_evaluation_id',$1,'comparison_count',$2,
                   'comparison_id_set_hash',$3
               ))"#,
        )
        .bind(request.evaluation_id)
        .bind(obligation_count)
        .bind(&comparison_id_set_hash)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        sqlx::query(
            r#"UPDATE verification_campaign_shadow_evaluations
                  SET state='closed',comparison_count=$1,comparison_id_set_hash=$2,
                      receipt_hash=$3,row_version=row_version+1,
                      closed_at=statement_timestamp()
                WHERE shadow_evaluation_id=$4 AND row_version=$5 AND state='open'"#,
        )
        .bind(obligation_count)
        .bind(&comparison_id_set_hash)
        .bind(&receipt_hash)
        .bind(request.evaluation_id)
        .bind(request.expected_row_version)
        .execute(&mut *tx)
        .await
        .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        tx.commit()
            .await
            .map_err(|error| map_storage_error(anyhow::Error::new(error)))?;
        Ok(ShadowEvaluationReceipt {
            evaluation_id: request.evaluation_id,
            comparison_count: obligation_count as u32,
            comparison_id_set_hash,
            receipt_hash,
            row_version: request.expected_row_version + 1,
            replayed: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use async_trait::async_trait;
    use golish_agent_kit::db_traits::*;
    use uuid::Uuid;

    use super::*;

    struct RecordingRepository {
        calls: AtomicUsize,
        prepared_action_calls: AtomicUsize,
        expected_operation_id: Uuid,
    }

    #[async_trait]
    impl VerificationCampaignRepository for RecordingRepository {
        async fn seal_wave_coverage_denominator(
            &self,
            request: SealWaveCoverage,
        ) -> RepoResult<WaveCoverageSeal> {
            assert_eq!(request.operation_id, self.expected_operation_id);
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(WaveCoverageSeal {
                seal_id: Uuid::new_v4(),
                operation_id: request.operation_id,
                generation_seal_id: request.generation_seal_id,
                member_count: 2,
                member_set_hash: "sha256:members".to_owned(),
                replayed: false,
            })
        }

        async fn propose_prepared_action(
            &self,
            request: ProposePreparedAction,
        ) -> RepoResult<PreparedActionProposal> {
            assert_eq!(request.operation_id, self.expected_operation_id);
            self.prepared_action_calls.fetch_add(1, Ordering::SeqCst);
            Ok(PreparedActionProposal {
                prepared_action_id: Uuid::new_v4(),
                campaign_id: request.campaign_id,
                capability_id: "verify.directory_fingerprint.v1".to_owned(),
                coverage_member_hash: "sha256:coverage".to_owned(),
                private_manifest_hash: "sha256:manifest".to_owned(),
                row_version: 0,
                replayed: false,
            })
        }
    }

    #[tokio::test]
    async fn verification_campaign_repository_bridge_forwards_server_owned_identity() {
        let operation_id = Uuid::new_v4();
        let repository = Arc::new(RecordingRepository {
            calls: AtomicUsize::new(0),
            prepared_action_calls: AtomicUsize::new(0),
            expected_operation_id: operation_id,
        });
        let bridge = VerificationCampaignBridge::new(repository.clone());

        let result = bridge
            .seal_wave_coverage_denominator(SealWaveCoverage {
                stable_request_id: Uuid::new_v4(),
                operation_id,
                scope_snapshot_id: Uuid::new_v4(),
                organization_id: Uuid::new_v4(),
                generation_seal_id: Uuid::new_v4(),
                verification_plan_id: Uuid::new_v4(),
            })
            .await
            .expect("forwarding succeeds");

        assert_eq!(result.operation_id, operation_id);
        assert_eq!(repository.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn verification_campaign_repository_bridge_forwards_prepared_action_identity() {
        let operation_id = Uuid::new_v4();
        let campaign_id = Uuid::new_v4();
        let repository = Arc::new(RecordingRepository {
            calls: AtomicUsize::new(0),
            prepared_action_calls: AtomicUsize::new(0),
            expected_operation_id: operation_id,
        });
        let bridge = VerificationCampaignBridge::new(repository.clone());

        let result = bridge
            .propose_prepared_action(ProposePreparedAction {
                stable_request_id: Uuid::new_v4(),
                operation_id,
                campaign_id,
                round_id: Uuid::new_v4(),
                strategy_artifact_id: Uuid::new_v4(),
                strategy_obligation_id: Uuid::new_v4(),
            })
            .await
            .expect("forwarding succeeds");

        assert_eq!(result.campaign_id, campaign_id);
        assert_eq!(repository.prepared_action_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn verification_campaign_pg_adapter_installs_recovered_canonical_compounds() {
        let source = include_str!("verification_campaign.rs");

        for unavailable in [
            "self.unavailable(\"persist_strategy_decision\")",
            "self.unavailable(\"begin_action\")",
            "self.unavailable(\"seal_oracle_census\")",
        ] {
            assert!(
                !source.contains(unavailable),
                "production Campaign adapter still contains {unavailable}"
            );
        }
        for canonical_compound in [
            "verification_campaigns::record_strategy_decision",
            "verification_prepared_actions::persist_compiled_prepared_action",
            "verification_prepared_actions::begin_authorized_action_with_fresh_tool_truth",
            "verification_oracles::seal_oracle_census",
        ] {
            assert!(
                source.contains(canonical_compound),
                "production Campaign adapter is missing {canonical_compound}"
            );
        }
    }

    #[test]
    fn verification_campaign_coverage_denominator_keeps_objective_exact_set() {
        let source = include_str!("verification_campaign.rs");
        let pg_impl = source
            .split("impl VerificationCampaignRepository for PgVerificationCampaignRepository")
            .nth(1)
            .expect("production Campaign repository impl is present");
        let coverage_method = pg_impl
            .split("async fn seal_coverage_denominator")
            .nth(1)
            .and_then(|tail| tail.split("async fn propose_prepared_action").next())
            .expect("production coverage denominator method is bounded");

        assert!(
            !coverage_method.contains("expected_capability_kind=$3"),
            "Campaign denominator must cover the objective exact set, not one capability"
        );
        assert!(
            coverage_method
                .contains("WHERE wave_denominator_id=$1 AND verification_objective_id=$2"),
            "Campaign denominator must bind the sealed wave and objective identities"
        );
    }

    #[test]
    fn prepared_action_compiler_resolves_one_frozen_subject_and_scopes_non_public_exceptions() {
        let source = include_str!("verification_campaign.rs");
        let pg_impl = source
            .split("impl VerificationCampaignRepository for PgVerificationCampaignRepository")
            .nth(1)
            .expect("production Campaign repository impl is present");
        let compiler = pg_impl
            .split("async fn propose_prepared_action")
            .nth(1)
            .and_then(|tail| tail.split("async fn begin_action").next())
            .expect("production Prepared Action compiler is bounded");

        for frozen_subject_kind in [
            "revision.subject_kind='asset'",
            "revision.subject_kind='endpoint'",
            "revision.subject_kind='web_origin'",
        ] {
            assert!(
                compiler.contains(frozen_subject_kind),
                "Prepared Action compiler omits {frozen_subject_kind} resolution"
            );
        }
        assert!(
            compiler.contains("HAVING COUNT(*)=1"),
            "subject-to-target resolution must fail closed on zero or ambiguous matches"
        );
        assert!(
            compiler.contains("SELECT DISTINCT ON (origin.id)")
                && compiler.contains("WHEN 'url' THEN 0")
                && compiler.contains("WHEN 'domain' THEN 1"),
            "one frozen Web Origin must collapse its in-scope URL/domain aliases deterministically"
        );
        assert!(
            compiler.contains("operation_org_scope_snapshots scope_snapshot")
                && compiler.contains("operation_org_scope_units scope_unit")
                && compiler.contains("verification_exact_scope_exception.v1"),
            "a non-public exception must be derived from the sealed operation scope"
        );
        assert!(
            compiler.contains("error.reason_code == \"destination_non_public\"")
                && compiler.contains("scope_exception_hash.is_none()"),
            "sealed scope authority must only add an exception after the compiler classifies a non-public destination"
        );
    }

    #[test]
    fn capability_assessment_set_seal_selects_the_requested_objective() {
        let source = include_str!("verification_campaign.rs");
        let pg_impl = source
            .split("impl VerificationCampaignRepository for PgVerificationCampaignRepository")
            .nth(1)
            .expect("production Campaign repository impl is present");
        let seal_method = pg_impl
            .split("async fn seal_capability_assessment_set")
            .nth(1)
            .and_then(|tail| {
                tail.split("async fn admit_campaign_with_fresh_tool_truth")
                    .next()
            })
            .expect("production capability-assessment seal method is bounded");

        assert_eq!(
            seal_method.matches(".bind(request.objective_id)").count(),
            2,
            "assessment-set replay and first seal must select the requested objective"
        );
        assert!(
            !seal_method.contains(".bind(request.wave_coverage_seal_id)"),
            "wave-denominator identity cannot substitute for the objective selector"
        );
    }

    #[test]
    fn oracle_census_reads_predicate_and_control_authority_from_the_wave_member() {
        let source = include_str!("verification_campaign.rs");
        let pg_impl = source
            .split("impl VerificationCampaignRepository for PgVerificationCampaignRepository")
            .nth(1)
            .expect("production Campaign repository impl is present");
        let census_method = pg_impl
            .split("async fn seal_oracle_census")
            .nth(1)
            .and_then(|tail| tail.split("async fn close_campaign_objective").next())
            .expect("production oracle-census method is bounded");

        for wave_authority in [
            "wave.predicate_component_id",
            "wave.control_binding_kind",
            "wave.required_control_id",
            "wave.required_control_hash",
            "wave.no_control_marker_hash",
        ] {
            assert!(
                census_method.contains(wave_authority),
                "oracle census must read {wave_authority} from the joined wave member"
            );
        }
        assert!(!census_method.contains("member.predicate_component_id"));
        assert!(!census_method.contains("member.required_control_id"));
    }

    #[test]
    fn unexecuted_campaign_coverage_is_not_an_epistemic_oracle_outcome() {
        assert_eq!(
            campaign_coverage_epistemic_outcome("blocked", None),
            Some("not_assessed")
        );
        assert_eq!(
            campaign_coverage_epistemic_outcome("untested", None),
            Some("not_assessed")
        );
        for verdict in ["proof", "refutation", "inconclusive"] {
            assert_eq!(
                campaign_coverage_epistemic_outcome("assessed", Some(verdict)),
                Some(verdict)
            );
        }
        assert_eq!(
            campaign_coverage_epistemic_outcome("blocked", Some("inconclusive")),
            None
        );
        assert_eq!(campaign_coverage_epistemic_outcome("assessed", None), None);
    }

    #[test]
    fn verification_campaign_repository_bridge_preserves_stable_error_class() {
        let mapped = map_storage_error(anyhow::anyhow!("VERIFICATION_CAMPAIGN_OWNERSHIP_MISMATCH"));
        assert_eq!(
            mapped.code(),
            "verification_campaign_repository_authority_mismatch"
        );

        let unavailable = map_storage_error(anyhow::anyhow!(
            "verification_campaign_repository_unavailable"
        ));
        assert_eq!(
            unavailable.code(),
            "verification_campaign_repository_unavailable"
        );
    }
}
