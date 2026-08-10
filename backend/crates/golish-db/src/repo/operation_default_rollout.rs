//! Joint Tool Truth + Investigation default promotion coordinator.
//!
//! This is the sole mutation seam for either deployment singleton. Evidence is
//! reloaded under the same transaction; callers submit only CAS versions,
//! target rank, principal and reason.

use golish_core::{InvestigationContractVersion, InvestigationRolloutMode};
use golish_pentest_domain::tool_truth::ToolTruthContract;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::operation_rollout::{
    joint_contract_rank, promotion_evidence_shape, OperationRolloutError, OperationRolloutResult,
};

const CRITERIA_VERSION: &str = "operation_default_promotion.v2";

pub const OPERATION_PROMOTION_COMPONENT_KINDS: [&str; 7] = [
    "profile",
    "graph",
    "read_model",
    "report",
    "pentagi_task_identity",
    "legacy_replay",
    "whole_record_compatibility",
];

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct OperationPromotionComponentRow {
    pub component_kind: String,
    pub component_sha256: String,
    pub member_sha256: String,
}

#[derive(sqlx::FromRow)]
struct OperationPromotionComponentCensusRow {
    census_id: Uuid,
    component_member_count: i64,
    component_set_sha256: String,
}

#[derive(Clone, Debug)]
pub struct PromoteOperationDefaults {
    pub expected_safety_hold_row_version: i64,
    pub expected_campaign_dispatch_generation: i64,
    pub expected_operation_admission_generation: i64,
    pub expected_tool_truth_row_version: i64,
    pub expected_investigation_row_version: i64,
    pub target_joint_rank: i16,
    /// Dry-run leaves this empty. Apply must echo the dry-run manifest hash;
    /// any evidence change between the two phases fails closed.
    pub expected_evidence_manifest_hash: Option<String>,
    pub principal_id: Uuid,
    pub reason: String,
}

#[derive(Clone, Debug, sqlx::FromRow, PartialEq, Eq)]
pub struct OperationDefaultPromotionReceipt {
    pub receipt_id: Uuid,
    pub from_joint_rank: i16,
    pub to_joint_rank: i16,
    pub evidence_manifest_hash: String,
    pub evidence_member_count: i64,
    pub promoted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
struct EvidenceMember {
    kind: &'static str,
    operation_id: Option<Uuid>,
    source_ref_hash: String,
    tool_truth_readiness_receipt_id: Option<Uuid>,
    registry_readiness_receipt_id: Option<Uuid>,
    comparison_id: Option<Uuid>,
    tool_truth_authority_bundle_id: Option<Uuid>,
    canary_action_execution_id: Option<Uuid>,
    canary_oracle_assessment_id: Option<Uuid>,
    canary_wave_coverage_receipt_id: Option<Uuid>,
    canary_revision_adjudication_id: Option<Uuid>,
    canary_report_dry_run_receipt_id: Option<Uuid>,
    adversarial_acceptance_receipt_id: Option<Uuid>,
    legacy_retirement_receipt_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct ComparisonAggregateRow {
    cohort_id: Uuid,
    cutoff_manifest_hash: String,
    expected_record_count: i64,
    sampled_record_count: i64,
    matched_record_count: i64,
    mismatch_record_count: i64,
    missing_record_count: i64,
    invalid_record_count: i64,
    admission_closed: bool,
}

impl EvidenceMember {
    fn identity(&self) -> Uuid {
        self.tool_truth_readiness_receipt_id
            .or(self.registry_readiness_receipt_id)
            .or(self.comparison_id)
            .or(self.tool_truth_authority_bundle_id)
            .or(self.canary_action_execution_id)
            .or(self.canary_oracle_assessment_id)
            .or(self.canary_wave_coverage_receipt_id)
            .or(self.canary_revision_adjudication_id)
            .or(self.canary_report_dry_run_receipt_id)
            .or(self.adversarial_acceptance_receipt_id)
            .or(self.legacy_retirement_receipt_id)
            .expect("typed promotion evidence has exactly one identity")
    }

