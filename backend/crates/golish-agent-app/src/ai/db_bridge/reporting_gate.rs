//! DB-authoritative adapter for the pure Reporting stage gate contract.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;

use golish_agent_kit::harness::reporting_gate::{
    validate_reporting_gate_truth, ReportingGateTruth,
};
use golish_reporting_app::ReportReadModelBuilder;
use golish_reporting_domain::{ReportClaim, ReportClaimKind, ReportValidationResult};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::reporting::{
    cleanup_closeout_truth_on, current_reportable_source_snapshot_on, frozen_organization_ids_on,
    load_report_bundle_on, load_report_gate_integrity_on, load_reporting_project_authority,
    lock_reporting_project_authority, PgReportTruthPort, ReportingProjectAuthority,
};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn claim_kind(value: &str) -> Option<ReportClaimKind> {
    match value {
        "scope" => Some(ReportClaimKind::Scope),
        "finding" => Some(ReportClaimKind::Finding),
        "candidate_disposition" => Some(ReportClaimKind::CandidateDisposition),
        "technique_outcome" => Some(ReportClaimKind::TechniqueOutcome),
        "attack_path" => Some(ReportClaimKind::AttackPath),
        "objective_outcome" => Some(ReportClaimKind::ObjectiveOutcome),
        "cleanup_residual" => Some(ReportClaimKind::CleanupResidual),
        "limitation" => Some(ReportClaimKind::Limitation),
        _ => None,
    }
}

fn contains_forbidden_secret_key(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "password" | "secret" | "token" | "api_key" | "private_key" | "cookie"
            ) || contains_forbidden_secret_key(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_forbidden_secret_key),
        _ => false,
    }
}

pub(super) fn stored_claim_hashes_are_valid(
    claims: &[golish_db::repo::report_claims::ReportClaimRow],
    citations: &[golish_db::repo::report_claim_citations::ReportClaimCitationRow],
) -> anyhow::Result<bool> {
    let mut citation_ids = BTreeMap::<Uuid, Vec<(i32, Uuid)>>::new();
    for citation in citations {
        citation_ids
            .entry(citation.claim_id)
            .or_default()
            .push((citation.citation_ordinal, citation.citation_id));
    }
    for ids in citation_ids.values_mut() {
        ids.sort_unstable_by_key(|(ordinal, id)| (*ordinal, *id));
    }
    for row in claims {
        let Some(kind) = claim_kind(&row.claim_kind) else {
            return Ok(false);
        };
        if contains_forbidden_secret_key(&row.object_value) {
            return Ok(false);
        }
        let claim = ReportClaim {
            claim_id: row.claim_id,
            revision_id: row.revision_id,
            section_id: row.section_id,
            organization_id_at_time: row.organization_id_at_time,
            claim_kind: kind,
            subject_ref: row.subject_ref.clone(),
            predicate: row.predicate.clone(),
            value: row.object_value.clone(),
            citation_ids: citation_ids
                .remove(&row.claim_id)
                .unwrap_or_default()
                .into_iter()
                .map(|(_, id)| id)
                .collect(),
            ordinal: row.ordinal,
        };
        if hex(&Sha256::digest(serde_json::to_vec(&claim)?)) != row.claim_hash {
            return Ok(false);
        }
    }
    Ok(citation_ids.is_empty())
}

pub async fn load_reporting_gate_truth(
    pool: &Arc<PgPool>,
    operation_id: Uuid,
) -> anyhow::Result<Option<ReportingGateTruth>> {
    load_reporting_gate_truth_with_barrier(pool, operation_id, || async {}).await
}

