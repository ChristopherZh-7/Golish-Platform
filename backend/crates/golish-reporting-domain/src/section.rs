use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;
use uuid::Uuid;

use crate::ReportAuthorityClass;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSectionKind {
    ExecutiveSummary,
    Organization,
    Findings,
    AttackPaths,
    CleanupResiduals,
    Methodology,
    Limitations,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportClaimKind {
    Scope,
    Finding,
    CandidateDisposition,
    TechniqueOutcome,
    AttackPath,
    ObjectiveOutcome,
    CleanupResidual,
    Limitation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum SecurityVerdictProjection {
    Verified,
    Refuted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum CoverageSufficiencyProjection {
    NotAssessed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "contract",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum SecurityVerdictAuthority {
    RevisionAdjudicationV1 {
        verification_plan_seal_id: Uuid,
        verification_plan_seal_hash: String,
        proof_path_set_hash: String,
        claim_component_set_hash: String,
        revision_adjudication_id: Uuid,
        revision_adjudication_hash: String,
        revision_terminal_decision_id: Uuid,
        revision_terminal_decision_hash: String,
        latest_objective_outcome_member_count: u64,
        latest_objective_outcome_set_hash: String,
        finding_id: Option<Uuid>,
        refutation_receipt_id: Option<Uuid>,
    },
    LegacyAttemptV1 {
        candidate_id: Uuid,
        attempt_id: Uuid,
        legacy_attempt_authority_receipt_id: Uuid,
        legacy_attempt_authority_receipt_hash: String,
        legacy_report_authority_seal_id: Uuid,
        legacy_report_authority_seal_hash: String,
        legacy_contract_version: String,
        terminal_status: String,
        source_record_hash: String,
        evidence_membership_hash: String,
        adapter_version: String,
        adapter_digest: String,
        finding_id: Option<Uuid>,
        refutation_receipt_id: Option<Uuid>,
        limitation_codes: Vec<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub enum ReportClaimValue {
    SecurityVerdict {
        verdict: SecurityVerdictProjection,
        hypothesis_revision_id: Uuid,
        authority: SecurityVerdictAuthority,
    },
    Coverage {
        final_wave_coverage_receipt_id: Uuid,
        final_wave_coverage_receipt_hash: String,
        denominator_id: Uuid,
        denominator_hash: String,
        planned: u64,
        tested_complete: u64,
        tested_degraded: u64,
        untested: u64,
        blocked: u64,
        residual_ids: Vec<Uuid>,
        coverage_sufficiency: CoverageSufficiencyProjection,
    },
    ObservationAudit {
        source_id: String,
        source_hash: String,
        provenance: String,
        outcome_code: String,
    },
    MethodAudit {
        method_code: String,
        disposition_code: String,
    },
    AuthorizationAudit {
        prepared_action_id: Uuid,
        risk_tier: String,
        decision_code: String,
        request_digest: String,
        policy_digest: String,
    },
    Limitation {
        reason_code: String,
        affected_input_ids: Vec<String>,
        residual_ids: Vec<Uuid>,
        owner_code: String,
        next_action_code: String,
    },
}

impl ReportClaimValue {
    /// Closed section routing for claims whose semantics must not be selected
    /// by a renderer or caller. Audit-only values retain their canonical
    /// source section, but verdict/coverage/limitation placement is fixed.
    pub const fn required_section_kind(&self) -> Option<ReportSectionKind> {
        match self {
            Self::SecurityVerdict { .. } => Some(ReportSectionKind::Findings),
            Self::Coverage { .. } => Some(ReportSectionKind::Organization),
            Self::MethodAudit { .. } => Some(ReportSectionKind::Methodology),
            Self::Limitation { .. } => Some(ReportSectionKind::Limitations),
            Self::ObservationAudit { .. } | Self::AuthorizationAudit { .. } => None,
        }
    }

    pub fn validate_authority(&self, authority_class: ReportAuthorityClass) -> Result<(), String> {
        match self {
            Self::SecurityVerdict {
                verdict, authority, ..
            } => validate_security_verdict(*verdict, authority, authority_class),
            Self::Coverage {
                final_wave_coverage_receipt_id,
                final_wave_coverage_receipt_hash,
                denominator_id,
                denominator_hash,
                planned,
                tested_complete,
                tested_degraded,
                untested,
                blocked,
                ..
            } => {
                let partition = tested_complete
                    .checked_add(*tested_degraded)
                    .and_then(|value| value.checked_add(*untested))
                    .and_then(|value| value.checked_add(*blocked));
                if authority_class != ReportAuthorityClass::CoverageAuthority
                    || final_wave_coverage_receipt_id.is_nil()
                    || denominator_id.is_nil()
                    || !is_sha256(final_wave_coverage_receipt_hash)
                    || !is_sha256(denominator_hash)
                    || partition != Some(*planned)
                {
                    return Err("report_coverage_authority_invalid".to_owned());
                }
                Ok(())
            }
            Self::ObservationAudit { .. } => matches!(
                authority_class,
                ReportAuthorityClass::ExecutionObservationAudit
                    | ReportAuthorityClass::MethodAuditOnly
                    | ReportAuthorityClass::HistoricalArtifactReadOnly
            )
            .then_some(())
            .ok_or_else(|| "report_observation_authority_invalid".to_owned()),
            Self::MethodAudit { .. } | Self::Limitation { .. } => (authority_class
                == ReportAuthorityClass::MethodAuditOnly)
                .then_some(())
                .ok_or_else(|| "report_method_authority_invalid".to_owned()),
            Self::AuthorizationAudit { .. } => (authority_class
                == ReportAuthorityClass::AuthorizationAudit)
                .then_some(())
                .ok_or_else(|| "report_authorization_authority_invalid".to_owned()),
        }
    }

    /// Compatibility adapter for pre-Plan-D canonical rows. It emits only a
    /// digest-bound audit projection and never manufactures verdict authority.
    pub fn from_legacy_redacted(
        source_id: impl Into<String>,
        provenance: impl Into<String>,
        outcome_code: impl Into<String>,
        value: &serde_json::Value,
    ) -> Result<Self, String> {
        let bytes = serde_json::to_vec(value).map_err(|_| "report_legacy_projection_invalid")?;
        let digest = Sha256::digest(bytes);
        Ok(Self::ObservationAudit {
            source_id: bounded_code(source_id.into(), 512)?,
            source_hash: format!(
                "sha256:{}",
                digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
            provenance: bounded_code(provenance.into(), 128)?,
            outcome_code: bounded_code(outcome_code.into(), 128)?,
        })
    }
}

fn validate_security_verdict(
    verdict: SecurityVerdictProjection,
    authority: &SecurityVerdictAuthority,
    authority_class: ReportAuthorityClass,
) -> Result<(), String> {
    let (finding_id, refutation_id, hashes_valid, identity_valid, legacy_limitation) =
        match authority {
            SecurityVerdictAuthority::RevisionAdjudicationV1 {
                verification_plan_seal_id,
                verification_plan_seal_hash,
                proof_path_set_hash,
                claim_component_set_hash,
                revision_adjudication_id,
                revision_adjudication_hash,
                revision_terminal_decision_id,
                revision_terminal_decision_hash,
                latest_objective_outcome_member_count,
                latest_objective_outcome_set_hash,
                finding_id,
                refutation_receipt_id,
            } => (
                *finding_id,
                *refutation_receipt_id,
                [
                    verification_plan_seal_hash,
                    proof_path_set_hash,
                    claim_component_set_hash,
                    revision_adjudication_hash,
                    revision_terminal_decision_hash,
                    latest_objective_outcome_set_hash,
                ]
                .into_iter()
                .all(|hash| is_sha256(hash)),
                !verification_plan_seal_id.is_nil()
                    && !revision_adjudication_id.is_nil()
                    && !revision_terminal_decision_id.is_nil()
                    && *latest_objective_outcome_member_count > 0
                    && authority_class == ReportAuthorityClass::SecurityVerdictAuthority,
                true,
            ),
            SecurityVerdictAuthority::LegacyAttemptV1 {
                candidate_id,
                attempt_id,
                legacy_attempt_authority_receipt_id,
                legacy_attempt_authority_receipt_hash,
                legacy_report_authority_seal_id,
                legacy_report_authority_seal_hash,
                terminal_status,
                source_record_hash,
                evidence_membership_hash,
                adapter_digest,
                finding_id,
                refutation_receipt_id,
                limitation_codes,
                ..
            } => (
                *finding_id,
                *refutation_receipt_id,
                [
                    legacy_attempt_authority_receipt_hash,
                    legacy_report_authority_seal_hash,
                    source_record_hash,
                    evidence_membership_hash,
                    adapter_digest,
                ]
                .into_iter()
                .all(|hash| is_sha256(hash)),
                !candidate_id.is_nil()
                    && !attempt_id.is_nil()
                    && !legacy_attempt_authority_receipt_id.is_nil()
                    && !legacy_report_authority_seal_id.is_nil()
                    && matches!(terminal_status.as_str(), "verified" | "refuted")
                    && authority_class == ReportAuthorityClass::GrandfatheredLegacySecurityVerdict,
                limitation_codes
                    .iter()
                    .any(|code| code == "legacy_coverage_unavailable"),
            ),
        };
    let lineage_exact = match verdict {
        SecurityVerdictProjection::Verified => finding_id.is_some() && refutation_id.is_none(),
        SecurityVerdictProjection::Refuted => finding_id.is_none() && refutation_id.is_some(),
    };
    if hashes_valid && identity_valid && lineage_exact && legacy_limitation {
        Ok(())
    } else {
        Err("report_security_verdict_authority_invalid".to_owned())
    }
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn bounded_code(value: String, max: usize) -> Result<String, String> {
    if value.trim().is_empty()
        || value.len() > max
        || value.chars().any(|character| {
            character == '\0'
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        Err("report_projection_forbidden_value".to_owned())
    } else {
        Ok(value)
    }
}

impl std::fmt::Display for ReportClaimValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecurityVerdict { verdict, .. } => write!(formatter, "{verdict:?}"),
            Self::Coverage {
                planned,
                tested_complete,
                tested_degraded,
                untested,
                blocked,
                ..
            } => write!(
                formatter,
                "planned={planned} complete={tested_complete} degraded={tested_degraded} untested={untested} blocked={blocked}"
            ),
            Self::ObservationAudit { outcome_code, .. } => formatter.write_str(outcome_code),
            Self::MethodAudit {
                disposition_code, ..
            } => formatter.write_str(disposition_code),
            Self::AuthorizationAudit { decision_code, .. } => formatter.write_str(decision_code),
            Self::Limitation { reason_code, .. } => formatter.write_str(reason_code),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportClaim {
    pub claim_id: Uuid,
    pub revision_id: Uuid,
    pub section_id: Uuid,
    pub organization_id_at_time: Option<Uuid>,
    pub claim_kind: ReportClaimKind,
    #[serde(default)]
    pub authority_class: ReportAuthorityClass,
    pub subject_ref: String,
    pub predicate: String,
    pub value: ReportClaimValue,
    pub citation_ids: Vec<Uuid>,
    pub ordinal: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportSectionModel {
    pub section_id: Uuid,
    pub revision_id: Uuid,
    pub organization_id_at_time: Option<Uuid>,
    pub organization_name_at_snapshot: Option<String>,
    pub kind: ReportSectionKind,
    pub claims: Vec<ReportClaim>,
    pub rendered_content: Option<String>,
    pub ordinal: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrganizationReportSection {
    pub organization_id_at_time: Uuid,
    pub organization_name_at_snapshot: String,
    pub section: ReportSectionModel,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportFinding {
    pub finding_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub candidate_id: Option<Uuid>,
    pub verified_lineage_id: Option<Uuid>,
    pub claim_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportResidual {
    pub obligation_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub status: String,
    pub claim_id: Uuid,
}