    fn member_hash(&self, ordinal: usize) -> String {
        tagged_hash(&json!({
            "domain": "operation_default_promotion_evidence_member.v1",
            "ordinal": ordinal,
            "kind": self.kind,
            "operation_id": self.operation_id,
            "identity": self.identity(),
            "source_ref_hash": self.source_ref_hash,
        }))
    }
}

fn tagged_hash(value: &serde_json::Value) -> String {
    let digest = Sha256::digest(
        serde_json::to_vec(value).expect("typed promotion evidence is serializable"),
    );
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn tagged_text_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

pub fn operation_promotion_component_member_hash(
    component_kind: &str,
    component_sha256: &str,
) -> String {
    tagged_text_hash(&format!(
        "operation_default_promotion_component_member.v1\n{component_kind}\n{component_sha256}"
    ))
}

pub fn operation_promotion_component_set_hash(rows: &[OperationPromotionComponentRow]) -> String {
    let mut members = rows
        .iter()
        .map(|row| (row.component_kind.as_str(), row.member_sha256.as_str()))
        .collect::<Vec<_>>();
    members.sort_unstable_by_key(|(kind, _)| *kind);
    tagged_text_hash(&format!(
        "operation_default_promotion_components.v1\n{}",
        members
            .into_iter()
            .map(|(_, member_sha256)| member_sha256)
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

pub fn validate_operation_promotion_component_rows(
    rows: &[OperationPromotionComponentRow],
    declared_set_sha256: &str,
    actual_set_sha256: &str,
) -> OperationRolloutResult<()> {
    let mut component_kinds = rows
        .iter()
        .map(|row| row.component_kind.as_str())
        .collect::<Vec<_>>();
    component_kinds.sort_unstable();
    if component_kinds.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_COMPONENT_DUPLICATE",
        });
    }
    if rows.len() != OPERATION_PROMOTION_COMPONENT_KINDS.len()
        || OPERATION_PROMOTION_COMPONENT_KINDS
            .iter()
            .any(|expected| !rows.iter().any(|row| row.component_kind == *expected))
        || rows
            .iter()
            .any(|row| !OPERATION_PROMOTION_COMPONENT_KINDS.contains(&row.component_kind.as_str()))
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_COMPONENT_SET_INCOMPLETE",
        });
    }
    if rows.iter().any(|row| {
        row.member_sha256
            != operation_promotion_component_member_hash(&row.component_kind, &row.component_sha256)
    }) {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_COMPONENT_MEMBER_HASH_DRIFT",
        });
    }
    if declared_set_sha256 != actual_set_sha256 {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_COMPONENT_SET_HASH_DRIFT",
        });
    }
    Ok(())
}

async fn load_promotion_component_census(
    tx: &mut Transaction<'_, Postgres>,
) -> OperationRolloutResult<OperationPromotionComponentCensusRow> {
    let census = sqlx::query_as::<_, OperationPromotionComponentCensusRow>(
        r#"SELECT census_id,component_member_count,component_set_sha256
             FROM operation_default_promotion_component_censuses
            WHERE criteria_version='operation_default_promotion.v2'
            ORDER BY sealed_at DESC,census_id DESC
            LIMIT 1 FOR SHARE"#,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(OperationRolloutError::Missing {
        entity: "OPERATION_PROMOTION_COMPONENT_CENSUS",
    })?;
    if census.component_member_count
        != i64::try_from(OPERATION_PROMOTION_COMPONENT_KINDS.len()).expect("seven fits i64")
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_COMPONENT_SET_INCOMPLETE",
        });
    }
    let rows = sqlx::query_as::<_, OperationPromotionComponentRow>(
        r#"SELECT component_kind,component_sha256,member_sha256
             FROM operation_default_promotion_component_members
            WHERE census_id=$1 ORDER BY component_kind FOR SHARE"#,
    )
    .bind(census.census_id)
    .fetch_all(&mut **tx)
    .await?;
    let actual_set_sha256: String = sqlx::query_scalar(
        r#"SELECT unified_investigation_exact_set_hash(
                   'operation_default_promotion_components.v1',
                   COALESCE(array_agg(member_sha256 ORDER BY component_kind),ARRAY[]::TEXT[]))
             FROM operation_default_promotion_component_members
            WHERE census_id=$1"#,
    )
    .bind(census.census_id)
    .fetch_one(&mut **tx)
    .await?;
    validate_operation_promotion_component_rows(
        &rows,
        &census.component_set_sha256,
        &actual_set_sha256,
    )?;
    Ok(census)
}

