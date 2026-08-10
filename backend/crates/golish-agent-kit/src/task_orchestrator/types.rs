//! Public types and the [`AgentExecutor`] trait used by [`TaskOrchestrator`].
//!
//! Includes the planning DTO (`PlannedSubtask`), per-call token usage
//! (`AgentTokenUsage`), execution context types (`ExecutionContext`,
//! `SubtaskResult`, `AgentResult`), and the [`AgentExecutor`] callback trait
//! that decouples the orchestrator from `AgentBridge`.

use serde::{Deserialize, Serialize};

use anyhow::Result;
use golish_core::WorkerLeaseContext;

const AU_MAX_ROUTES: usize = 256;
const AU_MAX_SERVICES: usize = 128;
const AU_MAX_FINGERPRINTS: usize = 128;
const AU_MAX_SUBJECTS: usize = 128;
const AU_MAX_PARAMETERS_PER_ROUTE: usize = 128;
const AU_MAX_MANIFEST_INPUTS: usize = 128;
const AU_MAX_PARTIAL_ITEMS: usize = 256;
const AU_MAX_UNKNOWNS: usize = 128;

/// Exact server-owned request for the closed-agent Application Understanding
/// stage. The runtime derives organization scope and predecessor inputs from
/// the frozen operation; neither the model nor an adapter may supply them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationUnderstandingStageRequest {
    pub operation_id: uuid::Uuid,
    pub stage_execution_id: uuid::Uuid,
    pub session_id: uuid::Uuid,
    /// Stable request identity of the single Primary `stage_run` envelope.
    /// Per-company worker/lead request ids are derived from this value by the
    /// DB-backed runtime; neither the model nor a child agent may choose it.
    pub stage_run_parent_request_id: String,
}

/// Aggregate result returned only after every frozen organization has reached
/// the direct Application Model Gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationUnderstandingStageOutcome {
    Passed {
        completed_units: usize,
        total_units: usize,
    },
    Blocked {
        code: String,
        refs: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelProducerInputContract {
    pub manifest_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub inputs: Vec<ApplicationModelProducerSourceContract>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelProducerSourceContract {
    pub input_key: String,
    pub input_kind: String,
    pub source_kind: String,
    pub source_id: String,
    pub source_version: i64,
    pub source_payload: serde_json::Value,
    pub evidence_ids: Vec<i64>,
}

/// Static kind for one server-seeded Application Understanding shard.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelWorkItemKindContract {
    WebOrigin,
    ServiceHost,
    UnknownAsset,
}

/// Manifest identity exposed to a shard or company synthesizer. It is an
/// immutable envelope only: predecessor payloads are deliberately absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelManifestInputRefContract {
    pub input_key: String,
    pub input_kind: String,
    pub source_id: String,
    pub source_version: i64,
    pub content_hash: String,
    pub evidence_ids: Vec<i64>,
}

/// Sanitized subject identity for application/service clustering. Query,
/// fragment, userinfo and other capture material have no representable field.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelSafeSubjectKindContract {
    Host,
    Ip,
    Cidr,
    WebOrigin,
    UnknownAsset,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelSafeSubjectContract {
    pub kind: ApplicationModelSafeSubjectKindContract,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelSafeParameterLocationContract {
    Path,
    Query,
    Header,
    Body,
    Cookie,
}

/// Parameter metadata only. Values, examples and raw request fragments are
/// intentionally unrepresentable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelSafeParameterContract {
    pub name: String,
    pub location: ApplicationModelSafeParameterLocationContract,
    pub value_type: String,
    pub required: bool,
}

/// Safe route projection. Header values, query values, raw bodies, capture
/// paths and redirect chains have no representable field in this contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelSafeRouteContract {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub method: String,
    pub route_shape: String,
    pub status_code: Option<u16>,
    pub content_type: Option<String>,
    pub parameters: Vec<ApplicationModelSafeParameterContract>,
}

