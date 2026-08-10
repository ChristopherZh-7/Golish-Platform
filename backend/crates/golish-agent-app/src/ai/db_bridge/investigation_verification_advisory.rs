//! Unified Investigation VerificationTask advisory compound.
//!
//! The PentAGI Primary supplies only typed cognitive strategy. This host
//! revalidates the exact task/plan/census/output/campaign authority, opens a
//! zero-consult Campaign round (there is no fixed role lane roster), seals the
//! Primary-selected closed capability strategy and invokes the existing
//! Prepared Action compiler for that exact capability only.
//! Oracle and FactDelta remain downstream of real Operator receipts.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use golish_agent_kit::db_traits::{
    ApplyInvestigationVerificationTaskAdvisory, InvestigationAnalysisHostError,
    InvestigationAnalysisHostResult, InvestigationVerificationApplyView, PersistStrategyDecision,
    ProposePreparedAction, ResumeInvestigationVerificationTaskAdvisory,
    SealCampaignCoverageDenominator, VerificationCampaignRepository,
    VerificationCampaignRepositoryError,
};
use golish_db::repo::verification_campaigns::{
    open_round_with_consult_census, OpenRoundWithConsultCensus,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::verification_campaign::PgVerificationCampaignRepository;

fn is_canonical_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

#[derive(sqlx::FromRow)]
struct CampaignAuthorityRow {
    campaign_id: Uuid,
    project_scope_id: Uuid,
    organization_id: Uuid,
    verification_objective_id: Uuid,
    verification_contract_id: Uuid,
    row_version: i64,
}

#[derive(sqlx::FromRow)]
struct CampaignReservationAuthorityRow {
    campaign_id: Uuid,
    plan_objective_id: Uuid,
    objective_id: Uuid,
    reservation_sha256: String,
    capability_assessment_set_sha256: String,
    campaign_authority_sha256: String,
    available_capability_ids: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct ExistingRoundRow {
    round_id: Uuid,
    expected_campaign_row_version: i64,
}

#[derive(sqlx::FromRow)]
struct FrozenAdvisoryHeaderRow {
    advisory_receipt_id: Uuid,
    stable_request_id: Uuid,
    envelope_sha256: String,
    status: String,
    campaign_member_count: i64,
    campaign_member_set_sha256: String,
    primary_residual_count: i64,
    primary_residual_set_sha256: String,
}

#[derive(sqlx::FromRow)]
struct ResumableAdvisoryHeaderRow {
    stable_request_id: Uuid,
    authority_id: Uuid,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    hypothesis_revision_id: Uuid,
    hypothesis_revision_sha256: String,
    verification_plan_id: Uuid,
    verification_plan_sha256: String,
    assignment_set_id: Uuid,
    assignment_set_sha256: String,
    campaign_denominator_sha256: String,
    subject_fingerprint_sha256: String,
    task_plan_id: Uuid,
    delegation_census_seal_id: Uuid,
    primary_worker_run_id: Uuid,
    accepted_output_count: i64,
    accepted_output_set_sha256: String,
    primary_residual_sha256: Vec<String>,
    primary_residual_count: i64,
    primary_residual_set_sha256: String,
}

#[derive(sqlx::FromRow)]
struct ResumableAdvisoryMemberRow {
    campaign_id: Uuid,
    verification_objective_id: Uuid,
    strategy_id: Uuid,
    capability_key: String,
    typed_strategy: serde_json::Value,
    intent_id: Uuid,
    typed_intent: serde_json::Value,
}

#[derive(sqlx::FromRow)]
struct DurableTaskPlanOutputRow {
    dispatch_receipt_id: Uuid,
    stage_work_item_id: Uuid,
    output_sha256: String,
}

#[derive(sqlx::FromRow)]
struct FrozenAdvisoryMemberRow {
    advisory_member_id: Uuid,
    campaign_id: Uuid,
    member_sha256: String,
}

#[derive(sqlx::FromRow)]
struct AdvisoryCampaignApplyRow {
    campaign_id: Uuid,
    round_id: Uuid,
    strategy_artifact_id: Uuid,
    strategy_obligation_id: Uuid,
    campaign_denominator_id: Uuid,
    campaign_coverage_member_id: Uuid,
    intent_id: Uuid,
    compiler_contract_version: String,
    compiler_input_sha256: String,
    compiler_result_authority_sha256: String,
    compiler_detail_sha256: Option<String>,
    result_kind: String,
    result_id: Uuid,
    result_sha256: String,
    apply_sha256: String,
}

#[derive(sqlx::FromRow)]
struct AppliedAdvisoryAuthorityRow {
    primary_residual_count: i64,
    primary_residual_set_sha256: String,
    envelope_sha256: String,
    campaign_member_count: i64,
    campaign_apply_count: i64,
    campaign_apply_set_sha256: String,
    prepared_action_count: i64,
    prepared_action_set_sha256: String,
    residual_count: i64,
    residual_set_sha256: String,
    seal_sha256: String,
}

struct FrozenAdvisory {
    receipt_id: Uuid,
    members: BTreeMap<Uuid, Uuid>,
    replayed: bool,
    applied: bool,
}

struct FrozenAdvisoryMemberBuild {
    advisory_member_id: Uuid,
    campaign_id: Uuid,
    plan_objective_id: Uuid,
    objective_id: Uuid,
    reservation_sha256: String,
    capability_assessment_set_sha256: String,
    strategy_id: Uuid,
    capability_key: String,
    typed_strategy: serde_json::Value,
    strategy_sha256: String,
    intent_id: Uuid,
    typed_intent: serde_json::Value,
    intent_sha256: String,
    member_sha256: String,
}

pub(super) async fn apply(
    pool: Arc<PgPool>,
    mut request: ApplyInvestigationVerificationTaskAdvisory,
) -> InvestigationAnalysisHostResult<InvestigationVerificationApplyView> {
    validate_envelope(&request)?;
    canonicalize_envelope(&mut request);
    validate_campaign_denominator(pool.as_ref(), &request).await?;
    validate_pentagi_authority(pool.as_ref(), &request).await?;
    let frozen = freeze_advisory_envelope(pool.as_ref(), &request).await?;
    if frozen.applied {
        return load_applied_advisory_view(pool.as_ref(), &request, frozen.receipt_id, true).await;
    }

    let campaign_ids = request
        .prepared_subject
        .campaigns
        .iter()
        .map(|campaign| campaign.campaign_id)
        .collect::<BTreeSet<_>>();
    let strategies = request
        .strategies
        .iter()
        .map(|strategy| (strategy.campaign_id, strategy))
        .collect::<BTreeMap<_, _>>();
    if strategies.keys().copied().collect::<BTreeSet<_>>() != campaign_ids {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "Verification strategy exact set differs from the task Campaign denominator"
                .to_owned(),
        });
    }

    let repository = PgVerificationCampaignRepository::new(pool.clone());
    let mut prepared_action_ids = Vec::new();
    let mut residual_receipt_ids = Vec::new();
    let mut all_replayed = frozen.replayed;
    for campaign_subject in &request.prepared_subject.campaigns {
        let campaign_id = campaign_subject.campaign_id;
        let strategy = strategies[&campaign_id];
        let intent = request
            .action_intents
            .iter()
            .find(|intent| intent.campaign_id == campaign_id)
            .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "frozen VerificationTask Campaign action intent is missing".to_owned(),
            })?;
        let authority = load_campaign_authority(pool.as_ref(), &request, campaign_subject).await?;
        if strategy.objective_id != campaign_subject.objective_id
            || strategy.objective_id != authority.verification_objective_id
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "Verification strategy objective is not the reserved Campaign objective"
                    .to_owned(),
            });
        }
        let round_request_id = Uuid::new_v5(
            &request.stable_request_id,
            format!("campaign:{campaign_id}:round").as_bytes(),
        );
        let existing_round = sqlx::query_as::<_, ExistingRoundRow>(
            r#"SELECT round_id,expected_campaign_row_version
                 FROM verification_campaign_rounds WHERE stable_request_id=$1"#,
        )
        .bind(round_request_id)
        .fetch_optional(pool.as_ref())
        .await
        .map_err(map_sqlx)?;
        let (round_id, campaign_row_version) = if let Some(round) = existing_round {
            (round.round_id, round.expected_campaign_row_version + 1)
        } else {
            all_replayed = false;
            let round = open_round_with_consult_census(
                pool.as_ref(),
                &OpenRoundWithConsultCensus {
                    stable_request_id: round_request_id,
                    campaign_id: authority.campaign_id,
                    operation_id: request.identity.stage.operation_id,
                    project_scope_id: authority.project_scope_id,
                    organization_id: authority.organization_id,
                    expected_campaign_row_version: authority.row_version,
                    round_input: serde_json::json!({
                        "schema":"investigation_verification_round.v1",
                        "verification_task_id":request.prepared_subject.verification_task_id,
                        "task_plan_id":request.task_plan_id,
                        "delegation_census_seal_id":request.delegation_census_seal_id,
                        "primary_worker_run_id":request.primary_worker_run_id,
                        "strategy_id":strategy.strategy_id,
                        "accepted_output_sha256":request.accepted_output_sha256,
                    }),
                    // Dynamic/nested PentAGI work is already sealed above. A
                    // Campaign round therefore has no fixed consult lanes.
                    consults: Vec::new(),
                },
            )
            .await
            .map_err(map_db)?;
            (round.round_id, authority.row_version + 1)
        };

        let strategy_body = serde_json::json!({
            "schema":"investigation_verification_strategy.v1",
            "advisory_request_id":request.stable_request_id,
            "strategy_id":strategy.strategy_id,
            "campaign_id":strategy.campaign_id,
            "objective_id":strategy.objective_id,
            "capability":super::investigation_analysis_host::verification_capability_name(strategy.capability),
            "purpose_code":strategy.purpose_code,
            "required_control_codes":strategy.required_control_codes,
            "evidence_authority_refs":strategy.evidence_authority_refs,
            "accepted_output_sha256":request.accepted_output_sha256,
            "action_intents":request.action_intents.iter()
                .filter(|intent| intent.strategy_id == strategy.strategy_id)
                .map(|intent| serde_json::json!({
                    "schema":"investigation_verification_action_intent.v1",
                    "intent_id":intent.intent_id,
                    "strategy_id":intent.strategy_id,
                    "campaign_id":intent.campaign_id,
                    "capability":super::investigation_analysis_host::verification_capability_name(intent.capability),
                    "purpose_code":intent.purpose_code,
                    "evidence_authority_refs":intent.evidence_authority_refs,
                }))
                .collect::<Vec<_>>(),
        });
        let strategy_hash: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(&strategy_body)
                .fetch_one(pool.as_ref())
                .await
                .map_err(map_sqlx)?;
        let obligation_hashes: Vec<String> = sqlx::query_scalar(
            r#"SELECT tool_truth_sha256(jsonb_build_object(
                       'semantic_key',member.semantic_key,
                       'expected_capability_kind',member.expected_capability_kind
                   )::TEXT)
                 FROM verification_wave_coverage_members member
                 JOIN verification_campaigns campaign
                   ON campaign.wave_denominator_id=member.wave_denominator_id
                  AND campaign.verification_objective_id=member.verification_objective_id
                WHERE campaign.campaign_id=$1
                  AND member.expected_capability_kind=$2
                ORDER BY member.member_ordinal"#,
        )
        .bind(campaign_id)
        .bind(super::investigation_analysis_host::verification_capability_name(strategy.capability))
        .fetch_all(pool.as_ref())
        .await
        .map_err(map_sqlx)?;
        if obligation_hashes.is_empty() {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "Primary selected a capability outside the Campaign available denominator"
                    .to_owned(),
            });
        }
        let obligation_set_hash: String = sqlx::query_scalar(
            "SELECT unified_investigation_exact_set_hash('investigation_verification_strategy_obligations.v1',$1::TEXT[])",
        )
        .bind(&obligation_hashes)
        .fetch_one(pool.as_ref())
        .await
        .map_err(map_sqlx)?;
        let strategy_request_id = Uuid::new_v5(
            &request.stable_request_id,
            format!("campaign:{campaign_id}:strategy").as_bytes(),
        );
        let strategy_artifact_id = Uuid::new_v5(&strategy_request_id, b"verification-strategy.v1");
        let strategy_existed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM verification_strategy_artifacts WHERE strategy_artifact_id=$1)",
        )
        .bind(strategy_artifact_id)
        .fetch_one(pool.as_ref())
        .await
        .map_err(map_sqlx)?;
        all_replayed &= strategy_existed;
        repository
            .persist_strategy_decision(PersistStrategyDecision {
                stable_request_id: strategy_request_id,
                operation_id: request.identity.stage.operation_id,
                campaign_id,
                round_id,
                strategy_decision_id: strategy.strategy_id,
                strategy_schema: "investigation_verification_strategy".to_owned(),
                strategy_version: 1,
                typed_strategy: strategy_body,
                strategy_hash,
                obligation_set_hash,
                expected_round_row_version: campaign_row_version,
            })
            .await
            .map_err(map_campaign)?;
        let coverage = repository
            .seal_coverage_denominator(SealCampaignCoverageDenominator {
                stable_request_id: Uuid::new_v5(
                    &request.stable_request_id,
                    format!("campaign:{campaign_id}:coverage").as_bytes(),
                ),
                operation_id: request.identity.stage.operation_id,
                campaign_id,
                round_id,
                objective_id: authority.verification_objective_id,
                verification_contract_id: authority.verification_contract_id,
                expected_campaign_row_version: campaign_row_version,
            })
            .await
            .map_err(map_campaign)?;

        let obligations: Vec<(Uuid, Uuid)> = sqlx::query_as(
            r#"SELECT obligation.obligation_id,member.campaign_coverage_member_id
                 FROM verification_strategy_obligations obligation
                 JOIN verification_campaign_coverage_members member
                   ON member.campaign_denominator_id=$2
                  AND member.semantic_key=obligation.semantic_key
                  AND member.expected_capability_kind=obligation.obligation_kind
                WHERE obligation.strategy_artifact_id=$1
                  AND obligation.disposition='planned'
                ORDER BY obligation.obligation_ordinal"#,
        )
        .bind(strategy_artifact_id)
        .bind(coverage.seal_id)
        .fetch_all(pool.as_ref())
        .await
        .map_err(map_sqlx)?;
        if obligations.is_empty() {
            return Err(InvestigationAnalysisHostError::SnapshotBlocked {
                detail: "frozen Verification strategy has no planned exact obligation".to_owned(),
            });
        }
        for (obligation_id, campaign_coverage_member_id) in obligations {
            if let Some(checkpoint) = load_campaign_apply_checkpoint(
                pool.as_ref(),
                frozen.receipt_id,
                campaign_id,
                obligation_id,
            )
            .await?
            {
                if checkpoint.round_id != round_id
                    || checkpoint.strategy_artifact_id != strategy_artifact_id
                    || checkpoint.campaign_denominator_id != coverage.seal_id
                    || checkpoint.campaign_coverage_member_id != campaign_coverage_member_id
                    || checkpoint.intent_id != intent.intent_id
                {
                    return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                        detail: "frozen VerificationTask obligation checkpoint identity drifted"
                            .to_owned(),
                    });
                }
                match checkpoint.result_kind.as_str() {
                    "prepared_action" => prepared_action_ids.push(checkpoint.result_id),
                    "residual" => residual_receipt_ids.push(checkpoint.result_id),
                    _ => {
                        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                            detail: "frozen VerificationTask obligation checkpoint is malformed"
                                .to_owned(),
                        });
                    }
                }
                continue;
            }
            all_replayed = false;
            let action_request_id = Uuid::new_v5(
                &request.stable_request_id,
                format!("campaign:{campaign_id}:obligation:{obligation_id}").as_bytes(),
            );
            let (result_kind, result_id, result_sha256) = match repository
                .propose_prepared_action(ProposePreparedAction {
                    stable_request_id: action_request_id,
                    operation_id: request.identity.stage.operation_id,
                    campaign_id,
                    round_id,
                    strategy_artifact_id,
                    strategy_obligation_id: obligation_id,
                })
                .await
            {
                Ok(action) => {
                    all_replayed &= action.replayed;
                    prepared_action_ids.push(action.prepared_action_id);
                    (
                        "prepared_action",
                        action.prepared_action_id,
                        action.private_manifest_hash,
                    )
                }
                Err(VerificationCampaignRepositoryError::InvalidRequest { detail }) => {
                    all_replayed = false;
                    let residual_id = persist_compile_residual(
                        pool.as_ref(),
                        &request,
                        campaign_id,
                        obligation_id,
                        &detail,
                    )
                    .await?;
                    let residual_sha256: String = sqlx::query_scalar(
                        "SELECT residual_hash FROM hypothesis_residual_risks WHERE residual_id=$1",
                    )
                    .bind(residual_id)
                    .fetch_one(pool.as_ref())
                    .await
                    .map_err(map_sqlx)?;
                    residual_receipt_ids.push(residual_id);
                    ("residual", residual_id, residual_sha256)
                }
                Err(error) => return Err(map_campaign(error)),
            };
            record_campaign_apply_checkpoint(
                pool.as_ref(),
                frozen.receipt_id,
                frozen.members[&campaign_id],
                campaign_id,
                round_id,
                strategy_artifact_id,
                obligation_id,
                coverage.seal_id,
                campaign_coverage_member_id,
                intent.intent_id,
                result_kind,
                result_id,
                &result_sha256,
            )
            .await?;
        }
    }
    prepared_action_ids.sort_unstable();
    prepared_action_ids.dedup();
    residual_receipt_ids.sort_unstable();
    residual_receipt_ids.dedup();
    let seal_replayed = seal_advisory_apply(pool.as_ref(), &request, frozen.receipt_id).await?;
    load_applied_advisory_view(
        pool.as_ref(),
        &request,
        frozen.receipt_id,
        all_replayed && seal_replayed,
    )
    .await
}