fn pair_for_rank(
    rank: i16,
) -> OperationRolloutResult<(
    ToolTruthContract,
    InvestigationContractVersion,
    InvestigationRolloutMode,
)> {
    use InvestigationContractVersion::{HypothesisRegistryV1, LegacyCandidateV1};
    use InvestigationRolloutMode::{
        DualReadCompare, LegacyOnly, NewOnly, RegistryAuthoritativeLegacyProjection, ShadowRegistry,
    };
    use ToolTruthContract::{LegacyV1, ReceiptV1, ShadowV1};
    match rank {
        0 => Ok((LegacyV1, LegacyCandidateV1, LegacyOnly)),
        1 => Ok((ShadowV1, LegacyCandidateV1, LegacyOnly)),
        2 => Ok((ShadowV1, HypothesisRegistryV1, ShadowRegistry)),
        3 => Ok((ShadowV1, HypothesisRegistryV1, DualReadCompare)),
        4 => Ok((ReceiptV1, HypothesisRegistryV1, DualReadCompare)),
        5 => Ok((
            ReceiptV1,
            HypothesisRegistryV1,
            RegistryAuthoritativeLegacyProjection,
        )),
        6 => Ok((ReceiptV1, HypothesisRegistryV1, NewOnly)),
        _ => Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_TARGET_RANK_INVALID",
        }),
    }
}

async fn collect_summary_receipt(
    tx: &mut Transaction<'_, Postgres>,
    rank: i16,
) -> OperationRolloutResult<Vec<EvidenceMember>> {
    let row: Option<(Uuid, String, i64, i64)> = if rank == 0 {
        sqlx::query_as(
            r#"SELECT receipt_id,assessment_set_hash,readiness_member_count,
                      (SELECT COUNT(*) FROM tool_truth_shadow_writer_readiness_members member
                        WHERE member.receipt_id=receipt.receipt_id)
                 FROM tool_truth_shadow_writer_readiness_receipts receipt
                WHERE missing_assessment_count=0 AND orphan_reconciliation_count=0
                  AND corrupt_artifact_count=0
                ORDER BY created_at DESC LIMIT 1 FOR SHARE"#,
        )
        .fetch_optional(&mut **tx)
        .await?
    } else if rank == 1 {
        sqlx::query_as(
            r#"SELECT receipt_id,evaluation_membership_hash,evaluation_member_count,
                      (SELECT COUNT(*) FROM registry_shadow_evaluator_readiness_members member
                        WHERE member.receipt_id=receipt.receipt_id)
                 FROM registry_shadow_evaluator_readiness_receipts receipt
                WHERE external_port_call_count=0 AND canonical_mutation_count=0
                  AND incomplete_or_corrupt_count=0
                ORDER BY created_at DESC LIMIT 1 FOR SHARE"#,
        )
        .fetch_optional(&mut **tx)
        .await?
    } else {
        sqlx::query_as(
            r#"SELECT receipt_id,retirement_manifest_hash,consumer_member_count,
                      (SELECT COUNT(*) FROM legacy_consumer_retirement_members member
                        WHERE member.receipt_id=receipt.receipt_id)
                 FROM legacy_consumer_retirement_receipts receipt
                WHERE unmigrated_consumer_count=0 AND legacy_mutation_call_count=0
                  AND legacy_read_fallback_call_count=0
                ORDER BY created_at DESC LIMIT 1 FOR SHARE"#,
        )
        .fetch_optional(&mut **tx)
        .await?
    };
    let (receipt_id, source_ref_hash, declared, actual) =
        row.ok_or(OperationRolloutError::Missing {
            entity: "OPERATION_PROMOTION_READINESS_RECEIPT",
        })?;
    if declared != actual || declared <= 0 {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_EVIDENCE_SET_INVALID",
        });
    }
    Ok(vec![EvidenceMember {
        kind: match rank {
            0 => "tool_truth_shadow_writer_readiness_receipt",
            1 => "registry_shadow_evaluator_readiness_receipt",
            _ => "legacy_consumer_retirement_receipt",
        },
        operation_id: None,
        source_ref_hash,
        tool_truth_readiness_receipt_id: (rank == 0).then_some(receipt_id),
        registry_readiness_receipt_id: (rank == 1).then_some(receipt_id),
        comparison_id: None,
        tool_truth_authority_bundle_id: None,
        canary_action_execution_id: None,
        canary_oracle_assessment_id: None,
        canary_wave_coverage_receipt_id: None,
        canary_revision_adjudication_id: None,
        canary_report_dry_run_receipt_id: None,
        adversarial_acceptance_receipt_id: None,
        legacy_retirement_receipt_id: (rank == 5).then_some(receipt_id),
    }])
}