pub(super) async fn load_reporting_gate_truth_with_barrier<F, Fut>(
    pool: &Arc<PgPool>,
    operation_id: Uuid,
    after_bundle: F,
) -> anyhow::Result<Option<ReportingGateTruth>>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = ()> + Send,
{
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let Some(bundle) = load_report_bundle_on(&mut tx, operation_id).await? else {
        tx.commit().await?;
        return Ok(None);
    };
    after_bundle().await;
    let Some(revision) = bundle.current_revision.as_ref() else {
        tx.commit().await?;
        return Ok(None);
    };
    let Some(stored_snapshot) = bundle.source_snapshot.as_ref() else {
        tx.commit().await?;
        return Ok(None);
    };
    let current_snapshot = current_reportable_source_snapshot_on(&mut tx, operation_id).await?;
    let source_snapshot_exact = stored_snapshot.ordered_sources == current_snapshot.ordered_sources
        && stored_snapshot.source_set_hash == current_snapshot.source_set_hash
        && revision.source_set_hash == hex(&stored_snapshot.source_set_hash);

    let integrity = load_report_gate_integrity_on(
        &mut tx,
        revision.revision_id,
        operation_id,
        bundle.report.scope_snapshot_id,
    )
    .await?;

    let validation_result = revision
        .validation_result
        .clone()
        .map(serde_json::from_value::<ReportValidationResult>)
        .transpose()?;
    let claim_hashes_valid = stored_claim_hashes_are_valid(&bundle.claims, &bundle.citations)?;
    let validation_attestation_valid = validation_result.as_ref().is_some_and(|result| {
        result.revision_id == revision.revision_id
            && i64::try_from(result.claim_count).ok() == Some(integrity.claim_count)
            && i64::try_from(result.citation_count).ok() == Some(integrity.citation_count)
            && i64::try_from(result.source_count).ok() == Some(integrity.source_count)
            && revision.validated_at.is_some()
            && claim_hashes_valid
    });
    let claims_citations_valid = integrity.uncited_claim_count == 0
        && integrity.invalid_citation_count == 0
        && integrity.invalid_blocked_residual_count == 0
        && integrity.invalid_technique_claim_count == 0
        && integrity.out_of_scope_section_count == 0;

    let organization_ids = frozen_organization_ids_on(&mut tx, operation_id).await?;
    let cleanup = cleanup_closeout_truth_on(&mut tx, operation_id, &organization_ids).await?;
    let disclosed_residuals = bundle
        .claims
        .iter()
        .filter(|claim| claim.claim_kind == "cleanup_residual")
        .filter_map(|claim| {
            claim
                .subject_ref
                .strip_prefix("cleanup_obligation:")
                .and_then(|value| Uuid::parse_str(value).ok())
        })
        .collect::<BTreeSet<_>>();
    let cleanup_closeout_valid = cleanup.iter().all(|truth| {
        truth.missing_obligation_count == 0
            && truth.nonterminal_obligation_count == 0
            && truth.undisclosed_residual_count == 0
            && truth.invalid_terminal_truth_count == 0
            && truth
                .residual_obligation_ids
                .is_subset(&disclosed_residuals)
    });

    let truth = ReportingGateTruth {
        operation_id,
        report_id: bundle.report.report_id,
        current_revision_id: bundle
            .report
            .current_revision_id
            .ok_or_else(|| anyhow::anyhow!("report_current_revision_missing"))?,
        revision_id: revision.revision_id,
        validation_status: revision.validation_status.clone(),
        publication_status: revision.publication_status.clone(),
        stored_source_set_hash: revision.source_set_hash.clone(),
        current_source_set_hash: hex(&current_snapshot.source_set_hash),
        source_snapshot_exact,
        claims_citations_valid,
        validation_attestation_valid,
        cleanup_closeout_valid,
    };
    tx.commit().await?;
    Ok(Some(truth))
}

pub async fn build_or_reuse_validated_report(
    pool: Arc<PgPool>,
    operation_id: Uuid,
) -> anyhow::Result<ReportingGateTruth> {
    let authority = load_reporting_project_authority(&pool, operation_id).await?;
    build_or_reuse_validated_report_on(pool, operation_id, authority).await
}

pub async fn build_or_reuse_validated_report_with_project_authority(
    pool: Arc<PgPool>,
    operation_id: Uuid,
    authority: ReportingProjectAuthority,
) -> anyhow::Result<ReportingGateTruth> {
    build_or_reuse_validated_report_on(pool, operation_id, authority).await
}

async fn build_or_reuse_validated_report_on(
    pool: Arc<PgPool>,
    operation_id: Uuid,
    authority: ReportingProjectAuthority,
) -> anyhow::Result<ReportingGateTruth> {
    lock_reporting_project_authority(&pool, operation_id, &authority).await?;
    if let Some(current) = load_reporting_gate_truth(&pool, operation_id).await? {
        if validate_reporting_gate_truth(&current).is_ok() {
            lock_reporting_project_authority(&pool, operation_id, &authority).await?;
            return Ok(current);
        }
    }
    let truth_port = PgReportTruthPort::with_project_authority(pool.clone(), authority.clone());
    let builder = ReportReadModelBuilder::new(truth_port);
    builder
        .build_and_validate(operation_id)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let truth = load_reporting_gate_truth(&pool, operation_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("report_validated_revision_missing"))?;
    lock_reporting_project_authority(&pool, operation_id, &authority).await?;
    Ok(truth)
}