/// Resume the exact frozen AI envelope before the runtime dispatches another
/// model turn. No receipt means the normal cognitive path remains authoritative;
/// any receipt means replay/continuation is mandatory and drift fails closed.
pub(super) async fn resume(
    pool: Arc<PgPool>,
    request: ResumeInvestigationVerificationTaskAdvisory,
) -> InvestigationAnalysisHostResult<Option<InvestigationVerificationApplyView>> {
    let header = sqlx::query_as::<_, ResumableAdvisoryHeaderRow>(
        r#"SELECT stable_request_id,authority_id,operation_id,stage_execution_id,
                  stage_run_unit_id,scope_snapshot_id,organization_id,
                  hypothesis_revision_id,hypothesis_revision_sha256,
                  verification_plan_id,verification_plan_sha256,assignment_set_id,
                  assignment_set_sha256,campaign_denominator_sha256,
                  subject_fingerprint_sha256,task_plan_id,delegation_census_seal_id,
                  primary_worker_run_id,accepted_output_count,accepted_output_set_sha256,
                  primary_residual_sha256,primary_residual_count,
                  primary_residual_set_sha256
             FROM investigation_verification_task_advisory_receipts
            WHERE verification_task_id=$1"#,
    )
    .bind(request.prepared_subject.verification_task_id)
    .fetch_optional(pool.as_ref())
    .await
    .map_err(map_sqlx)?;
    let Some(header) = header else {
        return Ok(None);
    };
    if header.stable_request_id != request.stable_request_id
        || header.authority_id != request.identity.stage.authority_id
        || header.operation_id != request.identity.stage.operation_id
        || header.stage_execution_id != request.identity.stage.stage_execution_id
        || header.stage_run_unit_id != request.identity.stage_run_unit_id
        || header.scope_snapshot_id != request.identity.stage.scope_snapshot_id
        || header.organization_id != request.identity.organization_id
        || header.hypothesis_revision_id != request.prepared_subject.hypothesis_revision_id
        || header.hypothesis_revision_sha256 != request.prepared_subject.hypothesis_revision_sha256
        || header.verification_plan_id != request.prepared_subject.verification_plan_id
        || header.verification_plan_sha256 != request.prepared_subject.verification_plan_sha256
        || header.assignment_set_id != request.prepared_subject.assignment_set_id
        || header.assignment_set_sha256 != request.prepared_subject.assignment_set_sha256
        || header.campaign_denominator_sha256
            != request.prepared_subject.campaign_denominator_sha256
        || header.subject_fingerprint_sha256 != request.prepared_subject.subject_fingerprint_sha256
        || header.task_plan_id != request.task_plan_id
        || header.primary_worker_run_id != request.primary_worker_run_id
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "frozen VerificationTask advisory resume authority drifted".to_owned(),
        });
    }
    let accepted_output_sha256 =
        load_durable_task_plan_output_sha256(pool.as_ref(), header.task_plan_id).await?;
    let accepted_output_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_verification_accepted_outputs.v1',$1::TEXT[])",
    )
    .bind(&accepted_output_sha256)
    .fetch_one(pool.as_ref())
    .await
    .map_err(map_sqlx)?;
    if i64::try_from(accepted_output_sha256.len()).ok() != Some(header.accepted_output_count)
        || accepted_output_set_sha256 != header.accepted_output_set_sha256
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "frozen VerificationTask accepted output set drifted".to_owned(),
        });
    }
    let primary_residual_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_verification_primary_residuals.v1',$1::TEXT[])",
    )
    .bind(&header.primary_residual_sha256)
    .fetch_one(pool.as_ref())
    .await
    .map_err(map_sqlx)?;
    if i64::try_from(header.primary_residual_sha256.len()).ok()
        != Some(header.primary_residual_count)
        || primary_residual_set_sha256 != header.primary_residual_set_sha256
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "frozen VerificationTask Primary residual set drifted".to_owned(),
        });
    }
    let members = sqlx::query_as::<_, ResumableAdvisoryMemberRow>(
        r#"SELECT campaign_id,verification_objective_id,strategy_id,capability_key,
                  typed_strategy,intent_id,typed_intent
             FROM investigation_verification_task_advisory_members member
             JOIN investigation_verification_task_advisory_receipts receipt
               ON receipt.advisory_receipt_id=member.advisory_receipt_id
            WHERE receipt.verification_task_id=$1 ORDER BY member.member_ordinal"#,
    )
    .bind(request.prepared_subject.verification_task_id)
    .fetch_all(pool.as_ref())
    .await
    .map_err(map_sqlx)?;
    if members.is_empty() {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "frozen VerificationTask advisory has no members".to_owned(),
        });
    }
    let mut strategies = Vec::with_capacity(members.len());
    let mut action_intents = Vec::with_capacity(members.len());
    for member in members {
        let capability = resume_capability(&member.capability_key)?;
        strategies.push(
            golish_agent_kit::db_traits::InvestigationVerificationStrategyV1 {
                strategy_id: member.strategy_id,
                campaign_id: member.campaign_id,
                objective_id: member.verification_objective_id,
                capability,
                purpose_code: resume_json_string(&member.typed_strategy, "purpose_code")?,
                required_control_codes: resume_json_string_vec(
                    &member.typed_strategy,
                    "required_control_codes",
                )?,
                evidence_authority_refs: resume_json_string_vec(
                    &member.typed_strategy,
                    "evidence_authority_refs",
                )?,
            },
        );
        action_intents.push(
            golish_agent_kit::db_traits::InvestigationVerificationActionIntentV1 {
                intent_id: member.intent_id,
                strategy_id: member.strategy_id,
                campaign_id: member.campaign_id,
                capability,
                purpose_code: resume_json_string(&member.typed_intent, "purpose_code")?,
                evidence_authority_refs: resume_json_string_vec(
                    &member.typed_intent,
                    "evidence_authority_refs",
                )?,
            },
        );
    }
    apply(
        pool,
        ApplyInvestigationVerificationTaskAdvisory {
            stable_request_id: header.stable_request_id,
            identity: request.identity,
            prepared_subject: request.prepared_subject,
            task_plan_id: header.task_plan_id,
            delegation_census_seal_id: header.delegation_census_seal_id,
            primary_worker_run_id: header.primary_worker_run_id,
            accepted_output_sha256,
            primary_residual_sha256: header.primary_residual_sha256,
            strategies,
            action_intents,
        },
    )
    .await
    .map(Some)
}

