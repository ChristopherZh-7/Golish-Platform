//! Content-addressed methodology corpus contracts for Investigation.
//!
//! Methodology documents are untrusted knowledge signals. They never carry
//! instruction authority, tool authority, scope, or proof authority. The host
//! validates a corpus manifest and exposes only deterministic document refs to
//! the Investigation query layer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const METHODOLOGY_PARSER_CONTRACT_V1: &str = "golish-methodology-skill-parser@1";
pub const METHODOLOGY_INDEX_CONTRACT_V1: &str = "golish-methodology-tag-index@1";
pub const MAX_METHODOLOGY_QUERY_TAGS: usize = 64;
pub const MAX_METHODOLOGY_HITS: u32 = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DeterministicCorpusId(String);

impl DeterministicCorpusId {
    pub fn derive(material: &MethodologyCorpusIdentityMaterial<'_>) -> Self {
        Self(format!("corpus:{}", sha256_json(material)))
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, MethodologyContractError> {
        let value = value.into();
        validate_prefixed_sha256(&value, "corpus:", "corpus_id")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DeterministicDocumentId(String);

impl DeterministicDocumentId {
    pub fn derive(relative_path: &str, content_sha256: &str) -> Self {
        Self(format!(
            "document:{}",
            sha256_json(&(relative_path, content_sha256))
        ))
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, MethodologyContractError> {
        let value = value.into();
        validate_prefixed_sha256(&value, "document:", "document_id")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodologySourceKindV1 {
    GolishBuiltin,
    ThirdPartySkillCorpus,
    CustomerApprovedCorpus,
}

impl MethodologySourceKindV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GolishBuiltin => "golish_builtin",
            Self::ThirdPartySkillCorpus => "third_party_skill_corpus",
            Self::CustomerApprovedCorpus => "customer_approved_corpus",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodologySignatureStateV1 {
    Verified,
    Unknown,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MethodologyCorpusManifestV1 {
    pub corpus_id: DeterministicCorpusId,
    pub source_kind: MethodologySourceKindV1,
    pub upstream_url: Option<String>,
    pub upstream_revision: String,
    pub license_spdx: String,
    pub license_text_sha256: String,
    pub signature_state: MethodologySignatureStateV1,
    pub trust_store_epoch: u64,
    pub document_count: u32,
    pub content_root_sha256: String,
    pub parser_contract_version: String,
    pub index_contract_version: String,
    pub ingested_at: DateTime<Utc>,
    pub superseded_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    instruction_authority: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MethodologyCorpusIdentityMaterial<'a> {
    pub source_kind: MethodologySourceKindV1,
    pub upstream_url: Option<&'a str>,
    pub upstream_revision: &'a str,
    pub license_spdx: &'a str,
    pub license_text_sha256: &'a str,
    pub document_count: u32,
    pub content_root_sha256: &'a str,
    pub parser_contract_version: &'a str,
    pub index_contract_version: &'a str,
}

#[derive(Debug, Clone)]
pub struct NewMethodologyCorpusManifestV1 {
    pub claimed_corpus_id: DeterministicCorpusId,
    pub source_kind: MethodologySourceKindV1,
    pub upstream_url: Option<String>,
    pub upstream_revision: String,
    pub license_spdx: String,
    pub license_text_sha256: String,
    pub signature_state: MethodologySignatureStateV1,
    pub trust_store_epoch: u64,
    pub document_count: u32,
    pub content_root_sha256: String,
    pub parser_contract_version: String,
    pub index_contract_version: String,
    pub ingested_at: DateTime<Utc>,
    pub superseded_at: Option<DateTime<Utc>>,
}

impl MethodologyCorpusManifestV1 {
    pub fn validate(
        input: NewMethodologyCorpusManifestV1,
    ) -> Result<Self, MethodologyContractError> {
        validate_nonempty_bounded(&input.upstream_revision, 256, "upstream_revision")?;
        validate_nonempty_bounded(&input.license_spdx, 128, "license_spdx")?;
        validate_sha256(&input.license_text_sha256, "license_text_sha256")?;
        validate_sha256(&input.content_root_sha256, "content_root_sha256")?;
        if input.document_count == 0 {
            return Err(MethodologyContractError::InvalidField(
                "document_count must be positive".into(),
            ));
        }
        if input.trust_store_epoch == 0 {
            return Err(MethodologyContractError::InvalidField(
                "trust_store_epoch must be positive".into(),
            ));
        }
        if input.parser_contract_version != METHODOLOGY_PARSER_CONTRACT_V1 {
            return Err(MethodologyContractError::UnsupportedContract(
                input.parser_contract_version,
            ));
        }
        if input.index_contract_version != METHODOLOGY_INDEX_CONTRACT_V1 {
            return Err(MethodologyContractError::UnsupportedContract(
                input.index_contract_version,
            ));
        }
        if input
            .upstream_url
            .as_deref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 2_048)
        {
            return Err(MethodologyContractError::InvalidField(
                "upstream_url is empty or too long".into(),
            ));
        }
        if input
            .superseded_at
            .is_some_and(|value| value < input.ingested_at)
        {
            return Err(MethodologyContractError::InvalidField(
                "superseded_at predates ingested_at".into(),
            ));
        }
        let identity = MethodologyCorpusIdentityMaterial {
            source_kind: input.source_kind,
            upstream_url: input.upstream_url.as_deref(),
            upstream_revision: &input.upstream_revision,
            license_spdx: &input.license_spdx,
            license_text_sha256: &input.license_text_sha256,
            document_count: input.document_count,
            content_root_sha256: &input.content_root_sha256,
            parser_contract_version: &input.parser_contract_version,
            index_contract_version: &input.index_contract_version,
        };
        let derived = DeterministicCorpusId::derive(&identity);
        if derived != input.claimed_corpus_id {
            return Err(MethodologyContractError::IdentityMismatch {
                kind: "corpus_id",
                expected: derived.0,
                actual: input.claimed_corpus_id.0,
            });
        }
        Ok(Self {
            corpus_id: derived,
            source_kind: input.source_kind,
            upstream_url: input.upstream_url,
            upstream_revision: input.upstream_revision,
            license_spdx: input.license_spdx,
            license_text_sha256: input.license_text_sha256,
            signature_state: input.signature_state,
            trust_store_epoch: input.trust_store_epoch,
            document_count: input.document_count,
            content_root_sha256: input.content_root_sha256,
            parser_contract_version: input.parser_contract_version,
            index_contract_version: input.index_contract_version,
            ingested_at: input.ingested_at,
            superseded_at: input.superseded_at,
            instruction_authority: false,
        })
    }

    pub const fn instruction_authority(&self) -> bool {
        self.instruction_authority
    }

    pub fn authorize_for_query(
        &self,
        policy: &MethodologyTrustPolicyV1,
    ) -> Result<(), MethodologyContractError> {
        if self.instruction_authority {
            return Err(MethodologyContractError::InstructionAuthorityForbidden);
        }
        if self.signature_state != MethodologySignatureStateV1::Verified {
            return Err(MethodologyContractError::UntrustedSignature(
                self.signature_state,
            ));
        }
        if self.superseded_at.is_some() {
            return Err(MethodologyContractError::SupersededCorpus);
        }
        if self.trust_store_epoch != policy.required_trust_store_epoch {
            return Err(MethodologyContractError::StaleTrustEpoch {
                expected: policy.required_trust_store_epoch,
                actual: self.trust_store_epoch,
            });
        }
        if !policy.allowed_license_spdx.contains(&self.license_spdx) {
            return Err(MethodologyContractError::LicenseRejected(
                self.license_spdx.clone(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodologyTrustPolicyV1 {
    pub required_trust_store_epoch: u64,
    pub allowed_license_spdx: BTreeSet<String>,
}

impl MethodologyTrustPolicyV1 {
    pub fn new(
        required_trust_store_epoch: u64,
        allowed_license_spdx: impl IntoIterator<Item = String>,
    ) -> Result<Self, MethodologyContractError> {
        if required_trust_store_epoch == 0 {
            return Err(MethodologyContractError::InvalidField(
                "required_trust_store_epoch must be positive".into(),
            ));
        }
        let allowed_license_spdx = allowed_license_spdx
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .collect::<BTreeSet<_>>();
        if allowed_license_spdx.is_empty() {
            return Err(MethodologyContractError::InvalidField(
                "allowed license set must not be empty".into(),
            ));
        }
        Ok(Self {
            required_trust_store_epoch,
            allowed_license_spdx,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MethodologyDocumentDescriptorV1 {
    pub document_id: DeterministicDocumentId,
    pub relative_path: String,
    pub content_sha256: String,
    pub normalized_tags: Vec<String>,
    pub safe_excerpt_ref: String,
    #[serde(skip)]
    instruction_authority: bool,
}

impl MethodologyDocumentDescriptorV1 {
    pub fn validate(
        claimed_document_id: DeterministicDocumentId,
        relative_path: String,
        content_sha256: String,
        tags: impl IntoIterator<Item = String>,
    ) -> Result<Self, MethodologyContractError> {
        validate_relative_path_text(&relative_path)?;
        validate_sha256(&content_sha256, "content_sha256")?;
        let normalized_tags = normalize_methodology_tags(tags)?;
        if normalized_tags.is_empty() {
            return Err(MethodologyContractError::InvalidField(
                "methodology document requires at least one tag".into(),
            ));
        }
        let derived = DeterministicDocumentId::derive(&relative_path, &content_sha256);
        if derived != claimed_document_id {
            return Err(MethodologyContractError::IdentityMismatch {
                kind: "document_id",
                expected: derived.0,
                actual: claimed_document_id.0,
            });
        }
        Ok(Self {
            document_id: derived,
            relative_path: relative_path.clone(),
            content_sha256: content_sha256.clone(),
            normalized_tags,
            safe_excerpt_ref: format!("methodology://{content_sha256}/{relative_path}"),
            instruction_authority: false,
        })
    }

    pub const fn instruction_authority(&self) -> bool {
        self.instruction_authority
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodologyQueryV1 {
    pub normalized_tags: Vec<String>,
    pub top_k: u32,
}

impl MethodologyQueryV1 {
    pub fn new(
        tags: impl IntoIterator<Item = String>,
        top_k: u32,
    ) -> Result<Self, MethodologyContractError> {
        if top_k == 0 || top_k > MAX_METHODOLOGY_HITS {
            return Err(MethodologyContractError::InvalidField(format!(
                "top_k must be between 1 and {MAX_METHODOLOGY_HITS}"
            )));
        }
        let normalized_tags = normalize_methodology_tags(tags)?;
        if normalized_tags.is_empty() {
            return Err(MethodologyContractError::InvalidField(
                "methodology query requires at least one tag".into(),
            ));
        }
        if normalized_tags.len() > MAX_METHODOLOGY_QUERY_TAGS {
            return Err(MethodologyContractError::InvalidField(format!(
                "methodology query exceeds {MAX_METHODOLOGY_QUERY_TAGS} tags"
            )));
        }
        Ok(Self {
            normalized_tags,
            top_k,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MethodologyHitV1 {
    pub corpus_id: DeterministicCorpusId,
    pub document_id: DeterministicDocumentId,
    pub relative_path: String,
    pub content_sha256: String,
    pub safe_excerpt_ref: String,
    pub score_micros: i64,
    pub matched_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MethodologyQueryResultV1 {
    pub hits: Vec<MethodologyHitV1>,
    pub omitted_hit_count: u32,
    pub result_set_sha256: String,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MethodologyContractError {
    #[error("invalid methodology field: {0}")]
    InvalidField(String),
    #[error("unsupported methodology contract: {0}")]
    UnsupportedContract(String),
    #[error("{kind} mismatch: expected {expected}, got {actual}")]
    IdentityMismatch {
        kind: &'static str,
        expected: String,
        actual: String,
    },
    #[error("methodology signature is not trusted: {0:?}")]
    UntrustedSignature(MethodologySignatureStateV1),
    #[error("methodology trust epoch is stale: expected {expected}, got {actual}")]
    StaleTrustEpoch { expected: u64, actual: u64 },
    #[error("methodology license is not allowed: {0}")]
    LicenseRejected(String),
    #[error("methodology corpus is superseded")]
    SupersededCorpus,
    #[error("methodology content cannot carry instruction authority")]
    InstructionAuthorityForbidden,
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{digest}")
}

pub fn sha256_json(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("methodology identity material is serializable");
    sha256_bytes(&bytes)
        .strip_prefix("sha256:")
        .expect("sha helper always prefixes")
        .to_string()
}

pub fn methodology_result_set_sha256(hits: &[MethodologyHitV1]) -> String {
    sha256_bytes(&serde_json::to_vec(hits).expect("methodology result material is serializable"))
}

fn validate_nonempty_bounded(
    value: &str,
    max_len: usize,
    field: &str,
) -> Result<(), MethodologyContractError> {
    if value.trim().is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        return Err(MethodologyContractError::InvalidField(format!(
            "{field} is empty, too long, or contains control characters"
        )));
    }
    Ok(())
}

fn validate_sha256(value: &str, field: &str) -> Result<(), MethodologyContractError> {
    validate_prefixed_sha256(value, "sha256:", field)
}

fn validate_prefixed_sha256(
    value: &str,
    prefix: &str,
    field: &str,
) -> Result<(), MethodologyContractError> {
    let Some(hex) = value.strip_prefix(prefix) else {
        return Err(MethodologyContractError::InvalidField(format!(
            "{field} must start with {prefix}"
        )));
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(MethodologyContractError::InvalidField(format!(
            "{field} must contain 64 lowercase hexadecimal characters"
        )));
    }
    Ok(())
}

fn validate_relative_path_text(value: &str) -> Result<(), MethodologyContractError> {
    validate_nonempty_bounded(value, 1_024, "relative_path")?;
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains("//")
        || value
            .split(['/', '\\'])
            .any(|part| part == ".." || part == "." || part.is_empty())
    {
        return Err(MethodologyContractError::InvalidField(
            "relative_path is absolute or contains traversal".into(),
        ));
    }
    Ok(())
}

fn normalize_methodology_tags(
    tags: impl IntoIterator<Item = String>,
) -> Result<Vec<String>, MethodologyContractError> {
    let mut normalized = BTreeSet::new();
    for tag in tags {
        let tag = tag.trim().to_ascii_lowercase();
        if tag.is_empty()
            || tag.len() > 128
            || tag.chars().any(char::is_control)
            || !tag
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.' | '/'))
        {
            return Err(MethodologyContractError::InvalidField(
                "methodology tag is invalid".into(),
            ));
        }
        normalized.insert(tag);
    }
    Ok(normalized.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methodology_contract_rejects_instruction_authority_and_untrusted_policy() {
        let content_root = sha256_bytes(b"root");
        let identity = MethodologyCorpusIdentityMaterial {
            source_kind: MethodologySourceKindV1::GolishBuiltin,
            upstream_url: None,
            upstream_revision: "fixture-rev",
            license_spdx: "Apache-2.0",
            license_text_sha256: &sha256_bytes(b"license"),
            document_count: 1,
            content_root_sha256: &content_root,
            parser_contract_version: METHODOLOGY_PARSER_CONTRACT_V1,
            index_contract_version: METHODOLOGY_INDEX_CONTRACT_V1,
        };
        let manifest = MethodologyCorpusManifestV1::validate(NewMethodologyCorpusManifestV1 {
            claimed_corpus_id: DeterministicCorpusId::derive(&identity),
            source_kind: identity.source_kind,
            upstream_url: None,
            upstream_revision: identity.upstream_revision.into(),
            license_spdx: identity.license_spdx.into(),
            license_text_sha256: identity.license_text_sha256.into(),
            signature_state: MethodologySignatureStateV1::Unknown,
            trust_store_epoch: 1,
            document_count: 1,
            content_root_sha256: content_root.clone(),
            parser_contract_version: identity.parser_contract_version.into(),
            index_contract_version: identity.index_contract_version.into(),
            ingested_at: Utc::now(),
            superseded_at: None,
        })
        .unwrap();
        assert!(!manifest.instruction_authority());
        let policy = MethodologyTrustPolicyV1::new(1, ["Apache-2.0".into()]).unwrap();
        assert_eq!(
            manifest.authorize_for_query(&policy),
            Err(MethodologyContractError::UntrustedSignature(
                MethodologySignatureStateV1::Unknown
            ))
        );
    }
}