async fn collect_comparison(
    tx: &mut Transaction<'_, Postgres>,
    rank: i16,
) -> OperationRolloutResult<(Uuid, String, Vec<EvidenceMember>)> {
    let aggregate = sqlx::query_as::<_, ComparisonAggregateRow>(
        r#"SELECT cohort_id,cutoff_manifest_hash,expected_record_count,sampled_record_count,
                      matched_record_count,mismatch_record_count,missing_record_count,
                      incomplete_record_count+corrupt_record_count AS invalid_record_count,
                      admission_closed
                 FROM investigation_projection_compare_aggregates
                WHERE from_joint_rank=$1 AND to_joint_rank=$2
                ORDER BY aggregated_at DESC LIMIT 1 FOR SHARE"#,
    )
    .bind(rank)
    .bind(rank + 1)
    .fetch_optional(&mut **tx)
    .await?;
    let aggregate = aggregate.ok_or(OperationRolloutError::Missing {
        entity: "OPERATION_PROMOTION_COMPARISON_COHORT",
    })?;
    if !aggregate.admission_closed
        || aggregate.expected_record_count <= 0
        || aggregate.sampled_record_count != aggregate.expected_record_count
        || aggregate.matched_record_count != aggregate.sampled_record_count
        || aggregate.mismatch_record_count != 0
        || aggregate.missing_record_count != 0
        || aggregate.invalid_record_count != 0
    {
        return Err(OperationRolloutError::Conflict {
            code: "INVESTIGATION_ROLLOUT_COMPARISON_NOT_EXACT",
        });
    }
    let rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        r#"SELECT sample.comparison_id,sample.operation_id,
                  tool_truth_sha256(to_jsonb(sample)::TEXT)
             FROM investigation_projection_compare_cohort_members member
             JOIN investigation_projection_compare_samples sample
               ON sample.operation_id=member.operation_id
              AND sample.as_of_change_seq=member.as_of_change_seq
            WHERE member.cohort_id=$1 AND sample.comparison_state='match'
            ORDER BY sample.operation_id,sample.record_kind,sample.record_key"#,
    )
    .bind(aggregate.cohort_id)
    .fetch_all(&mut **tx)
    .await?;
    if i64::try_from(rows.len()).ok() != Some(aggregate.expected_record_count) {
        return Err(OperationRolloutError::Conflict {
            code: "INVESTIGATION_ROLLOUT_COMPARISON_NOT_EXACT",
        });
    }
    let kind = if rank == 2 {
        "shadow_comparison_sample"
    } else {
        "dual_comparison_sample"
    };
    Ok((
        aggregate.cohort_id,
        aggregate.cutoff_manifest_hash,
        rows.into_iter()
            .map(
                |(comparison_id, operation_id, source_ref_hash)| EvidenceMember {
                    kind,
                    operation_id: Some(operation_id),
                    source_ref_hash,
                    tool_truth_readiness_receipt_id: None,
                    registry_readiness_receipt_id: None,
                    comparison_id: Some(comparison_id),
                    tool_truth_authority_bundle_id: None,
                    canary_action_execution_id: None,
                    canary_oracle_assessment_id: None,
                    canary_wave_coverage_receipt_id: None,
                    canary_revision_adjudication_id: None,
                    canary_report_dry_run_receipt_id: None,
                    adversarial_acceptance_receipt_id: None,
                    legacy_retirement_receipt_id: None,
                },
            )
            .collect(),
    ))
}