fn resume_capability(
    capability: &str,
) -> InvestigationAnalysisHostResult<
    golish_agent_kit::db_traits::InvestigationVerificationCapabilityV1,
> {
    use golish_agent_kit::db_traits::InvestigationVerificationCapabilityV1;
    match capability {
        "verify.anonymous_authenticated_differential.v1" => {
            Ok(InvestigationVerificationCapabilityV1::AnonymousAuthenticatedDifferential)
        }
        "verify.directory_fingerprint.v1" => {
            Ok(InvestigationVerificationCapabilityV1::DirectoryFingerprint)
        }
        "verify.nuclei_exact_replay.v1" => {
            Ok(InvestigationVerificationCapabilityV1::NucleiExactReplay)
        }
        "verify.concurrent_race_differential.v1" => {
            Ok(InvestigationVerificationCapabilityV1::ConcurrentRaceDifferential)
        }
        _ => Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "frozen VerificationTask capability is not in the closed registry".to_owned(),
        }),
    }
}

fn resume_json_string(
    value: &serde_json::Value,
    key: &str,
) -> InvestigationAnalysisHostResult<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
            detail: format!("frozen VerificationTask advisory field {key} is malformed"),
        })
}

fn resume_json_string_vec(
    value: &serde_json::Value,
    key: &str,
) -> InvestigationAnalysisHostResult<Vec<String>> {
    serde_json::from_value(value.get(key).cloned().ok_or_else(|| {
        InvestigationAnalysisHostError::AuthorityMismatch {
            detail: format!("frozen VerificationTask advisory field {key} is absent"),
        }
    })?)
    .map_err(|_| InvestigationAnalysisHostError::AuthorityMismatch {
        detail: format!("frozen VerificationTask advisory field {key} is malformed"),
    })
}

fn canonicalize_envelope(request: &mut ApplyInvestigationVerificationTaskAdvisory) {
    request.accepted_output_sha256.sort();
    request.accepted_output_sha256.dedup();
    request.primary_residual_sha256.sort();
    request.primary_residual_sha256.dedup();
    request.prepared_subject.campaign_ids.sort_unstable();
    request.prepared_subject.campaign_ids.dedup();
    request
        .prepared_subject
        .campaigns
        .sort_by_key(|campaign| campaign.campaign_id);
    for campaign in &mut request.prepared_subject.campaigns {
        campaign.available_capability_ids.sort();
        campaign.available_capability_ids.dedup();
    }
    request
        .strategies
        .sort_by_key(|strategy| (strategy.campaign_id, strategy.strategy_id));
    for strategy in &mut request.strategies {
        strategy.required_control_codes.sort();
        strategy.required_control_codes.dedup();
        strategy.evidence_authority_refs.sort();
        strategy.evidence_authority_refs.dedup();
    }
    request
        .action_intents
        .sort_by_key(|intent| (intent.campaign_id, intent.strategy_id, intent.intent_id));
    for intent in &mut request.action_intents {
        intent.evidence_authority_refs.sort();
        intent.evidence_authority_refs.dedup();
    }
}

