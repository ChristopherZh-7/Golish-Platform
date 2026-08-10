//! Fixture/dev-only semantic Target Intel authority and receipt pipeline.
//!
//! The ordering in this module is intentional and testable:
//! collect -> persist redacted artifact -> append evidence -> authorize ->
//! project -> append immutable audit receipt -> expose a bounded summary.
//! No function here writes organization profiles, targets, DNS rows, or
//! `target_assets`; production continues to use the legacy hydrate path.

use std::collections::BTreeSet;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::types::AssetIntelExecutionRequest;
use super::PlannedNativeQuery;
use golish_pentest_domain::models::{AssetIntelPivot, AssetIntelPivotKind};

pub const INTEL_SEMANTIC_RECEIPT_KIND: &str = "intel.semantic_pivot_receipt.v1";
const MODEL_REF_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FixtureScopeSubject {
    ExactDomain(String),
    WildcardDomain(String),
    ExactIp(String),
    Cidr(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionAuthorization {
    exact_domains: BTreeSet<String>,
    wildcard_domains: BTreeSet<String>,
    exact_ips: BTreeSet<IpAddr>,
    cidrs: BTreeSet<String>,
}

impl ProjectionAuthorization {
    pub fn from_fixture_scope(subject: FixtureScopeSubject) -> Result<Self, String> {
        Self::from_fixture_scopes([subject])
    }

    pub fn from_fixture_scopes(
        subjects: impl IntoIterator<Item = FixtureScopeSubject>,
    ) -> Result<Self, String> {
        let mut authorization = Self::default();
        for subject in subjects {
            match subject {
                FixtureScopeSubject::ExactDomain(value) => {
                    authorization
                        .exact_domains
                        .insert(canonical_domain(&value)?);
                }
                FixtureScopeSubject::WildcardDomain(value) => {
                    let value = value
                        .trim()
                        .strip_prefix("*.")
                        .ok_or_else(|| "wildcard scope must start with *.".to_string())?;
                    authorization
                        .wildcard_domains
                        .insert(canonical_domain(value)?);
                }
                FixtureScopeSubject::ExactIp(value) => {
                    authorization.exact_ips.insert(
                        value
                            .trim()
                            .parse()
                            .map_err(|_| "invalid exact IP scope".to_string())?,
                    );
                }
                FixtureScopeSubject::Cidr(value) => {
                    let pivot = AssetIntelPivot::parse(AssetIntelPivotKind::Cidr, &value)
                        .map_err(|error| error.to_string())?;
                    authorization.cidrs.insert(pivot.value);
                }
            }
        }
        Ok(authorization)
    }

    pub fn allows_domain(&self, value: &str) -> bool {
        let Ok(value) = canonical_domain(value) else {
            return false;
        };
        self.exact_domains.contains(&value)
            || self.wildcard_domains.iter().any(|apex| {
                value != *apex
                    && value
                        .strip_suffix(apex)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
    }

    pub fn allows_ip(&self, value: &str) -> bool {
        let Ok(value) = value.trim().parse::<IpAddr>() else {
            return false;
        };
        self.exact_ips.contains(&value) || self.cidrs.iter().any(|cidr| cidr_contains(cidr, value))
    }

    pub fn allows_pivot(&self, pivot: &AssetIntelPivot) -> bool {
        match pivot.kind {
            AssetIntelPivotKind::Domain | AssetIntelPivotKind::Hostname => {
                self.allows_domain(&pivot.value)
            }
            AssetIntelPivotKind::Ip => self.allows_ip(&pivot.value),
            // Company/brand/profile/provider/certificate/ASN/ICP/email/GitHub/
            // repository/app identifiers never grant projection authority.
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectedIntelObservation {
    pub pivot: AssetIntelPivot,
    pub provider_id: String,
    pub confidence: f64,
    #[serde(default)]
    pub source_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectedIntelBatch {
    pub raw_payload: Value,
    pub observations: Vec<CollectedIntelObservation>,
}

impl CollectedIntelBatch {
    /// Collection is observation-only. Profile mutation belongs to projection
    /// after server-owned authorization and is zero at this boundary.
    pub const fn profile_writes(&self) -> usize {
        0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntelSemanticTerminalStatus {
    Succeeded,
    Empty,
    Blocked,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedIntelArtifact {
    pub artifact_ref: String,
    pub sha256: String,
    pub redacted_payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntelPivotReceiptV1 {
    pub kind: String,
    pub operation_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub session_id: uuid::Uuid,
    pub pivot: AssetIntelPivot,
    pub stable_query_key: String,
    pub provider_id: String,
    pub query_type: String,
    pub adapter_version: String,
    pub status: IntelSemanticTerminalStatus,
    pub artifact_ref: String,
    pub artifact_sha256: String,
    pub evidence_ref: String,
    pub landed_refs: Vec<String>,
    pub candidate_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassiveIntelSemanticSummary {
    pub pivot_ref: String,
    pub status: IntelSemanticTerminalStatus,
    pub query_receipts: Vec<String>,
    pub artifact_refs: Vec<String>,
    pub landed_refs: Vec<String>,
    pub landed_count: usize,
    pub candidate_refs: Vec<String>,
    pub candidate_count: usize,
    pub discovered_pivots: Vec<AssetIntelPivot>,
    pub discovered_count: usize,
    pub complete_set_sha256: String,
    pub duplicate_terminal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SemanticIntelPipelineError {
    #[error("fixture semantic intel requires strict-passive fake transport")]
    FixtureAuthorityMissing,
    #[error("semantic intel {stage} persistence failed: {reason}")]
    RetryablePersistence { stage: &'static str, reason: String },
}

/// Application adapters implement these four short persistence operations.
/// The existing `audit_log` is the receipt authority; `expansion_queue` is not
/// present in this contract and therefore cannot become a duplicate/frontier
/// source accidentally.
#[async_trait::async_trait]
pub trait SemanticIntelReceiptStore: Send + Sync {
    async fn load_terminal_receipt(
        &self,
        stable_query_key: &str,
    ) -> Result<Option<IntelPivotReceiptV1>, String>;
    async fn save_redacted_artifact(&self, artifact: &RedactedIntelArtifact) -> Result<(), String>;
    async fn append_evidence(
        &self,
        request: &AssetIntelExecutionRequest,
        artifact: &RedactedIntelArtifact,
        observations: &[CollectedIntelObservation],
    ) -> Result<String, String>;
    async fn append_audit_receipt(&self, receipt: &IntelPivotReceiptV1) -> Result<(), String>;
}

pub fn stable_semantic_query_key(
    request: &AssetIntelExecutionRequest,
    query: &PlannedNativeQuery,
) -> String {
    let value_hash = sha256_hex(request.pivot.value.as_bytes());
    let config_hash = sha256_hex(
        &serde_json::to_vec(&request.legacy_config).unwrap_or_else(|_| b"config-error".to_vec()),
    );
    format!(
        "pivot:v1:{}:{}:{}:{}:{}:{}",
        request.pivot.kind.as_str(),
        value_hash,
        query.adapter_version,
        query.provider_id,
        query.query_type,
        config_hash
    )
}

pub async fn run_fixture_semantic_query(
    request: &AssetIntelExecutionRequest,
    query: &PlannedNativeQuery,
    collected: CollectedIntelBatch,
    store: &dyn SemanticIntelReceiptStore,
) -> Result<PassiveIntelSemanticSummary, SemanticIntelPipelineError> {
    if !request.fixture_context.strict_passive || !request.fixture_context.fake_transport {
        return Err(SemanticIntelPipelineError::FixtureAuthorityMissing);
    }
    let stable_query_key = stable_semantic_query_key(request, query);
    if let Some(receipt) = store
        .load_terminal_receipt(&stable_query_key)
        .await
        .map_err(|reason| persistence_error("receipt_read", reason))?
    {
        return Ok(summary_from_receipt(receipt, Vec::new(), true));
    }

    let redacted_payload = redact_provider_payload(&collected.raw_payload);
    let artifact_bytes = serde_json::to_vec(&redacted_payload).unwrap_or_default();
    let artifact_sha256 = sha256_hex(&artifact_bytes);
    let artifact = RedactedIntelArtifact {
        artifact_ref: format!("intel-artifact:sha256:{artifact_sha256}"),
        sha256: artifact_sha256,
        redacted_payload,
    };
    store
        .save_redacted_artifact(&artifact)
        .await
        .map_err(|reason| persistence_error("artifact", reason))?;
    let evidence_ref = store
        .append_evidence(request, &artifact, &collected.observations)
        .await
        .map_err(|reason| persistence_error("evidence", reason))?;

    let mut landed_refs = Vec::new();
    let mut candidate_refs = Vec::new();
    for observation in &collected.observations {
        let reference = pivot_ref(&observation.pivot);
        if request
            .projection_authorization
            .allows_pivot(&observation.pivot)
        {
            landed_refs.push(reference);
        } else {
            candidate_refs.push(reference);
        }
    }
    landed_refs.sort();
    landed_refs.dedup();
    candidate_refs.sort();
    candidate_refs.dedup();
    let status = if collected.observations.is_empty() {
        IntelSemanticTerminalStatus::Empty
    } else {
        IntelSemanticTerminalStatus::Succeeded
    };
    let receipt = IntelPivotReceiptV1 {
        kind: INTEL_SEMANTIC_RECEIPT_KIND.to_string(),
        operation_id: request.fixture_context.operation_id,
        organization_id: request.fixture_context.organization_id,
        session_id: request.fixture_context.session_id,
        pivot: request.pivot.clone(),
        stable_query_key,
        provider_id: query.provider_id.clone(),
        query_type: query.query_type.clone(),
        adapter_version: query.adapter_version.clone(),
        status,
        artifact_ref: artifact.artifact_ref,
        artifact_sha256: artifact.sha256,
        evidence_ref,
        landed_refs,
        candidate_refs,
        capability: None,
        reason: None,
    };
    store
        .append_audit_receipt(&receipt)
        .await
        .map_err(|reason| persistence_error("audit_receipt", reason))?;

    Ok(summary_from_receipt(receipt, collected.observations, false))
}

pub async fn record_unsupported_semantic_query(
    request: &AssetIntelExecutionRequest,
    capability: &str,
    reason: &str,
    store: &dyn SemanticIntelReceiptStore,
) -> Result<PassiveIntelSemanticSummary, SemanticIntelPipelineError> {
    if !request.fixture_context.strict_passive || !request.fixture_context.fake_transport {
        return Err(SemanticIntelPipelineError::FixtureAuthorityMissing);
    }
    let query = PlannedNativeQuery {
        semantic_pivot: request.pivot.clone(),
        intent: request.intent,
        provider_id: "unsupported".to_string(),
        query_type: request.pivot.kind.as_str().to_string(),
        adapter_version: "unsupported.v1".to_string(),
        wire_query: String::new(),
        promotion_eligible: false,
    };
    let stable_query_key = stable_semantic_query_key(request, &query);
    if let Some(receipt) = store
        .load_terminal_receipt(&stable_query_key)
        .await
        .map_err(|reason| persistence_error("receipt_read", reason))?
    {
        return Ok(summary_from_receipt(receipt, Vec::new(), true));
    }
    let redacted_payload = serde_json::json!({
        "status": "unsupported",
        "capability": capability,
        "reason": reason,
    });
    let artifact_sha256 = sha256_hex(&serde_json::to_vec(&redacted_payload).unwrap_or_default());
    let artifact = RedactedIntelArtifact {
        artifact_ref: format!("intel-artifact:sha256:{artifact_sha256}"),
        sha256: artifact_sha256,
        redacted_payload,
    };
    store
        .save_redacted_artifact(&artifact)
        .await
        .map_err(|reason| persistence_error("artifact", reason))?;
    let evidence_ref = store
        .append_evidence(request, &artifact, &[])
        .await
        .map_err(|reason| persistence_error("evidence", reason))?;
    let receipt = IntelPivotReceiptV1 {
        kind: INTEL_SEMANTIC_RECEIPT_KIND.to_string(),
        operation_id: request.fixture_context.operation_id,
        organization_id: request.fixture_context.organization_id,
        session_id: request.fixture_context.session_id,
        pivot: request.pivot.clone(),
        stable_query_key,
        provider_id: query.provider_id,
        query_type: query.query_type,
        adapter_version: query.adapter_version,
        status: IntelSemanticTerminalStatus::Unsupported,
        artifact_ref: artifact.artifact_ref,
        artifact_sha256: artifact.sha256,
        evidence_ref,
        landed_refs: Vec::new(),
        candidate_refs: Vec::new(),
        capability: Some(capability.to_string()),
        reason: Some(reason.to_string()),
    };
    store
        .append_audit_receipt(&receipt)
        .await
        .map_err(|reason| persistence_error("audit_receipt", reason))?;
    Ok(summary_from_receipt(receipt, Vec::new(), false))
}

fn summary_from_receipt(
    receipt: IntelPivotReceiptV1,
    observations: Vec<CollectedIntelObservation>,
    duplicate_terminal: bool,
) -> PassiveIntelSemanticSummary {
    let mut discovered = observations
        .into_iter()
        .map(|observation| observation.pivot)
        .collect::<Vec<_>>();
    discovered.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.value.cmp(&right.value))
    });
    discovered.dedup();
    let complete_set_sha256 = sha256_hex(
        &serde_json::to_vec(&(&receipt.landed_refs, &receipt.candidate_refs, &discovered))
            .unwrap_or_default(),
    );
    PassiveIntelSemanticSummary {
        pivot_ref: pivot_ref(&receipt.pivot),
        status: receipt.status.clone(),
        query_receipts: vec![receipt.stable_query_key],
        artifact_refs: vec![receipt.artifact_ref],
        landed_count: receipt.landed_refs.len(),
        landed_refs: bounded(receipt.landed_refs),
        candidate_count: receipt.candidate_refs.len(),
        candidate_refs: bounded(receipt.candidate_refs),
        discovered_count: discovered.len(),
        discovered_pivots: discovered.into_iter().take(MODEL_REF_LIMIT).collect(),
        complete_set_sha256,
        duplicate_terminal,
        capability: receipt.capability,
        reason: receipt.reason,
    }
}

fn bounded(mut refs: Vec<String>) -> Vec<String> {
    refs.truncate(MODEL_REF_LIMIT);
    refs
}

fn pivot_ref(pivot: &AssetIntelPivot) -> String {
    format!("{}:{}", pivot.kind.as_str(), pivot.value)
}

fn persistence_error(stage: &'static str, reason: String) -> SemanticIntelPipelineError {
    SemanticIntelPipelineError::RetryablePersistence { stage, reason }
}

fn redact_provider_payload(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let lower = key.to_ascii_lowercase();
                    let sensitive = [
                        "api_key",
                        "apikey",
                        "authorization",
                        "cookie",
                        "password",
                        "secret",
                        "token",
                    ]
                    .iter()
                    .any(|needle| lower.contains(needle));
                    (
                        key.clone(),
                        if sensitive {
                            Value::String("[REDACTED]".to_string())
                        } else {
                            redact_provider_payload(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_provider_payload).collect()),
        other => other.clone(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical_domain(value: &str) -> Result<String, String> {
    AssetIntelPivot::parse(AssetIntelPivotKind::Domain, value)
        .map(|pivot| pivot.value)
        .map_err(|error| error.to_string())
}

fn cidr_contains(cidr: &str, candidate: IpAddr) -> bool {
    let Some((network, prefix)) = cidr.split_once('/') else {
        return false;
    };
    let (Ok(network), Ok(prefix)) = (network.parse::<IpAddr>(), prefix.parse::<u8>()) else {
        return false;
    };
    match (network, candidate) {
        (IpAddr::V4(network), IpAddr::V4(candidate)) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(network) & mask == u32::from(candidate) & mask
        }
        (IpAddr::V6(network), IpAddr::V6(candidate)) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(network) & mask == u128::from(candidate) & mask
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::asset_intel::{
        fixture_capability_matrix, AssetIntelFixtureContext, AssetIntelHydrateConfig,
        NativePivotPlanner,
    };
    use golish_pentest_domain::models::IntelSearchIntent;

    #[derive(Default)]
    struct FakeStore {
        events: Mutex<Vec<&'static str>>,
        receipts: Mutex<Vec<IntelPivotReceiptV1>>,
        fail_evidence: bool,
    }

    #[async_trait::async_trait]
    impl SemanticIntelReceiptStore for FakeStore {
        async fn load_terminal_receipt(
            &self,
            stable_query_key: &str,
        ) -> Result<Option<IntelPivotReceiptV1>, String> {
            self.events.lock().unwrap().push("receipt_read");
            Ok(self
                .receipts
                .lock()
                .unwrap()
                .iter()
                .find(|receipt| receipt.stable_query_key == stable_query_key)
                .cloned())
        }

        async fn save_redacted_artifact(
            &self,
            _artifact: &RedactedIntelArtifact,
        ) -> Result<(), String> {
            self.events.lock().unwrap().push("artifact");
            Ok(())
        }

        async fn append_evidence(
            &self,
            _request: &AssetIntelExecutionRequest,
            _artifact: &RedactedIntelArtifact,
            _observations: &[CollectedIntelObservation],
        ) -> Result<String, String> {
            self.events.lock().unwrap().push("evidence");
            if self.fail_evidence {
                Err("fixture evidence failure".to_string())
            } else {
                Ok("evidence:1".to_string())
            }
        }

        async fn append_audit_receipt(&self, receipt: &IntelPivotReceiptV1) -> Result<(), String> {
            self.events.lock().unwrap().push("receipt");
            self.receipts.lock().unwrap().push(receipt.clone());
            Ok(())
        }
    }

    fn request(authorization: ProjectionAuthorization) -> AssetIntelExecutionRequest {
        AssetIntelExecutionRequest {
            legacy_config: AssetIntelHydrateConfig::default(),
            pivot: AssetIntelPivot::parse(AssetIntelPivotKind::Domain, "example.com").unwrap(),
            intent: IntelSearchIntent::VerifyAttribution,
            projection_authorization: authorization,
            fixture_context: AssetIntelFixtureContext {
                operation_id: uuid::Uuid::new_v4(),
                organization_id: uuid::Uuid::new_v4(),
                session_id: uuid::Uuid::new_v4(),
                strict_passive: true,
                fake_transport: true,
            },
        }
    }

    fn domain_query(request: &AssetIntelExecutionRequest) -> PlannedNativeQuery {
        NativePivotPlanner::plan(&request.pivot, request.intent, &fixture_capability_matrix())
            .unwrap()
            .remove(0)
    }

    #[test]
    fn exact_domain_authorizes_only_itself_not_children() {
        let authorization = ProjectionAuthorization::from_fixture_scope(
            FixtureScopeSubject::ExactDomain("example.com".to_string()),
        )
        .unwrap();
        assert!(authorization.allows_domain("example.com"));
        assert!(!authorization.allows_domain("app.example.com"));
    }

    #[test]
    fn only_explicit_wildcard_and_cidr_authorize_descendants() {
        let wildcard = ProjectionAuthorization::from_fixture_scope(
            FixtureScopeSubject::WildcardDomain("*.example.com".to_string()),
        )
        .unwrap();
        assert!(wildcard.allows_domain("app.example.com"));
        assert!(!wildcard.allows_domain("example.com"));
        let cidr = ProjectionAuthorization::from_fixture_scope(FixtureScopeSubject::Cidr(
            "203.0.113.0/24".to_string(),
        ))
        .unwrap();
        assert!(cidr.allows_ip("203.0.113.8"));
        assert!(!cidr.allows_ip("203.0.114.8"));
    }

    #[tokio::test]
    async fn collection_cannot_write_profile_or_landing_before_projection_authorization() {
        let request = request(
            ProjectionAuthorization::from_fixture_scope(FixtureScopeSubject::ExactDomain(
                "example.com".to_string(),
            ))
            .unwrap(),
        );
        let collected = CollectedIntelBatch {
            raw_payload: serde_json::json!({"items": 2}),
            observations: vec![
                CollectedIntelObservation {
                    pivot: AssetIntelPivot::parse(AssetIntelPivotKind::Domain, "example.com")
                        .unwrap(),
                    provider_id: "fixture".to_string(),
                    confidence: 1.0,
                    source_ref: None,
                },
                CollectedIntelObservation {
                    pivot: AssetIntelPivot::parse(
                        AssetIntelPivotKind::Domain,
                        "shared.vendor.test",
                    )
                    .unwrap(),
                    provider_id: "fixture".to_string(),
                    confidence: 0.4,
                    source_ref: None,
                },
            ],
        };
        assert_eq!(collected.profile_writes(), 0);
        let store = FakeStore::default();
        let summary =
            run_fixture_semantic_query(&request, &domain_query(&request), collected, &store)
                .await
                .unwrap();
        assert_eq!(summary.landed_count, 1);
        assert_eq!(summary.candidate_count, 1);
        assert_eq!(
            *store.events.lock().unwrap(),
            ["receipt_read", "artifact", "evidence", "receipt"]
        );
    }

    #[tokio::test]
    async fn native_raw_payload_is_redacted_artifact_before_model_visibility() {
        let request = request(ProjectionAuthorization::default());
        let store = FakeStore::default();
        run_fixture_semantic_query(
            &request,
            &domain_query(&request),
            CollectedIntelBatch {
                raw_payload: serde_json::json!({"api_key": "secret", "value": "visible"}),
                observations: Vec::new(),
            },
            &store,
        )
        .await
        .unwrap();
        let receipt = store.receipts.lock().unwrap()[0].clone();
        assert!(!receipt.artifact_sha256.is_empty());
        assert!(!serde_json::to_string(&receipt).unwrap().contains("secret"));
    }

    #[tokio::test]
    async fn unsupported_is_terminal_but_never_reported_as_empty() {
        let request = request(ProjectionAuthorization::default());
        let store = FakeStore::default();
        let summary = record_unsupported_semantic_query(
            &request,
            "public_web_readonly",
            "disabled in Plan A",
            &store,
        )
        .await
        .unwrap();
        assert_eq!(summary.status, IntelSemanticTerminalStatus::Unsupported);
        assert_eq!(summary.capability.as_deref(), Some("public_web_readonly"));
    }

    #[tokio::test]
    async fn receipt_persistence_failure_remains_retryable_and_hides_result() {
        let request = request(ProjectionAuthorization::default());
        let store = FakeStore {
            fail_evidence: true,
            ..FakeStore::default()
        };
        let error = run_fixture_semantic_query(
            &request,
            &domain_query(&request),
            CollectedIntelBatch {
                raw_payload: serde_json::json!({"private_body": "must-not-return"}),
                observations: Vec::new(),
            },
            &store,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            SemanticIntelPipelineError::RetryablePersistence {
                stage: "evidence",
                ..
            }
        ));
        assert!(store.receipts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn audit_receipt_not_expansion_queue_is_frontier_source_of_truth() {
        let request = request(ProjectionAuthorization::default());
        let store = FakeStore::default();
        let query = domain_query(&request);
        let first = run_fixture_semantic_query(
            &request,
            &query,
            CollectedIntelBatch {
                raw_payload: serde_json::json!({"result": "first"}),
                observations: Vec::new(),
            },
            &store,
        )
        .await
        .unwrap();
        assert!(!first.duplicate_terminal);
        let duplicate = run_fixture_semantic_query(
            &request,
            &query,
            CollectedIntelBatch {
                raw_payload: serde_json::json!({"result": "must-not-be-read"}),
                observations: Vec::new(),
            },
            &store,
        )
        .await
        .unwrap();
        assert!(duplicate.duplicate_terminal);
        assert_eq!(store.receipts.lock().unwrap().len(), 1);
    }
}
