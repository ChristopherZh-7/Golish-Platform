//! Immutable exact-set context authority for one Investigation organization Unit.
//!
//! Contracts contain references and hashes only. Raw canonical/RAG/methodology
//! bodies stay in the organization read partition; Main sees only a census.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const BASELINE_CONTEXT_SNAPSHOT_CONTRACT_V1: &str =
    "investigation_baseline_context_snapshot.v1";
pub const INVESTIGATION_ANALYSIS_SNAPSHOT_CONTRACT_V1: &str = "investigation_analysis_snapshot.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationContextAuthorityV1 {
    CanonicalFact,
    RuntimeState,
    EvidenceLedger,
    ApplicationUnderstanding,
    CoverageGap,
    ToolTruth,
    RagPrior,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationContextMemberV1 {
    pub member_id: String,
    pub authority: InvestigationContextAuthorityV1,
    pub source_ref: String,
    pub content_sha256: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationMethodologyQueryIntentV1 {
    pub normalized_tags: Vec<String>,
    pub top_k: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationMethodologyHitRefV1 {
    pub corpus_id: String,
    pub document_id: String,
    pub content_sha256: String,
    pub safe_excerpt_ref: String,
    pub score_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvestigationContextOmissionV1 {
    pub layer: String,
    pub reason_code: String,
    pub omitted_subject_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaselineContextSnapshotV1 {
    pub baseline_snapshot_id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub context_chain_id: Uuid,
    pub transcript_partition_id: Uuid,
    pub application_model_revision_id: Uuid,
    pub application_model_revision_sha256: String,
    pub tool_truth_bundle_seal_id: Uuid,
    pub tool_truth_member_set_sha256: String,
    pub canonical_item_count: u32,
    pub canonical_item_set_sha256: String,
    pub contract_version: String,
    pub baseline_snapshot_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InvestigationAnalysisSnapshotV1 {
    pub snapshot_id: Uuid,
    pub baseline_snapshot_id: Uuid,
    pub methodology_query: InvestigationMethodologyQueryIntentV1,
    pub context_item_count: u32,
    pub context_item_set_sha256: String,
    pub methodology_hit_count: u32,
    pub methodology_result_set_sha256: String,
    pub omission_count: u32,
    pub omission_set_sha256: String,
    pub contract_version: String,
    pub snapshot_sha256: String,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InvestigationSnapshotError {
    #[error("invalid investigation snapshot identity: {0}")]
    InvalidIdentity(&'static str),
    #[error("invalid investigation snapshot field: {0}")]
    InvalidField(&'static str),
    #[error("investigation snapshot exact set is empty: {0}")]
    EmptyExactSet(&'static str),
    #[error("duplicate investigation snapshot member: {0}")]
    DuplicateMember(&'static str),
    #[error("methodology absence is not represented by an omission")]
    VacuousMethodologyResult,
}

#[derive(Serialize)]
struct BaselineHashMaterial<'a> {
    operation_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    context_chain_id: Uuid,
    transcript_partition_id: Uuid,
    application_model_revision_id: Uuid,
    application_model_revision_sha256: &'a str,
    tool_truth_bundle_seal_id: Uuid,
    tool_truth_member_set_sha256: &'a str,
    canonical_item_count: u32,
    canonical_item_set_sha256: &'a str,
    contract_version: &'static str,
}

#[derive(Serialize)]
struct AnalysisHashMaterial<'a> {
    baseline_snapshot_id: Uuid,
    methodology_query: &'a InvestigationMethodologyQueryIntentV1,
    context_item_count: u32,
    context_item_set_sha256: &'a str,
    methodology_hit_count: u32,
    methodology_result_set_sha256: &'a str,
    omission_count: u32,
    omission_set_sha256: &'a str,
    contract_version: &'static str,
}

impl BaselineContextSnapshotV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn host_seal(
        baseline_snapshot_id: Uuid,
        operation_id: Uuid,
        stage_execution_id: Uuid,
        stage_run_unit_id: Uuid,
        scope_snapshot_id: Uuid,
        organization_id: Uuid,
        context_chain_id: Uuid,
        transcript_partition_id: Uuid,
        application_model_revision_id: Uuid,
        application_model_revision_sha256: String,
        tool_truth_bundle_seal_id: Uuid,
        tool_truth_member_set_sha256: String,
        mut canonical_members: Vec<InvestigationContextMemberV1>,
    ) -> Result<Self, InvestigationSnapshotError> {
        for (field, value) in [
            ("baseline_snapshot_id", baseline_snapshot_id),
            ("operation_id", operation_id),
            ("stage_execution_id", stage_execution_id),
            ("stage_run_unit_id", stage_run_unit_id),
            ("scope_snapshot_id", scope_snapshot_id),
            ("organization_id", organization_id),
            ("context_chain_id", context_chain_id),
            ("transcript_partition_id", transcript_partition_id),
            (
                "application_model_revision_id",
                application_model_revision_id,
            ),
            ("tool_truth_bundle_seal_id", tool_truth_bundle_seal_id),
        ] {
            if value.is_nil() {
                return Err(InvestigationSnapshotError::InvalidIdentity(field));
            }
        }
        if context_chain_id == transcript_partition_id {
            return Err(InvestigationSnapshotError::InvalidIdentity(
                "read_partition_alias",
            ));
        }
        validate_sha256(&application_model_revision_sha256)?;
        validate_sha256(&tool_truth_member_set_sha256)?;
        normalize_context_members(&mut canonical_members, false)?;
        if canonical_members.is_empty() {
            return Err(InvestigationSnapshotError::EmptyExactSet(
                "canonical_members",
            ));
        }
        if !canonical_members.iter().any(|member| {
            member.authority == InvestigationContextAuthorityV1::ApplicationUnderstanding
        }) || !canonical_members
            .iter()
            .any(|member| member.authority == InvestigationContextAuthorityV1::ToolTruth)
        {
            return Err(InvestigationSnapshotError::InvalidField(
                "mandatory_authority_census",
            ));
        }
        let canonical_item_count = count(canonical_members.len(), "canonical_item_count")?;
        let canonical_item_set_sha256 = sha256_json(&canonical_members);
        let material = BaselineHashMaterial {
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            scope_snapshot_id,
            organization_id,
            context_chain_id,
            transcript_partition_id,
            application_model_revision_id,
            application_model_revision_sha256: &application_model_revision_sha256,
            tool_truth_bundle_seal_id,
            tool_truth_member_set_sha256: &tool_truth_member_set_sha256,
            canonical_item_count,
            canonical_item_set_sha256: &canonical_item_set_sha256,
            contract_version: BASELINE_CONTEXT_SNAPSHOT_CONTRACT_V1,
        };
        let baseline_snapshot_sha256 = sha256_json(&material);
        Ok(Self {
            baseline_snapshot_id,
            operation_id,
            stage_execution_id,
            stage_run_unit_id,
            scope_snapshot_id,
            organization_id,
            context_chain_id,
            transcript_partition_id,
            application_model_revision_id,
            application_model_revision_sha256,
            tool_truth_bundle_seal_id,
            tool_truth_member_set_sha256,
            canonical_item_count,
            canonical_item_set_sha256,
            contract_version: BASELINE_CONTEXT_SNAPSHOT_CONTRACT_V1.into(),
            baseline_snapshot_sha256,
        })
    }
}

impl InvestigationAnalysisSnapshotV1 {
    pub fn host_seal(
        snapshot_id: Uuid,
        baseline: &BaselineContextSnapshotV1,
        mut context_members: Vec<InvestigationContextMemberV1>,
        mut methodology_query: InvestigationMethodologyQueryIntentV1,
        mut methodology_hits: Vec<InvestigationMethodologyHitRefV1>,
        mut omissions: Vec<InvestigationContextOmissionV1>,
    ) -> Result<Self, InvestigationSnapshotError> {
        if snapshot_id.is_nil() {
            return Err(InvestigationSnapshotError::InvalidIdentity("snapshot_id"));
        }
        normalize_query(&mut methodology_query)?;
        normalize_context_members(&mut context_members, true)?;
        if context_members.is_empty() {
            return Err(InvestigationSnapshotError::EmptyExactSet("context_members"));
        }
        normalize_hits(&mut methodology_hits)?;
        normalize_omissions(&mut omissions)?;
        if methodology_hits.is_empty()
            && !omissions
                .iter()
                .any(|omission| omission.layer == "methodology")
        {
            return Err(InvestigationSnapshotError::VacuousMethodologyResult);
        }
        let context_item_count = count(context_members.len(), "context_item_count")?;
        let methodology_hit_count = count(methodology_hits.len(), "methodology_hit_count")?;
        let omission_count = count(omissions.len(), "omission_count")?;
        let context_item_set_sha256 = sha256_json(&context_members);
        let methodology_result_set_sha256 = sha256_json(&methodology_hits);
        let omission_set_sha256 = sha256_json(&omissions);
        let material = AnalysisHashMaterial {
            baseline_snapshot_id: baseline.baseline_snapshot_id,
            methodology_query: &methodology_query,
            context_item_count,
            context_item_set_sha256: &context_item_set_sha256,
            methodology_hit_count,
            methodology_result_set_sha256: &methodology_result_set_sha256,
            omission_count,
            omission_set_sha256: &omission_set_sha256,
            contract_version: INVESTIGATION_ANALYSIS_SNAPSHOT_CONTRACT_V1,
        };
        let snapshot_sha256 = sha256_json(&material);
        Ok(Self {
            snapshot_id,
            baseline_snapshot_id: baseline.baseline_snapshot_id,
            methodology_query,
            context_item_count,
            context_item_set_sha256,
            methodology_hit_count,
            methodology_result_set_sha256,
            omission_count,
            omission_set_sha256,
            contract_version: INVESTIGATION_ANALYSIS_SNAPSHOT_CONTRACT_V1.into(),
            snapshot_sha256,
        })
    }
}

fn normalize_context_members(
    members: &mut [InvestigationContextMemberV1],
    allow_rag: bool,
) -> Result<(), InvestigationSnapshotError> {
    for member in members.iter_mut() {
        validate_text(&member.member_id, "member_id")?;
        validate_text(&member.source_ref, "source_ref")?;
        validate_sha256(&member.content_sha256)?;
        if !allow_rag && member.authority == InvestigationContextAuthorityV1::RagPrior {
            return Err(InvestigationSnapshotError::InvalidField(
                "rag_prior_in_baseline",
            ));
        }
        member.evidence_ids.sort_unstable();
        if member.evidence_ids.iter().any(|id| *id <= 0)
            || member
                .evidence_ids
                .windows(2)
                .any(|pair| pair[0] == pair[1])
        {
            return Err(InvestigationSnapshotError::InvalidField("evidence_ids"));
        }
    }
    members.sort_by(|left, right| {
        left.authority
            .cmp(&right.authority)
            .then_with(|| left.member_id.cmp(&right.member_id))
            .then_with(|| left.content_sha256.cmp(&right.content_sha256))
    });
    let mut identities = BTreeSet::new();
    if members
        .iter()
        .any(|member| !identities.insert((member.authority, member.member_id.clone())))
    {
        return Err(InvestigationSnapshotError::DuplicateMember(
            "context_member",
        ));
    }
    Ok(())
}

fn normalize_query(
    query: &mut InvestigationMethodologyQueryIntentV1,
) -> Result<(), InvestigationSnapshotError> {
    let mut normalized = BTreeSet::new();
    for tag in std::mem::take(&mut query.normalized_tags) {
        let tag = tag.trim().to_ascii_lowercase();
        if tag.is_empty()
            || tag.len() > 128
            || !tag
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.' | '/'))
        {
            return Err(InvestigationSnapshotError::InvalidField("methodology_tag"));
        }
        normalized.insert(tag);
    }
    query.normalized_tags = normalized.into_iter().collect();
    if query.normalized_tags.is_empty() || query.normalized_tags.len() > 64 {
        return Err(InvestigationSnapshotError::InvalidField("methodology_tags"));
    }
    if query.top_k == 0 || query.top_k > 64 {
        return Err(InvestigationSnapshotError::InvalidField(
            "methodology_top_k",
        ));
    }
    Ok(())
}

fn normalize_hits(
    hits: &mut [InvestigationMethodologyHitRefV1],
) -> Result<(), InvestigationSnapshotError> {
    for hit in hits.iter() {
        validate_text(&hit.corpus_id, "corpus_id")?;
        validate_text(&hit.document_id, "document_id")?;
        validate_text(&hit.safe_excerpt_ref, "safe_excerpt_ref")?;
        validate_sha256(&hit.content_sha256)?;
        if hit.score_micros < 0 {
            return Err(InvestigationSnapshotError::InvalidField(
                "methodology_score",
            ));
        }
    }
    hits.sort_by(|left, right| {
        right
            .score_micros
            .cmp(&left.score_micros)
            .then_with(|| left.corpus_id.cmp(&right.corpus_id))
            .then_with(|| left.document_id.cmp(&right.document_id))
    });
    let mut identities = BTreeSet::new();
    if hits
        .iter()
        .any(|hit| !identities.insert((hit.corpus_id.clone(), hit.document_id.clone())))
    {
        return Err(InvestigationSnapshotError::DuplicateMember(
            "methodology_hit",
        ));
    }
    Ok(())
}

fn normalize_omissions(
    omissions: &mut [InvestigationContextOmissionV1],
) -> Result<(), InvestigationSnapshotError> {
    for omission in omissions.iter_mut() {
        omission.layer = omission.layer.trim().to_ascii_lowercase();
        omission.reason_code = omission.reason_code.trim().to_ascii_lowercase();
        validate_text(&omission.layer, "omission_layer")?;
        validate_text(&omission.reason_code, "omission_reason")?;
        validate_sha256(&omission.omitted_subject_sha256)?;
    }
    omissions.sort_by(|left, right| {
        left.layer
            .cmp(&right.layer)
            .then_with(|| left.reason_code.cmp(&right.reason_code))
            .then_with(|| {
                left.omitted_subject_sha256
                    .cmp(&right.omitted_subject_sha256)
            })
    });
    let mut identities = BTreeSet::new();
    if omissions.iter().any(|item| {
        !identities.insert((
            item.layer.clone(),
            item.reason_code.clone(),
            item.omitted_subject_sha256.clone(),
        ))
    }) {
        return Err(InvestigationSnapshotError::DuplicateMember("omission"));
    }
    Ok(())
}

fn count(value: usize, field: &'static str) -> Result<u32, InvestigationSnapshotError> {
    u32::try_from(value).map_err(|_| InvestigationSnapshotError::InvalidField(field))
}

fn validate_text(value: &str, field: &'static str) -> Result<(), InvestigationSnapshotError> {
    if value.trim().is_empty() || value.len() > 2_048 || value.chars().any(char::is_control) {
        return Err(InvestigationSnapshotError::InvalidField(field));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), InvestigationSnapshotError> {
    let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    });
    if !valid {
        return Err(InvestigationSnapshotError::InvalidField("sha256"));
    }
    Ok(())
}

fn sha256_json(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("snapshot identity material is serializable");
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(seed: char) -> String {
        format!("sha256:{}", seed.to_string().repeat(64))
    }

    fn member(
        authority: InvestigationContextAuthorityV1,
        id: &str,
    ) -> InvestigationContextMemberV1 {
        InvestigationContextMemberV1 {
            member_id: id.into(),
            authority,
            source_ref: format!("db://{id}"),
            content_sha256: hash('a'),
            evidence_ids: vec![2, 1],
        }
    }

    fn baseline() -> BaselineContextSnapshotV1 {
        BaselineContextSnapshotV1::host_seal(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Uuid::from_u128(4),
            Uuid::from_u128(5),
            Uuid::from_u128(6),
            Uuid::from_u128(7),
            Uuid::from_u128(8),
            Uuid::from_u128(9),
            hash('b'),
            Uuid::from_u128(10),
            hash('c'),
            vec![
                member(
                    InvestigationContextAuthorityV1::ApplicationUnderstanding,
                    "au",
                ),
                member(InvestigationContextAuthorityV1::ToolTruth, "tool-truth"),
                member(InvestigationContextAuthorityV1::CanonicalFact, "target"),
            ],
        )
        .unwrap()
    }

    #[test]
    fn baseline_is_order_independent_and_requires_mandatory_authorities() {
        let first = baseline();
        let second = BaselineContextSnapshotV1::host_seal(
            first.baseline_snapshot_id,
            first.operation_id,
            first.stage_execution_id,
            first.stage_run_unit_id,
            first.scope_snapshot_id,
            first.organization_id,
            first.context_chain_id,
            first.transcript_partition_id,
            first.application_model_revision_id,
            first.application_model_revision_sha256.clone(),
            first.tool_truth_bundle_seal_id,
            first.tool_truth_member_set_sha256.clone(),
            vec![
                member(InvestigationContextAuthorityV1::ToolTruth, "tool-truth"),
                member(InvestigationContextAuthorityV1::CanonicalFact, "target"),
                member(
                    InvestigationContextAuthorityV1::ApplicationUnderstanding,
                    "au",
                ),
            ],
        )
        .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn final_snapshot_rejects_vacuous_methodology() {
        let error = InvestigationAnalysisSnapshotV1::host_seal(
            Uuid::from_u128(11),
            &baseline(),
            vec![member(
                InvestigationContextAuthorityV1::CanonicalFact,
                "target",
            )],
            InvestigationMethodologyQueryIntentV1 {
                normalized_tags: vec!["http".into()],
                top_k: 8,
            },
            vec![],
            vec![],
        )
        .unwrap_err();
        assert_eq!(error, InvestigationSnapshotError::VacuousMethodologyResult);
    }

    #[test]
    fn explicit_methodology_omission_is_non_vacuous_and_raw_bodies_are_absent() {
        let sealed = InvestigationAnalysisSnapshotV1::host_seal(
            Uuid::from_u128(11),
            &baseline(),
            vec![member(
                InvestigationContextAuthorityV1::CanonicalFact,
                "target",
            )],
            InvestigationMethodologyQueryIntentV1 {
                normalized_tags: vec!["HTTP".into(), "http".into()],
                top_k: 8,
            },
            vec![],
            vec![InvestigationContextOmissionV1 {
                layer: "methodology".into(),
                reason_code: "no_match".into(),
                omitted_subject_sha256: hash('d'),
            }],
        )
        .unwrap();
        assert_eq!(sealed.methodology_query.normalized_tags, ["http"]);
        let serialized = serde_json::to_value(member(
            InvestigationContextAuthorityV1::CanonicalFact,
            "target",
        ))
        .unwrap();
        assert!(serialized.get("value").is_none());
        assert!(serialized.get("body").is_none());
    }
}