async fn freeze_advisory_envelope(
    pool: &PgPool,
    request: &ApplyInvestigationVerificationTaskAdvisory,
) -> InvestigationAnalysisHostResult<FrozenAdvisory> {
    let receipt_id = Uuid::new_v5(
        &request.stable_request_id,
        b"investigation-verification-task-advisory.v1",
    );
    let strategies = request
        .strategies
        .iter()
        .map(|strategy| (strategy.campaign_id, strategy))
        .collect::<BTreeMap<_, _>>();
    let intents = request
        .action_intents
        .iter()
        .map(|intent| (intent.campaign_id, intent))
        .collect::<BTreeMap<_, _>>();
    let mut subjects = request
        .prepared_subject
        .campaigns
        .iter()
        .collect::<Vec<_>>();
    subjects.sort_by_key(|subject| subject.campaign_id);
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    // Serialize the first freeze for this exact VerificationTask before the
    // existence probe. READ COMMITTED then observes a winner that committed
    // while this caller waited, so concurrent first writers converge through
    // the normal frozen-envelope replay path instead of surfacing a unique-key
    // infrastructure error.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(request.prepared_subject.verification_task_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    let mut members = Vec::with_capacity(subjects.len());
    for subject in subjects {
        let strategy = strategies.get(&subject.campaign_id).ok_or_else(|| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "frozen advisory is missing one Campaign strategy".to_owned(),
            }
        })?;
        let intent = intents.get(&subject.campaign_id).ok_or_else(|| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "frozen advisory is missing one Campaign action intent".to_owned(),
            }
        })?;
        let capability_key =
            super::investigation_analysis_host::verification_capability_name(strategy.capability)
                .to_owned();
        let mut required_control_codes = strategy.required_control_codes.clone();
        required_control_codes.sort();
        required_control_codes.dedup();
        let mut strategy_evidence_refs = strategy.evidence_authority_refs.clone();
        strategy_evidence_refs.sort();
        strategy_evidence_refs.dedup();
        let mut intent_evidence_refs = intent.evidence_authority_refs.clone();
        intent_evidence_refs.sort();
        intent_evidence_refs.dedup();
        let typed_strategy = serde_json::json!({
            "schema":"investigation_verification_strategy.v1",
            "advisory_request_id":request.stable_request_id,
            "strategy_id":strategy.strategy_id,
            "campaign_id":strategy.campaign_id,
            "objective_id":strategy.objective_id,
            "capability":capability_key,
            "purpose_code":strategy.purpose_code,
            "required_control_codes":required_control_codes,
            "evidence_authority_refs":strategy_evidence_refs,
            "accepted_output_sha256":request.accepted_output_sha256,
            "action_intents":request.action_intents.iter()
                .filter(|candidate| candidate.strategy_id == strategy.strategy_id)
                .map(|candidate| serde_json::json!({
                    "schema":"investigation_verification_action_intent.v1",
                    "intent_id":candidate.intent_id,
                    "strategy_id":candidate.strategy_id,
                    "campaign_id":candidate.campaign_id,
                    "capability":super::investigation_analysis_host::verification_capability_name(candidate.capability),
                    "purpose_code":candidate.purpose_code,
                    "evidence_authority_refs":candidate.evidence_authority_refs,
                }))
                .collect::<Vec<_>>(),
        });
        let typed_intent = serde_json::json!({
            "schema":"investigation_verification_action_intent.v1",
            "intent_id":intent.intent_id,
            "strategy_id":intent.strategy_id,
            "campaign_id":intent.campaign_id,
            "capability":capability_key,
            "purpose_code":intent.purpose_code,
            "evidence_authority_refs":intent_evidence_refs,
        });
        let strategy_sha256: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(&typed_strategy)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        let intent_sha256: String =
            sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
                .bind(&typed_intent)
                .fetch_one(&mut *tx)
                .await
                .map_err(map_sqlx)?;
        let member_sha256: String = sqlx::query_scalar(
            r#"SELECT tool_truth_sha256(jsonb_build_object(
                   'contract_version','investigation-verification-advisory-member.v1',
                   'campaign_id',$1,'plan_objective_id',$2,'objective_id',$3,
                   'reservation_sha256',$4,'capability_assessment_set_sha256',$5,
                   'strategy_sha256',$6,'intent_sha256',$7
               )::TEXT)"#,
        )
        .bind(subject.campaign_id)
        .bind(subject.plan_objective_id)
        .bind(subject.objective_id)
        .bind(&subject.reservation_sha256)
        .bind(&subject.capability_assessment_set_sha256)
        .bind(&strategy_sha256)
        .bind(&intent_sha256)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        members.push(FrozenAdvisoryMemberBuild {
            advisory_member_id: Uuid::new_v5(&receipt_id, subject.campaign_id.as_bytes()),
            campaign_id: subject.campaign_id,
            plan_objective_id: subject.plan_objective_id,
            objective_id: subject.objective_id,
            reservation_sha256: subject.reservation_sha256.clone(),
            capability_assessment_set_sha256: subject.capability_assessment_set_sha256.clone(),
            strategy_id: strategy.strategy_id,
            capability_key,
            typed_strategy,
            strategy_sha256,
            intent_id: intent.intent_id,
            typed_intent,
            intent_sha256,
            member_sha256,
        });
    }
    let mut accepted_outputs = request.accepted_output_sha256.clone();
    accepted_outputs.sort();
    accepted_outputs.dedup();
    let accepted_output_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_verification_accepted_outputs.v1',$1::TEXT[])",
    )
    .bind(&accepted_outputs)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let primary_residuals = request.primary_residual_sha256.clone();
    let primary_residual_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_verification_primary_residuals.v1',$1::TEXT[])",
    )
    .bind(&primary_residuals)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let member_hashes = members
        .iter()
        .map(|member| member.member_sha256.clone())
        .collect::<Vec<_>>();
    let campaign_member_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_verification_advisory_members.v1',$1::TEXT[])",
    )
    .bind(&member_hashes)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let envelope_material = serde_json::json!({
        "contract_version":"investigation-verification-task-advisory.v1",
        "authority_id":request.identity.stage.authority_id,
        "operation_id":request.identity.stage.operation_id,
        "stage_execution_id":request.identity.stage.stage_execution_id,
        "stage_run_unit_id":request.identity.stage_run_unit_id,
        "scope_snapshot_id":request.identity.stage.scope_snapshot_id,
        "organization_id":request.identity.organization_id,
        "verification_task_id":request.prepared_subject.verification_task_id,
        "hypothesis_revision_id":request.prepared_subject.hypothesis_revision_id,
        "hypothesis_revision_sha256":request.prepared_subject.hypothesis_revision_sha256,
        "verification_plan_id":request.prepared_subject.verification_plan_id,
        "verification_plan_sha256":request.prepared_subject.verification_plan_sha256,
        "assignment_set_id":request.prepared_subject.assignment_set_id,
        "assignment_set_sha256":request.prepared_subject.assignment_set_sha256,
        "campaign_denominator_sha256":request.prepared_subject.campaign_denominator_sha256,
        "subject_fingerprint_sha256":request.prepared_subject.subject_fingerprint_sha256,
        "task_plan_id":request.task_plan_id,
        "delegation_census_seal_id":request.delegation_census_seal_id,
        "primary_worker_run_id":request.primary_worker_run_id,
        "accepted_output_set_sha256":accepted_output_set_sha256,
        "primary_residual_count":primary_residuals.len(),
        "primary_residual_set_sha256":primary_residual_set_sha256,
        "campaign_member_set_sha256":campaign_member_set_sha256,
    });
    let envelope_sha256: String = sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
        .bind(envelope_material)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    let existing = sqlx::query_as::<_, FrozenAdvisoryHeaderRow>(
        r#"SELECT advisory_receipt_id,stable_request_id,envelope_sha256,status,
                  campaign_member_count,campaign_member_set_sha256,
                  primary_residual_count,primary_residual_set_sha256
             FROM investigation_verification_task_advisory_receipts
            WHERE verification_task_id=$1 OR stable_request_id=$2 FOR UPDATE"#,
    )
    .bind(request.prepared_subject.verification_task_id)
    .bind(request.stable_request_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    if let Some(existing) = existing {
        if existing.advisory_receipt_id != receipt_id
            || existing.stable_request_id != request.stable_request_id
            || existing.envelope_sha256 != envelope_sha256
            || existing.campaign_member_count != i64::try_from(members.len()).unwrap_or(-1)
            || existing.campaign_member_set_sha256 != campaign_member_set_sha256
            || existing.primary_residual_count
                != i64::try_from(primary_residuals.len()).unwrap_or(-1)
            || existing.primary_residual_set_sha256 != primary_residual_set_sha256
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "VerificationTask advisory aggregate replay drifted".to_owned(),
            });
        }
        let stored = sqlx::query_as::<_, FrozenAdvisoryMemberRow>(
            r#"SELECT advisory_member_id,campaign_id,member_sha256
                 FROM investigation_verification_task_advisory_members
                WHERE advisory_receipt_id=$1 ORDER BY campaign_id FOR SHARE"#,
        )
        .bind(receipt_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        if stored.len() != members.len()
            || stored.iter().zip(&members).any(|(left, right)| {
                left.advisory_member_id != right.advisory_member_id
                    || left.campaign_id != right.campaign_id
                    || left.member_sha256 != right.member_sha256
            })
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "VerificationTask advisory frozen member set drifted".to_owned(),
            });
        }
        tx.commit().await.map_err(map_sqlx)?;
        return Ok(FrozenAdvisory {
            receipt_id,
            members: stored
                .into_iter()
                .map(|member| (member.campaign_id, member.advisory_member_id))
                .collect(),
            replayed: true,
            applied: existing.status == "applied",
        });
    }
    let unbound_artifacts: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM hypothesis_verification_task_campaigns reservation
               JOIN verification_campaign_rounds round
                 ON round.campaign_id=reservation.campaign_id
              WHERE reservation.task_id=$1
           )"#,
    )
    .bind(request.prepared_subject.verification_task_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    if unbound_artifacts {
        return Err(InvestigationAnalysisHostError::SnapshotBlocked {
            detail: "VerificationTask has unbound Campaign artifacts without an aggregate advisory receipt"
                .to_owned(),
        });
    }
    sqlx::query(
        r#"INSERT INTO investigation_verification_task_advisory_receipts(
               advisory_receipt_id,stable_request_id,verification_task_id,authority_id,
               operation_id,stage_execution_id,stage_run_unit_id,scope_snapshot_id,
               organization_id,hypothesis_revision_id,hypothesis_revision_sha256,
               verification_plan_id,verification_plan_sha256,assignment_set_id,
               assignment_set_sha256,campaign_denominator_sha256,
               subject_fingerprint_sha256,task_plan_id,delegation_census_seal_id,
               primary_worker_run_id,accepted_output_count,accepted_output_set_sha256,
               primary_residual_sha256,primary_residual_count,primary_residual_set_sha256,
               campaign_member_count,campaign_member_set_sha256,envelope_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,
                  $18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28)"#,
    )
    .bind(receipt_id)
    .bind(request.stable_request_id)
    .bind(request.prepared_subject.verification_task_id)
    .bind(request.identity.stage.authority_id)
    .bind(request.identity.stage.operation_id)
    .bind(request.identity.stage.stage_execution_id)
    .bind(request.identity.stage_run_unit_id)
    .bind(request.identity.stage.scope_snapshot_id)
    .bind(request.identity.organization_id)
    .bind(request.prepared_subject.hypothesis_revision_id)
    .bind(&request.prepared_subject.hypothesis_revision_sha256)
    .bind(request.prepared_subject.verification_plan_id)
    .bind(&request.prepared_subject.verification_plan_sha256)
    .bind(request.prepared_subject.assignment_set_id)
    .bind(&request.prepared_subject.assignment_set_sha256)
    .bind(&request.prepared_subject.campaign_denominator_sha256)
    .bind(&request.prepared_subject.subject_fingerprint_sha256)
    .bind(request.task_plan_id)
    .bind(request.delegation_census_seal_id)
    .bind(request.primary_worker_run_id)
    .bind(i64::try_from(accepted_outputs.len()).unwrap_or(-1))
    .bind(&accepted_output_set_sha256)
    .bind(&primary_residuals)
    .bind(i64::try_from(primary_residuals.len()).unwrap_or(-1))
    .bind(&primary_residual_set_sha256)
    .bind(i64::try_from(members.len()).unwrap_or(-1))
    .bind(&campaign_member_set_sha256)
    .bind(&envelope_sha256)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    for (ordinal, member) in members.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO investigation_verification_task_advisory_members(
                   advisory_member_id,advisory_receipt_id,verification_task_id,
                   assignment_set_id,member_ordinal,campaign_id,plan_objective_id,
                   verification_objective_id,reservation_sha256,
                   capability_assessment_set_sha256,strategy_id,capability_key,
                   typed_strategy,strategy_sha256,intent_id,typed_intent,
                   intent_sha256,member_sha256)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
        )
        .bind(member.advisory_member_id)
        .bind(receipt_id)
        .bind(request.prepared_subject.verification_task_id)
        .bind(request.prepared_subject.assignment_set_id)
        .bind(i32::try_from(ordinal).unwrap_or(-1))
        .bind(member.campaign_id)
        .bind(member.plan_objective_id)
        .bind(member.objective_id)
        .bind(&member.reservation_sha256)
        .bind(&member.capability_assessment_set_sha256)
        .bind(member.strategy_id)
        .bind(&member.capability_key)
        .bind(&member.typed_strategy)
        .bind(&member.strategy_sha256)
        .bind(member.intent_id)
        .bind(&member.typed_intent)
        .bind(&member.intent_sha256)
        .bind(&member.member_sha256)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }
    tx.commit().await.map_err(map_sqlx)?;
    Ok(FrozenAdvisory {
        receipt_id,
        members: members
            .into_iter()
            .map(|member| (member.campaign_id, member.advisory_member_id))
            .collect(),
        replayed: false,
        applied: false,
    })
}

async fn load_campaign_apply_checkpoint(
    pool: &PgPool,
    receipt_id: Uuid,
    campaign_id: Uuid,
    strategy_obligation_id: Uuid,
) -> InvestigationAnalysisHostResult<Option<AdvisoryCampaignApplyRow>> {
    sqlx::query_as::<_, AdvisoryCampaignApplyRow>(
        r#"SELECT campaign_id,round_id,strategy_artifact_id,strategy_obligation_id,
                  campaign_denominator_id,campaign_coverage_member_id,intent_id,
                  compiler_contract_version,compiler_input_sha256,
                  compiler_result_authority_sha256,compiler_detail_sha256,
                  result_kind,result_id,result_sha256,apply_sha256
             FROM investigation_verification_advisory_campaign_applies
            WHERE advisory_receipt_id=$1 AND campaign_id=$2
              AND strategy_obligation_id=$3"#,
    )
    .bind(receipt_id)
    .bind(campaign_id)
    .bind(strategy_obligation_id)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)
}