async fn collect_all_fresh_bundles(
    tx: &mut Transaction<'_, Postgres>,
    required_rank: i16,
) -> OperationRolloutResult<Vec<EvidenceMember>> {
    let expected_operation_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM operation_state
            WHERE operation_joint_contract_rank(tool_truth_contract,
                  investigation_contract_version,investigation_rollout_mode)=$1"#,
    )
    .bind(required_rank)
    .fetch_one(&mut **tx)
    .await?;
    let rows: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        r#"SELECT DISTINCT ON(bundle.operation_id) bundle.id,bundle.operation_id,
                  bundle.semantic_authority_bundle_hash
             FROM tool_truth_authority_bundle_seals bundle
             JOIN operation_state operation ON operation.operation_id=bundle.operation_id
            WHERE operation_joint_contract_rank(operation.tool_truth_contract,
                  operation.investigation_contract_version,
                  operation.investigation_rollout_mode)=$1
              AND bundle.sealed_at IS NOT NULL AND bundle.relevant_root_count=3
              AND bundle.member_count=3 AND bundle.consistent_fresh_count=3
              AND bundle.stale_or_invalid_count=0
              AND transaction_timestamp()<=bundle.effective_valid_until
            ORDER BY bundle.operation_id,bundle.sealed_at DESC"#,
    )
    .bind(required_rank)
    .fetch_all(&mut **tx)
    .await?;
    if expected_operation_count <= 0
        || i64::try_from(rows.len()).ok() != Some(expected_operation_count)
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_ALL_FRESH_AUTHORITY_INCOMPLETE",
        });
    }
    Ok(rows
        .into_iter()
        .map(
            |(bundle_id, operation_id, source_ref_hash)| EvidenceMember {
                kind: "tool_truth_all_fresh_authority_bundle",
                operation_id: Some(operation_id),
                source_ref_hash,
                tool_truth_readiness_receipt_id: None,
                registry_readiness_receipt_id: None,
                comparison_id: None,
                tool_truth_authority_bundle_id: Some(bundle_id),
                canary_action_execution_id: None,
                canary_oracle_assessment_id: None,
                canary_wave_coverage_receipt_id: None,
                canary_revision_adjudication_id: None,
                canary_report_dry_run_receipt_id: None,
                adversarial_acceptance_receipt_id: None,
                legacy_retirement_receipt_id: None,
            },
        )
        .collect())
}

#[derive(sqlx::FromRow)]
struct CanaryEvidenceRow {
    operation_id: Uuid,
    bundle_id: Uuid,
    bundle_hash: String,
    action_execution_id: Uuid,
    action_hash: String,
    oracle_assessment_id: Uuid,
    oracle_hash: String,
    wave_coverage_receipt_id: Uuid,
    coverage_hash: String,
    revision_adjudication_id: Uuid,
    adjudication_hash: String,
    dry_run_receipt_id: Uuid,
    dry_run_hash: String,
    adversarial_receipt_id: Uuid,
    adversarial_hash: String,
}

async fn collect_canary(
    tx: &mut Transaction<'_, Postgres>,
) -> OperationRolloutResult<(Uuid, Vec<EvidenceMember>)> {
    let row = sqlx::query_as::<_, CanaryEvidenceRow>(
        r#"SELECT dry.operation_id,adjudication.tool_truth_authority_bundle_seal_id AS bundle_id,
                  adjudication.semantic_authority_bundle_hash AS bundle_hash,
                  execution.action_execution_id,
                  COALESCE(execution.closeout_hash,execution.durable_begin_hash) AS action_hash,
                  oracle.oracle_assessment_id,oracle.assessment_hash AS oracle_hash,
                  coverage.wave_coverage_receipt_id,coverage.receipt_hash AS coverage_hash,
                  adjudication.revision_adjudication_id,
                  adjudication.adjudication_hash,
                  dry.receipt_id AS dry_run_receipt_id,dry.receipt_hash AS dry_run_hash,
                  corpus.receipt_id AS adversarial_receipt_id,
                  corpus.observed_outcome_membership_hash AS adversarial_hash
             FROM authoritative_report_dry_run_receipts dry
             JOIN verification_wave_coverage_receipts coverage
               ON coverage.wave_coverage_receipt_id=dry.wave_coverage_receipt_id
              AND coverage.operation_id=dry.operation_id
             JOIN hypothesis_revision_adjudications adjudication
               ON adjudication.operation_id=dry.operation_id
              AND adjudication.outcome IN ('verified','refuted')
              AND transaction_timestamp()<=adjudication.effective_valid_until
             JOIN verification_oracle_assessments oracle
               ON oracle.operation_id=dry.operation_id AND oracle.verdict IN ('proof','refutation')
             JOIN verification_action_executions execution
               ON execution.action_execution_id=oracle.action_execution_id
              AND execution.state IN ('succeeded','failed')
             CROSS JOIN LATERAL (
                 SELECT * FROM adversarial_acceptance_corpus_receipts candidate
                  WHERE candidate.fixture_member_count=9 AND candidate.mismatch_count=0
                    AND candidate.missing_count=0 AND candidate.extra_count=0
                  ORDER BY candidate.sealed_at DESC LIMIT 1
             ) corpus
            ORDER BY dry.created_at DESC,adjudication.created_at DESC,oracle.assessed_at DESC
            LIMIT 1 FOR SHARE"#,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(OperationRolloutError::Missing {
        entity: "OPERATION_PROMOTION_AUTHORITATIVE_CANARY",
    })?;
    let specs = [
        (
            "tool_truth_all_fresh_authority_bundle",
            row.bundle_id,
            row.bundle_hash,
        ),
        (
            "authoritative_canary_action_receipt",
            row.action_execution_id,
            row.action_hash,
        ),
        (
            "authoritative_canary_oracle_receipt",
            row.oracle_assessment_id,
            row.oracle_hash,
        ),
        (
            "authoritative_canary_coverage_receipt",
            row.wave_coverage_receipt_id,
            row.coverage_hash,
        ),
        (
            "authoritative_canary_revision_adjudication",
            row.revision_adjudication_id,
            row.adjudication_hash,
        ),
        (
            "authoritative_canary_report_dry_run_receipt",
            row.dry_run_receipt_id,
            row.dry_run_hash,
        ),
        (
            "adversarial_acceptance_corpus_receipt",
            row.adversarial_receipt_id,
            row.adversarial_hash,
        ),
    ];
    let members = specs
        .into_iter()
        .map(|(kind, id, source_ref_hash)| EvidenceMember {
            kind,
            operation_id: Some(row.operation_id),
            source_ref_hash,
            tool_truth_readiness_receipt_id: None,
            registry_readiness_receipt_id: None,
            comparison_id: None,
            tool_truth_authority_bundle_id: (kind == "tool_truth_all_fresh_authority_bundle")
                .then_some(id),
            canary_action_execution_id: (kind == "authoritative_canary_action_receipt")
                .then_some(id),
            canary_oracle_assessment_id: (kind == "authoritative_canary_oracle_receipt")
                .then_some(id),
            canary_wave_coverage_receipt_id: (kind == "authoritative_canary_coverage_receipt")
                .then_some(id),
            canary_revision_adjudication_id: (kind == "authoritative_canary_revision_adjudication")
                .then_some(id),
            canary_report_dry_run_receipt_id: (kind
                == "authoritative_canary_report_dry_run_receipt")
                .then_some(id),
            adversarial_acceptance_receipt_id: (kind == "adversarial_acceptance_corpus_receipt")
                .then_some(id),
            legacy_retirement_receipt_id: None,
        })
        .collect();
    Ok((row.adversarial_receipt_id, members))
}