/// Safe network-service projection. In particular this carries no banner or
/// raw probe output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelSafeServiceContract {
    pub host: String,
    pub port: u16,
    pub transport: String,
    pub service_name: Option<String>,
    pub product: Option<String>,
    pub version: Option<String>,
    pub tls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelSafeFingerprintContract {
    pub name: String,
    pub category: String,
    pub version: Option<String>,
    pub confidence: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelWorkItemProjectionContract {
    pub subjects: Vec<ApplicationModelSafeSubjectContract>,
    pub routes: Vec<ApplicationModelSafeRouteContract>,
    pub services: Vec<ApplicationModelSafeServiceContract>,
    pub fingerprints: Vec<ApplicationModelSafeFingerprintContract>,
    pub manifest_inputs: Vec<ApplicationModelManifestInputRefContract>,
    pub projection_incomplete: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplicationModelWorkItemInputContract {
    pub operation_id: uuid::Uuid,
    pub manifest_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub stage_run_unit_id: uuid::Uuid,
    pub work_item_id: uuid::Uuid,
    pub work_item_key: String,
    pub work_item_kind: ApplicationModelWorkItemKindContract,
    pub projection_hash: String,
    pub projection: ApplicationModelWorkItemProjectionContract,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedApplicationModelWorkItemInputContract {
    operation_id: uuid::Uuid,
    manifest_id: uuid::Uuid,
    organization_id: uuid::Uuid,
    stage_run_unit_id: uuid::Uuid,
    work_item_id: uuid::Uuid,
    work_item_key: String,
    work_item_kind: ApplicationModelWorkItemKindContract,
    projection_hash: String,
    projection: ApplicationModelWorkItemProjectionContract,
}

impl<'de> Deserialize<'de> for ApplicationModelWorkItemInputContract {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let unchecked = UncheckedApplicationModelWorkItemInputContract::deserialize(deserializer)?;
        Self::try_from(unchecked).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<UncheckedApplicationModelWorkItemInputContract>
    for ApplicationModelWorkItemInputContract
{
    type Error = ApplicationModelContractViolation;

    fn try_from(
        value: UncheckedApplicationModelWorkItemInputContract,
    ) -> std::result::Result<Self, Self::Error> {
        if [
            value.operation_id,
            value.manifest_id,
            value.organization_id,
            value.stage_run_unit_id,
            value.work_item_id,
        ]
        .contains(&uuid::Uuid::nil())
            || !bounded_text(&value.work_item_key, 256)
            || !projection_sha256(&value.projection_hash)
            || !valid_projection(&value.projection)
        {
            return Err(ApplicationModelContractViolation::NonContract);
        }
        Ok(Self {
            operation_id: value.operation_id,
            manifest_id: value.manifest_id,
            organization_id: value.organization_id,
            stage_run_unit_id: value.stage_run_unit_id,
            work_item_id: value.work_item_id,
            work_item_key: value.work_item_key,
            work_item_kind: value.work_item_kind,
            projection_hash: value.projection_hash,
            projection: value.projection,
        })
    }
}

/// Exact JSON Schema for one frozen Application Understanding shard result.
///
/// The same value is used by the provider-facing response contract and the
/// SubAgent terminal barrier. Identity constants are host-owned and must not
/// be reconstructed from model prose.
pub fn application_model_work_item_output_json_schema(
    input: &ApplicationModelWorkItemInputContract,
) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "organization_id", "work_item_id", "work_item_key", "projection_hash",
            "summary", "items", "unknowns"
        ],
        "properties": {
            "organization_id": {
                "type": "string", "format": "uuid", "const": input.organization_id.to_string()
            },
            "work_item_id": {
                "type": "string", "format": "uuid", "const": input.work_item_id.to_string()
            },
            "work_item_key": {"type": "string", "const": input.work_item_key},
            "projection_hash": {"type": "string", "const": input.projection_hash},
            "summary": {"type": "string", "minLength": 1, "maxLength": 4096},
            "items": {
                "type": "array",
                "maxItems": AU_MAX_PARTIAL_ITEMS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": [
                        "item_key", "item_kind", "truth_state", "summary",
                        "source_input_keys", "evidence"
                    ],
                    "properties": {
                        "item_key": {"type": "string", "minLength": 1, "maxLength": 256},
                        "item_kind": {"enum": [
                            "technology", "route_or_page", "api_surface", "role_or_identity",
                            "business_entity", "workflow", "state_transition", "ownership_rule",
                            "sensitive_operation", "trust_boundary", "unknown"
                        ]},
                        "truth_state": {
                            "enum": ["observed", "inferred", "unknown"],
                            "description": "observed requires at least one evidence entry with role observation; inferred and unknown accept support evidence only."
                        },
                        "summary": {"type": "string", "minLength": 1, "maxLength": 1024},
                        "source_input_keys": {
                            "type": "array", "maxItems": AU_MAX_MANIFEST_INPUTS,
                            "uniqueItems": true,
                            "items": {"type": "string", "minLength": 1, "maxLength": 256}
                        },
                        "evidence": {
                            "type": "array", "maxItems": AU_MAX_PARTIAL_ITEMS,
                            "description": "For observed items include at least one observation role. For inferred or unknown items every evidence role must be support.",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["evidence_id", "role"],
                                "properties": {
                                    "evidence_id": {"type": "integer", "minimum": 1},
                                    "role": {"enum": ["observation", "support"]}
                                }
                            }
                        }
                    }
                }
            },
            "unknowns": {
                "type": "array", "maxItems": AU_MAX_UNKNOWNS, "uniqueItems": true,
                "items": {"type": "string", "minLength": 1, "maxLength": 512}
            }
        }
    })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelPartialItemKindContract {
    Technology,
    RouteOrPage,
    ApiSurface,
    RoleOrIdentity,
    BusinessEntity,
    Workflow,
    StateTransition,
    OwnershipRule,
    SensitiveOperation,
    TrustBoundary,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelWorkItemPartialContract {
    pub item_key: String,
    pub item_kind: ApplicationModelPartialItemKindContract,
    pub truth_state: ApplicationModelTruthStateContract,
    pub summary: String,
    pub source_input_keys: Vec<String>,
    pub evidence: Vec<ApplicationModelEvidenceContract>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplicationModelWorkItemOutputContract {
    pub organization_id: uuid::Uuid,
    pub work_item_id: uuid::Uuid,
    pub work_item_key: String,
    pub projection_hash: String,
    pub summary: String,
    pub items: Vec<ApplicationModelWorkItemPartialContract>,
    pub unknowns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedApplicationModelWorkItemOutputContract {
    organization_id: uuid::Uuid,
    work_item_id: uuid::Uuid,
    work_item_key: String,
    projection_hash: String,
    summary: String,
    items: Vec<ApplicationModelWorkItemPartialContract>,
    unknowns: Vec<String>,
}

impl<'de> Deserialize<'de> for ApplicationModelWorkItemOutputContract {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let unchecked = UncheckedApplicationModelWorkItemOutputContract::deserialize(deserializer)?;
        Self::try_from(unchecked).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<UncheckedApplicationModelWorkItemOutputContract>
    for ApplicationModelWorkItemOutputContract
{
    type Error = ApplicationModelContractViolation;

    fn try_from(
        value: UncheckedApplicationModelWorkItemOutputContract,
    ) -> std::result::Result<Self, Self::Error> {
        if value.organization_id.is_nil()
            || value.work_item_id.is_nil()
            || !bounded_text(&value.work_item_key, 256)
            || !projection_sha256(&value.projection_hash)
            || !bounded_text(&value.summary, 4096)
            || value.items.len() > AU_MAX_PARTIAL_ITEMS
            || value.unknowns.len() > AU_MAX_UNKNOWNS
            || !unique_by(value.items.iter().map(|item| item.item_key.as_str()))
            || !unique_by(value.unknowns.iter().map(String::as_str))
            || !value
                .unknowns
                .iter()
                .all(|unknown| bounded_text(unknown, 512))
            || !value.items.iter().all(valid_partial_item)
        {
            return Err(ApplicationModelContractViolation::NonContract);
        }
        Ok(Self {
            organization_id: value.organization_id,
            work_item_id: value.work_item_id,
            work_item_key: value.work_item_key,
            projection_hash: value.projection_hash,
            summary: value.summary,
            items: value.items,
            unknowns: value.unknowns,
        })
    }
}

impl ApplicationModelWorkItemOutputContract {
    pub fn validate_against(
        &self,
        input: &ApplicationModelWorkItemInputContract,
    ) -> std::result::Result<(), ApplicationModelContractViolation> {
        if self.organization_id != input.organization_id
            || self.work_item_id != input.work_item_id
            || self.work_item_key != input.work_item_key
            || self.projection_hash != input.projection_hash
        {
            return Err(ApplicationModelContractViolation::IdentityMismatch);
        }
        let input_keys = input
            .projection
            .manifest_inputs
            .iter()
            .map(|reference| reference.input_key.as_str())
            .collect::<std::collections::HashSet<_>>();
        let evidence_ids = input
            .projection
            .manifest_inputs
            .iter()
            .flat_map(|reference| reference.evidence_ids.iter().copied())
            .collect::<std::collections::HashSet<_>>();
        for item in &self.items {
            if !item
                .source_input_keys
                .iter()
                .all(|key| input_keys.contains(key.as_str()))
            {
                return Err(ApplicationModelContractViolation::UnauthorizedInputReference);
            }
            if !item
                .evidence
                .iter()
                .all(|evidence| evidence_ids.contains(&evidence.evidence_id))
            {
                return Err(ApplicationModelContractViolation::UnauthorizedEvidenceReference);
            }
        }
        Ok(())
    }
}

/// Parse and validate a terminal shard result against the exact frozen input.
///
/// This is the single host-side semantic boundary used both after a completed
/// provider response and by the bound `submit_result` lifecycle.
pub fn parse_and_validate_application_model_work_item_output(
    value: serde_json::Value,
    input: &ApplicationModelWorkItemInputContract,
) -> std::result::Result<ApplicationModelWorkItemOutputContract, ApplicationModelContractViolation>
{
    let output = serde_json::from_value::<ApplicationModelWorkItemOutputContract>(value)
        .map_err(|_| ApplicationModelContractViolation::NonContract)?;
    output.validate_against(input)?;
    Ok(output)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelExpectedWorkItemContract {
    pub work_item_id: uuid::Uuid,
    pub work_item_key: String,
    pub projection_hash: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplicationModelSynthesisInputContract {
    pub operation_id: uuid::Uuid,
    pub manifest_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub stage_run_unit_id: uuid::Uuid,
    pub manifest_inputs: Vec<ApplicationModelManifestInputRefContract>,
    pub expected_work_items: Vec<ApplicationModelExpectedWorkItemContract>,
    pub partial_outputs: Vec<ApplicationModelWorkItemOutputContract>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct UncheckedApplicationModelSynthesisInputContract {
    operation_id: uuid::Uuid,
    manifest_id: uuid::Uuid,
    organization_id: uuid::Uuid,
    stage_run_unit_id: uuid::Uuid,
    manifest_inputs: Vec<ApplicationModelManifestInputRefContract>,
    expected_work_items: Vec<ApplicationModelExpectedWorkItemContract>,
    partial_outputs: Vec<ApplicationModelWorkItemOutputContract>,
}

impl<'de> Deserialize<'de> for ApplicationModelSynthesisInputContract {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let unchecked = UncheckedApplicationModelSynthesisInputContract::deserialize(deserializer)?;
        Self::try_from(unchecked).map_err(serde::de::Error::custom)
    }
}

impl TryFrom<UncheckedApplicationModelSynthesisInputContract>
    for ApplicationModelSynthesisInputContract
{
    type Error = ApplicationModelContractViolation;

    fn try_from(
        value: UncheckedApplicationModelSynthesisInputContract,
    ) -> std::result::Result<Self, Self::Error> {
        if [
            value.operation_id,
            value.manifest_id,
            value.organization_id,
            value.stage_run_unit_id,
        ]
        .contains(&uuid::Uuid::nil())
            || !valid_manifest_inputs(&value.manifest_inputs)
            || value.expected_work_items.is_empty()
            || value.expected_work_items.len() > AU_MAX_PARTIAL_ITEMS
            || value.expected_work_items.len() != value.partial_outputs.len()
            || !unique_by(
                value
                    .expected_work_items
                    .iter()
                    .map(|expected| expected.work_item_key.as_str()),
            )
            || !unique_by(
                value
                    .expected_work_items
                    .iter()
                    .map(|expected| expected.work_item_id),
            )
            || !value.expected_work_items.iter().all(|expected| {
                !expected.work_item_id.is_nil()
                    && bounded_text(&expected.work_item_key, 256)
                    && projection_sha256(&expected.projection_hash)
            })
        {
            return Err(ApplicationModelContractViolation::InexactShardSet);
        }

        let expected = value
            .expected_work_items
            .iter()
            .map(|item| {
                (
                    item.work_item_id,
                    item.work_item_key.as_str(),
                    item.projection_hash.as_str(),
                )
            })
            .collect::<std::collections::HashSet<_>>();
        let actual = value
            .partial_outputs
            .iter()
            .map(|output| {
                (
                    output.work_item_id,
                    output.work_item_key.as_str(),
                    output.projection_hash.as_str(),
                )
            })
            .collect::<std::collections::HashSet<_>>();
        if expected != actual
            || !value
                .partial_outputs
                .iter()
                .all(|output| output.organization_id == value.organization_id)
            || !partials_reference_manifest(&value.partial_outputs, &value.manifest_inputs)
        {
            return Err(ApplicationModelContractViolation::InexactShardSet);
        }

        Ok(Self {
            operation_id: value.operation_id,
            manifest_id: value.manifest_id,
            organization_id: value.organization_id,
            stage_run_unit_id: value.stage_run_unit_id,
            manifest_inputs: value.manifest_inputs,
            expected_work_items: value.expected_work_items,
            partial_outputs: value.partial_outputs,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationModelContractViolation {
    NonContract,
    IdentityMismatch,
    UnauthorizedInputReference,
    UnauthorizedEvidenceReference,
    InexactShardSet,
}

impl ApplicationModelContractViolation {
    pub const fn code(self) -> &'static str {
        match self {
            Self::NonContract => "application_model_contract_non_contract",
            Self::IdentityMismatch => "application_model_work_item_identity_mismatch",
            Self::UnauthorizedInputReference => {
                "application_model_work_item_input_reference_unauthorized"
            }
            Self::UnauthorizedEvidenceReference => {
                "application_model_work_item_evidence_reference_unauthorized"
            }
            Self::InexactShardSet => "application_model_synthesis_shard_set_inexact",
        }
    }
}

impl std::fmt::Display for ApplicationModelContractViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ApplicationModelContractViolation {}

fn bounded_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}

fn bare_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn projection_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(bare_sha256_hex)
}

fn unique_by<T: Eq + std::hash::Hash>(values: impl Iterator<Item = T>) -> bool {
    let mut seen = std::collections::HashSet::new();
    values.into_iter().all(|value| seen.insert(value))
}

fn valid_manifest_inputs(inputs: &[ApplicationModelManifestInputRefContract]) -> bool {
    inputs.len() <= AU_MAX_MANIFEST_INPUTS
        && unique_by(inputs.iter().map(|input| input.input_key.as_str()))
        && inputs.iter().all(|input| {
            bounded_text(&input.input_key, 256)
                && bounded_text(&input.input_kind, 128)
                && bounded_text(&input.source_id, 256)
                && input.source_version > 0
                && bare_sha256_hex(&input.content_hash)
                && input.evidence_ids.iter().all(|id| *id > 0)
                && unique_by(input.evidence_ids.iter().copied())
        })
}

fn valid_projection(projection: &ApplicationModelWorkItemProjectionContract) -> bool {
    !projection.subjects.is_empty()
        && projection.subjects.len() <= AU_MAX_SUBJECTS
        && unique_by(
            projection
                .subjects
                .iter()
                .map(|subject| (subject.kind, subject.value.as_str())),
        )
        && projection.subjects.iter().all(valid_safe_subject)
        && projection.routes.len() <= AU_MAX_ROUTES
        && projection.services.len() <= AU_MAX_SERVICES
        && projection.fingerprints.len() <= AU_MAX_FINGERPRINTS
        && valid_manifest_inputs(&projection.manifest_inputs)
        && projection.routes.iter().all(|route| {
            matches!(route.scheme.as_str(), "http" | "https")
                && bounded_text(&route.host, 253)
                && route.port > 0
                && bounded_text(&route.method, 16)
                && route
                    .method
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'-')
                && bounded_text(&route.route_shape, 512)
                && route
                    .status_code
                    .is_none_or(|status| (100..=599).contains(&status))
                && route
                    .content_type
                    .as_deref()
                    .is_none_or(|value| bounded_text(value, 128))
                && route.parameters.len() <= AU_MAX_PARAMETERS_PER_ROUTE
                && unique_by(
                    route
                        .parameters
                        .iter()
                        .map(|parameter| (parameter.location, parameter.name.as_str())),
                )
                && route.parameters.iter().all(|parameter| {
                    bounded_safe_token(&parameter.name, 128)
                        && bounded_safe_token(&parameter.value_type, 64)
                })
        })
        && projection.services.iter().all(|service| {
            bounded_text(&service.host, 253)
                && service.port > 0
                && matches!(service.transport.as_str(), "tcp" | "udp")
                && service
                    .service_name
                    .as_deref()
                    .is_none_or(|value| bounded_text(value, 128))
                && service
                    .product
                    .as_deref()
                    .is_none_or(|value| bounded_text(value, 256))
                && service
                    .version
                    .as_deref()
                    .is_none_or(|value| bounded_text(value, 128))
        })
        && projection.fingerprints.iter().all(|fingerprint| {
            bounded_text(&fingerprint.name, 128)
                && bounded_text(&fingerprint.category, 128)
                && fingerprint
                    .version
                    .as_deref()
                    .is_none_or(|value| bounded_text(value, 128))
                && fingerprint.confidence <= 100
        })
}

fn bounded_safe_token(value: &str, max: usize) -> bool {
    bounded_text(value, max)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'[' | b']')
        })
}

fn valid_safe_subject(subject: &ApplicationModelSafeSubjectContract) -> bool {
    if !bounded_text(&subject.value, 512)
        || subject
            .value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return false;
    }
    match subject.kind {
        ApplicationModelSafeSubjectKindContract::Host => {
            subject.value.len() <= 253
                && subject
                    .value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        }
        ApplicationModelSafeSubjectKindContract::Ip => {
            subject.value.parse::<std::net::IpAddr>().is_ok()
        }
        ApplicationModelSafeSubjectKindContract::Cidr => valid_cidr(&subject.value),
        ApplicationModelSafeSubjectKindContract::WebOrigin => url::Url::parse(&subject.value)
            .is_ok_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && matches!(url.path(), "" | "/")
            }),
        ApplicationModelSafeSubjectKindContract::UnknownAsset => {
            subject.value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'.' | b'-' | b'_' | b':' | b'/' | b'*')
            })
        }
    }
}