#[allow(clippy::too_many_arguments)]
async fn record_campaign_apply_checkpoint(
    pool: &PgPool,
    receipt_id: Uuid,
    member_id: Uuid,
    campaign_id: Uuid,
    round_id: Uuid,
    strategy_artifact_id: Uuid,
    strategy_obligation_id: Uuid,
    campaign_denominator_id: Uuid,
    campaign_coverage_member_id: Uuid,
    intent_id: Uuid,
    result_kind: &str,
    result_id: Uuid,
    result_sha256: &str,
) -> InvestigationAnalysisHostResult<()> {
    let stable_request_id = Uuid::new_v5(
        &receipt_id,
        format!("campaign:{campaign_id}:obligation:{strategy_obligation_id}").as_bytes(),
    );
    let apply_receipt_id = Uuid::new_v5(
        &stable_request_id,
        b"investigation-verification-campaign-apply.v1",
    );
    let compiler_contract_version = "investigation-verification-action-compiler.v1";
    let compiler_input_sha256: String = sqlx::query_scalar(
        "SELECT investigation_verification_action_compiler_input_sha256_v1($1,$2,$3,$4,$5)",
    )
    .bind(member_id)
    .bind(strategy_artifact_id)
    .bind(strategy_obligation_id)
    .bind(campaign_denominator_id)
    .bind(campaign_coverage_member_id)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    let compiler_result_authority_sha256: String = sqlx::query_scalar(
        "SELECT investigation_verification_action_compiler_result_sha256_v1($1,$2)",
    )
    .bind(result_kind)
    .bind(result_id)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    let compiler_detail_sha256: Option<String> = if result_kind == "residual" {
        sqlx::query_scalar(
            "SELECT substr(affected_inputs->>3,length('compiler_detail_sha256:')+1)
               FROM hypothesis_residual_risks WHERE residual_id=$1",
        )
        .bind(result_id)
        .fetch_optional(pool)
        .await
        .map_err(map_sqlx)?
        .flatten()
    } else {
        None
    };
    let apply_material = serde_json::json!({
        "contract_version":"investigation-verification-campaign-apply.v1",
        "advisory_receipt_id":receipt_id,
        "advisory_member_id":member_id,
        "campaign_id":campaign_id,
        "round_id":round_id,
        "strategy_artifact_id":strategy_artifact_id,
        "strategy_obligation_id":strategy_obligation_id,
        "campaign_denominator_id":campaign_denominator_id,
        "campaign_coverage_member_id":campaign_coverage_member_id,
        "intent_id":intent_id,
        "compiler_contract_version":compiler_contract_version,
        "compiler_input_sha256":compiler_input_sha256,
        "compiler_result_authority_sha256":compiler_result_authority_sha256,
        "compiler_detail_sha256":compiler_detail_sha256,
        "result_kind":result_kind,
        "result_id":result_id,
        "result_sha256":result_sha256,
    });
    let apply_sha256: String = sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
        .bind(apply_material)
        .fetch_one(pool)
        .await
        .map_err(map_sqlx)?;
    let inserted = sqlx::query(
        r#"INSERT INTO investigation_verification_advisory_campaign_applies(
               campaign_apply_receipt_id,stable_request_id,advisory_receipt_id,
               advisory_member_id,campaign_id,round_id,strategy_artifact_id,
               strategy_obligation_id,campaign_denominator_id,
               campaign_coverage_member_id,intent_id,compiler_contract_version,
               compiler_input_sha256,compiler_result_authority_sha256,
               compiler_detail_sha256,result_kind,result_id,result_sha256,apply_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)
           ON CONFLICT(advisory_receipt_id,campaign_id,strategy_obligation_id)
           DO NOTHING"#,
    )
    .bind(apply_receipt_id)
    .bind(stable_request_id)
    .bind(receipt_id)
    .bind(member_id)
    .bind(campaign_id)
    .bind(round_id)
    .bind(strategy_artifact_id)
    .bind(strategy_obligation_id)
    .bind(campaign_denominator_id)
    .bind(campaign_coverage_member_id)
    .bind(intent_id)
    .bind(compiler_contract_version)
    .bind(&compiler_input_sha256)
    .bind(&compiler_result_authority_sha256)
    .bind(&compiler_detail_sha256)
    .bind(result_kind)
    .bind(result_id)
    .bind(result_sha256)
    .bind(&apply_sha256)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    if inserted.rows_affected() == 0 {
        let existing =
            load_campaign_apply_checkpoint(pool, receipt_id, campaign_id, strategy_obligation_id)
                .await?
                .ok_or_else(|| InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "VerificationTask Campaign checkpoint conflict lost its row".to_owned(),
                })?;
        if existing.round_id != round_id
            || existing.strategy_artifact_id != strategy_artifact_id
            || existing.strategy_obligation_id != strategy_obligation_id
            || existing.campaign_denominator_id != campaign_denominator_id
            || existing.campaign_coverage_member_id != campaign_coverage_member_id
            || existing.intent_id != intent_id
            || existing.compiler_contract_version != compiler_contract_version
            || existing.compiler_input_sha256 != compiler_input_sha256
            || existing.compiler_result_authority_sha256 != compiler_result_authority_sha256
            || existing.compiler_detail_sha256 != compiler_detail_sha256
            || existing.result_kind != result_kind
            || existing.result_id != result_id
            || existing.result_sha256 != result_sha256
            || existing.apply_sha256 != apply_sha256
        {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "VerificationTask Campaign apply checkpoint replay drifted".to_owned(),
            });
        }
    }
    Ok(())
}

async fn seal_advisory_apply(
    pool: &PgPool,
    request: &ApplyInvestigationVerificationTaskAdvisory,
    receipt_id: Uuid,
) -> InvestigationAnalysisHostResult<bool> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    let header = sqlx::query_as::<_, FrozenAdvisoryHeaderRow>(
        r#"SELECT advisory_receipt_id,stable_request_id,envelope_sha256,status,
                  campaign_member_count,campaign_member_set_sha256,
                  primary_residual_count,primary_residual_set_sha256
             FROM investigation_verification_task_advisory_receipts
            WHERE advisory_receipt_id=$1 AND verification_task_id=$2 FOR UPDATE"#,
    )
    .bind(receipt_id)
    .bind(request.prepared_subject.verification_task_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let applies = sqlx::query_as::<_, AdvisoryCampaignApplyRow>(
        r#"SELECT campaign_id,round_id,strategy_artifact_id,strategy_obligation_id,
                  campaign_denominator_id,campaign_coverage_member_id,intent_id,
                  compiler_contract_version,compiler_input_sha256,
                  compiler_result_authority_sha256,compiler_detail_sha256,
                  result_kind,result_id,result_sha256,apply_sha256
             FROM investigation_verification_advisory_campaign_applies
            WHERE advisory_receipt_id=$1
            ORDER BY campaign_id,strategy_obligation_id FOR SHARE"#,
    )
    .bind(receipt_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let applied_campaign_count = applies
        .iter()
        .map(|apply| apply.campaign_id)
        .collect::<BTreeSet<_>>()
        .len();
    let planned_obligation_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM investigation_verification_task_advisory_members member
             JOIN (SELECT DISTINCT advisory_receipt_id,advisory_member_id,campaign_id,
                                   strategy_artifact_id
                     FROM investigation_verification_advisory_campaign_applies
                    WHERE advisory_receipt_id=$1) representative
               ON representative.advisory_receipt_id=member.advisory_receipt_id
              AND representative.advisory_member_id=member.advisory_member_id
              AND representative.campaign_id=member.campaign_id
             JOIN verification_strategy_obligations obligation
               ON obligation.strategy_artifact_id=representative.strategy_artifact_id
              AND obligation.disposition='planned'
            WHERE member.advisory_receipt_id=$1"#,
    )
    .bind(receipt_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    if i64::try_from(applies.len()).ok() != Some(planned_obligation_count)
        || i64::try_from(applied_campaign_count).ok() != Some(header.campaign_member_count)
    {
        return Err(InvestigationAnalysisHostError::SnapshotBlocked {
            detail: "VerificationTask advisory cannot seal with pending Campaign applies"
                .to_owned(),
        });
    }
    let apply_hashes = applies
        .iter()
        .map(|apply| apply.apply_sha256.clone())
        .collect::<Vec<_>>();
    let action_hashes = applies
        .iter()
        .filter(|apply| apply.result_kind == "prepared_action")
        .map(|apply| apply.result_sha256.clone())
        .collect::<Vec<_>>();
    let residual_hashes = applies
        .iter()
        .filter(|apply| apply.result_kind == "residual")
        .map(|apply| apply.result_sha256.clone())
        .collect::<Vec<_>>();
    let campaign_apply_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_verification_campaign_applies.v1',$1::TEXT[])",
    )
    .bind(&apply_hashes)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let prepared_action_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_verification_prepared_actions.v1',$1::TEXT[])",
    )
    .bind(&action_hashes)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let residual_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('investigation_verification_residuals.v1',$1::TEXT[])",
    )
    .bind(&residual_hashes)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let seal_material = serde_json::json!({
        "contract_version":"investigation-verification-task-advisory-seal.v1",
        "advisory_receipt_id":receipt_id,
        "verification_task_id":request.prepared_subject.verification_task_id,
        "envelope_sha256":header.envelope_sha256,
        "campaign_apply_set_sha256":campaign_apply_set_sha256,
        "prepared_action_set_sha256":prepared_action_set_sha256,
        "residual_set_sha256":residual_set_sha256,
    });
    let seal_sha256: String = sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
        .bind(seal_material)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    let seal_id = Uuid::new_v5(&receipt_id, b"investigation-verification-advisory-seal.v1");
    let existing: Option<(Uuid, String)> = sqlx::query_as(
        r#"SELECT advisory_seal_id,seal_sha256
             FROM investigation_verification_task_advisory_seals
            WHERE advisory_receipt_id=$1 FOR SHARE"#,
    )
    .bind(receipt_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    if let Some((existing_id, existing_hash)) = existing {
        if existing_id != seal_id || existing_hash != seal_sha256 || header.status != "applied" {
            return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "VerificationTask advisory aggregate seal replay drifted".to_owned(),
            });
        }
        tx.commit().await.map_err(map_sqlx)?;
        return Ok(true);
    }
    sqlx::query(
        r#"INSERT INTO investigation_verification_task_advisory_seals(
               advisory_seal_id,stable_request_id,advisory_receipt_id,
               verification_task_id,campaign_apply_count,campaign_apply_set_sha256,
               prepared_action_count,prepared_action_set_sha256,residual_count,
               residual_set_sha256,seal_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
    )
    .bind(seal_id)
    .bind(Uuid::new_v5(
        &receipt_id,
        b"investigation-verification-advisory-seal:request",
    ))
    .bind(receipt_id)
    .bind(request.prepared_subject.verification_task_id)
    .bind(i64::try_from(applies.len()).unwrap_or(-1))
    .bind(&campaign_apply_set_sha256)
    .bind(i64::try_from(action_hashes.len()).unwrap_or(-1))
    .bind(&prepared_action_set_sha256)
    .bind(i64::try_from(residual_hashes.len()).unwrap_or(-1))
    .bind(&residual_set_sha256)
    .bind(&seal_sha256)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    sqlx::query(
        r#"UPDATE investigation_verification_task_advisory_receipts
              SET status='applied',row_version=row_version+1,
                  applied_at=statement_timestamp()
            WHERE advisory_receipt_id=$1 AND status='building'"#,
    )
    .bind(receipt_id)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    tx.commit().await.map_err(map_sqlx)?;
    Ok(false)
}