pub async fn promote_operation_defaults(
    tx: &mut Transaction<'_, Postgres>,
    request: PromoteOperationDefaults,
) -> OperationRolloutResult<OperationDefaultPromotionReceipt> {
    if request.reason.trim().is_empty() || request.reason.len() > 2048 {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_REASON_INVALID",
        });
    }
    let principal_is_active: Option<bool> = sqlx::query_scalar(
        r#"SELECT active FROM operator_principals
            WHERE id=$1 AND principal_kind='local_operator' AND active FOR SHARE"#,
    )
    .bind(request.principal_id)
    .fetch_optional(&mut **tx)
    .await?;
    if principal_is_active != Some(true) {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_PRINCIPAL_INVALID",
        });
    }
    let safety_hold: (bool, bool, i64, i64, i64) = sqlx::query_as(
        r#"SELECT campaign_dispatch_held,operation_admission_held,
                  campaign_dispatch_generation,operation_admission_generation,row_version
             FROM verification_campaign_safety_holds
            WHERE singleton=TRUE FOR SHARE"#,
    )
    .fetch_one(&mut **tx)
    .await?;
    if safety_hold.2 != request.expected_campaign_dispatch_generation
        || safety_hold.3 != request.expected_operation_admission_generation
        || safety_hold.4 != request.expected_safety_hold_row_version
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_SAFETY_HOLD_CAS_STALE",
        });
    }
    if !safety_hold.0 || !safety_hold.1 {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_SAFETY_HOLD_REQUIRED",
        });
    }
    let tool: (String, i64) = sqlx::query_as(
        r#"SELECT new_operation_contract,row_version FROM tool_truth_rollout
            WHERE singleton=TRUE FOR UPDATE"#,
    )
    .fetch_one(&mut **tx)
    .await?;
    let investigation: (String, String, i16, i64) = sqlx::query_as(
        r#"SELECT contract_version,rollout_mode,mode_rank,row_version
             FROM investigation_rollout WHERE singleton=TRUE FOR UPDATE"#,
    )
    .fetch_one(&mut **tx)
    .await?;
    if tool.1 != request.expected_tool_truth_row_version
        || investigation.3 != request.expected_investigation_row_version
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_CAS_STALE",
        });
    }
    let current_tool = ToolTruthContract::try_from(tool.0.as_str()).map_err(|_| {
        OperationRolloutError::Conflict {
            code: "OPERATION_TOOL_TRUTH_CONTRACT_UNKNOWN",
        }
    })?;
    let (current_contract, current_mode) =
        super::investigation_rollout::parse_frozen_pair(&investigation.0, &investigation.1)
            .map_err(|_| OperationRolloutError::Conflict {
                code: "OPERATION_INVESTIGATION_CONTRACT_UNKNOWN",
            })?;
    if investigation.2 != current_mode.mode_rank() {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_INVESTIGATION_ROLLOUT_RANK_MISMATCH",
        });
    }
    let from_rank = joint_contract_rank(current_tool, current_contract, current_mode).ok_or(
        OperationRolloutError::Conflict {
            code: "OPERATION_JOINT_CONTRACT_PAIR_INVALID",
        },
    )?;
    promotion_evidence_shape(from_rank, request.target_joint_rank).map_err(|_| {
        OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_TRANSITION_INVALID",
        }
    })?;
    let component_census = load_promotion_component_census(tx).await?;
    let (target_tool, target_contract, target_mode) = pair_for_rank(request.target_joint_rank)?;
    let (cohort_id, cohort_cutoff_manifest_hash, mut members) = match from_rank {
        0 | 1 | 5 => (None, None, collect_summary_receipt(tx, from_rank).await?),
        2 => {
            let (id, cutoff, rows) = collect_comparison(tx, from_rank).await?;
            (Some(id), Some(cutoff), rows)
        }
        3 => (None, None, collect_all_fresh_bundles(tx, from_rank).await?),
        4 => {
            let (id, cutoff, mut rows) = collect_comparison(tx, from_rank).await?;
            let (_, mut canary) = collect_canary(tx).await?;
            rows.append(&mut canary);
            (Some(id), Some(cutoff), rows)
        }
        _ => {
            return Err(OperationRolloutError::Conflict {
                code: "OPERATION_PROMOTION_TRANSITION_INVALID",
            })
        }
    };
    members.sort_by_key(|member| (member.kind, member.operation_id, member.identity()));
    let member_hashes = members
        .iter()
        .enumerate()
        .map(|(ordinal, member)| member.member_hash(ordinal))
        .collect::<Vec<_>>();
    let evidence_manifest_hash = tagged_hash(&json!({
        "domain": "operation_default_promotion_evidence_manifest.v2",
        "component_census_id": component_census.census_id,
        "component_set_sha256": component_census.component_set_sha256,
        "members": member_hashes,
    }));
    if request
        .expected_evidence_manifest_hash
        .as_deref()
        .is_some_and(|expected| expected != evidence_manifest_hash)
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_DRY_RUN_DRIFT",
        });
    }
    let canary_manifest_hash = (from_rank == 4).then(|| {
        tagged_hash(&json!({
            "domain": "operation_default_promotion_canary_manifest.v1",
            "members": member_hashes,
        }))
    });
    let adversarial_acceptance_receipt_id = members
        .iter()
        .find_map(|member| member.adversarial_acceptance_receipt_id);
    let receipt_id = Uuid::new_v5(
        &request.principal_id,
        format!(
            "operation-default-promotion:v2:{}:{}:{}:{}",
            from_rank,
            request.target_joint_rank,
            evidence_manifest_hash,
            component_census.component_set_sha256
        )
        .as_bytes(),
    );
    let receipt = sqlx::query_as::<_, OperationDefaultPromotionReceipt>(
        r#"INSERT INTO operation_default_promotion_receipts(
               receipt_id,from_joint_rank,to_joint_rank,criteria_version,
               tool_truth_from,tool_truth_to,investigation_contract_from,
               investigation_mode_from,investigation_contract_to,investigation_mode_to,
               cohort_id,cohort_cutoff_manifest_hash,evidence_manifest_hash,
               evidence_member_count,canary_manifest_hash,adversarial_acceptance_receipt_id,
               component_census_grandfathered,component_census_id,component_census_sha256,
               expected_tool_truth_row_version,expected_investigation_row_version,
               promoted_by_principal_id,reason
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,
                    FALSE,$17,$18,$19,$20,$21,$22)
           RETURNING receipt_id,from_joint_rank,to_joint_rank,evidence_manifest_hash,
                     evidence_member_count,promoted_at"#,
    )
    .bind(receipt_id)
    .bind(from_rank)
    .bind(request.target_joint_rank)
    .bind(CRITERIA_VERSION)
    .bind(current_tool.as_str())
    .bind(target_tool.as_str())
    .bind(current_contract.as_str())
    .bind(current_mode.as_str())
    .bind(target_contract.as_str())
    .bind(target_mode.as_str())
    .bind(cohort_id)
    .bind(cohort_cutoff_manifest_hash)
    .bind(&evidence_manifest_hash)
    .bind(
        i64::try_from(members.len()).map_err(|_| OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_EVIDENCE_SET_INVALID",
        })?,
    )
    .bind(canary_manifest_hash)
    .bind(adversarial_acceptance_receipt_id)
    .bind(component_census.census_id)
    .bind(&component_census.component_set_sha256)
    .bind(request.expected_tool_truth_row_version)
    .bind(request.expected_investigation_row_version)
    .bind(request.principal_id)
    .bind(request.reason.trim())
    .fetch_one(&mut **tx)
    .await?;
    for (ordinal, (member, member_hash)) in members.iter().zip(member_hashes.iter()).enumerate() {
        sqlx::query(
            r#"INSERT INTO operation_default_promotion_evidence_members(
                   receipt_id,ordinal,evidence_kind,operation_id,
                   tool_truth_readiness_receipt_id,registry_readiness_receipt_id,
                   comparison_id,tool_truth_authority_bundle_id,
                   canary_action_execution_id,canary_oracle_assessment_id,
                   canary_wave_coverage_receipt_id,canary_revision_adjudication_id,
                   canary_report_dry_run_receipt_id,adversarial_acceptance_receipt_id,
                   legacy_retirement_receipt_id,source_ref_hash,member_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
        )
        .bind(receipt_id)
        .bind(
            i32::try_from(ordinal).map_err(|_| OperationRolloutError::Conflict {
                code: "OPERATION_PROMOTION_EVIDENCE_SET_INVALID",
            })?,
        )
        .bind(member.kind)
        .bind(member.operation_id)
        .bind(member.tool_truth_readiness_receipt_id)
        .bind(member.registry_readiness_receipt_id)
        .bind(member.comparison_id)
        .bind(member.tool_truth_authority_bundle_id)
        .bind(member.canary_action_execution_id)
        .bind(member.canary_oracle_assessment_id)
        .bind(member.canary_wave_coverage_receipt_id)
        .bind(member.canary_revision_adjudication_id)
        .bind(member.canary_report_dry_run_receipt_id)
        .bind(member.adversarial_acceptance_receipt_id)
        .bind(member.legacy_retirement_receipt_id)
        .bind(&member.source_ref_hash)
        .bind(member_hash)
        .execute(&mut **tx)
        .await?;
    }
    let persisted_member_hashes: Vec<String> = sqlx::query_scalar(
        r#"SELECT member_hash
             FROM operation_default_promotion_evidence_members
            WHERE receipt_id=$1 ORDER BY ordinal FOR SHARE"#,
    )
    .bind(receipt_id)
    .fetch_all(&mut **tx)
    .await?;
    if persisted_member_hashes != member_hashes
        || i64::try_from(persisted_member_hashes.len()).ok() != Some(receipt.evidence_member_count)
    {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_EVIDENCE_SET_INVALID",
        });
    }
    sqlx::query("SELECT set_config('golish.operation_default_promotion_receipt_id',$1,TRUE)")
        .bind(receipt_id.to_string())
        .execute(&mut **tx)
        .await?;
    let tool_updated = sqlx::query(
        r#"UPDATE tool_truth_rollout
              SET new_operation_contract=$1,row_version=row_version+1,
                  updated_at=statement_timestamp()
            WHERE singleton=TRUE AND row_version=$2"#,
    )
    .bind(target_tool.as_str())
    .bind(request.expected_tool_truth_row_version)
    .execute(&mut **tx)
    .await?;
    let investigation_updated = sqlx::query(
        r#"UPDATE investigation_rollout
              SET contract_version=$1,rollout_mode=$2,mode_rank=$3,
                  row_version=row_version+1,updated_at=statement_timestamp()
            WHERE singleton=TRUE AND row_version=$4"#,
    )
    .bind(target_contract.as_str())
    .bind(target_mode.as_str())
    .bind(target_mode.mode_rank())
    .bind(request.expected_investigation_row_version)
    .execute(&mut **tx)
    .await?;
    if tool_updated.rows_affected() != 1 || investigation_updated.rows_affected() != 1 {
        return Err(OperationRolloutError::Conflict {
            code: "OPERATION_PROMOTION_CAS_STALE",
        });
    }
    Ok(receipt)
}