fn valid_cidr(value: &str) -> bool {
    let Some((address, prefix)) = value.split_once('/') else {
        return false;
    };
    let Ok(address) = address.parse::<std::net::IpAddr>() else {
        return false;
    };
    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    match address {
        std::net::IpAddr::V4(_) => prefix <= 32,
        std::net::IpAddr::V6(_) => prefix <= 128,
    }
}

fn valid_partial_item(item: &ApplicationModelWorkItemPartialContract) -> bool {
    bounded_text(&item.item_key, 256)
        && bounded_text(&item.summary, 1024)
        && item.source_input_keys.len() <= AU_MAX_MANIFEST_INPUTS
        && unique_by(item.source_input_keys.iter().map(String::as_str))
        && item
            .source_input_keys
            .iter()
            .all(|key| bounded_text(key, 256))
        && item.evidence.len() <= 256
        && item
            .evidence
            .iter()
            .all(|evidence| evidence.evidence_id > 0)
        && unique_by(item.evidence.iter().map(|evidence| evidence.evidence_id))
        && match item.truth_state {
            ApplicationModelTruthStateContract::Observed => item
                .evidence
                .iter()
                .any(|evidence| evidence.role == ApplicationModelEvidenceRoleContract::Observation),
            ApplicationModelTruthStateContract::Inferred
            | ApplicationModelTruthStateContract::Unknown => item
                .evidence
                .iter()
                .all(|evidence| evidence.role == ApplicationModelEvidenceRoleContract::Support),
        }
}

fn partials_reference_manifest(
    outputs: &[ApplicationModelWorkItemOutputContract],
    inputs: &[ApplicationModelManifestInputRefContract],
) -> bool {
    let keys = inputs
        .iter()
        .map(|input| input.input_key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let evidence = inputs
        .iter()
        .flat_map(|input| input.evidence_ids.iter().copied())
        .collect::<std::collections::HashSet<_>>();
    outputs.iter().flat_map(|output| &output.items).all(|item| {
        item.source_input_keys
            .iter()
            .all(|key| keys.contains(key.as_str()))
            && item
                .evidence
                .iter()
                .all(|reference| evidence.contains(&reference.evidence_id))
    })
}

/// Closed, version-1 business/application model body. The schema version is
/// carried by the immutable revision envelope (`application_model.v1`), not by
/// this body. Every collection contains stable keys into the proposal's
/// normalized `items` rows; richer item data remains in those rows so this
/// index can evolve by introducing a new revision schema instead of accepting
/// arbitrary JSON fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelV1Contract {
    pub organization_id: uuid::Uuid,
    pub summary: String,
    pub technologies: Vec<String>,
    pub routes_and_pages: Vec<String>,
    pub api_surfaces: Vec<String>,
    pub roles_and_identities: Vec<String>,
    pub business_entities: Vec<String>,
    pub workflows: Vec<String>,
    pub state_transitions: Vec<String>,
    pub ownership_rules: Vec<String>,
    pub sensitive_operations: Vec<String>,
    pub trust_boundaries: Vec<String>,
    pub unknowns: Vec<String>,
}

impl ApplicationModelV1Contract {
    fn has_bounded_content(&self) -> bool {
        if self.summary.trim().is_empty() || self.summary.len() > 4096 {
            return false;
        }
        let collections = [
            &self.technologies,
            &self.routes_and_pages,
            &self.api_surfaces,
            &self.roles_and_identities,
            &self.business_entities,
            &self.workflows,
            &self.state_transitions,
            &self.ownership_rules,
            &self.sensitive_operations,
            &self.trust_boundaries,
            &self.unknowns,
        ];
        let total = collections.iter().map(|values| values.len()).sum::<usize>();
        total <= 50_000
            && collections.iter().all(|values| {
                values
                    .iter()
                    .all(|value| !value.trim().is_empty() && value.len() <= 256)
            })
            && {
                let mut keys = collections
                    .into_iter()
                    .flat_map(|values| values.iter())
                    .collect::<Vec<_>>();
                keys.sort_unstable();
                keys.windows(2).all(|pair| pair[0] != pair[1])
            }
    }
}

fn deserialize_application_model_v1_value<'de, D>(
    deserializer: D,
) -> Result<serde_json::Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let model = ApplicationModelV1Contract::deserialize(deserializer)?;
    if !model.has_bounded_content() {
        return Err(serde::de::Error::custom(
            "application_model.v1 content is empty, duplicated, or out of bounds",
        ));
    }
    serde_json::to_value(model).map_err(serde::de::Error::custom)
}

/// JSON Schema used by the tool-free bridge. It is deliberately hand-shaped
/// from the same closed DTO because provider schemas need `additionalProperties`
/// explicitly set to false.
pub fn application_model_v1_json_schema() -> serde_json::Value {
    let key_array = || {
        serde_json::json!({
            "type": "array",
            "maxItems": 50000,
            "uniqueItems": true,
            "items": {"type": "string", "minLength": 1, "maxLength": 256}
        })
    };
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "organization_id", "summary", "technologies", "routes_and_pages",
            "api_surfaces", "roles_and_identities", "business_entities", "workflows",
            "state_transitions", "ownership_rules", "sensitive_operations",
            "trust_boundaries", "unknowns"
        ],
        "properties": {
            "organization_id": {"type": "string", "format": "uuid"},
            "summary": {"type": "string", "minLength": 1, "maxLength": 4096},
            "technologies": key_array(),
            "routes_and_pages": key_array(),
            "api_surfaces": key_array(),
            "roles_and_identities": key_array(),
            "business_entities": key_array(),
            "workflows": key_array(),
            "state_transitions": key_array(),
            "ownership_rules": key_array(),
            "sensitive_operations": key_array(),
            "trust_boundaries": key_array(),
            "unknowns": key_array()
        }
    })
}

/// Exact JSON Schema for the unique per-company Application Model proposal.
/// The organization identity is a server-owned constant shared by both the
/// provider response contract and the SubAgent terminal barrier.
pub fn application_model_proposal_json_schema(organization_id: uuid::Uuid) -> serde_json::Value {
    let mut structured_model_schema = application_model_v1_json_schema();
    structured_model_schema["properties"]["organization_id"] = serde_json::json!({
        "type": "string",
        "format": "uuid",
        "const": organization_id.to_string()
    });
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["structured_model", "decisions", "items"],
        "properties": {
            "structured_model": structured_model_schema,
            "decisions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": [
                        "input_key", "disposition", "item_keys", "duplicate_input_key",
                        "reason_code"
                    ],
                    "properties": {
                        "input_key": {"type": "string"},
                        "disposition": {
                            "enum": ["incorporated", "duplicate", "not_relevant", "unknown"]
                        },
                        "item_keys": {"type": "array", "items": {"type": "string"}},
                        "duplicate_input_key": {"type": ["string", "null"]},
                        "reason_code": {"type": ["string", "null"]}
                    }
                }
            },
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": [
                        "item_key", "item_kind", "truth_state", "source_input_keys",
                        "referenced_item_keys", "payload", "evidence"
                    ],
                    "properties": {
                        "item_key": {"type": "string"},
                        "item_kind": {"type": "string"},
                        "truth_state": {"enum": ["observed", "inferred", "unknown"]},
                        "source_input_keys": {
                            "type": "array", "items": {"type": "string"}
                        },
                        "referenced_item_keys": {
                            "type": "array", "items": {"type": "string"}
                        },
                        "payload": {"type": "object"},
                        "evidence": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["evidence_id", "role"],
                                "properties": {
                                    "evidence_id": {"type": "integer", "minimum": 1},
                                    "role": {"enum": ["observation", "support"]}
                                }
                            }
                        }
                    }
                }
            }
        }
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelProposalContract {
    #[serde(deserialize_with = "deserialize_application_model_v1_value")]
    pub structured_model: serde_json::Value,
    pub decisions: Vec<ApplicationModelDecisionContract>,
    pub items: Vec<ApplicationModelItemContract>,
}

/// Parse the closed proposal and enforce its server-owned organization.
pub fn parse_and_validate_application_model_proposal(
    value: serde_json::Value,
    organization_id: uuid::Uuid,
) -> std::result::Result<ApplicationModelProposalContract, ApplicationModelContractViolation> {
    let proposal = serde_json::from_value::<ApplicationModelProposalContract>(value)
        .map_err(|_| ApplicationModelContractViolation::NonContract)?;
    let model =
        serde_json::from_value::<ApplicationModelV1Contract>(proposal.structured_model.clone())
            .map_err(|_| ApplicationModelContractViolation::NonContract)?;
    if model.organization_id != organization_id {
        return Err(ApplicationModelContractViolation::IdentityMismatch);
    }
    Ok(proposal)
}