async fn load_applied_advisory_view(
    pool: &PgPool,
    request: &ApplyInvestigationVerificationTaskAdvisory,
    receipt_id: Uuid,
    replayed: bool,
) -> InvestigationAnalysisHostResult<InvestigationVerificationApplyView> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    let header = sqlx::query_as::<_, AppliedAdvisoryAuthorityRow>(
        "SELECT receipt.primary_residual_count,receipt.primary_residual_set_sha256,
                receipt.envelope_sha256,receipt.campaign_member_count,
                seal.campaign_apply_count,seal.campaign_apply_set_sha256,
                seal.prepared_action_count,seal.prepared_action_set_sha256,
                seal.residual_count,seal.residual_set_sha256,seal.seal_sha256
           FROM investigation_verification_task_advisory_receipts receipt
           JOIN investigation_verification_task_advisory_seals seal
             ON seal.advisory_receipt_id=receipt.advisory_receipt_id
            AND seal.verification_task_id=receipt.verification_task_id
          WHERE receipt.advisory_receipt_id=$1 AND receipt.verification_task_id=$2
            AND receipt.status='applied'",
    )
    .bind(receipt_id)
    .bind(request.prepared_subject.verification_task_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let Some(header) = header else {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "applied VerificationTask advisory is missing its aggregate seal".to_owned(),
        });
    };
    let checkpoints = sqlx::query_as::<_, AdvisoryCampaignApplyRow>(
        r#"SELECT campaign_id,round_id,strategy_artifact_id,strategy_obligation_id,
                  campaign_denominator_id,campaign_coverage_member_id,intent_id,
                  compiler_contract_version,compiler_input_sha256,
                  compiler_result_authority_sha256,compiler_detail_sha256,
                  result_kind,result_id,result_sha256,apply_sha256
             FROM investigation_verification_advisory_campaign_applies
            WHERE advisory_receipt_id=$1
            ORDER BY campaign_id,strategy_obligation_id"#,
    )
    .bind(receipt_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    let apply_hashes = checkpoints
        .iter()
        .map(|checkpoint| checkpoint.apply_sha256.clone())
        .collect::<Vec<_>>();
    let action_hashes = checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.result_kind == "prepared_action")
        .map(|checkpoint| checkpoint.result_sha256.clone())
        .collect::<Vec<_>>();
    let residual_hashes = checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.result_kind == "residual")
        .map(|checkpoint| checkpoint.result_sha256.clone())
        .collect::<Vec<_>>();
    let campaign_apply_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash(
             'investigation_verification_campaign_applies.v1',$1::TEXT[])",
    )
    .bind(&apply_hashes)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    let prepared_action_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash(
             'investigation_verification_prepared_actions.v1',$1::TEXT[])",
    )
    .bind(&action_hashes)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    let residual_set_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash(
             'investigation_verification_residuals.v1',$1::TEXT[])",
    )
    .bind(&residual_hashes)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    let expected_seal_sha256: String =
        sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
            .bind(serde_json::json!({
                "contract_version":"investigation-verification-task-advisory-seal.v1",
                "advisory_receipt_id":receipt_id,
                "verification_task_id":request.prepared_subject.verification_task_id,
                "envelope_sha256":header.envelope_sha256,
                "campaign_apply_set_sha256":campaign_apply_set_sha256,
                "prepared_action_set_sha256":prepared_action_set_sha256,
                "residual_set_sha256":residual_set_sha256,
            }))
            .fetch_one(&mut *tx)
            .await
            .map_err(map_sqlx)?;
    let planned_obligation_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM investigation_verification_task_advisory_members member
             JOIN (SELECT DISTINCT advisory_receipt_id,advisory_member_id,campaign_id,
                                   strategy_artifact_id
                     FROM investigation_verification_advisory_campaign_applies
                    WHERE advisory_receipt_id=$1) representative
               ON representative.advisory_receipt_id=member.advisory_receipt_id
              AND representative.advisory_member_id=member.advisory_member_id
              AND representative.campaign_id=member.campaign_id
             JOIN verification_strategy_obligations obligation
               ON obligation.strategy_artifact_id=representative.strategy_artifact_id
              AND obligation.disposition='planned'
            WHERE member.advisory_receipt_id=$1"#,
    )
    .bind(receipt_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_sqlx)?;
    let checkpoint_campaign_count = checkpoints
        .iter()
        .map(|checkpoint| checkpoint.campaign_id)
        .collect::<BTreeSet<_>>()
        .len();
    if i64::try_from(checkpoints.len()).ok() != Some(planned_obligation_count)
        || i64::try_from(checkpoint_campaign_count).ok() != Some(header.campaign_member_count)
        || header.campaign_apply_count != planned_obligation_count
        || header.campaign_apply_set_sha256 != campaign_apply_set_sha256
        || i64::try_from(action_hashes.len()).ok() != Some(header.prepared_action_count)
        || header.prepared_action_set_sha256 != prepared_action_set_sha256
        || i64::try_from(residual_hashes.len()).ok() != Some(header.residual_count)
        || header.residual_set_sha256 != residual_set_sha256
        || header.seal_sha256 != expected_seal_sha256
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "applied VerificationTask advisory aggregate seal drifted".to_owned(),
        });
    }
    let mut campaign_ids = Vec::with_capacity(checkpoints.len());
    let mut prepared_action_ids = Vec::new();
    let mut residual_receipt_ids = Vec::new();
    for checkpoint in checkpoints {
        campaign_ids.push(checkpoint.campaign_id);
        match checkpoint.result_kind.as_str() {
            "prepared_action" => prepared_action_ids.push(checkpoint.result_id),
            "residual" => residual_receipt_ids.push(checkpoint.result_id),
            _ => {
                return Err(InvestigationAnalysisHostError::AuthorityMismatch {
                    detail: "applied VerificationTask advisory checkpoint is malformed".to_owned(),
                })
            }
        }
    }
    campaign_ids.sort_unstable();
    campaign_ids.dedup();
    prepared_action_ids.sort_unstable();
    prepared_action_ids.dedup();
    residual_receipt_ids.sort_unstable();
    residual_receipt_ids.dedup();
    tx.commit().await.map_err(map_sqlx)?;
    Ok(InvestigationVerificationApplyView {
        verification_task_id: request.prepared_subject.verification_task_id,
        campaign_ids,
        prepared_action_ids,
        residual_receipt_ids,
        primary_residual_count: u32::try_from(header.primary_residual_count).map_err(|_| {
            InvestigationAnalysisHostError::AuthorityMismatch {
                detail: "applied VerificationTask Primary residual count overflowed".to_owned(),
            }
        })?,
        primary_residual_set_sha256: header.primary_residual_set_sha256,
        fact_delta_bundle_ids: Vec::new(),
        replayed,
    })
}

fn validate_envelope(
    request: &ApplyInvestigationVerificationTaskAdvisory,
) -> InvestigationAnalysisHostResult<()> {
    let valid_hash = |value: &str| {
        value.len() == 71
            && value.starts_with("sha256:")
            && value[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    let mut outputs = BTreeSet::new();
    let mut primary_residuals = BTreeSet::new();
    let mut strategy_ids = BTreeSet::new();
    let mut campaign_ids = BTreeSet::new();
    let prepared_campaign_ids = request
        .prepared_subject
        .campaign_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mapped_campaign_ids = request
        .prepared_subject
        .campaigns
        .iter()
        .map(|campaign| campaign.campaign_id)
        .collect::<BTreeSet<_>>();
    if request.prepared_subject.verification_task_id.is_nil()
        || request.prepared_subject.campaign_ids.is_empty()
        || request.prepared_subject.campaign_ids.len() != prepared_campaign_ids.len()
        || request.prepared_subject.campaigns.len() != mapped_campaign_ids.len()
        || prepared_campaign_ids != mapped_campaign_ids
        || request.prepared_subject.campaigns.iter().any(|campaign| {
            campaign.campaign_id.is_nil()
                || campaign.plan_objective_id.is_nil()
                || campaign.objective_id.is_nil()
                || !valid_hash(&campaign.reservation_sha256)
                || !valid_hash(&campaign.capability_assessment_set_sha256)
                || campaign.available_capability_ids.is_empty()
                || campaign
                    .available_capability_ids
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || campaign.available_capability_ids.iter().any(|capability| {
                    !matches!(
                        capability.as_str(),
                        "verify.anonymous_authenticated_differential.v1"
                            | "verify.directory_fingerprint.v1"
                            | "verify.nuclei_exact_replay.v1"
                            | "verify.concurrent_race_differential.v1"
                    )
                })
        })
        || request.accepted_output_sha256.is_empty()
        || request
            .accepted_output_sha256
            .iter()
            .any(|hash| !valid_hash(hash) || !outputs.insert(hash))
        || request
            .primary_residual_sha256
            .iter()
            .any(|hash| !valid_hash(hash) || !primary_residuals.insert(hash))
        || request.strategies.is_empty()
        || request.strategies.iter().any(|strategy| {
            strategy.strategy_id.is_nil()
                || strategy.campaign_id.is_nil()
                || strategy.objective_id.is_nil()
                || !strategy_ids.insert(strategy.strategy_id)
                || !campaign_ids.insert(strategy.campaign_id)
                || strategy.purpose_code.trim().is_empty()
                || strategy
                    .required_control_codes
                    .iter()
                    .any(|code| code.trim().is_empty())
                || strategy
                    .evidence_authority_refs
                    .iter()
                    .any(|hash| !valid_hash(hash))
        })
    {
        return Err(InvestigationAnalysisHostError::InvalidRequest {
            detail: "VerificationTask strategy/output envelope is invalid".to_owned(),
        });
    }
    let strategies = request
        .strategies
        .iter()
        .map(|strategy| (strategy.strategy_id, strategy))
        .collect::<BTreeMap<_, _>>();
    let prepared_campaigns = request
        .prepared_subject
        .campaigns
        .iter()
        .map(|campaign| (campaign.campaign_id, campaign))
        .collect::<BTreeMap<_, _>>();
    if request.strategies.iter().any(|strategy| {
        prepared_campaigns
            .get(&strategy.campaign_id)
            .is_none_or(|campaign| {
                !campaign.available_capability_ids.iter().any(|capability| {
                    capability
                        == super::investigation_analysis_host::verification_capability_name(
                            strategy.capability,
                        )
                })
            })
    }) {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "Verification strategy selected a capability absent from the exact available catalog"
                .to_owned(),
        });
    }
    let mut intent_ids = BTreeSet::new();
    let mut intent_strategy_ids = BTreeSet::new();
    if request.action_intents.is_empty()
        || request.action_intents.iter().any(|intent| {
            let Some(strategy) = strategies.get(&intent.strategy_id) else {
                return true;
            };
            intent.intent_id.is_nil()
                || !intent_ids.insert(intent.intent_id)
                || !intent_strategy_ids.insert(intent.strategy_id)
                || intent.campaign_id != strategy.campaign_id
                || intent.capability != strategy.capability
                || intent.purpose_code.trim().is_empty()
                || intent
                    .evidence_authority_refs
                    .iter()
                    .any(|hash| !valid_hash(hash))
        })
        || intent_strategy_ids != strategy_ids
    {
        return Err(InvestigationAnalysisHostError::InvalidRequest {
            detail: "VerificationTask action intent is not strategy-bound".to_owned(),
        });
    }
    Ok(())
}

