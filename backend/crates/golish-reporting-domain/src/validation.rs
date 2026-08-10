use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    canonical_source_set_hash, PublicationStatus, ReportClaim, ReportReadModel, ReportSourceKind,
    ReportSourceVersion, ValidationStatus,
};

use golish_memory_domain::source_ref::CanonicalRowId;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CleanupCloseoutTruth {
    pub organization_id_at_time: Uuid,
    pub missing_obligation_count: i64,
    pub nonterminal_obligation_count: i64,
    pub undisclosed_residual_count: i64,
    pub invalid_terminal_truth_count: i64,
    pub residual_obligation_ids: BTreeSet<Uuid>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceAuditTruth {
    pub evidence_audit_id: i64,
    pub run_id: Option<Uuid>,
    pub organization_id_at_time: Option<Uuid>,
    pub audit_role: Option<String>,
    pub referenced_organization_ids: BTreeSet<Uuid>,
    pub source: Option<ReportSourceVersion>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CleanupBlockedDecisionTruth {
    pub decision_id: Uuid,
    pub obligation_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub decided_by_principal_id: Uuid,
    pub reason: String,
    pub residual_risk: serde_json::Value,
    pub evidence_ids: BTreeSet<i64>,
    pub decision_evidence_ids: BTreeSet<i64>,
    pub source: ReportSourceVersion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportValidationTruth {
    pub current_revision_id: Uuid,
    pub validation_status: ValidationStatus,
    pub publication_status: PublicationStatus,
    pub allowed_organization_ids: BTreeSet<Uuid>,
    pub current_sources: Vec<ReportSourceVersion>,
    pub evidence_audits: BTreeMap<i64, EvidenceAuditTruth>,
    pub cleanup_blocked_decisions: BTreeMap<Uuid, CleanupBlockedDecisionTruth>,
    pub cleanup: Vec<CleanupCloseoutTruth>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportValidationIssue {
    pub code: String,
    pub claim_id: Option<Uuid>,
    pub organization_id_at_time: Option<Uuid>,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportValidationResult {
    pub revision_id: Uuid,
    pub claim_count: usize,
    pub citation_count: usize,
    pub source_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("report validation failed with {count} issue(s)", count = .0.len())]
pub struct ReportValidationError(pub Vec<ReportValidationIssue>);

fn issue(
    issues: &mut Vec<ReportValidationIssue>,
    code: &str,
    claim_id: Option<Uuid>,
    organization_id_at_time: Option<Uuid>,
    detail: impl Into<String>,
) {
    issues.push(ReportValidationIssue {
        code: code.to_string(),
        claim_id,
        organization_id_at_time,
        detail: detail.into(),
    });
}

fn contains_secret(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "password" | "secret" | "token" | "api_key" | "private_key" | "cookie"
            ) || contains_secret(value)
        }),
        serde_json::Value::Array(items) => items.iter().any(contains_secret),
        _ => false,
    }
}

fn all_claims(model: &ReportReadModel) -> impl Iterator<Item = &ReportClaim> {
    model
        .organization_sections
        .iter()
        .flat_map(|section| section.section.claims.iter())
}

pub fn validate_report(
    model: &ReportReadModel,
    truth: &ReportValidationTruth,
) -> Result<ReportValidationResult, ReportValidationError> {
    let mut issues = Vec::new();
    if truth.current_revision_id != model.revision_id {
        issue(
            &mut issues,
            "REPORT_REVISION_NOT_CURRENT",
            None,
            None,
            "validation only accepts the report current revision",
        );
    }
    if truth.validation_status != ValidationStatus::Validated {
        issue(
            &mut issues,
            "REPORT_REVISION_NOT_VALIDATED",
            None,
            None,
            "Gate requires the independent validation attestation",
        );
    }
    if truth.publication_status == PublicationStatus::Superseded {
        issue(
            &mut issues,
            "REPORT_REVISION_SUPERSEDED",
            None,
            None,
            "a superseded historical revision cannot be current",
        );
    }

    let current = crate::ReportSourceSnapshot::freeze("current", truth.current_sources.clone());
    match current {
        Ok(snapshot)
            if snapshot.ordered_sources == model.source_snapshot.ordered_sources
                && snapshot.source_set_hash == model.source_snapshot.source_set_hash => {}
        _ => issue(
            &mut issues,
            "REPORT_SOURCE_SNAPSHOT_STALE",
            None,
            None,
            "the complete ordered canonical source set changed",
        ),
    }
    if canonical_source_set_hash(&model.source_snapshot.ordered_sources).ok()
        != Some(model.source_snapshot.source_set_hash)
    {
        issue(
            &mut issues,
            "REPORT_SOURCE_HASH_INVALID",
            None,
            None,
            "stored source-set hash does not match the canonical manifest",
        );
    }

    for (evidence_id, evidence) in &truth.evidence_audits {
        if evidence.evidence_audit_id != *evidence_id
            || evidence.run_id != Some(model.operation_id)
            || evidence.audit_role.as_deref() != Some("evidence")
        {
            issue(
                &mut issues,
                "REPORT_EVIDENCE_AUTHORITY_INVALID",
                None,
                evidence.organization_id_at_time,
                "evidence must be a positive exact-operation evidence-ledger row",
            );
        }
        if evidence.referenced_organization_ids.len() != 1
            || evidence
                .organization_id_at_time
                .is_none_or(|organization_id| {
                    !evidence
                        .referenced_organization_ids
                        .contains(&organization_id)
                        || !truth.allowed_organization_ids.contains(&organization_id)
                })
        {
            issue(
                &mut issues,
                "REPORT_EVIDENCE_ORG_MISMATCH",
                None,
                evidence.organization_id_at_time,
                "evidence ownership must exactly match its one frozen source organization",
            );
        }
        let source_valid = evidence.source.as_ref().is_some_and(|source| {
            source.kind == ReportSourceKind::EvidenceAudit
                && source.id == CanonicalRowId::Int64(*evidence_id)
                && model
                    .source_snapshot
                    .ordered_sources
                    .iter()
                    .any(|current| current == source)
        });
        if !source_valid {
            issue(
                &mut issues,
                "REPORT_EVIDENCE_SOURCE_MISSING",
                None,
                evidence.organization_id_at_time,
                "referenced evidence body and authority metadata must be frozen in the manifest",
            );
        }
    }
    for source in model
        .source_snapshot
        .ordered_sources
        .iter()
        .filter(|source| source.kind == ReportSourceKind::EvidenceAudit)
    {
        let CanonicalRowId::Int64(evidence_id) = &source.id else {
            issue(
                &mut issues,
                "REPORT_EVIDENCE_SOURCE_INVALID",
                None,
                None,
                "evidence manifest entries require an int64 audit id",
            );
            continue;
        };
        if truth
            .evidence_audits
            .get(evidence_id)
            .and_then(|evidence| evidence.source.as_ref())
            != Some(source)
        {
            issue(
                &mut issues,
                "REPORT_EVIDENCE_SOURCE_UNBOUND",
                None,
                None,
                "evidence manifest entry has no exact typed evidence authority",
            );
        }
    }

    let citations = model
        .citations
        .iter()
        .map(|citation| (citation.citation_id, citation))
        .collect::<BTreeMap<_, _>>();
    for organization in &model.organization_sections {
        for claim in &organization.section.claims {
            if claim.value.required_section_kind().is_some_and(|required| {
                required != organization.section.kind
                    && !(claim.claim_kind == crate::ReportClaimKind::CleanupResidual
                        && matches!(&claim.value, crate::ReportClaimValue::Limitation { .. })
                        && organization.section.kind == crate::ReportSectionKind::CleanupResiduals)
            }) {
                issue(
                    &mut issues,
                    "REPORT_CLAIM_SECTION_INVALID",
                    Some(claim.claim_id),
                    claim.organization_id_at_time,
                    "typed verdict, coverage or limitation is in the wrong report section",
                );
            }
        }
    }
    for claim in all_claims(model) {
        if let Err(code) = claim.value.validate_authority(claim.authority_class) {
            issue(
                &mut issues,
                "REPORT_CLAIM_AUTHORITY_INVALID",
                Some(claim.claim_id),
                claim.organization_id_at_time,
                code,
            );
        }
        if claim.citation_ids.is_empty() {
            issue(
                &mut issues,
                "CLAIM_CITATION_REQUIRED",
                Some(claim.claim_id),
                claim.organization_id_at_time,
                "every report fact requires a canonical citation",
            );
        }
        if serde_json::to_value(&claim.value)
            .ok()
            .is_some_and(|value| contains_secret(&value))
        {
            issue(
                &mut issues,
                "SECRET_VALUE_FORBIDDEN",
                Some(claim.claim_id),
                claim.organization_id_at_time,
                "secret-bearing fields are forbidden from reports",
            );
        }
        for citation_id in &claim.citation_ids {
            let Some(citation) = citations.get(citation_id) else {
                issue(
                    &mut issues,
                    "REPORT_CITATION_UNRESOLVED",
                    Some(claim.claim_id),
                    claim.organization_id_at_time,
                    "claim points to an unknown citation",
                );
                continue;
            };
            if citation.revision_id != model.revision_id || citation.claim_id != claim.claim_id {
                issue(
                    &mut issues,
                    "REPORT_CITATION_REVISION_MISMATCH",
                    Some(claim.claim_id),
                    claim.organization_id_at_time,
                    "citation is not owned by this claim and revision",
                );
            }
            if claim.organization_id_at_time != Some(citation.organization_id_at_time)
                || !truth
                    .allowed_organization_ids
                    .contains(&citation.organization_id_at_time)
            {
                issue(
                    &mut issues,
                    "CITATION_ORG_MISMATCH",
                    Some(claim.claim_id),
                    claim.organization_id_at_time,
                    "citation crosses the frozen organization boundary",
                );
            }
            if !model
                .source_snapshot
                .ordered_sources
                .iter()
                .any(|source| source == &citation.source)
            {
                issue(
                    &mut issues,
                    "REPORT_CITATION_SOURCE_MISSING",
                    Some(claim.claim_id),
                    claim.organization_id_at_time,
                    "citation source is absent from the complete manifest",
                );
            }
            let canonical_compound_authority = (matches!(
                claim.authority_class,
                crate::ReportAuthorityClass::SecurityVerdictAuthority
                    | crate::ReportAuthorityClass::GrandfatheredLegacySecurityVerdict
                    | crate::ReportAuthorityClass::CoverageAuthority
            ) || matches!(
                &claim.value,
                crate::ReportClaimValue::Limitation { .. }
            ) && matches!(
                citation.source.kind,
                ReportSourceKind::HypothesisResidual
                    | ReportSourceKind::InvestigationClosureResidual
                    | ReportSourceKind::LegacyReportAuthoritySeal
                    | ReportSourceKind::InputProcessingDisposition
            )) && citation.evidence_audit_id.is_none()
                && citation.source.authority_class == claim.authority_class;
            let evidence = citation
                .evidence_audit_id
                .and_then(|id| truth.evidence_audits.get(&id));
            if !canonical_compound_authority
                && evidence.is_none_or(|evidence| {
                    evidence.run_id != Some(model.operation_id)
                        || evidence.audit_role.as_deref() != Some("evidence")
                        || evidence.organization_id_at_time
                            != Some(citation.organization_id_at_time)
                        || evidence.source.as_ref().is_none_or(|source| {
                            !model
                                .source_snapshot
                                .ordered_sources
                                .iter()
                                .any(|current| current == source)
                        })
                })
            {
                issue(
                    &mut issues,
                    "REPORT_EVIDENCE_CITATION_REQUIRED",
                    Some(claim.claim_id),
                    claim.organization_id_at_time,
                    "citation has no resolvable evidence-ledger entry",
                );
            }
        }
    }

    let claims_by_id = all_claims(model)
        .map(|claim| (claim.claim_id, claim))
        .collect::<BTreeMap<_, _>>();
    for finding in &model.findings {
        if finding.candidate_id.is_some() && finding.verified_lineage_id.is_none() {
            issue(
                &mut issues,
                "FINDING_LINEAGE_REQUIRED",
                Some(finding.claim_id),
                Some(finding.organization_id_at_time),
                "a Candidate only enters Findings through current verified lineage",
            );
        }
        let exact_verified_authority = claims_by_id.get(&finding.claim_id).is_some_and(|claim| {
            let authority_finding_id = match &claim.value {
                crate::ReportClaimValue::SecurityVerdict {
                    verdict: crate::SecurityVerdictProjection::Verified,
                    authority:
                        crate::SecurityVerdictAuthority::RevisionAdjudicationV1 {
                            finding_id,
                            refutation_receipt_id,
                            ..
                        },
                    ..
                }
                | crate::ReportClaimValue::SecurityVerdict {
                    verdict: crate::SecurityVerdictProjection::Verified,
                    authority:
                        crate::SecurityVerdictAuthority::LegacyAttemptV1 {
                            finding_id,
                            refutation_receipt_id,
                            ..
                        },
                    ..
                } if refutation_receipt_id.is_none() => *finding_id,
                _ => None,
            };
            claim.claim_kind == crate::ReportClaimKind::Finding
                && claim.organization_id_at_time == Some(finding.organization_id_at_time)
                && authority_finding_id == Some(finding.finding_id)
        });
        if !exact_verified_authority {
            issue(
                &mut issues,
                "FINDING_SECURITY_VERDICT_AUTHORITY_REQUIRED",
                Some(finding.claim_id),
                Some(finding.organization_id_at_time),
                "a report Finding must bind one exact typed verified security verdict authority",
            );
        }
    }
    for (decision_id, decision) in &truth.cleanup_blocked_decisions {
        let source_is_exact = decision.decision_id == *decision_id
            && decision.source.kind == ReportSourceKind::CleanupBlockedDecision
            && decision.source.id == CanonicalRowId::Uuid(*decision_id)
            && model
                .source_snapshot
                .ordered_sources
                .iter()
                .any(|source| source == &decision.source);
        if !source_is_exact || decision.decision_evidence_ids.is_empty() {
            issue(
                &mut issues,
                "CLEANUP_BLOCKED_DECISION_AUTHORITY_INVALID",
                None,
                Some(decision.organization_id_at_time),
                "blocked residual authority requires one retained decision row and decision evidence",
            );
        }
    }
    for source in model
        .source_snapshot
        .ordered_sources
        .iter()
        .filter(|source| source.kind == ReportSourceKind::CleanupBlockedDecision)
    {
        let CanonicalRowId::Uuid(decision_id) = source.id else {
            issue(
                &mut issues,
                "CLEANUP_BLOCKED_DECISION_SOURCE_INVALID",
                None,
                None,
                "blocked decision manifest entries require a UUID decision id",
            );
            continue;
        };
        if truth
            .cleanup_blocked_decisions
            .get(&decision_id)
            .is_none_or(|decision| decision.source != *source)
        {
            issue(
                &mut issues,
                "CLEANUP_BLOCKED_DECISION_SOURCE_UNBOUND",
                None,
                None,
                "blocked decision manifest entry lacks exact typed authority",
            );
        }
    }
    for residual in model
        .cleanup_residuals
        .iter()
        .filter(|residual| residual.status == "blocked")
    {
        let matching = truth
            .cleanup_blocked_decisions
            .values()
            .filter(|decision| decision.obligation_id == residual.obligation_id)
            .collect::<Vec<_>>();
        let Some(decision) = (matching.len() == 1).then(|| matching[0]) else {
            issue(
                &mut issues,
                "CLEANUP_BLOCKED_DECISION_REQUIRED",
                Some(residual.claim_id),
                Some(residual.organization_id_at_time),
                "blocked cleanup must bind exactly one retained operator decision",
            );
            continue;
        };
        let Some(claim) = claims_by_id.get(&residual.claim_id) else {
            issue(
                &mut issues,
                "CLEANUP_BLOCKED_CLAIM_MISSING",
                Some(residual.claim_id),
                Some(residual.organization_id_at_time),
                "blocked cleanup residual points to no report claim",
            );
            continue;
        };
        if decision.organization_id_at_time != residual.organization_id_at_time
            || claim.claim_kind != crate::ReportClaimKind::CleanupResidual
            || claim.organization_id_at_time != Some(decision.organization_id_at_time)
            || claim.subject_ref != format!("cleanup_obligation:{}", decision.obligation_id)
            || claim.predicate != "residual_risk"
            || !matches!(
                &claim.value,
                crate::ReportClaimValue::Limitation { reason_code, .. }
                    if reason_code == &decision.reason
            )
        {
            issue(
                &mut issues,
                "CLEANUP_BLOCKED_CLAIM_AUTHORITY_INVALID",
                Some(residual.claim_id),
                Some(residual.organization_id_at_time),
                "blocked residual content must be projected exactly from its retained decision",
            );
        }
        let mut cited_decision_evidence = BTreeSet::new();
        let citations_are_exact = claim.citation_ids.iter().all(|citation_id| {
            citations.get(citation_id).is_some_and(|citation| {
                citation.source == decision.source
                    && citation.organization_id_at_time == decision.organization_id_at_time
                    && citation.evidence_audit_id.is_some_and(|evidence_id| {
                        cited_decision_evidence.insert(evidence_id);
                        decision.evidence_ids.contains(&evidence_id)
                    })
            })
        });
        if !citations_are_exact || cited_decision_evidence != decision.evidence_ids {
            issue(
                &mut issues,
                "CLEANUP_BLOCKED_DECISION_EVIDENCE_MISMATCH",
                Some(residual.claim_id),
                Some(residual.organization_id_at_time),
                "blocked residual citations must equal the retained decision-evidence set",
            );
        }
    }
    for decision in truth.cleanup_blocked_decisions.values() {
        if !model.cleanup_residuals.iter().any(|residual| {
            residual.status == "blocked"
                && residual.obligation_id == decision.obligation_id
                && residual.organization_id_at_time == decision.organization_id_at_time
        }) {
            issue(
                &mut issues,
                "CLEANUP_BLOCKED_DECISION_UNDISCLOSED",
                None,
                Some(decision.organization_id_at_time),
                "every retained blocked decision must project one residual claim",
            );
        }
    }

    let residuals = model
        .cleanup_residuals
        .iter()
        .map(|residual| residual.obligation_id)
        .collect::<BTreeSet<_>>();
    let cleanup_organizations = truth
        .cleanup
        .iter()
        .map(|cleanup| cleanup.organization_id_at_time)
        .collect::<BTreeSet<_>>();
    for organization_id in truth
        .allowed_organization_ids
        .difference(&cleanup_organizations)
    {
        issue(
            &mut issues,
            "CLEANUP_CLOSEOUT_TRUTH_MISSING",
            None,
            Some(*organization_id),
            "every frozen organization requires a deterministic cleanup closeout query",
        );
    }
    for cleanup in &truth.cleanup {
        if cleanup.missing_obligation_count > 0 {
            issue(
                &mut issues,
                "CLEANUP_OBLIGATION_MISSING",
                None,
                Some(cleanup.organization_id_at_time),
                "side-effect actions without cleanup obligations fail closed",
            );
        }
        if cleanup.nonterminal_obligation_count > 0 {
            issue(
                &mut issues,
                "CLEANUP_OBLIGATION_NONTERMINAL",
                None,
                Some(cleanup.organization_id_at_time),
                "cleanup closeout is not terminal",
            );
        }
        if cleanup.invalid_terminal_truth_count > 0 {
            issue(
                &mut issues,
                "CLEANUP_TERMINAL_TRUTH_INVALID",
                None,
                Some(cleanup.organization_id_at_time),
                "terminal cleanup state lacks authoritative relational proof",
            );
        }
        if cleanup.undisclosed_residual_count > 0
            || !cleanup.residual_obligation_ids.is_subset(&residuals)
        {
            issue(
                &mut issues,
                "CLEANUP_RESIDUAL_REQUIRED",
                None,
                Some(cleanup.organization_id_at_time),
                "blocked or waived cleanup requires a cited residual claim",
            );
        }
    }

    if issues.is_empty() {
        Ok(ReportValidationResult {
            revision_id: model.revision_id,
            claim_count: all_claims(model).count(),
            citation_count: model.citations.len(),
            source_count: model.source_snapshot.ordered_sources.len(),
        })
    } else {
        Err(ReportValidationError(issues))
    }
}

#[cfg(test)]
mod tests {
    use golish_memory_domain::source_ref::CanonicalRowId;
    use serde_json::json;

    use super::*;
    use crate::{
        CitationSourceType, OrganizationReportSection, ReportAuthorityClass, ReportCitation,
        ReportClaimKind, ReportClaimValue, ReportFinding, ReportSectionKind, ReportSectionModel,
        ReportSourceKind, ReportSourceSnapshot,
    };

    fn fixture() -> (ReportReadModel, ReportValidationTruth) {
        let revision_id = Uuid::new_v4();
        let organization_id = Uuid::new_v4();
        let claim_id = Uuid::new_v4();
        let citation_id = Uuid::new_v4();
        let source = ReportSourceVersion {
            kind: ReportSourceKind::Finding,
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            id: CanonicalRowId::Uuid(Uuid::new_v4()),
            row_version: 4,
            content_hash: [7; 32],
        };
        let evidence_source = ReportSourceVersion {
            kind: ReportSourceKind::EvidenceAudit,
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            id: CanonicalRowId::Int64(42),
            row_version: 0,
            content_hash: [8; 32],
        };
        let source_snapshot =
            ReportSourceSnapshot::freeze("tx", vec![source.clone(), evidence_source.clone()])
                .expect("source snapshot");
        let claim = ReportClaim {
            claim_id,
            revision_id,
            section_id: Uuid::new_v4(),
            organization_id_at_time: Some(organization_id),
            claim_kind: ReportClaimKind::Finding,
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            subject_ref: "finding-1".into(),
            predicate: "verified".into(),
            value: ReportClaimValue::from_legacy_redacted(
                "finding-1",
                "legacy_reporting",
                "verified",
                &json!({"severity":"high"}),
            )
            .expect("typed legacy projection"),
            citation_ids: vec![citation_id],
            ordinal: 0,
        };
        let section_id = claim.section_id;
        let model = ReportReadModel {
            report_id: Uuid::new_v4(),
            revision_id,
            operation_id: Uuid::new_v4(),
            project_scope_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            scope_snapshot_hash: "a".repeat(64),
            source_snapshot,
            organization_sections: vec![OrganizationReportSection {
                organization_id_at_time: organization_id,
                organization_name_at_snapshot: "Acme".into(),
                section: ReportSectionModel {
                    section_id,
                    revision_id,
                    organization_id_at_time: Some(organization_id),
                    organization_name_at_snapshot: Some("Acme".into()),
                    kind: ReportSectionKind::Findings,
                    claims: vec![claim],
                    rendered_content: None,
                    ordinal: 0,
                },
            }],
            findings: vec![],
            cleanup_residuals: vec![],
            citations: vec![ReportCitation {
                citation_id,
                revision_id,
                claim_id,
                source: source.clone(),
                source_type: CitationSourceType::CanonicalFact,
                evidence_audit_id: Some(42),
                organization_id_at_time: organization_id,
                display_label: "Finding evidence".into(),
                ordinal: 0,
            }],
        };
        let truth = ReportValidationTruth {
            current_revision_id: revision_id,
            validation_status: ValidationStatus::Validated,
            publication_status: PublicationStatus::Unpublished,
            allowed_organization_ids: BTreeSet::from([organization_id]),
            current_sources: vec![source, evidence_source.clone()],
            evidence_audits: BTreeMap::from([(
                42,
                EvidenceAuditTruth {
                    evidence_audit_id: 42,
                    run_id: Some(model.operation_id),
                    organization_id_at_time: Some(organization_id),
                    audit_role: Some("evidence".to_string()),
                    referenced_organization_ids: BTreeSet::from([organization_id]),
                    source: Some(evidence_source),
                },
            )]),
            cleanup_blocked_decisions: BTreeMap::new(),
            cleanup: vec![CleanupCloseoutTruth {
                organization_id_at_time: organization_id,
                missing_obligation_count: 0,
                nonterminal_obligation_count: 0,
                undisclosed_residual_count: 0,
                invalid_terminal_truth_count: 0,
                residual_obligation_ids: BTreeSet::new(),
            }],
        };
        (model, truth)
    }

    #[test]
    fn missing_cleanup_obligation_fails_closed() {
        let (model, mut truth) = fixture();
        truth.cleanup[0].missing_obligation_count = 1;
        let errors = validate_report(&model, &truth).expect_err("must fail closed");
        assert!(errors
            .0
            .iter()
            .any(|issue| issue.code == "CLEANUP_OBLIGATION_MISSING"));
    }

    #[test]
    fn missing_cleanup_closeout_row_fails_closed() {
        let (model, mut truth) = fixture();
        truth.cleanup.clear();
        let errors = validate_report(&model, &truth).expect_err("must fail closed");
        assert!(errors
            .0
            .iter()
            .any(|issue| issue.code == "CLEANUP_CLOSEOUT_TRUTH_MISSING"));
    }

    #[test]
    fn invalid_cleanup_terminal_truth_fails_closed() {
        let (model, mut truth) = fixture();
        truth.cleanup[0].invalid_terminal_truth_count = 1;
        let errors = validate_report(&model, &truth).expect_err("must fail closed");
        assert!(errors
            .0
            .iter()
            .any(|issue| issue.code == "CLEANUP_TERMINAL_TRUTH_INVALID"));
    }

    #[test]
    fn validation_and_publication_are_independent_axes() {
        let (model, mut truth) = fixture();
        truth.publication_status = PublicationStatus::Final;
        validate_report(&model, &truth).expect("published validated revision still gates");

        truth.validation_status = ValidationStatus::Draft;
        assert!(validate_report(&model, &truth).is_err());
    }

    #[test]
    fn sibling_citation_secret_and_unverified_candidate_are_rejected() {
        let (mut model, truth) = fixture();
        model.citations[0].organization_id_at_time = Uuid::new_v4();
        model.findings.push(ReportFinding {
            finding_id: Uuid::new_v4(),
            organization_id_at_time: model.organization_sections[0].organization_id_at_time,
            candidate_id: Some(Uuid::new_v4()),
            verified_lineage_id: None,
            claim_id: model.organization_sections[0].section.claims[0].claim_id,
        });
        let errors = validate_report(&model, &truth).expect_err("invalid report");
        for code in ["CITATION_ORG_MISMATCH", "FINDING_LINEAGE_REQUIRED"] {
            assert!(errors.0.iter().any(|issue| issue.code == code));
        }
    }

    #[test]
    fn newly_inserted_source_makes_snapshot_stale() {
        let (model, mut truth) = fixture();
        truth.current_sources.push(ReportSourceVersion {
            kind: ReportSourceKind::TechniqueOutcome,
            authority_class: ReportAuthorityClass::MethodAuditOnly,
            id: CanonicalRowId::Uuid(Uuid::new_v4()),
            row_version: 1,
            content_hash: [9; 32],
        });
        let errors = validate_report(&model, &truth).expect_err("source set changed");
        assert!(errors
            .0
            .iter()
            .any(|issue| issue.code == "REPORT_SOURCE_SNAPSHOT_STALE"));
    }

    #[test]
    fn methodology_audit_cannot_be_projected_as_finding_proof() {
        let (mut model, truth) = fixture();
        let claim = &mut model.organization_sections[0].section.claims[0];
        claim.value = ReportClaimValue::MethodAudit {
            method_code: "rag_context".into(),
            disposition_code: "consulted".into(),
        };
        claim.claim_kind = ReportClaimKind::Finding;
        claim.authority_class = ReportAuthorityClass::MethodAuditOnly;
        model.organization_sections[0].section.kind = ReportSectionKind::Findings;

        let errors = validate_report(&model, &truth).expect_err("methodology is never proof");
        assert!(errors
            .0
            .iter()
            .any(|issue| issue.code == "REPORT_CLAIM_SECTION_INVALID"));
    }
}