/// Validate a company proposal against the exact frozen synthesis denominator.
///
/// This mirrors the pure Application Model Gate before the terminal tool is
/// allowed to land, so a correctable semantic response does not consume a new
/// durable WorkItem attempt.
pub fn parse_and_validate_application_model_proposal_against_synthesis(
    value: serde_json::Value,
    input: &ApplicationModelSynthesisInputContract,
) -> std::result::Result<ApplicationModelProposalContract, ApplicationModelContractViolation> {
    use crate::harness::{
        validate_application_model_gate_truth, ApplicationModelAuthorityKind,
        ApplicationModelGateSnapshot, ApplicationModelInputDecisionTruth,
        ApplicationModelInputDisposition, ApplicationModelItemTruth, ApplicationModelTruthState,
    };

    let proposal = parse_and_validate_application_model_proposal(value, input.organization_id)?;
    let model =
        serde_json::from_value::<ApplicationModelV1Contract>(proposal.structured_model.clone())
            .map_err(|_| ApplicationModelContractViolation::NonContract)?;
    let item_keys = proposal
        .items
        .iter()
        .map(|item| item.item_key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let model_keys = [
        &model.technologies,
        &model.routes_and_pages,
        &model.api_surfaces,
        &model.roles_and_identities,
        &model.business_entities,
        &model.workflows,
        &model.state_transitions,
        &model.ownership_rules,
        &model.sensitive_operations,
        &model.trust_boundaries,
        &model.unknowns,
    ];
    if model_keys
        .into_iter()
        .flatten()
        .any(|key| !item_keys.contains(key.as_str()))
        || proposal.items.iter().any(|item| {
            item.item_kind.trim().is_empty()
                || item.item_kind.len() > 64
                || !item.payload.is_object()
        })
    {
        return Err(ApplicationModelContractViolation::NonContract);
    }

    let decisions = proposal
        .decisions
        .iter()
        .map(|decision| ApplicationModelInputDecisionTruth {
            input_key: decision.input_key.clone(),
            disposition: match decision.disposition {
                ApplicationModelInputDispositionContract::Incorporated => {
                    ApplicationModelInputDisposition::Incorporated
                }
                ApplicationModelInputDispositionContract::Duplicate => {
                    ApplicationModelInputDisposition::Duplicate
                }
                ApplicationModelInputDispositionContract::NotRelevant => {
                    ApplicationModelInputDisposition::NotRelevant
                }
                ApplicationModelInputDispositionContract::Unknown => {
                    ApplicationModelInputDisposition::Unknown
                }
            },
            item_keys: decision.item_keys.clone(),
            duplicate_input_key: decision.duplicate_input_key.clone(),
            reason_code: decision.reason_code.clone(),
        })
        .collect();
    let items = proposal
        .items
        .iter()
        .map(|item| ApplicationModelItemTruth {
            item_key: item.item_key.clone(),
            truth_state: match item.truth_state {
                ApplicationModelTruthStateContract::Observed => {
                    ApplicationModelTruthState::Observed
                }
                ApplicationModelTruthStateContract::Inferred => {
                    ApplicationModelTruthState::Inferred
                }
                ApplicationModelTruthStateContract::Unknown => ApplicationModelTruthState::Unknown,
            },
            source_input_keys: item.source_input_keys.clone(),
            evidence_ids: item
                .evidence
                .iter()
                .map(|evidence| evidence.evidence_id)
                .collect(),
            observed_evidence_ids: item
                .evidence
                .iter()
                .filter(|evidence| {
                    evidence.role == ApplicationModelEvidenceRoleContract::Observation
                })
                .map(|evidence| evidence.evidence_id)
                .collect(),
            referenced_item_keys: item.referenced_item_keys.clone(),
        })
        .collect();
    let stable_hash = format!("sha256:{}", "0".repeat(64));
    validate_application_model_gate_truth(&ApplicationModelGateSnapshot {
        authority_kind: ApplicationModelAuthorityKind::Model,
        operation_id: input.operation_id,
        scope_snapshot_id: input.manifest_id,
        stage_execution_id: input.stage_run_unit_id,
        stage_run_unit_id: input.stage_run_unit_id,
        organization_id: input.organization_id,
        manifest_hash: stable_hash.clone(),
        expected_manifest_hash: stable_hash.clone(),
        schema_version: Some("application_model.v1".to_string()),
        model_hash: Some(stable_hash.clone()),
        expected_model_hash: Some(stable_hash.clone()),
        replay_material_hash: stable_hash.clone(),
        expected_replay_material_hash: stable_hash,
        manifest_input_keys: input
            .manifest_inputs
            .iter()
            .map(|manifest_input| manifest_input.input_key.clone())
            .collect(),
        authorized_evidence_ids: input
            .manifest_inputs
            .iter()
            .flat_map(|manifest_input| manifest_input.evidence_ids.iter().copied())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        decisions,
        items,
        foreign_reference_keys: Vec::new(),
        forbidden_activity_refs: Vec::new(),
        pending_producer_refs: Vec::new(),
    })
    .map_err(|_| ApplicationModelContractViolation::NonContract)?;
    Ok(proposal)
}

/// Assemble the company-level proposal from the exact, already validated shard
/// denominator without asking a provider to reproduce a large JSON document.
///
/// Shard models remain semantic, model-authored results. This boundary only
/// normalizes their closed contracts into stable keys and the revision index,
/// then proves the result through the same pure gate used for provider output.
pub fn deterministically_synthesize_application_model(
    input: &ApplicationModelSynthesisInputContract,
) -> std::result::Result<ApplicationModelProposalContract, ApplicationModelContractViolation> {
    let mut partial_outputs = input.partial_outputs.iter().collect::<Vec<_>>();
    partial_outputs.sort_by(|left, right| {
        left.work_item_key
            .cmp(&right.work_item_key)
            .then_with(|| left.work_item_id.cmp(&right.work_item_id))
    });

    let mut items = Vec::new();
    for output in partial_outputs {
        let mut partial_items = output.items.iter().collect::<Vec<_>>();
        partial_items.sort_by(|left, right| left.item_key.cmp(&right.item_key));
        for (ordinal, partial) in partial_items.into_iter().enumerate() {
            let mut source_input_keys = partial.source_input_keys.clone();
            source_input_keys.sort();
            source_input_keys.dedup();
            let mut evidence = partial.evidence.clone();
            evidence.sort_by_key(|reference| {
                (
                    reference.evidence_id,
                    match reference.role {
                        ApplicationModelEvidenceRoleContract::Observation => 0_u8,
                        ApplicationModelEvidenceRoleContract::Support => 1_u8,
                    },
                )
            });
            items.push(ApplicationModelItemContract {
                item_key: format!("shard:{}:{ordinal:04}", output.work_item_id),
                item_kind: application_model_partial_kind_name(partial.item_kind).to_string(),
                truth_state: partial.truth_state,
                source_input_keys,
                referenced_item_keys: Vec::new(),
                payload: serde_json::json!({
                    "source_work_item_key": output.work_item_key,
                    "source_item_key": partial.item_key,
                    "summary": partial.summary,
                }),
                evidence,
            });
        }

        let source_input_keys = output
            .items
            .iter()
            .flat_map(|item| item.source_input_keys.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if !source_input_keys.is_empty() {
            let mut unknowns = output.unknowns.iter().collect::<Vec<_>>();
            unknowns.sort();
            for (ordinal, unknown) in unknowns.into_iter().enumerate() {
                items.push(ApplicationModelItemContract {
                    item_key: format!("unknown:{}:{ordinal:04}", output.work_item_id),
                    item_kind: "unknown".to_string(),
                    truth_state: ApplicationModelTruthStateContract::Unknown,
                    source_input_keys: source_input_keys.clone(),
                    referenced_item_keys: Vec::new(),
                    payload: serde_json::json!({
                        "source_work_item_key": output.work_item_key,
                        "statement": unknown,
                    }),
                    evidence: Vec::new(),
                });
            }
        }
    }
    items.sort_by(|left, right| left.item_key.cmp(&right.item_key));

    let keys_for_kind = |kind: &str| {
        items
            .iter()
            .filter(|item| item.item_kind == kind)
            .map(|item| item.item_key.clone())
            .collect::<Vec<_>>()
    };
    let structured_model = ApplicationModelV1Contract {
        organization_id: input.organization_id,
        summary: format!(
            "Deterministic company model assembled from {} validated Application Understanding shards with exact frozen input and evidence lineage.",
            input.partial_outputs.len()
        ),
        technologies: keys_for_kind("technology"),
        routes_and_pages: keys_for_kind("route_or_page"),
        api_surfaces: keys_for_kind("api_surface"),
        roles_and_identities: keys_for_kind("role_or_identity"),
        business_entities: keys_for_kind("business_entity"),
        workflows: keys_for_kind("workflow"),
        state_transitions: keys_for_kind("state_transition"),
        ownership_rules: keys_for_kind("ownership_rule"),
        sensitive_operations: keys_for_kind("sensitive_operation"),
        trust_boundaries: keys_for_kind("trust_boundary"),
        unknowns: keys_for_kind("unknown"),
    };

    let mut manifest_inputs = input.manifest_inputs.iter().collect::<Vec<_>>();
    manifest_inputs.sort_by(|left, right| left.input_key.cmp(&right.input_key));
    let decisions = manifest_inputs
        .into_iter()
        .map(|manifest_input| {
            let item_keys = items
                .iter()
                .filter(|item| {
                    item.source_input_keys
                        .iter()
                        .any(|key| key == &manifest_input.input_key)
                })
                .map(|item| item.item_key.clone())
                .collect::<Vec<_>>();
            if item_keys.is_empty() {
                ApplicationModelDecisionContract {
                    input_key: manifest_input.input_key.clone(),
                    disposition: ApplicationModelInputDispositionContract::NotRelevant,
                    item_keys,
                    duplicate_input_key: None,
                    reason_code: Some("no_validated_shard_item".to_string()),
                }
            } else {
                ApplicationModelDecisionContract {
                    input_key: manifest_input.input_key.clone(),
                    disposition: ApplicationModelInputDispositionContract::Incorporated,
                    item_keys,
                    duplicate_input_key: None,
                    reason_code: None,
                }
            }
        })
        .collect();

    let proposal = ApplicationModelProposalContract {
        structured_model: serde_json::to_value(structured_model)
            .map_err(|_| ApplicationModelContractViolation::NonContract)?,
        decisions,
        items,
    };
    parse_and_validate_application_model_proposal_against_synthesis(
        serde_json::to_value(&proposal)
            .map_err(|_| ApplicationModelContractViolation::NonContract)?,
        input,
    )
}

const fn application_model_partial_kind_name(
    kind: ApplicationModelPartialItemKindContract,
) -> &'static str {
    match kind {
        ApplicationModelPartialItemKindContract::Technology => "technology",
        ApplicationModelPartialItemKindContract::RouteOrPage => "route_or_page",
        ApplicationModelPartialItemKindContract::ApiSurface => "api_surface",
        ApplicationModelPartialItemKindContract::RoleOrIdentity => "role_or_identity",
        ApplicationModelPartialItemKindContract::BusinessEntity => "business_entity",
        ApplicationModelPartialItemKindContract::Workflow => "workflow",
        ApplicationModelPartialItemKindContract::StateTransition => "state_transition",
        ApplicationModelPartialItemKindContract::OwnershipRule => "ownership_rule",
        ApplicationModelPartialItemKindContract::SensitiveOperation => "sensitive_operation",
        ApplicationModelPartialItemKindContract::TrustBoundary => "trust_boundary",
        ApplicationModelPartialItemKindContract::Unknown => "unknown",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelDecisionContract {
    pub input_key: String,
    pub disposition: ApplicationModelInputDispositionContract,
    #[serde(default)]
    pub item_keys: Vec<String>,
    #[serde(default)]
    pub duplicate_input_key: Option<String>,
    #[serde(default)]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelInputDispositionContract {
    Incorporated,
    Duplicate,
    NotRelevant,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelItemContract {
    pub item_key: String,
    pub item_kind: String,
    pub truth_state: ApplicationModelTruthStateContract,
    pub source_input_keys: Vec<String>,
    #[serde(default)]
    pub referenced_item_keys: Vec<String>,
    pub payload: serde_json::Value,
    #[serde(default)]
    pub evidence: Vec<ApplicationModelEvidenceContract>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelTruthStateContract {
    Observed,
    Inferred,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplicationModelEvidenceContract {
    pub evidence_id: i64,
    pub role: ApplicationModelEvidenceRoleContract,
}

/// Stable, non-sensitive classification for failures at the tool-free
/// Application Model producer boundary. Provider and parser details stay in
/// backend tracing; durable runtime state records only [`Self::code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationModelProducerFailure {
    CompletionTransport,
    ResponseNonContract,
    Unavailable,
}

impl ApplicationModelProducerFailure {
    pub const fn code(self) -> &'static str {
        match self {
            Self::CompletionTransport => "application_model_completion_transport_failed",
            Self::ResponseNonContract => "application_model_response_non_contract",
            Self::Unavailable => "application_model_producer_unavailable",
        }
    }
}

impl std::fmt::Display for ApplicationModelProducerFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ApplicationModelProducerFailure {}

/// Host-only durable identity for one AU SubAgent execution. This object is
/// never serialized into the model-visible task. The runner uses it to bind
/// the exact WorkerRun/message chain and the app runtime uses it to verify the
/// returned checkpoint witness before any output or Gate mutation.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationModelAgentBinding {
    pub operation_id: uuid::Uuid,
    pub stage_execution_id: uuid::Uuid,
    pub stage_run_unit_id: uuid::Uuid,
    pub stage_team_plan_id: uuid::Uuid,
    pub work_item_id: uuid::Uuid,
    pub worker_run_id: uuid::Uuid,
    pub organization_id: uuid::Uuid,
    pub work_item_key: String,
    pub work_item_kind: String,
    pub work_item_role: String,
    pub lease_token: uuid::Uuid,
    pub attempt_epoch: i64,
    pub session_id: uuid::Uuid,
    pub message_chain_id: uuid::Uuid,
    pub checkpoint_version: i64,
    pub checkpoint_body: serde_json::Value,
    pub parent_request_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationModelAgentOutcome<T> {
    Completed(T),
    Failed(ApplicationModelProducerFailure),
}

/// Result of one real SubAgent invocation. Checkpoint fields are sampled only
/// after the executor has durably persisted its terminal provider turn.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationModelAgentAttempt<T> {
    pub outcome: ApplicationModelAgentOutcome<T>,
    pub checkpoint_version: i64,
    pub checkpoint_body: serde_json::Value,
}

/// Worker-aware, object-safe Application Understanding execution port.
/// Durable scheduling and Gate publication remain owned by the app runtime;
/// this port owns only one exact bound SubAgent turn and its typed result.
#[async_trait::async_trait]
pub trait ApplicationModelAgentRunner: Send + Sync {
    async fn run_work_item(
        &self,
        binding: ApplicationModelAgentBinding,
        input: ApplicationModelWorkItemInputContract,
    ) -> Result<ApplicationModelAgentAttempt<ApplicationModelWorkItemOutputContract>>;

    async fn run_synthesis(
        &self,
        binding: ApplicationModelAgentBinding,
        input: ApplicationModelSynthesisInputContract,
    ) -> Result<ApplicationModelAgentAttempt<ApplicationModelProposalContract>>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelEvidenceRoleContract {
    Observation,
    Support,
}

/// Narrow, tool-free model surface consumed by Application Understanding.
///
/// The controller must not receive the full Primary [`AgentExecutor`] surface:
/// it only needs closed typed completions over server-frozen inputs.
#[async_trait::async_trait]
pub trait ApplicationModelProducer: Send + Sync {
    async fn produce_application_model(
        &self,
        input: ApplicationModelProducerInputContract,
    ) -> Result<ApplicationModelProposalContract>;

    async fn analyze_application_model_work_item(
        &self,
        input: ApplicationModelWorkItemInputContract,
    ) -> Result<ApplicationModelWorkItemOutputContract>;

    async fn synthesize_application_model(
        &self,
        input: ApplicationModelSynthesisInputContract,
    ) -> Result<ApplicationModelProposalContract>;
}

/// App-composition seam for the DB-backed Application Understanding Controller
/// and its closed Modeler/Synthesizer Agents. Keeping this trait in the kit lets `stage_run`
/// invoke the product runtime without introducing a runtime -> app dependency.
#[async_trait::async_trait]
pub trait ApplicationUnderstandingStageRuntime: Send + Sync {
    async fn run(
        &self,
        request: ApplicationUnderstandingStageRequest,
        runner: &dyn ApplicationModelAgentRunner,
    ) -> Result<ApplicationUnderstandingStageOutcome>;
}

/// Maximum reflector attempts before giving up (originally 3, matching PentAGI's
/// maxReflectorCallsPerChain). Raised to 5 (design 2026-07-02-eas-worker-evidence):
/// active stages (EAS/enumeration) burn turns on async scan landing + evidence-id
/// reconciliation, so 3 was too tight to reach a legitimate PASS before
/// `paused_needs_user`. This stays the single source of the repair budget (design
/// 2026-05-26 O3) — do not add a second constant.
pub(super) const MAX_REFLECTOR_RETRIES: usize = 5;

/// A planned subtask from the Generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedSubtask {
    pub title: String,
    pub description: String,
    /// Which specialist should handle this (e.g. "pentester", "coder").
    /// The primary agent uses this as guidance, not a hard constraint.
    pub agent: Option<String>,
    /// Doc 3 §5.2 stage harness hint · 当 subtask 归属某 stage 时填入.
    /// `None` → subtask 不挂任何 stage (gate hook 透传, 不推进游标).
    /// `Some(_)` → execute_single_subtask 末端 hook 走 StageHarness validate_gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_stage: Option<crate::harness::HarnessStageHint>,
    /// Doc 3 §6 NlSlice (终态 4 字段) · stage 内 inner loop 用.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nl_slice: Option<crate::harness::NlSlice>,
    /// 自由文本验收标准 · gate validator 之外的 soft acceptance.
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
}

/// Token usage statistics for a single agent call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Agent phase that consumed these tokens (e.g. "generator", "primary_agent", "refiner", "reporter").
    pub phase: String,
}

/// Context accumulated during task execution, passed between agents.
///
/// Renders in PentAGI-compatible XML format for injection into agent prompts.
#[derive(Debug, Clone, Default)]
pub struct ExecutionContext {
    /// Current harness operation id (Task id in graph-flow task mode). Threaded
    /// into the bridge so loop-level tools such as `stage_run` can update the same
    /// `operation_state` row that the graph checkpointer uses.
    pub operation_id: Option<uuid::Uuid>,
    /// Trusted `stage_runs.id` for this execution. Fresh runs receive it from
    /// compound operation creation; resumes load the exact active row.
    pub stage_execution_id: Option<uuid::Uuid>,
    /// Trusted per-organization stage unit for the active stage.
    pub stage_run_unit_id: Option<uuid::Uuid>,
    /// Specialist worker fencing tuple. Its duplicated unit id must match
    /// `stage_run_unit_id` before the bridge publishes the context.
    pub worker_lease: Option<WorkerLeaseContext>,
    /// Server-owned control-plane metadata for the no-agent/no-provider
    /// Pentest TargetIntel closeout. The durable Unit/Worker ids remain in the
    /// ordinary fields above; this carries only the version/hash material
    /// needed to finalize that exact real fence after the outer Gate passes.
    pub server_authored_stage_control: Option<ServerAuthoredStageControlContext>,
    /// Accumulated results from completed subtasks.
    pub completed_results: Vec<SubtaskResult>,
    /// Request-local top-level input exposed to agents for this execution.
    /// Fresh runs use the operation's original task input. A resumed run uses
    /// the current non-blank continuation/steering message, falling back to the
    /// durable original for an empty message; the durable task row is unchanged.
    pub task_input: String,
    /// Current subtask being executed (if any).
    pub current_subtask: Option<CurrentSubtask>,
    /// Remaining planned subtasks (after the current one).
    pub planned_subtasks: Vec<PlannedSubtaskInfo>,
    /// C3 · harness stage of the current subtask (when stage_mode on). Threaded
    /// to the bridge → agentic loop so per-tool dispatch can enforce the stage's
    /// forbidden-tool barrier. `None` = no stage / flag off.
    pub harness_stage: Option<crate::harness::StageKind>,
    /// C3 · authorization context (profile ceiling + classified intent) for the
    /// current subtask. Threaded alongside `harness_stage` so per-tool dispatch
    /// can run the full pre-action authorizer (allowed_tools confinement + intent
    /// vs ceiling) on real executor tools. `None` = no stage / flag off.
    pub harness_authz: Option<crate::harness::HarnessAuthz>,
    /// C1 · the operation's profile id (from `operation_state.profile`, e.g.
    /// "assessment" / "pentest"). Threaded so the gate hook constructs the
    /// `StageHarness` with the real profile instead of a hardcoded placeholder.
    /// `None` = flag off / no operation_state row (hook falls back to "assessment").
    pub harness_profile_id: Option<String>,
    /// Trusted profile-policy guard. This is true only while the current
    /// TargetIntel stage is being deterministically closed for a profile whose
    /// passive-intel policy is `skip`. Runtime dispatch treats it as a
    /// server-owned deny bit; model/tool arguments can never enable or disable it.
    pub target_intel_provider_hard_skip: bool,
    /// 设计 2026-06-11 (weak-model-submit-channel) · `true` only on a targeted
    /// gate-repair retry pass where the stage work is already evidenced in the
    /// ledger and the ONLY remaining action is the submission. Threaded to the
    /// bridge → agentic loop so the turn's `tool_choice` is locked to
    /// `submit_stage_deliverable` (released once it is dispatched). `false` =
    /// normal pass, no behavior change.
    pub harness_submit_only: bool,
    /// Optional one-shot tool lock for deterministic harness continuations.
    ///
    /// Unlike `harness_submit_only`, this is not a gate-repair semantic; it is a
    /// routing hint for cases where the orchestrator already knows the next
    /// action (for example, a bare "继续" while parked on a specialist stage).
    /// The runtime releases the lock once the named tool has been dispatched.
    pub harness_forced_tool: Option<String>,
    /// Engagement-org isolation (设计 2026-06-15-engagement-org-isolation): the
    /// scoping-confirmed root organization id of THIS operation. Threaded to the
    /// bridge → agentic loop so fan-out / in-scope reads confine to this org's
    /// subtree (root + subsidiaries) and never leak a sibling engagement's org
    /// tree left in the same workspace. `None` = no bound org (legacy whole-DB
    /// axis; flag off / pre-scoping).
    pub harness_org_id: Option<uuid::Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAuthoredStageControlContext {
    pub scope_hash: String,
    pub unit_row_version: i64,
    pub worker_checkpoint_version: i64,
    pub terminal_checkpoint: serde_json::Value,
}

pub const TARGET_INTEL_PROFILE_SKIP_CLAIM: &str = "passive_intel_skipped_by_profile";
pub const TARGET_INTEL_PROFILE_SKIP_REASON: &str =
    "assets confirmed in scoping; passive intel skipped per profile";
pub const TARGET_INTEL_PROFILE_SKIP_WORKFLOW: &str = "pentest_passive_intel_skip";

/// Mark the one direct tool dispatch owned by the Pentest profile policy. The
/// source is task-local server state, never a model-visible deliverable field.
pub fn server_authored_target_intel_tool_source(
    operation_id: uuid::Uuid,
    stage_execution_id: uuid::Uuid,
) -> golish_core::events::ToolSource {
    golish_core::events::ToolSource::Workflow {
        workflow_id: format!("server_policy:target_intel:{operation_id}"),
        workflow_name: TARGET_INTEL_PROFILE_SKIP_WORKFLOW.to_string(),
        step_name: Some(stage_execution_id.to_string()),
        step_index: None,
    }
}

/// Authenticate the task-local tool context for the direct server-policy
/// TargetIntel closeout. Ordinary Primary/model calls remain `ToolSource::Main`
/// and cannot opt themselves into this path through tool arguments.
pub fn is_server_authored_target_intel_tool_context(
    context: &golish_core::AgentToolContext,
) -> bool {
    let (Some(operation_id), Some(stage_execution_id), Some(unit_id), Some(worker)) = (
        context.operation_id,
        context.stage_execution_id,
        context.stage_run_unit_id,
        context.worker_lease.as_ref(),
    ) else {
        return false;
    };
    context.tool_name == "submit_stage_deliverable"
        && context.organization_id.is_some()
        && worker.stage_run_unit_id == unit_id
        && context.candidate_attempt.is_none()
        && context.source
            == server_authored_target_intel_tool_source(operation_id, stage_execution_id)
}

/// Exact deterministic payload accepted for the Pentest TargetIntel hard skip.
/// This validation is shared by the orchestrator, bridge and submit preview so
/// a generic provider-coverage Gate cannot accidentally re-grade a server-owned
/// profile decision, while ordinary agent submissions cannot spoof it.
pub fn validate_server_authored_target_intel_deliverable(
    deliverable: &crate::harness::StageDeliverable,
    organization_id: uuid::Uuid,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        deliverable.stage_id == crate::harness::StageKind::TargetIntel.as_str()
            && deliverable.claims.len() == 1
            && !deliverable.coverage.is_empty()
            && deliverable.evidence_refs.is_empty()
            && deliverable.skipped_checks.is_empty()
            && deliverable.findings.is_empty()
            && deliverable.required_checks_done.is_empty()
            && deliverable.candidates.is_empty()
            && deliverable.candidate_decisions.is_empty(),
        "SERVER_AUTHORED_TARGET_INTEL_PAYLOAD_SHAPE_INVALID"
    );
    let claim = &deliverable.claims[0];
    anyhow::ensure!(
        claim.kind == TARGET_INTEL_PROFILE_SKIP_CLAIM
            && claim.subject == organization_id.to_string()
            && claim.summary == TARGET_INTEL_PROFILE_SKIP_REASON
            && claim.evidence_ids.is_empty()
            && claim.technique.is_none(),
        "SERVER_AUTHORED_TARGET_INTEL_CLAIM_INVALID"
    );
    let mut pairs = std::collections::HashSet::with_capacity(deliverable.coverage.len());
    anyhow::ensure!(
        deliverable.coverage.iter().all(|cell| {
            !cell.asset.trim().is_empty()
                && !cell.technique.trim().is_empty()
                && cell.status == crate::harness::CoverageStatus::NotApplicable
                && cell.evidence_refs.is_empty()
                && cell.note.as_deref() == Some(TARGET_INTEL_PROFILE_SKIP_REASON)
                && cell.reason_kind == Some(crate::harness::types::ReasonKind::NotApplicable)
                && cell.tested_units == 0
                && cell.total_units == 0
                && cell.sampling_rationale.is_none()
                && pairs.insert((cell.asset.as_str(), cell.technique.as_str()))
        }),
        "SERVER_AUTHORED_TARGET_INTEL_COVERAGE_INVALID"
    );
    Ok(())
}

/// Info about a subtask being currently executed.
#[derive(Debug, Clone)]
pub struct CurrentSubtask {
    pub title: String,
    pub description: String,
    pub agent: Option<String>,
}

/// Lightweight info about a planned subtask for context display.
#[derive(Debug, Clone)]
pub struct PlannedSubtaskInfo {
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct SubtaskResult {
    pub title: String,
    pub result: String,
    /// Token usage for executing this subtask (if tracked).
    pub token_usage: Option<AgentTokenUsage>,
}

impl ExecutionContext {
    /// Whether the redundant worker-lease unit witness agrees with this
    /// execution context. Keeping this check at the bridge boundary prevents a
    /// lease from one unit being paired with another unit's trusted identity.
    pub fn trusted_worker_lease_is_consistent(&self) -> bool {
        self.worker_lease
            .as_ref()
            .is_none_or(|lease| self.stage_run_unit_id == Some(lease.stage_run_unit_id))
    }

    pub fn summary(&self) -> String {
        if self.completed_results.is_empty() {
            return "No subtasks completed yet.".to_string();
        }
        let mut s = String::new();
        for (i, r) in self.completed_results.iter().enumerate() {
            s.push_str(&format!(
                "### Subtask {} — {}\n{}\n\n",
                i + 1,
                r.title,
                r.result
            ));
        }
        s
    }

    /// Render the execution context in PentAGI-compatible XML format.
    ///
    /// This format is injected into the orchestrator's prompt as `{{execution_context}}`.
    pub fn render_xml(&self) -> String {
        let mut out = String::new();

        out.push_str("<global_task>\n");
        out.push_str(&self.task_input);
        out.push_str("\n</global_task>\n\n");

        out.push_str("<completed_subtasks>\n");
        if self.completed_results.is_empty() {
            out.push_str("<status>none</status>\n");
            out.push_str(
                "<message>No completed subtasks yet. This is the first subtask.</message>\n",
            );
        } else {
            for (i, r) in self.completed_results.iter().enumerate() {
                out.push_str(&format!(
                    "<subtask>\n<index>{}</index>\n<title>{}</title>\n<result>{}</result>\n</subtask>\n",
                    i + 1, r.title, r.result
                ));
            }
        }
        out.push_str("</completed_subtasks>\n\n");

        if let Some(ref current) = self.current_subtask {
            out.push_str("<current_subtask>\n");
            out.push_str(&format!("<title>{}</title>\n", current.title));
            out.push_str(&format!(
                "<description>{}</description>\n",
                current.description
            ));
            if let Some(ref agent) = current.agent {
                out.push_str(&format!("<assigned_agent>{}</assigned_agent>\n", agent));
            }
            out.push_str("</current_subtask>\n\n");
        }

        out.push_str("<planned_subtasks>\n");
        if self.planned_subtasks.is_empty() {
            out.push_str("<status>none</status>\n");
            out.push_str("<message>No remaining subtasks in the backlog.</message>\n");
        } else {
            for (i, p) in self.planned_subtasks.iter().enumerate() {
                out.push_str(&format!(
                    "<subtask>\n<index>{}</index>\n<title>{}</title>\n<description>{}</description>\n</subtask>\n",
                    i + 1, p.title, p.description
                ));
            }
        }
        out.push_str("</planned_subtasks>");

        out
    }
}

/// Result from an agent execution that includes token tracking.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub content: String,
    pub token_usage: Option<AgentTokenUsage>,
    /// Durable trusted submission captured by `submit_stage_deliverable` during
    /// this agent turn. `None` for ordinary chat and explicit legacy fixtures.
    pub captured_stage_submission: Option<crate::db_traits::CapturedStageSubmission>,
}

impl AgentResult {
    pub fn new(content: String) -> Self {
        Self {
            content,
            token_usage: None,
            captured_stage_submission: None,
        }
    }

    pub fn with_usage(content: String, usage: AgentTokenUsage) -> Self {
        Self {
            content,
            token_usage: Some(usage),
            captured_stage_submission: None,
        }
    }

    pub fn with_captured_stage_submission(
        mut self,
        submission: Option<crate::db_traits::CapturedStageSubmission>,
    ) -> Self {
        self.captured_stage_submission = submission;
        self
    }
}

/// Callback trait for the orchestrator to invoke LLM agents.
///
/// This decouples the orchestrator from `AgentBridge` directly,
/// making it testable and allowing different execution strategies.
///
/// All methods return `AgentResult` to enable per-call token tracking
/// (PentAGI-style per-chain cost accounting).
#[async_trait::async_trait]
pub trait AgentExecutor: Send + Sync {
    /// Execute a single subtask as the primary agent.
    /// Returns the result text and optional token usage.
    /// `agent_type` is the specialist type assigned by the Generator (e.g., "pentester", "coder").
    async fn execute_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: Option<&str>,
    ) -> Result<AgentResult>;

    /// Persist one deterministic, server-authored stage deliverable without
    /// entering an LLM loop. The default is deliberately unsupported so a
    /// profile-owned hard-skip can never fall back to model/provider execution.
    async fn submit_server_authored_stage_deliverable(
        &self,
        _deliverable: crate::harness::StageDeliverable,
        _execution_context: &mut ExecutionContext,
    ) -> Result<AgentResult> {
        anyhow::bail!("SERVER_AUTHORED_STAGE_SUBMISSION_UNSUPPORTED")
    }

    /// Final-seal the real durable server-policy Unit/Worker only after the
    /// ordinary outer stage Gate and trusted-submission checks have passed.
    async fn finalize_server_authored_stage_deliverable(
        &self,
        _deliverable: &crate::harness::StageDeliverable,
        _submission: &crate::db_traits::CapturedStageSubmission,
        _execution_context: &ExecutionContext,
    ) -> Result<()> {
        anyhow::bail!("SERVER_AUTHORED_STAGE_FINALIZATION_UNSUPPORTED")
    }

    /// Terminalize a prepared server-policy Worker after submission/Gate
    /// failure so a no-provider closeout cannot leave a live orphan lease.
    async fn abort_server_authored_stage_deliverable(
        &self,
        _execution_context: &ExecutionContext,
    ) -> Result<()> {
        anyhow::bail!("SERVER_AUTHORED_STAGE_ABORT_UNSUPPORTED")
    }

    /// One-shot completion with no tool registry, browser, shell, network, or
    /// sub-agent surface. Application Understanding uses this narrow path to
    /// produce typed model JSON from already-frozen predecessor data.
    async fn complete_tool_free(
        &self,
        _system_prompt: &str,
        _user_message: &str,
        _phase_key: &str,
    ) -> Result<String> {
        anyhow::bail!("tool-free completion is unavailable for this executor")
    }

    /// Typed Application Understanding producer. Implementations must use a
    /// tool-free completion and reject any response that is not exactly this
    /// closed JSON contract.
    async fn produce_application_model(
        &self,
        _input: ApplicationModelProducerInputContract,
    ) -> Result<ApplicationModelProposalContract> {
        Err(ApplicationModelProducerFailure::Unavailable.into())
    }

    /// Analyze one server-frozen, redacted Application Understanding shard.
    /// The implementation must remain tool-free and validate the echoed
    /// identity and references against `input` before returning.
    async fn analyze_application_model_work_item(
        &self,
        _input: ApplicationModelWorkItemInputContract,
    ) -> Result<ApplicationModelWorkItemOutputContract> {
        Err(ApplicationModelProducerFailure::Unavailable.into())
    }

    /// Synthesize one company's exact validated shard set. Implementations
    /// receive only the manifest envelope and partial outputs, never raw
    /// predecessor payloads.
    async fn synthesize_application_model(
        &self,
        _input: ApplicationModelSynthesisInputContract,
    ) -> Result<ApplicationModelProposalContract> {
        Err(ApplicationModelProducerFailure::Unavailable.into())
    }

    /// Whether `stage_run` consumed this top-level request's bounded retry
    /// budget for `stage`.
    ///
    /// Executors without a request-scoped stage runner keep the default false.
    /// The task orchestrator uses this runtime signal to stop its own automatic
    /// gate-repair loop without conflating that loop with a later, explicit user
    /// continuation (which starts a new request and resets the guard).
    fn stage_run_retry_budget_exhausted(&self, _stage: crate::harness::StageKind) -> bool {
        false
    }

    /// Run the reporter to generate the final summary.
    async fn generate_report(&self, execution_context: &ExecutionContext) -> Result<AgentResult>;

    /// Run the reflector to redirect an agent that returned plain text.
    ///
    /// Returns a corrective message that should be injected as a user message
    /// before retrying the subtask. The reflector acts as a "proxy user" that
    /// guides the agent back to tool usage (PentAGI's Reflector pattern).
    ///
    /// Deprecated（设计 2026-06-12-unified-refiner PR-R4）：task_orchestrator 不再
    /// 调用此方法——text-only 响应改由 `task_orchestrator::refiner::refine_text_only`
    /// 的确定性模板纠正。实现与本方法的删除留给 bridge 清理 PR。
    async fn reflect(&self, subtask_title: &str, agent_response: &str) -> Result<String>;

    /// Enrich a subtask with supplementary context before execution.
    ///
    /// Mirrors PentAGI's `enricher.tmpl`: searches memory, knowledge base,
    /// and completed subtask results to add context the executing agent
    /// wouldn't otherwise have. Returns the enrichment text to prepend.
    ///
    /// Default returns `Ok(None)` (no enrichment).
    async fn enrich_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: &str,
    ) -> Result<Option<String>> {
        let _ = (
            subtask_title,
            subtask_description,
            execution_context,
            agent_type,
        );
        Ok(None)
    }

    /// Generate an execution plan for a subtask before delegating it.
    ///
    /// Mirrors PentAGI's `question_task_planner.tmpl` + `task_assignment_wrapper.tmpl`:
    /// the Adviser creates a concise checklist (3-7 steps) that is wrapped
    /// around the original task description.
    ///
    /// Default returns `Ok(None)` (no pre-planning).
    async fn plan_subtask(
        &self,
        subtask_title: &str,
        subtask_description: &str,
        execution_context: &ExecutionContext,
        agent_type: &str,
    ) -> Result<Option<String>> {
        let _ = (
            subtask_title,
            subtask_description,
            execution_context,
            agent_type,
        );
        Ok(None)
    }

    /// Monitor execution progress and provide corrective advice.
    ///
    /// Mirrors PentAGI's `question_execution_monitor.tmpl`: when the agentic
    /// loop detects repetitive tool usage, this method is called to generate
    /// strategic advice that is injected into the next tool response.
    ///
    /// Default returns `Ok(None)` (no advice).
    async fn monitor_execution(
        &self,
        subtask_description: &str,
        repeated_tool: &str,
        repeat_count: usize,
        recent_tool_calls: &str,
    ) -> Result<Option<String>> {
        let _ = (
            subtask_description,
            repeated_tool,
            repeat_count,
            recent_tool_calls,
        );
        Ok(None)
    }

    /// Serialize the current message chain for persistence.
    ///
    /// Returns the conversation messages as JSON for storage in the
    /// `message_chains` table. Default returns `None` (no persistence).
    fn current_message_chain(&self) -> Option<serde_json::Value> {
        None
    }
}