async fn validate_campaign_denominator(
    pool: &PgPool,
    request: &ApplyInvestigationVerificationTaskAdvisory,
) -> InvestigationAnalysisHostResult<()> {
    let durable = sqlx::query_as::<_, CampaignReservationAuthorityRow>(
        r#"SELECT reservation.campaign_id,reservation.plan_objective_id,
                  reservation.verification_objective_id AS objective_id,
                  reservation.reservation_sha256,
                  assessment_set.member_set_hash AS capability_assessment_set_sha256,
                  unified_investigation_campaign_authority_sha256_v4(
                      reservation.campaign_id,reservation.reservation_sha256
                  ) AS campaign_authority_sha256,
                  ARRAY(
                      SELECT assessment.capability_key
                        FROM verification_campaigns campaign
                        JOIN verification_capability_assessment_set_members member
                          ON member.assessment_set_seal_id=campaign.capability_assessment_set_seal_id
                        JOIN verification_capability_assessments assessment
                          ON assessment.assessment_id=member.assessment_id
                       WHERE campaign.campaign_id=reservation.campaign_id
                         AND campaign.operation_id=$3
                         AND assessment.status='available'
                       ORDER BY assessment.capability_key
                  ) AS available_capability_ids
             FROM hypothesis_verification_task_campaigns reservation
             JOIN verification_campaigns materialized
               ON materialized.campaign_id=reservation.campaign_id
              AND materialized.operation_id=$3
              AND materialized.organization_id=$4
              AND materialized.state IN ('admitted','running')
              AND materialized.terminal_at IS NULL
              AND materialized.superseded_at IS NULL
              AND materialized.effective_valid_until>statement_timestamp()
             JOIN verification_capability_assessment_set_seals assessment_set
               ON assessment_set.assessment_set_seal_id=
                  materialized.capability_assessment_set_seal_id
              AND assessment_set.sealed_at IS NOT NULL
            WHERE reservation.task_id=$1 AND reservation.assignment_set_id=$2
            ORDER BY reservation.campaign_id"#,
    )
    .bind(request.prepared_subject.verification_task_id)
    .bind(request.prepared_subject.assignment_set_id)
    .bind(request.identity.stage.operation_id)
    .bind(request.identity.organization_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    let durable_by_campaign = durable
        .iter()
        .map(|campaign| {
            (
                campaign.campaign_id,
                (
                    campaign.plan_objective_id,
                    campaign.objective_id,
                    campaign.reservation_sha256.as_str(),
                    campaign.capability_assessment_set_sha256.as_str(),
                    campaign.available_capability_ids.as_slice(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let prepared_by_campaign = request
        .prepared_subject
        .campaigns
        .iter()
        .map(|campaign| {
            (
                campaign.campaign_id,
                (
                    campaign.plan_objective_id,
                    campaign.objective_id,
                    campaign.reservation_sha256.as_str(),
                    campaign.capability_assessment_set_sha256.as_str(),
                    campaign.available_capability_ids.as_slice(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if durable_by_campaign != prepared_by_campaign {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "VerificationTask Campaign/objective reservation exact set drifted".to_owned(),
        });
    }
    if durable
        .iter()
        .any(|campaign| !is_canonical_sha256(&campaign.campaign_authority_sha256))
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "VerificationTask Campaign authority hash is malformed".to_owned(),
        });
    }
    let campaign_authority_hashes = durable
        .iter()
        .map(|campaign| campaign.campaign_authority_sha256.clone())
        .collect::<Vec<_>>();
    let durable_denominator_sha256: String = sqlx::query_scalar(
        "SELECT unified_investigation_exact_set_hash('verification_task_campaigns.v4',$1::TEXT[])",
    )
    .bind(campaign_authority_hashes)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    if durable_denominator_sha256 != request.prepared_subject.campaign_denominator_sha256 {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "VerificationTask Campaign denominator hash drifted".to_owned(),
        });
    }
    Ok(())
}

async fn validate_pentagi_authority(
    pool: &PgPool,
    request: &ApplyInvestigationVerificationTaskAdvisory,
) -> InvestigationAnalysisHostResult<()> {
    let valid: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM investigation_pentagi_task_plans plan
               JOIN investigation_pentagi_delegation_census_seals census
                 ON census.census_seal_id=$10 AND census.task_plan_id=plan.task_plan_id
                AND census.primary_worker_run_id=$11
              WHERE plan.task_plan_id=$9 AND plan.authority_id=$1
                AND plan.operation_id=$2 AND plan.stage_execution_id=$3
                AND plan.stage_run_unit_id=$4 AND plan.scope_snapshot_id=$5
                AND plan.organization_id=$6 AND plan.subject_kind='verification_task'
                AND plan.subject_id=$7 AND plan.subject_fingerprint_sha256=$8
                AND plan.status='sealed'
                AND EXISTS(SELECT 1 FROM investigation_pentagi_pipeline_events event
                            WHERE event.task_plan_id=plan.task_plan_id
                              AND event.event_kind='primary_synthesis'
                              AND event.actor_worker_run_id=$11))"#,
    )
    .bind(request.identity.stage.authority_id)
    .bind(request.identity.stage.operation_id)
    .bind(request.identity.stage.stage_execution_id)
    .bind(request.identity.stage_run_unit_id)
    .bind(request.identity.stage.scope_snapshot_id)
    .bind(request.identity.organization_id)
    .bind(request.prepared_subject.verification_task_id)
    .bind(&request.prepared_subject.subject_fingerprint_sha256)
    .bind(request.task_plan_id)
    .bind(request.delegation_census_seal_id)
    .bind(request.primary_worker_run_id)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    if !valid {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "VerificationTask PentAGI plan/census/Primary authority drifted".to_owned(),
        });
    }
    let durable_outputs = load_durable_task_plan_output_sha256(pool, request.task_plan_id).await?;
    if durable_outputs.into_iter().collect::<BTreeSet<_>>()
        != request
            .accepted_output_sha256
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "accepted VerificationTask output exact set drifted".to_owned(),
        });
    }
    Ok(())
}

async fn load_durable_task_plan_output_sha256(
    pool: &PgPool,
    task_plan_id: Uuid,
) -> InvestigationAnalysisHostResult<Vec<String>> {
    let expected_dispatch_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM pentagi_logical_dispatch_receipts dispatch
            WHERE dispatch.task_plan_id=$1
              AND dispatch.actor_kind IN ('worker','nested_worker')"#,
    )
    .bind(task_plan_id)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    let rows = sqlx::query_as::<_, DurableTaskPlanOutputRow>(
        r#"SELECT dispatch.dispatch_receipt_id,dispatch.stage_work_item_id,
                  output.output_hash AS output_sha256
             FROM pentagi_logical_dispatch_receipts dispatch
             JOIN investigation_pentagi_task_plans task_plan
               ON task_plan.task_plan_id=dispatch.task_plan_id
              AND task_plan.operation_id=dispatch.operation_id
              AND task_plan.stage_execution_id=dispatch.stage_execution_id
              AND task_plan.stage_run_unit_id=dispatch.stage_run_unit_id
              AND task_plan.scope_snapshot_id=dispatch.scope_snapshot_id
              AND task_plan.organization_id=dispatch.organization_id
             JOIN stage_team_plans team_plan
               ON team_plan.id=task_plan.stage_team_plan_id
              AND team_plan.operation_id=task_plan.operation_id
              AND team_plan.stage_execution_id=task_plan.stage_execution_id
              AND team_plan.stage_run_unit_id=task_plan.stage_run_unit_id
              AND team_plan.scope_snapshot_id=task_plan.scope_snapshot_id
              AND team_plan.organization_id=task_plan.organization_id
             JOIN stage_work_items item
               ON item.id=dispatch.stage_work_item_id
              AND item.team_plan_id=task_plan.stage_team_plan_id
              AND item.operation_id=task_plan.operation_id
              AND item.stage_execution_id=task_plan.stage_execution_id
              AND item.stage_run_unit_id=task_plan.stage_run_unit_id
              AND item.scope_snapshot_id=task_plan.scope_snapshot_id
              AND item.organization_id=task_plan.organization_id
              AND item.terminal_at IS NOT NULL
             JOIN stage_worker_outputs output
               ON output.work_item_id=dispatch.stage_work_item_id
              AND output.team_plan_id=task_plan.stage_team_plan_id
              AND output.operation_id=task_plan.operation_id
              AND output.stage_execution_id=task_plan.stage_execution_id
              AND output.stage_run_unit_id=task_plan.stage_run_unit_id
              AND output.scope_snapshot_id=task_plan.scope_snapshot_id
              AND output.organization_id=task_plan.organization_id
             JOIN stage_worker_runs output_worker
               ON output_worker.id=output.worker_run_id
              AND output_worker.work_item_id=dispatch.stage_work_item_id
              AND output_worker.operation_id=task_plan.operation_id
              AND output_worker.stage_execution_id=task_plan.stage_execution_id
              AND output_worker.stage_run_unit_id=task_plan.stage_run_unit_id
              AND output_worker.organization_id=task_plan.organization_id
              AND output_worker.terminal_at IS NOT NULL
              AND output_worker.active_tool_call_id IS NULL
              AND (
                  (output_worker.status='passed' AND item.status='completed')
                  OR (
                      output_worker.status='failed'
                      AND item.status='exhausted'
                      AND output.business_disposition='blocked'
                  )
              )
            WHERE dispatch.task_plan_id=$1
              AND dispatch.actor_kind IN ('worker','nested_worker')
            ORDER BY dispatch.dispatch_ordinal,dispatch.dispatch_receipt_id"#,
    )
    .bind(task_plan_id)
    .fetch_all(pool)
    .await
    .map_err(map_sqlx)?;
    let receipt_count = rows
        .iter()
        .map(|row| row.dispatch_receipt_id)
        .collect::<BTreeSet<_>>()
        .len();
    let work_item_count = rows
        .iter()
        .map(|row| row.stage_work_item_id)
        .collect::<BTreeSet<_>>()
        .len();
    if i64::try_from(rows.len()).ok() != Some(expected_dispatch_count)
        || receipt_count != rows.len()
        || work_item_count != rows.len()
        || rows
            .iter()
            .any(|row| !is_canonical_sha256(&row.output_sha256))
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "VerificationTask durable output manifest is missing or foreign".to_owned(),
        });
    }
    let mut output_sha256 = rows
        .into_iter()
        .map(|row| row.output_sha256)
        .collect::<Vec<_>>();
    output_sha256.sort();
    Ok(output_sha256)
}

