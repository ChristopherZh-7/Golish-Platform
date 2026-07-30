//! Safe projection from sealed Candidate chunk rows into untrusted data-only
//! analyst/critic envelopes.  There is deliberately no live-source loader,
//! filesystem path, URL, network client, or feed updater in this module.

use golish_agent_kit::task_orchestrator::hypothesis_analysis::{
    CandidateBoundedPayload, CandidateChunkRef, CandidateInputKind,
    CandidateKnowledgeSignalAuthority,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_INPUT_KEY_CHARS: usize = 512;
const MAX_KIND_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateInputProvenance {
    ToolTruthAuthority,
    FrozenKnowledgeFeed,
    PreviousGeneration,
    CandidateResidual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateKnowledgeFeedEligibility {
    CurrentKnownVersionSigned,
    Stale,
    UnknownProductVersion,
    InvalidSignature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateAtTimeSubject {
    pub kind: String,
    pub identity_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UntrustedCandidateInputChunkEnvelope {
    pub input_id: Uuid,
    pub input_key: String,
    pub input_kind: CandidateInputKind,
    pub provenance: CandidateInputProvenance,
    pub at_time_subject: CandidateAtTimeSubject,
    pub source_hash: String,
    pub source_size: u64,
    pub chunk_ordinal: u32,
    pub chunk_census_hash: String,
    pub chunking_contract_version: u32,
    pub redaction_contract_version: u32,
    pub bounded_payload: CandidateBoundedPayload,
    pub bounded_payload_hash: String,
    pub instruction_authority: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SealedCandidateChunkProjectionRow {
    pub snapshot_ready: bool,
    pub input_id: Uuid,
    pub expected_input_id: Uuid,
    pub input_key: String,
    pub input_kind: CandidateInputKind,
    pub knowledge_feed_eligibility: Option<CandidateKnowledgeFeedEligibility>,
    pub provenance: CandidateInputProvenance,
    pub at_time_subject: CandidateAtTimeSubject,
    pub source_hash: String,
    pub source_size: u64,
    pub chunk_ordinal: u32,
    pub expected_chunk_ordinal: u32,
    pub chunk_census_hash: String,
    pub expected_chunk_census_hash: String,
    pub chunking_contract_version: u32,
    pub expected_chunking_contract_version: u32,
    pub redaction_contract_version: u32,
    pub expected_redaction_contract_version: u32,
    pub bounded_payload: CandidateBoundedPayload,
    pub persisted_payload_hash: String,
    pub max_chunk_bytes: u32,
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn payload_hash(payload: &CandidateBoundedPayload) -> anyhow::Result<(String, usize)> {
    let bytes = serde_json::to_vec(payload)?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok((format!("sha256:{digest}"), bytes.len()))
}

fn kind_matches_payload(kind: &CandidateInputKind, payload: &CandidateBoundedPayload) -> bool {
    matches!(
        (kind, payload),
        (_, CandidateBoundedPayload::ContentAddressedBlob { .. })
            | (
                CandidateInputKind::ToolTruthFact
                    | CandidateInputKind::ToolTruthObservation
                    | CandidateInputKind::ToolTruthEvidence
                    | CandidateInputKind::FactDelta
                    | CandidateInputKind::Relation,
                CandidateBoundedPayload::ToolTruthRecord { .. },
            )
            | (
                CandidateInputKind::TechniqueOutcome,
                CandidateBoundedPayload::TechniqueOutcome { .. },
            )
            | (
                CandidateInputKind::KnowledgeSignal,
                CandidateBoundedPayload::KnowledgeFeedMatch {
                    source_authority: CandidateKnowledgeSignalAuthority::KnowledgeSignalOnly,
                    ..
                },
            )
            | (
                CandidateInputKind::PreviousGeneration,
                CandidateBoundedPayload::PreviousGeneration { .. },
            )
            | (
                CandidateInputKind::ResidualRisk | CandidateInputKind::OpenObligation,
                CandidateBoundedPayload::ResidualOrObligation { .. },
            )
    )
}

pub(crate) fn project_sealed_candidate_chunk(
    row: SealedCandidateChunkProjectionRow,
) -> anyhow::Result<UntrustedCandidateInputChunkEnvelope> {
    anyhow::ensure!(row.snapshot_ready, "candidate snapshot is not sealed_ready");
    anyhow::ensure!(
        row.input_id == row.expected_input_id
            && row.chunk_ordinal == row.expected_chunk_ordinal
            && row.chunk_census_hash == row.expected_chunk_census_hash,
        "candidate chunk authority mismatch"
    );
    anyhow::ensure!(
        row.chunking_contract_version == row.expected_chunking_contract_version
            && row.redaction_contract_version == row.expected_redaction_contract_version,
        "candidate chunk contract mismatch"
    );
    anyhow::ensure!(
        !row.input_key.is_empty()
            && row.input_key.chars().count() <= MAX_INPUT_KEY_CHARS
            && !row.at_time_subject.kind.is_empty()
            && row.at_time_subject.kind.chars().count() <= MAX_KIND_CHARS,
        "candidate projection string bounds invalid"
    );
    anyhow::ensure!(
        valid_hash(&row.source_hash)
            && valid_hash(&row.chunk_census_hash)
            && valid_hash(&row.at_time_subject.identity_hash)
            && valid_hash(&row.persisted_payload_hash),
        "candidate projection hash malformed"
    );
    anyhow::ensure!(
        kind_matches_payload(&row.input_kind, &row.bounded_payload),
        "candidate input kind and payload disagree"
    );
    if row.input_kind == CandidateInputKind::KnowledgeSignal {
        anyhow::ensure!(
            row.knowledge_feed_eligibility
                == Some(CandidateKnowledgeFeedEligibility::CurrentKnownVersionSigned),
            "candidate knowledge feed is not a current signed known-version match"
        );
        let CandidateBoundedPayload::KnowledgeFeedMatch {
            feed_snapshot_id,
            feed_match_member_id,
            feed_kind,
            feed_version,
            published_at_unix_seconds,
            content_hash,
            manifest_hash,
            provenance_hash,
            signature_receipt_hash,
            product_version_match_hash,
            matcher_hash,
            member_hash,
            ..
        } = &row.bounded_payload
        else {
            unreachable!("kind/payload agreement checked above")
        };
        anyhow::ensure!(
            !feed_snapshot_id.is_nil()
                && !feed_match_member_id.is_nil()
                && !feed_kind.is_empty()
                && feed_kind.chars().count() <= MAX_KIND_CHARS
                && !feed_version.is_empty()
                && feed_version.chars().count() <= MAX_KIND_CHARS
                && *published_at_unix_seconds > 0
                && [
                    content_hash,
                    manifest_hash,
                    provenance_hash,
                    signature_receipt_hash,
                    product_version_match_hash,
                    matcher_hash,
                    member_hash,
                ]
                .into_iter()
                .all(|hash| valid_hash(hash)),
            "candidate knowledge feed match authority is malformed"
        );
    }
    let (derived_payload_hash, payload_bytes) = payload_hash(&row.bounded_payload)?;
    anyhow::ensure!(
        derived_payload_hash == row.persisted_payload_hash,
        "candidate payload hash mismatch"
    );
    anyhow::ensure!(
        payload_bytes <= row.max_chunk_bytes as usize,
        "candidate payload exceeds frozen chunk ceiling"
    );
    Ok(UntrustedCandidateInputChunkEnvelope {
        input_id: row.input_id,
        input_key: row.input_key,
        input_kind: row.input_kind,
        provenance: row.provenance,
        at_time_subject: row.at_time_subject,
        source_hash: row.source_hash,
        source_size: row.source_size,
        chunk_ordinal: row.chunk_ordinal,
        chunk_census_hash: row.chunk_census_hash,
        chunking_contract_version: row.chunking_contract_version,
        redaction_contract_version: row.redaction_contract_version,
        bounded_payload: row.bounded_payload,
        bounded_payload_hash: derived_payload_hash,
        instruction_authority: false,
    })
}

impl From<&UntrustedCandidateInputChunkEnvelope> for CandidateChunkRef {
    fn from(value: &UntrustedCandidateInputChunkEnvelope) -> Self {
        Self {
            input_id: value.input_id,
            input_key: value.input_key.clone(),
            input_kind: value.input_kind.clone(),
            chunk_id: Uuid::new_v5(&value.input_id, value.bounded_payload_hash.as_bytes()),
            chunk_ordinal: value.chunk_ordinal,
            chunk_census_hash: value.chunk_census_hash.clone(),
            source_hash: value.source_hash.clone(),
            bounded_payload: value.bounded_payload.clone(),
            bounded_payload_hash: value.bounded_payload_hash.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_always_removes_instruction_authority() {
        let payload = CandidateBoundedPayload::ResidualOrObligation {
            reason_code: "feed_stale".into(),
            authority_hash: format!("sha256:{}", "a".repeat(64)),
        };
        let persisted_payload_hash = payload_hash(&payload).unwrap().0;
        let input_id = Uuid::new_v4();
        let hash = format!("sha256:{}", "b".repeat(64));
        let row = SealedCandidateChunkProjectionRow {
            snapshot_ready: true,
            input_id,
            expected_input_id: input_id,
            input_key: "residual:feed".into(),
            input_kind: CandidateInputKind::ResidualRisk,
            knowledge_feed_eligibility: None,
            provenance: CandidateInputProvenance::CandidateResidual,
            at_time_subject: CandidateAtTimeSubject {
                kind: "organization".into(),
                identity_hash: hash.clone(),
            },
            source_hash: hash.clone(),
            source_size: 12,
            chunk_ordinal: 0,
            expected_chunk_ordinal: 0,
            chunk_census_hash: hash.clone(),
            expected_chunk_census_hash: hash,
            chunking_contract_version: 1,
            expected_chunking_contract_version: 1,
            redaction_contract_version: 1,
            expected_redaction_contract_version: 1,
            bounded_payload: payload,
            persisted_payload_hash,
            max_chunk_bytes: 16_384,
        };
        let projected = project_sealed_candidate_chunk(row).unwrap();
        assert!(!projected.instruction_authority);
    }
}