#[async_trait::async_trait]
impl<T> ApplicationModelProducer for T
where
    T: AgentExecutor + ?Sized,
{
    async fn produce_application_model(
        &self,
        input: ApplicationModelProducerInputContract,
    ) -> Result<ApplicationModelProposalContract> {
        AgentExecutor::produce_application_model(self, input).await
    }

    async fn analyze_application_model_work_item(
        &self,
        input: ApplicationModelWorkItemInputContract,
    ) -> Result<ApplicationModelWorkItemOutputContract> {
        AgentExecutor::analyze_application_model_work_item(self, input).await
    }

    async fn synthesize_application_model(
        &self,
        input: ApplicationModelSynthesisInputContract,
    ) -> Result<ApplicationModelProposalContract> {
        AgentExecutor::synthesize_application_model(self, input).await
    }
}

#[cfg(test)]
mod trusted_context_tests {
    use super::*;

    struct DefaultApplicationModelExecutor;

    #[async_trait::async_trait]
    impl AgentExecutor for DefaultApplicationModelExecutor {
        async fn execute_subtask(
            &self,
            _subtask_title: &str,
            _subtask_description: &str,
            _execution_context: &ExecutionContext,
            _agent_type: Option<&str>,
        ) -> Result<AgentResult> {
            unreachable!("not used by the producer-port test")
        }