async fn load_campaign_authority(
    pool: &PgPool,
    request: &ApplyInvestigationVerificationTaskAdvisory,
    campaign_subject: &golish_agent_kit::db_traits::InvestigationVerificationCampaignSubjectV1,
) -> InvestigationAnalysisHostResult<CampaignAuthorityRow> {
    sqlx::query_as::<_, CampaignAuthorityRow>(
        r#"SELECT campaign.campaign_id,campaign.project_scope_id,campaign.organization_id,
                  campaign.verification_objective_id,campaign.verification_contract_id,
                  campaign.row_version
             FROM hypothesis_verification_task_campaigns reservation
            JOIN verification_campaigns campaign ON campaign.campaign_id=reservation.campaign_id
            JOIN verification_capability_assessment_set_seals assessment_set
              ON assessment_set.assessment_set_seal_id=
                 campaign.capability_assessment_set_seal_id
             AND assessment_set.sealed_at IS NOT NULL
            WHERE reservation.task_id=$1 AND reservation.assignment_set_id=$2
              AND reservation.campaign_id=$3 AND reservation.plan_objective_id=$4
              AND reservation.verification_objective_id=$5
              AND reservation.reservation_sha256=$6 AND campaign.operation_id=$7
              AND campaign.organization_id=$8 AND campaign.state IN ('admitted','running')
              AND assessment_set.member_set_hash=$9
              AND campaign.terminal_at IS NULL AND campaign.superseded_at IS NULL"#,
    )
    .bind(request.prepared_subject.verification_task_id)
    .bind(request.prepared_subject.assignment_set_id)
    .bind(campaign_subject.campaign_id)
    .bind(campaign_subject.plan_objective_id)
    .bind(campaign_subject.objective_id)
    .bind(&campaign_subject.reservation_sha256)
    .bind(request.identity.stage.operation_id)
    .bind(request.identity.organization_id)
    .bind(&campaign_subject.capability_assessment_set_sha256)
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)?
    .ok_or_else(|| InvestigationAnalysisHostError::SnapshotBlocked {
        detail: "reserved VerificationTask Campaign is not admitted and current".to_owned(),
    })
}

async fn persist_compile_residual(
    pool: &PgPool,
    request: &ApplyInvestigationVerificationTaskAdvisory,
    campaign_id: Uuid,
    obligation_id: Uuid,
    compiler_detail: &str,
) -> InvestigationAnalysisHostResult<Uuid> {
    let residual_id = Uuid::new_v5(
        &request.stable_request_id,
        format!("campaign:{campaign_id}:obligation:{obligation_id}:residual").as_bytes(),
    );
    let compiler_detail_sha256: String =
        sqlx::query_scalar::<_, String>("SELECT tool_truth_sha256($1)")
            .bind(compiler_detail)
            .fetch_one(pool)
            .await
            .map_err(map_sqlx)?;
    let affected_inputs = serde_json::json!([
        format!(
            "verification_task:{}",
            request.prepared_subject.verification_task_id
        ),
        format!("campaign:{campaign_id}"),
        format!("strategy_obligation:{obligation_id}"),
        format!("compiler_detail_sha256:{compiler_detail_sha256}"),
    ]);
    let next_action = serde_json::json!({
        "kind":"verification_strategy_refinement_required",
        "retry":false,
    });
    let residual_hash: String = sqlx::query_scalar("SELECT tool_truth_sha256(($1::JSONB)::TEXT)")
        .bind(serde_json::json!({
        "reason_code":"investigation_verification_action_not_compilable",
        "affected_inputs":affected_inputs,
            "next_action":next_action,
            "compiler_detail_sha256":compiler_detail_sha256,
        }))
        .fetch_one(pool)
        .await
        .map_err(map_sqlx)?;
    sqlx::query(
        r#"INSERT INTO hypothesis_residual_risks(
               residual_id,operation_id,organization_id,revision_id,reason_code,
               owner_kind,affected_inputs,next_action,residual_hash)
           VALUES($1,$2,$3,$4,'investigation_verification_action_not_compilable',
                  'plan_c',$5,$6,$7)
           ON CONFLICT(residual_id) DO NOTHING"#,
    )
    .bind(residual_id)
    .bind(request.identity.stage.operation_id)
    .bind(request.identity.organization_id)
    .bind(request.prepared_subject.hypothesis_revision_id)
    .bind(&affected_inputs)
    .bind(&next_action)
    .bind(&residual_hash)
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    let stored: (
        Uuid,
        Uuid,
        Option<Uuid>,
        String,
        String,
        serde_json::Value,
        serde_json::Value,
        String,
    ) = sqlx::query_as(
        r#"SELECT operation_id,organization_id,revision_id,reason_code,owner_kind,
                  affected_inputs,next_action,residual_hash
             FROM hypothesis_residual_risks WHERE residual_id=$1"#,
    )
    .bind(residual_id)
    .fetch_one(pool)
    .await
    .map_err(map_sqlx)?;
    if stored.0 != request.identity.stage.operation_id
        || stored.1 != request.identity.organization_id
        || stored.2 != Some(request.prepared_subject.hypothesis_revision_id)
        || stored.3 != "investigation_verification_action_not_compilable"
        || stored.4 != "plan_c"
        || stored.5 != affected_inputs
        || stored.6 != next_action
        || stored.7 != residual_hash
    {
        return Err(InvestigationAnalysisHostError::AuthorityMismatch {
            detail: "Verification action compile residual replay drifted".to_owned(),
        });
    }
    Ok(residual_id)
}

fn map_sqlx(error: sqlx::Error) -> InvestigationAnalysisHostError {
    InvestigationAnalysisHostError::Infrastructure {
        detail: error.to_string(),
    }
}

fn map_db(error: golish_db::DbError) -> InvestigationAnalysisHostError {
    let detail = error.to_string();
    if detail.contains("AUTHORITY") || detail.contains("STALE") {
        InvestigationAnalysisHostError::AuthorityMismatch { detail }
    } else if detail.contains("INVALID") {
        InvestigationAnalysisHostError::InvalidRequest { detail }
    } else if detail.contains("REPLAY") || detail.contains("CONFLICT") {
        InvestigationAnalysisHostError::Conflict { detail }
    } else {
        InvestigationAnalysisHostError::Infrastructure { detail }
    }
}

fn map_campaign(error: VerificationCampaignRepositoryError) -> InvestigationAnalysisHostError {
    let detail = error.to_string();
    match error {
        VerificationCampaignRepositoryError::Unavailable { operation } => {
            InvestigationAnalysisHostError::Unavailable { operation }
        }
        VerificationCampaignRepositoryError::InvalidRequest { .. } => {
            InvestigationAnalysisHostError::InvalidRequest { detail }
        }
        VerificationCampaignRepositoryError::NotFound { .. } => {
            InvestigationAnalysisHostError::NotFound { detail }
        }
        VerificationCampaignRepositoryError::Conflict { .. } => {
            InvestigationAnalysisHostError::Conflict { detail }
        }
        VerificationCampaignRepositoryError::AuthorityMismatch { .. } => {
            InvestigationAnalysisHostError::AuthorityMismatch { detail }
        }
        VerificationCampaignRepositoryError::Infrastructure { .. } => {
            InvestigationAnalysisHostError::Infrastructure { detail }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn advisory_apply_reads_every_from_row_authority_column() {
        let source = include_str!("investigation_verification_advisory.rs");
        let seal = source
            .split("async fn seal_advisory_apply")
            .nth(1)
            .and_then(|tail| tail.split("async fn load_applied_advisory_view").next())
            .expect("advisory seal implementation is bounded");
        for header_column in ["primary_residual_count", "primary_residual_set_sha256"] {
            assert!(
                seal.contains(header_column),
                "advisory seal header omits {header_column}"
            );
        }
        for checkpoint_column in [
            "compiler_contract_version",
            "compiler_input_sha256",
            "compiler_result_authority_sha256",
            "compiler_detail_sha256",
        ] {
            assert!(
                seal.contains(checkpoint_column),
                "advisory seal checkpoint omits {checkpoint_column}"
            );
        }

        let applied = source
            .split("async fn load_applied_advisory_view")
            .nth(1)
            .and_then(|tail| tail.split("fn validate_envelope").next())
            .expect("applied advisory loader is bounded");
        for checkpoint_column in [
            "compiler_contract_version",
            "compiler_input_sha256",
            "compiler_result_authority_sha256",
            "compiler_detail_sha256",
        ] {
            assert!(
                applied.contains(checkpoint_column),
                "applied advisory checkpoint omits {checkpoint_column}"
            );
        }
    }
}
