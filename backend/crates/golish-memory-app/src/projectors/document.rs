use std::cmp::max;

use chrono::{DateTime, Utc};
use golish_memory_domain::{
    assertion::{canonical_json_string, AssertionObject},
    classification::KnowledgeClassification,
    scope::ProjectScopeId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::ports::{DocumentProjectionPort, MemoryError};

pub const DOCUMENT_PROJECTION_SCHEMA_V1: i32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectedDocument {
    pub document_id: Uuid,
    pub document_key: String,
    pub project_scope_id: Option<ProjectScopeId>,
    pub source_stream_key: String,
    pub source_version: i64,
    pub projection_schema_version: i32,
    pub redaction_policy_version: i32,
    pub assertion_ids: Vec<Uuid>,
    pub redacted_content: String,
    pub content_hash: String,
    pub classification: KnowledgeClassification,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug)]
pub struct DocumentProjector {
    redaction_policy_version: i32,
}

impl DocumentProjector {
    pub const fn new(redaction_policy_version: i32) -> Self {
        Self {
            redaction_policy_version,
        }
    }

    pub async fn project<P>(self, port: &P, event_id: Uuid) -> Result<Uuid, MemoryError>
    where
        P: DocumentProjectionPort,
    {
        if self.redaction_policy_version <= 0 {
            return Err(MemoryError::Policy(
                "memory_redaction_policy_version_invalid".to_string(),
            ));
        }
        let mut assertions = port.load_promoted_assertions(event_id).await?;
        if assertions.is_empty() {
            return Err(MemoryError::NoPromotedAssertions);
        }
        assertions.sort_by(|left, right| {
            left.identity
                .identity_hash
                .cmp(&right.identity.identity_hash)
                .then_with(|| left.assertion_id.cmp(&right.assertion_id))
        });

        let first = &assertions[0];
        let project_scope_id = first.visibility.project_scope_id();
        let source_stream_key = first.source.source_stream_key.clone();
        let source_version = first.source.version;
        if assertions.iter().any(|assertion| {
            assertion.visibility.project_scope_id() != project_scope_id
                || assertion.source.source_stream_key != source_stream_key
                || assertion.source.version != source_version
        }) {
            return Err(MemoryError::MixedDocumentSources);
        }

        let document_key = hex_sha256(
            format!(
                "{}\0{}\0{}\0{}",
                project_scope_id
                    .map(|id| id.0.hyphenated().to_string())
                    .unwrap_or_else(|| "global_sanitized".to_string()),
                source_stream_key,
                source_version,
                self.redaction_policy_version
            )
            .as_bytes(),
        );
        let document_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, document_key.as_bytes());
        let mut classification = KnowledgeClassification::Public;
        let mut valid_from = first.valid_from;
        let mut valid_to = first.valid_to;
        let mut lines = Vec::with_capacity(assertions.len());
        let mut assertion_ids = Vec::with_capacity(assertions.len());

        for assertion in &assertions {
            classification = max_classification(classification, assertion.classification);
            valid_from = valid_from.min(assertion.valid_from);
            valid_to = max_optional_time(valid_to, assertion.valid_to);
            assertion_ids.push(assertion.assertion_id);
            let object = match &assertion.object {
                AssertionObject::Json(value) => serde_json::json!({
                    "kind": "json",
                    "value": value,
                }),
                AssertionObject::VaultRef(_) => serde_json::json!({
                    "kind": "vault_ref",
                    "redacted": true,
                    "object_hash": assertion.identity.object_hash,
                }),
            };
            let line = serde_json::json!({
                "assertion_id": assertion.assertion_id,
                "subject_key": assertion.identity.subject_key,
                "predicate": assertion.identity.predicate,
                "object": object,
                "assertion_kind": assertion.kind.as_str(),
                "classification": assertion.classification.as_str(),
                "evidence_ids": assertion.evidence_ids,
                "valid_from": assertion.valid_from,
                "valid_to": assertion.valid_to,
            });
            lines.push(canonical_json_string(&line).map_err(|_| MemoryError::Serialization)?);
        }
        let redacted_content = lines.join("\n");
        let content_hash = hex_sha256(redacted_content.as_bytes());
        let document = ProjectedDocument {
            document_id,
            document_key,
            project_scope_id,
            source_stream_key,
            source_version,
            projection_schema_version: DOCUMENT_PROJECTION_SCHEMA_V1,
            redaction_policy_version: self.redaction_policy_version,
            assertion_ids,
            redacted_content,
            content_hash,
            classification,
            valid_from,
            valid_to,
        };
        port.upsert_document(document).await
    }
}

fn max_classification(
    left: KnowledgeClassification,
    right: KnowledgeClassification,
) -> KnowledgeClassification {
    fn rank(value: KnowledgeClassification) -> u8 {
        match value {
            KnowledgeClassification::Public => 0,
            KnowledgeClassification::Internal => 1,
            KnowledgeClassification::CustomerConfidential => 2,
            KnowledgeClassification::Restricted => 3,
        }
    }
    if rank(left) >= rank(right) {
        left
    } else {
        right
    }
}

fn max_optional_time(
    left: Option<DateTime<Utc>>,
    right: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (left, right) {
        (Some(left), Some(right)) => Some(max(left, right)),
        (None, _) | (_, None) => None,
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