        async fn generate_report(
            &self,
            _execution_context: &ExecutionContext,
        ) -> Result<AgentResult> {
            unreachable!("not used by the producer-port test")
        }

        async fn reflect(&self, _subtask_title: &str, _agent_response: &str) -> Result<String> {
            unreachable!("not used by the producer-port test")
        }
    }

    fn valid_work_item_input_value() -> serde_json::Value {
        serde_json::json!({
            "operation_id": uuid::Uuid::from_u128(0x801),
            "manifest_id": uuid::Uuid::from_u128(0x802),
            "organization_id": uuid::Uuid::from_u128(0x803),
            "stage_run_unit_id": uuid::Uuid::from_u128(0x804),
            "work_item_id": uuid::Uuid::from_u128(0x805),
            "work_item_key": "web_origin:example-com",
            "work_item_kind": "web_origin",
            "projection_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "projection": {
                "subjects": [{
                    "kind": "web_origin",
                    "value": "https://app.example.com:443"
                }],
                "routes": [{
                    "scheme": "https",
                    "host": "app.example.com",
                    "port": 443,
                    "method": "GET",
                    "route_shape": "/orders/{id}",
                    "status_code": 200,
                    "content_type": "application/json",
                    "parameters": [{
                        "name": "id",
                        "location": "path",
                        "value_type": "string",
                        "required": true
                    }]
                }],
                "services": [{
                    "host": "app.example.com",
                    "port": 443,
                    "transport": "tcp",
                    "service_name": "https",
                    "product": "nginx",
                    "version": "1.25",
                    "tls": true
                }],
                "fingerprints": [{
                    "name": "React",
                    "category": "framework",
                    "version": "19",
                    "confidence": 95
                }],
                "manifest_inputs": [{
                    "input_key": "enumeration:app",
                    "input_kind": "enumeration_handoff",
                    "source_id": "handoff-7",
                    "source_version": 3,
                    "content_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "evidence_ids": [41, 42]
                }],
                "projection_incomplete": false
            }
        })
    }

    #[tokio::test]
    async fn application_model_producer_blanket_adapter_preserves_typed_contract() {
        let input = serde_json::from_value::<ApplicationModelWorkItemInputContract>(
            valid_work_item_input_value(),
        )
        .expect("valid frozen work-item input");
        let error = ApplicationModelProducer::analyze_application_model_work_item(
            &DefaultApplicationModelExecutor,
            input,
        )
        .await
        .expect_err("default AgentExecutor producer remains fail-closed");

        assert_eq!(
            error.downcast_ref::<ApplicationModelProducerFailure>(),
            Some(&ApplicationModelProducerFailure::Unavailable)
        );
    }

    #[test]
    fn application_understanding_stage_runtime_is_object_safe() {
        fn accepts_runtime(_runtime: Option<&dyn ApplicationUnderstandingStageRuntime>) {}

        accepts_runtime(None);
    }

    #[test]
    fn application_model_agent_runner_is_object_safe() {
        fn accepts_runner(_runner: Option<&dyn ApplicationModelAgentRunner>) {}

        accepts_runner(None);
    }

    #[test]
    fn application_model_agent_schemas_pin_host_owned_identity() {
        let input: ApplicationModelWorkItemInputContract =
            serde_json::from_value(valid_work_item_input_value()).unwrap();
        let shard = application_model_work_item_output_json_schema(&input);
        assert_eq!(
            shard["properties"]["organization_id"]["const"],
            serde_json::json!(input.organization_id.to_string())
        );
        assert_eq!(
            shard["properties"]["work_item_id"]["const"],
            serde_json::json!(input.work_item_id.to_string())
        );
        assert_eq!(
            shard["properties"]["projection_hash"]["const"],
            serde_json::json!(input.projection_hash)
        );

        let proposal = application_model_proposal_json_schema(input.organization_id);
        assert_eq!(
            proposal["properties"]["structured_model"]["properties"]["organization_id"]["const"],
            serde_json::json!(input.organization_id.to_string())
        );
    }

    #[test]
    fn application_model_work_item_schema_documents_truth_state_evidence_invariant() {
        let input: ApplicationModelWorkItemInputContract =
            serde_json::from_value(valid_work_item_input_value()).unwrap();
        let schema = application_model_work_item_output_json_schema(&input);
        let item_properties = &schema["properties"]["items"]["items"]["properties"];
        let truth_state_description = item_properties["truth_state"]["description"]
            .as_str()
            .expect("truth_state must document its evidence invariant");
        let evidence_description = item_properties["evidence"]["description"]
            .as_str()
            .expect("evidence must document its truth-state invariant");

        for required_phrase in ["observed", "observation"] {
            assert!(
                truth_state_description.contains(required_phrase),
                "truth_state description must mention `{required_phrase}`"
            );
        }
        for required_phrase in ["inferred", "unknown", "support"] {
            assert!(
                evidence_description.contains(required_phrase),
                "evidence description must mention `{required_phrase}`"
            );
        }
    }

    fn valid_work_item_output_value() -> serde_json::Value {
        serde_json::json!({
            "organization_id": uuid::Uuid::from_u128(0x803),
            "work_item_id": uuid::Uuid::from_u128(0x805),
            "work_item_key": "web_origin:example-com",
            "projection_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "summary": "Observed an authenticated order workflow.",
            "items": [{
                "item_key": "workflow:order-read",
                "item_kind": "workflow",
                "truth_state": "observed",
                "summary": "GET /orders/{id} exposes an order read workflow.",
                "source_input_keys": ["enumeration:app"],
                "evidence": [{"evidence_id": 41, "role": "observation"}]
            }],
            "unknowns": ["Authorization semantics are not established"]
        })
    }

    #[test]
    fn application_model_work_item_input_is_closed_and_bounded() {
        let valid = valid_work_item_input_value();
        assert!(
            serde_json::from_value::<ApplicationModelWorkItemInputContract>(valid.clone()).is_ok()
        );

        let mut unknown = valid.clone();
        unknown["source_payload"] = serde_json::json!({"Cookie": "secret-sentinel"});
        assert!(serde_json::from_value::<ApplicationModelWorkItemInputContract>(unknown).is_err());

        let mut invalid_hash = valid.clone();
        invalid_hash["projection_hash"] =
            serde_json::json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(
            serde_json::from_value::<ApplicationModelWorkItemInputContract>(invalid_hash).is_err()
        );

        let mut overlong_route = valid.clone();
        overlong_route["projection"]["routes"][0]["route_shape"] =
            serde_json::json!("x".repeat(513));
        assert!(
            serde_json::from_value::<ApplicationModelWorkItemInputContract>(overlong_route)
                .is_err()
        );

        let mut duplicate_evidence = valid;
        duplicate_evidence["projection"]["manifest_inputs"][0]["evidence_ids"] =
            serde_json::json!([41, 41]);
        assert!(
            serde_json::from_value::<ApplicationModelWorkItemInputContract>(duplicate_evidence)
                .is_err()
        );

        let mut unsafe_subject = valid_work_item_input_value();
        unsafe_subject["projection"]["subjects"][0]["value"] =
            serde_json::json!("https://user:pass@app.example.com/orders?token=secret#raw");
        assert!(
            serde_json::from_value::<ApplicationModelWorkItemInputContract>(unsafe_subject)
                .is_err()
        );

        let mut duplicate_parameter = valid_work_item_input_value();
        let duplicate = duplicate_parameter["projection"]["routes"][0]["parameters"][0].clone();
        duplicate_parameter["projection"]["routes"][0]["parameters"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(
            serde_json::from_value::<ApplicationModelWorkItemInputContract>(duplicate_parameter)
                .is_err()
        );
    }

    #[test]
    fn application_model_work_item_output_enforces_identity_and_authorized_references() {
        let input: ApplicationModelWorkItemInputContract =
            serde_json::from_value(valid_work_item_input_value()).unwrap();
        let valid: ApplicationModelWorkItemOutputContract =
            serde_json::from_value(valid_work_item_output_value()).unwrap();
        assert!(valid.validate_against(&input).is_ok());

        let mut wrong_identity = valid_work_item_output_value();
        wrong_identity["organization_id"] = serde_json::json!(uuid::Uuid::from_u128(0x806));
        let wrong_identity: ApplicationModelWorkItemOutputContract =
            serde_json::from_value(wrong_identity).unwrap();
        assert!(wrong_identity.validate_against(&input).is_err());

        let mut foreign_evidence = valid_work_item_output_value();
        foreign_evidence["items"][0]["evidence"][0]["evidence_id"] = serde_json::json!(99);
        let foreign_evidence: ApplicationModelWorkItemOutputContract =
            serde_json::from_value(foreign_evidence).unwrap();
        assert!(foreign_evidence.validate_against(&input).is_err());

        let mut foreign_input = valid_work_item_output_value();
        foreign_input["items"][0]["source_input_keys"] = serde_json::json!(["vuln:foreign"]);
        let foreign_input: ApplicationModelWorkItemOutputContract =
            serde_json::from_value(foreign_input).unwrap();
        assert!(foreign_input.validate_against(&input).is_err());
    }

    #[test]
    fn application_model_work_item_terminal_parse_rejects_semantic_and_authority_conflicts() {
        let input: ApplicationModelWorkItemInputContract =
            serde_json::from_value(valid_work_item_input_value()).unwrap();

        let parsed = parse_and_validate_application_model_work_item_output(
            valid_work_item_output_value(),
            &input,
        )
        .expect("valid terminal payload must return the typed output");
        assert_eq!(parsed.organization_id, input.organization_id);

        let mut observed_without_observation = valid_work_item_output_value();
        observed_without_observation["items"][0]["evidence"][0]["role"] =
            serde_json::json!("support");
        assert_eq!(
            parse_and_validate_application_model_work_item_output(
                observed_without_observation,
                &input,
            ),
            Err(ApplicationModelContractViolation::NonContract)
        );

        let mut foreign_evidence = valid_work_item_output_value();
        foreign_evidence["items"][0]["evidence"][0]["evidence_id"] = serde_json::json!(999);
        assert_eq!(
            parse_and_validate_application_model_work_item_output(foreign_evidence, &input),
            Err(ApplicationModelContractViolation::UnauthorizedEvidenceReference)
        );

        let mut foreign_input = valid_work_item_output_value();
        foreign_input["items"][0]["source_input_keys"] = serde_json::json!(["foreign:input"]);
        assert_eq!(
            parse_and_validate_application_model_work_item_output(foreign_input, &input),
            Err(ApplicationModelContractViolation::UnauthorizedInputReference)
        );
    }

    #[test]
    fn application_model_work_item_output_rejects_duplicate_and_invalid_evidence() {
        let mut duplicate_items = valid_work_item_output_value();
        let duplicate = duplicate_items["items"][0].clone();
        duplicate_items["items"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(
            serde_json::from_value::<ApplicationModelWorkItemOutputContract>(duplicate_items)
                .is_err()
        );

        let mut zero_evidence = valid_work_item_output_value();
        zero_evidence["items"][0]["evidence"][0]["evidence_id"] = serde_json::json!(0);
        assert!(
            serde_json::from_value::<ApplicationModelWorkItemOutputContract>(zero_evidence)
                .is_err()
        );

        let mut unknown = valid_work_item_output_value();
        unknown["commentary"] = serde_json::json!("outside contract");
        assert!(serde_json::from_value::<ApplicationModelWorkItemOutputContract>(unknown).is_err());
    }

    #[test]
    fn application_model_synthesis_input_requires_exact_expected_shards() {
        let input = serde_json::json!({
            "operation_id": uuid::Uuid::from_u128(0x801),
            "manifest_id": uuid::Uuid::from_u128(0x802),
            "organization_id": uuid::Uuid::from_u128(0x803),
            "stage_run_unit_id": uuid::Uuid::from_u128(0x804),
            "manifest_inputs": [{
                "input_key": "enumeration:app",
                "input_kind": "enumeration_handoff",
                "source_id": "handoff-7",
                "source_version": 3,
                "content_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "evidence_ids": [41, 42]
            }],
            "expected_work_items": [{
                "work_item_id": uuid::Uuid::from_u128(0x805),
                "work_item_key": "web_origin:example-com",
                "projection_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }],
            "partial_outputs": [valid_work_item_output_value()]
        });
        assert!(
            serde_json::from_value::<ApplicationModelSynthesisInputContract>(input.clone()).is_ok()
        );

        let mut missing = input.clone();
        missing["partial_outputs"] = serde_json::json!([]);
        assert!(serde_json::from_value::<ApplicationModelSynthesisInputContract>(missing).is_err());

        let mut drifted = input;
        drifted["partial_outputs"][0]["projection_hash"] = serde_json::json!(
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        );
        assert!(serde_json::from_value::<ApplicationModelSynthesisInputContract>(drifted).is_err());
    }

    #[test]
    fn deterministic_application_model_synthesis_preserves_exact_lineage() {
        let input: ApplicationModelSynthesisInputContract = serde_json::from_value(
            serde_json::json!({
                "operation_id": uuid::Uuid::from_u128(0x801),
                "manifest_id": uuid::Uuid::from_u128(0x802),
                "organization_id": uuid::Uuid::from_u128(0x803),
                "stage_run_unit_id": uuid::Uuid::from_u128(0x804),
                "manifest_inputs": [{
                    "input_key": "enumeration:app",
                    "input_kind": "enumeration_handoff",
                    "source_id": "handoff-7",
                    "source_version": 3,
                    "content_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "evidence_ids": [41, 42]
                }],
                "expected_work_items": [{
                    "work_item_id": uuid::Uuid::from_u128(0x805),
                    "work_item_key": "web_origin:example-com",
                    "projection_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }],
                "partial_outputs": [valid_work_item_output_value()]
            }),
        )
        .expect("valid synthesis denominator");

        let first = deterministically_synthesize_application_model(&input)
            .expect("validated shards must produce a closed proposal");
        let second = deterministically_synthesize_application_model(&input)
            .expect("the same denominator must be reproducible");
        assert_eq!(first, second);
        assert_eq!(first.items.len(), 2, "partial unknowns must not be dropped");
        assert_eq!(first.decisions.len(), input.manifest_inputs.len());
        assert_eq!(
            first.decisions[0].disposition,
            ApplicationModelInputDispositionContract::Incorporated
        );
        assert_eq!(first.decisions[0].item_keys.len(), first.items.len());

        parse_and_validate_application_model_proposal_against_synthesis(
            serde_json::to_value(first).expect("proposal serializes"),
            &input,
        )
        .expect("server assembly must pass the same semantic gate as a provider proposal");
    }

    #[test]
    fn application_model_producer_failure_codes_are_stable() {
        assert_eq!(
            ApplicationModelProducerFailure::CompletionTransport.code(),
            "application_model_completion_transport_failed"
        );
        assert_eq!(
            ApplicationModelProducerFailure::ResponseNonContract.code(),
            "application_model_response_non_contract"
        );
        assert_eq!(
            ApplicationModelProducerFailure::Unavailable.code(),
            "application_model_producer_unavailable"
        );
    }

    #[test]
    fn worker_lease_must_witness_the_same_stage_run_unit() {
        let unit_id = uuid::Uuid::from_u128(0x701);
        let matching = ExecutionContext {
            stage_execution_id: Some(uuid::Uuid::from_u128(0x702)),
            stage_run_unit_id: Some(unit_id),
            worker_lease: Some(WorkerLeaseContext {
                worker_run_id: uuid::Uuid::from_u128(0x703),
                stage_run_unit_id: unit_id,
                lease_token: uuid::Uuid::from_u128(0x704),
                attempt_epoch: 1,
            }),
            ..Default::default()
        };
        assert!(matching.trusted_worker_lease_is_consistent());

        let mismatched = ExecutionContext {
            stage_run_unit_id: Some(uuid::Uuid::from_u128(0x705)),
            ..matching
        };
        assert!(!mismatched.trusted_worker_lease_is_consistent());
    }

    #[test]
    fn application_model_proposal_rejects_unknown_fields_and_prose_wrappers() {
        let valid = serde_json::json!({
            "structured_model": {
                "organization_id": uuid::Uuid::from_u128(0x706),
                "summary": "Observed order workflow",
                "technologies": [],
                "routes_and_pages": [],
                "api_surfaces": [],
                "roles_and_identities": [],
                "business_entities": [],
                "workflows": ["workflow:order_read"],
                "state_transitions": [],
                "ownership_rules": [],
                "sensitive_operations": [],
                "trust_boundaries": [],
                "unknowns": []
            },
            "decisions": [{
                "input_key": "vuln-handoff",
                "disposition": "incorporated",
                "item_keys": ["workflow:order_read"],
                "duplicate_input_key": null,
                "reason_code": null
            }],
            "items": [{
                "item_key": "workflow:order_read",
                "item_kind": "workflow",
                "truth_state": "observed",
                "source_input_keys": ["vuln-handoff"],
                "referenced_item_keys": [],
                "payload": {"path": "/orders/{id}"},
                "evidence": [{"evidence_id": 1, "role": "observation"}]
            }]
        });
        assert!(serde_json::from_value::<ApplicationModelProposalContract>(valid.clone()).is_ok());

        let mut unknown_field = valid.clone();
        unknown_field["commentary"] = serde_json::json!("not part of the contract");
        assert!(serde_json::from_value::<ApplicationModelProposalContract>(unknown_field).is_err());
        assert!(serde_json::from_str::<ApplicationModelProposalContract>(
            "model follows: {\"structured_model\":{},\"decisions\":[],\"items\":[]}"
        )
        .is_err());

        let empty_model = serde_json::json!({
            "structured_model": {},
            "decisions": [],
            "items": []
        });
        assert!(serde_json::from_value::<ApplicationModelProposalContract>(empty_model).is_err());

        let mut unknown_model_field = valid;
        unknown_model_field["structured_model"]["commentary"] =
            serde_json::json!("not part of application_model.v1");
        assert!(
            serde_json::from_value::<ApplicationModelProposalContract>(unknown_model_field)
                .is_err()
        );

        let unknown_only = serde_json::json!({
            "structured_model": {
                "organization_id": uuid::Uuid::from_u128(0x706),
                "summary": "Authorized inputs were insufficient to infer application semantics",
                "technologies": [],
                "routes_and_pages": [],
                "api_surfaces": [],
                "roles_and_identities": [],
                "business_entities": [],
                "workflows": [],
                "state_transitions": [],
                "ownership_rules": [],
                "sensitive_operations": [],
                "trust_boundaries": [],
                "unknowns": []
            },
            "decisions": [{
                "input_key": "vuln-handoff",
                "disposition": "unknown",
                "item_keys": [],
                "duplicate_input_key": null,
                "reason_code": "insufficient_evidence"
            }],
            "items": []
        });
        assert!(
            serde_json::from_value::<ApplicationModelProposalContract>(unknown_only).is_ok(),
            "closed input decisions may truthfully produce an empty classified model"
        );
    }

    #[test]
    fn application_model_proposal_terminal_parse_rejects_wrong_organization_and_non_contract() {
        let organization_id = uuid::Uuid::from_u128(0x706);
        let valid = serde_json::json!({
            "structured_model": {
                "organization_id": organization_id,
                "summary": "Observed order workflow",
                "technologies": [],
                "routes_and_pages": [],
                "api_surfaces": [],
                "roles_and_identities": [],
                "business_entities": [],
                "workflows": ["workflow:order_read"],
                "state_transitions": [],
                "ownership_rules": [],
                "sensitive_operations": [],
                "trust_boundaries": [],
                "unknowns": []
            },
            "decisions": [{
                "input_key": "vuln-handoff",
                "disposition": "incorporated",
                "item_keys": ["workflow:order_read"],
                "duplicate_input_key": null,
                "reason_code": null
            }],
            "items": [{
                "item_key": "workflow:order_read",
                "item_kind": "workflow",
                "truth_state": "observed",
                "source_input_keys": ["vuln-handoff"],
                "referenced_item_keys": [],
                "payload": {"path": "/orders/{id}"},
                "evidence": [{"evidence_id": 1, "role": "observation"}]
            }]
        });

        let parsed = parse_and_validate_application_model_proposal(valid.clone(), organization_id)
            .expect("valid proposal must return the typed contract");
        assert_eq!(
            parsed.structured_model["organization_id"],
            serde_json::json!(organization_id)
        );

        let mut wrong_organization = valid;
        wrong_organization["structured_model"]["organization_id"] =
            serde_json::json!(uuid::Uuid::from_u128(0x707));
        assert_eq!(
            parse_and_validate_application_model_proposal(wrong_organization, organization_id),
            Err(ApplicationModelContractViolation::IdentityMismatch)
        );
        assert_eq!(
            parse_and_validate_application_model_proposal(
                serde_json::json!({"not": "a proposal"}),
                organization_id,
            ),
            Err(ApplicationModelContractViolation::NonContract)
        );
    }
}
