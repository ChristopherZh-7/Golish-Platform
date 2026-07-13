use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::classification::{AssertionVisibility, KnowledgeClassification};
use crate::source_ref::SourceRef;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionKind {
    Observation,
    CheckedEmpty,
    VerifiedOutcome,
    RefutedOutcome,
    TechniqueExperience,
    CleanupAttestation,
    ResidualRisk,
}

impl AssertionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::CheckedEmpty => "checked_empty",
            Self::VerifiedOutcome => "verified_outcome",
            Self::RefutedOutcome => "refuted_outcome",
            Self::TechniqueExperience => "technique_experience",
            Self::CleanupAttestation => "cleanup_attestation",
            Self::ResidualRisk => "residual_risk",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionStatus {
    Active,
    Superseded,
    Refuted,
    Expired,
}

impl AssertionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Refuted => "refuted",
            Self::Expired => "expired",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VaultRef(pub Uuid);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "object_type", content = "value", rename_all = "snake_case")]
pub enum AssertionObject {
    Json(serde_json::Value),
    VaultRef(VaultRef),
}

impl AssertionObject {
    pub fn canonical_hash(&self) -> Result<String, AssertionError> {
        let encoded = match self {
            Self::Json(value) => format!("json:{}", canonical_json_string(value)?),
            Self::VaultRef(value) => format!("vault_ref:{}", value.0.hyphenated()),
        };
        Ok(hex_sha256(encoded.as_bytes()))
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AssertionIdentity {
    pub subject_key: String,
    pub predicate: String,
    pub object_hash: String,
    pub identity_hash: String,
}

impl AssertionIdentity {
    pub fn derive(
        subject_key: impl Into<String>,
        predicate: impl Into<String>,
        object: &AssertionObject,
    ) -> Result<Self, AssertionError> {
        let subject_key = subject_key.into().trim().to_string();
        let predicate = predicate.into().trim().to_string();
        let object_hash = object.canonical_hash()?;
        let identity_hash =
            hex_sha256(format!("{subject_key}\0{predicate}\0{object_hash}").as_bytes());
        let identity = Self {
            subject_key,
            predicate,
            object_hash,
            identity_hash,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn validate(&self) -> Result<(), AssertionError> {
        if self.subject_key.trim().is_empty()
            || self.predicate.trim().is_empty()
            || self.object_hash.len() != 64
            || !self
                .object_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.identity_hash.len() != 64
            || !self
                .identity_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(AssertionError::InvalidIdentity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeAssertionDraft {
    pub assertion_id: Uuid,
    pub visibility: AssertionVisibility,
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_hash: String,
    pub source: SourceRef,
    pub identity: AssertionIdentity,
    pub kind: AssertionKind,
    pub status: AssertionStatus,
    pub object: AssertionObject,
    pub classification: KnowledgeClassification,
    pub evidence_ids: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
}

impl KnowledgeAssertionDraft {
    pub fn validate(self) -> Result<KnowledgeAssertion, AssertionError> {
        self.source
            .validate()
            .map_err(|_| AssertionError::InvalidSource)?;
        self.identity.validate()?;
        if self.source_scope_snapshot_hash.trim().is_empty() {
            return Err(AssertionError::EmptyScopeSnapshotHash);
        }
        if self.evidence_ids.is_empty() || self.evidence_ids.iter().any(|id| *id <= 0) {
            return Err(AssertionError::MissingEvidence);
        }
        if self.kind == AssertionKind::CheckedEmpty && self.fresh_until.is_none() {
            return Err(AssertionError::CheckedEmptyMissingFreshUntil);
        }
        if self
            .valid_to
            .is_some_and(|valid_to| valid_to < self.valid_from)
        {
            return Err(AssertionError::InvalidValidityWindow);
        }
        if self.identity.object_hash != self.object.canonical_hash()? {
            return Err(AssertionError::ObjectHashMismatch);
        }
        if matches!(&self.object, AssertionObject::Json(value) if contains_plaintext_secret(value))
        {
            return Err(AssertionError::PlaintextSecret);
        }
        if matches!(self.visibility, AssertionVisibility::GlobalSanitized)
            && (!self.classification.allows_global_sanitized()
                || self.kind != AssertionKind::TechniqueExperience
                || matches!(self.object, AssertionObject::VaultRef(_))
                || matches!(&self.object, AssertionObject::Json(value) if contains_customer_reference(value)))
        {
            return Err(AssertionError::GlobalContainsCustomerMaterial);
        }

        let content_hash = hex_sha256(
            serde_json::to_string(&(
                &self.visibility,
                self.source_operation_id,
                &self.source_scope_snapshot_hash,
                &self.source,
                &self.identity,
                self.kind,
                self.status,
                &self.object,
                self.classification,
                &self.evidence_ids,
                self.valid_from.timestamp_micros(),
                self.valid_to.map(|value| value.timestamp_micros()),
                self.fresh_until.map(|value| value.timestamp_micros()),
            ))
            .map_err(|_| AssertionError::Serialization)?
            .as_bytes(),
        );

        Ok(KnowledgeAssertion {
            assertion_id: self.assertion_id,
            visibility: self.visibility,
            source_operation_id: self.source_operation_id,
            source_scope_snapshot_hash: self.source_scope_snapshot_hash,
            source: self.source,
            identity: self.identity,
            kind: self.kind,
            status: self.status,
            object: self.object,
            classification: self.classification,
            evidence_ids: self.evidence_ids,
            valid_from: self.valid_from,
            valid_to: self.valid_to,
            fresh_until: self.fresh_until,
            content_hash,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeAssertion {
    pub assertion_id: Uuid,
    pub visibility: AssertionVisibility,
    pub source_operation_id: Uuid,
    pub source_scope_snapshot_hash: String,
    pub source: SourceRef,
    pub identity: AssertionIdentity,
    pub kind: AssertionKind,
    pub status: AssertionStatus,
    pub object: AssertionObject,
    pub classification: KnowledgeClassification,
    pub evidence_ids: Vec<i64>,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub fresh_until: Option<DateTime<Utc>>,
    pub content_hash: String,
}

impl KnowledgeAssertion {
    pub fn validate_integrity(&self) -> Result<(), AssertionError> {
        let validated = KnowledgeAssertionDraft {
            assertion_id: self.assertion_id,
            visibility: self.visibility.clone(),
            source_operation_id: self.source_operation_id,
            source_scope_snapshot_hash: self.source_scope_snapshot_hash.clone(),
            source: self.source.clone(),
            identity: self.identity.clone(),
            kind: self.kind,
            status: self.status,
            object: self.object.clone(),
            classification: self.classification,
            evidence_ids: self.evidence_ids.clone(),
            valid_from: self.valid_from,
            valid_to: self.valid_to,
            fresh_until: self.fresh_until,
        }
        .validate()?;
        if validated.content_hash != self.content_hash {
            return Err(AssertionError::ContentHashMismatch);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_for_test(
        visibility: AssertionVisibility,
        subject_key: impl Into<String>,
        predicate: impl Into<String>,
        object: AssertionObject,
        kind: AssertionKind,
        status: AssertionStatus,
        source: SourceRef,
        classification: KnowledgeClassification,
    ) -> Result<Self, AssertionError> {
        let identity = AssertionIdentity::derive(subject_key, predicate, &object)?;
        KnowledgeAssertionDraft {
            assertion_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, identity.identity_hash.as_bytes()),
            visibility,
            source_operation_id: Uuid::nil(),
            source_scope_snapshot_hash: "test-scope-snapshot".to_string(),
            source,
            identity,
            kind,
            status,
            object,
            classification,
            evidence_ids: vec![1],
            valid_from: Utc.timestamp_opt(0, 0).single().expect("unix epoch"),
            valid_to: None,
            fresh_until: None,
        }
        .validate()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AssertionError {
    #[error("invalid assertion identity")]
    InvalidIdentity,
    #[error("invalid canonical source")]
    InvalidSource,
    #[error("source scope snapshot hash cannot be empty")]
    EmptyScopeSnapshotHash,
    #[error("assertion must cite positive evidence ids")]
    MissingEvidence,
    #[error("checked-empty assertion requires fresh_until")]
    CheckedEmptyMissingFreshUntil,
    #[error("assertion validity window is inverted")]
    InvalidValidityWindow,
    #[error("assertion object hash does not match its identity")]
    ObjectHashMismatch,
    #[error("assertion content hash does not match its canonical fields")]
    ContentHashMismatch,
    #[error("global-sanitized assertion contains customer or vault material")]
    GlobalContainsCustomerMaterial,
    #[error("assertion serialization failed")]
    Serialization,
    #[error("plaintext secret material must be stored as a VaultRef")]
    PlaintextSecret,
}

impl AssertionError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidIdentity => "memory_assertion_identity_invalid",
            Self::InvalidSource => "memory_assertion_source_invalid",
            Self::EmptyScopeSnapshotHash => "memory_assertion_scope_hash_empty",
            Self::MissingEvidence => "memory_assertion_evidence_missing",
            Self::CheckedEmptyMissingFreshUntil => {
                "memory_assertion_checked_empty_freshness_missing"
            }
            Self::InvalidValidityWindow => "memory_assertion_validity_invalid",
            Self::ObjectHashMismatch => "memory_assertion_object_hash_mismatch",
            Self::ContentHashMismatch => "memory_assertion_content_hash_mismatch",
            Self::GlobalContainsCustomerMaterial => "memory_global_sanitized_policy_violation",
            Self::Serialization => "memory_assertion_serialization_failed",
            Self::PlaintextSecret => "memory_assertion_plaintext_secret_rejected",
        }
    }
}

pub fn canonical_json_string(value: &serde_json::Value) -> Result<String, AssertionError> {
    fn normalized(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(values) => {
                let mut keys = values.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                let mut result = serde_json::Map::new();
                for key in keys {
                    result.insert(key.clone(), normalized(&values[key]));
                }
                serde_json::Value::Object(result)
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(normalized).collect())
            }
            scalar => scalar.clone(),
        }
    }

    serde_json::to_string(&normalized(value)).map_err(|_| AssertionError::Serialization)
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn contains_plaintext_secret(value: &serde_json::Value) -> bool {
    contains_named_material(
        value,
        &[
            "api_key",
            "authorization",
            "cookie",
            "credential",
            "password",
            "passwd",
            "private_key",
            "secret",
            "session_cookie",
            "token",
        ],
    )
}

fn contains_customer_reference(value: &serde_json::Value) -> bool {
    contains_named_material(
        value,
        &[
            "evidence_id",
            "evidence_ids",
            "hostname",
            "ip",
            "organization_id",
            "project_scope_id",
            "source_operation_id",
            "target_id",
            "url",
        ],
    )
}

fn contains_named_material(value: &serde_json::Value, forbidden_keys: &[&str]) -> bool {
    match value {
        serde_json::Value::Object(values) => values.iter().any(|(key, value)| {
            (forbidden_keys.contains(&key.to_ascii_lowercase().as_str()) && !value.is_null())
                || contains_named_material(value, forbidden_keys)
        }),
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| contains_named_material(value, forbidden_keys)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assertion_identity_distinguishes_objects_for_same_source_predicate() {
        let base = AssertionIdentity::derive(
            "target:one",
            "has_ip",
            &AssertionObject::Json(serde_json::json!("192.0.2.1")),
        )
        .expect("base identity");
        let other = AssertionIdentity::derive(
            "target:one",
            "has_ip",
            &AssertionObject::Json(serde_json::json!("192.0.2.2")),
        )
        .expect("other identity");
        assert_ne!(base, other);
        assert!(base.validate().is_ok());
    }

    #[test]
    fn global_sanitized_rejects_vault_material() {
        let object = AssertionObject::VaultRef(VaultRef(Uuid::from_u128(7)));
        let error = KnowledgeAssertionDraft {
            assertion_id: Uuid::from_u128(1),
            visibility: AssertionVisibility::GlobalSanitized,
            source_operation_id: Uuid::from_u128(2),
            source_scope_snapshot_hash: "scope".to_string(),
            source: SourceRef {
                source_kind: crate::source_ref::CanonicalSourceKind::FactDelta,
                row_id: crate::source_ref::CanonicalRowId::Int64(3),
                source_stream_key: "fact:3".to_string(),
                version: 1,
            },
            identity: AssertionIdentity::derive("technique", "works", &object).expect("identity"),
            kind: AssertionKind::TechniqueExperience,
            status: AssertionStatus::Active,
            object,
            classification: KnowledgeClassification::Internal,
            evidence_ids: vec![4],
            valid_from: Utc::now(),
            valid_to: None,
            fresh_until: None,
        }
        .validate()
        .expect_err("vault material must be rejected");
        assert_eq!(error, AssertionError::GlobalContainsCustomerMaterial);
    }

    #[test]
    fn plaintext_secret_is_rejected_in_favor_of_vault_ref() {
        let object = AssertionObject::Json(serde_json::json!({"password": "not-allowed"}));
        let error = KnowledgeAssertionDraft {
            assertion_id: Uuid::from_u128(11),
            visibility: AssertionVisibility::OrganizationLongTerm {
                project_scope_id: crate::scope::ProjectScopeId(Uuid::from_u128(12)),
                organization_id_at_time: Uuid::from_u128(13),
            },
            source_operation_id: Uuid::from_u128(14),
            source_scope_snapshot_hash: "scope".to_string(),
            source: SourceRef {
                source_kind: crate::source_ref::CanonicalSourceKind::FactDelta,
                row_id: crate::source_ref::CanonicalRowId::Int64(15),
                source_stream_key: "fact:15".to_string(),
                version: 1,
            },
            identity: AssertionIdentity::derive("target", "credential", &object).expect("identity"),
            kind: AssertionKind::Observation,
            status: AssertionStatus::Active,
            object,
            classification: KnowledgeClassification::Restricted,
            evidence_ids: vec![16],
            valid_from: Utc::now(),
            valid_to: None,
            fresh_until: None,
        }
        .validate()
        .expect_err("plaintext secret must be rejected");
        assert_eq!(error, AssertionError::PlaintextSecret);
    }
}
