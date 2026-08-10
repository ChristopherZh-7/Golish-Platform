//! Guarded compound persistence for Enumeration endpoint provenance V2.
//!
//! Occurrence writes are append-only and deliberately separate from the
//! legacy `api_endpoints` aggregate. Only [`project_endpoint_groups`] may
//! project production-eligible groups into the legacy manifest.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

use super::capability_execution_receipts::{
    CapabilityReceiptInputRef, CoverageDenominatorRow, EvidenceAuthorityRef,
    ToolTruthExecutionAuthorityRef,
};
use crate::{DbError, Result};

const ENUMERATION_V2_WRITER_DISABLED: &str = "ENUMERATION_V2_WRITER_DISABLED";
const ENUMERATION_RECEIPT_V1_REQUIRED: &str = "ENUMERATION_RECEIPT_V1_REQUIRED";
const ENUMERATION_SHADOW_PROJECTION_FORBIDDEN: &str = "ENUMERATION_SHADOW_PROJECTION_FORBIDDEN";
const ENUMERATION_AUTHORITY_MISMATCH: &str = "ENUMERATION_AUTHORITY_MISMATCH";
const ENUMERATION_RESOLUTION_CLOSEOUT_INPUT_INVALID: &str =
    "ENUMERATION_RESOLUTION_CLOSEOUT_INPUT_INVALID";
const ENUMERATION_RESOLUTION_CLOSEOUT_PRODUCER_MISMATCH: &str =
    "ENUMERATION_RESOLUTION_CLOSEOUT_PRODUCER_MISMATCH";
const ENUMERATION_RESOLUTION_CLOSEOUT_WORKER_FENCE_MISMATCH: &str =
    "ENUMERATION_RESOLUTION_CLOSEOUT_WORKER_FENCE_MISMATCH";
const ENUMERATION_RESOLUTION_CLOSEOUT_UNIT_MISMATCH: &str =
    "ENUMERATION_RESOLUTION_CLOSEOUT_UNIT_MISMATCH";
const ENUMERATION_VALUE_BEARING_METADATA: &str = "ENUMERATION_VALUE_BEARING_METADATA";
const ENUMERATION_MANIFEST_DRIFT: &str = "ENUMERATION_MANIFEST_DRIFT";

fn fail(code: &'static str) -> DbError {
    DbError::Other(anyhow::anyhow!(code))
}

fn sha256_json(value: &serde_json::Value) -> Result<String> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| DbError::Other(anyhow::Error::new(error)))?;
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    Ok(format!("sha256:{encoded}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationWorkerFence {
    pub worker_run_id: Uuid,
    pub worker_attempt_epoch: i64,
    pub lease_token: Uuid,
    pub source_tool_call_id: Uuid,
}

/// Trusted-host request for converting an already sealed StageTeamUnit source
/// root into the worker/tool authority used by every Enumeration V2 child.
/// The caller never supplies an authority id/hash or denominator member set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealEnumerationWorkerAuthorityRoot {
    pub stable_authority_request_id: Uuid,
    pub stable_root_request_id: Uuid,
    pub source_root_denominator_id: Uuid,
    pub worker_fence: EnumerationWorkerFence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationWorkerAuthorityRoot {
    pub authority: ToolTruthExecutionAuthorityRef,
    pub root_denominator: CoverageDenominatorRow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationDerivedDenominatorItemWrite {
    pub input_key: String,
    pub target_id: Uuid,
    pub exact_asset: String,
    pub technique: String,
    pub expected_capability: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealEnumerationDerivedDenominator {
    pub stable_seal_request_id: Uuid,
    pub parent_denominator_id: Uuid,
    pub parent_denominator_item_id: Uuid,
    pub derived_ordinal: i32,
    pub items: Vec<EnumerationDerivedDenominatorItemWrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnumerationTerminalInputOutcome {
    Found,
    CheckedEmpty,
    NotApplicable,
    UnresolvedExhausted { coverage_gap_reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationTerminalReceiptInputWrite {
    pub denominator_item_id: Uuid,
    pub outcome: EnumerationTerminalInputOutcome,
    pub evidence_authorities: Vec<EvidenceAuthorityRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealEnumerationTerminalReceiptInputs {
    pub stable_seal_request_id: Uuid,
    pub receipt_id: Uuid,
    pub inputs: Vec<EnumerationTerminalReceiptInputWrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealEnumerationCandidateClosure {
    pub stable_closure_request_id: Uuid,
    pub candidate_input_id: Uuid,
    pub resolution_terminal_input: Option<CapabilityReceiptInputRef>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationCandidateClosureRow {
    pub id: Uuid,
    pub stable_closure_request_id: Uuid,
    pub candidate_input_id: Uuid,
    pub execution_authority_id: Uuid,
    pub terminal_receipt_id: Uuid,
    pub terminal_receipt_input_id: Uuid,
    pub resolution_execution_authority_id: Option<Uuid>,
    pub resolution_terminal_receipt_id: Option<Uuid>,
    pub resolution_terminal_receipt_input_id: Option<Uuid>,
    pub terminal_disposition: String,
    pub occurrence_count: i64,
    pub occurrence_set_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealEnumerationCandidateDenominatorClosure {
    pub stable_closure_request_id: Uuid,
    pub denominator_id: Uuid,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationCandidateDenominatorClosureRow {
    pub id: Uuid,
    pub stable_closure_request_id: Uuid,
    pub denominator_id: Uuid,
    pub execution_authority_id: Uuid,
    pub member_count: i64,
    pub member_set_hash: String,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationTerminalCoverage {
    pub js_expected: i64,
    pub js_terminal: i64,
    pub candidate_expected: i64,
    pub candidate_terminal: i64,
    pub parameter_expected: i64,
    pub parameter_terminal: i64,
    pub missing: i64,
}

impl EnumerationTerminalCoverage {
    pub fn is_complete_non_vacuous(&self) -> bool {
        self.js_expected > 0
            && self.candidate_expected > 0
            && self.parameter_expected > 0
            && self.js_terminal == self.js_expected
            && self.candidate_terminal == self.candidate_expected
            && self.parameter_terminal == self.parameter_expected
            && self.missing == 0
    }
}

pub const ENUMERATION_TERMINAL_COVERAGE_REDUCER_SQL: &str = r#"
WITH js_expected AS (
    SELECT denominator.id AS denominator_id,item.id AS item_id
      FROM coverage_denominators denominator
      JOIN coverage_denominator_items item ON item.denominator_id=denominator.id
     WHERE denominator.execution_authority_id=$1
       AND denominator.sealed_at IS NOT NULL
       AND item.expected_capability='enumeration.javascript'
       AND enumeration_denominator_has_worker_root(denominator.id,$1)
), js_terminal AS (
    SELECT DISTINCT expected.item_id
      FROM js_expected expected
      JOIN enumeration_js_analysis_items descriptor
        ON descriptor.denominator_id=expected.denominator_id
       AND descriptor.denominator_item_id=expected.item_id
       AND descriptor.execution_authority_id=$1
      JOIN capability_execution_receipt_inputs input
        ON input.id=descriptor.terminal_receipt_input_id
       AND input.receipt_id=descriptor.terminal_receipt_id
       AND input.denominator_item_id=expected.item_id
       AND input.execution_authority_id=$1
       AND input.sealed_at IS NOT NULL
      JOIN enumeration_receipt_input_census_seals census
        ON census.receipt_id=input.receipt_id
       AND census.denominator_id=expected.denominator_id
       AND census.execution_authority_id=$1
), candidate_expected AS (
    SELECT denominator.id AS denominator_id,item.id AS item_id
      FROM coverage_denominators denominator
      JOIN coverage_denominator_items item ON item.denominator_id=denominator.id
      JOIN js_expected parent
        ON parent.denominator_id=denominator.parent_denominator_id
       AND parent.item_id=denominator.parent_denominator_item_id
     WHERE denominator.execution_authority_id=$1
       AND denominator.sealed_at IS NOT NULL
       AND item.expected_capability='enumeration.candidate'
), candidate_terminal AS (
    SELECT DISTINCT expected.item_id
      FROM candidate_expected expected
      JOIN enumeration_endpoint_candidate_inputs candidate
        ON candidate.denominator_id=expected.denominator_id
       AND candidate.denominator_item_id=expected.item_id
       AND candidate.execution_authority_id=$1
      JOIN capability_execution_receipt_inputs input
        ON input.id=candidate.terminal_receipt_input_id
       AND input.receipt_id=candidate.terminal_receipt_id
       AND input.sealed_at IS NOT NULL
      JOIN enumeration_receipt_input_census_seals census
        ON census.receipt_id=input.receipt_id
       AND census.denominator_id=expected.denominator_id
       AND census.execution_authority_id=$1
      JOIN enumeration_endpoint_candidate_closure_receipts closure
        ON closure.candidate_input_id=candidate.id
       AND closure.execution_authority_id=$1
      JOIN enumeration_endpoint_candidate_denominator_closure_receipts denominator_closure
        ON denominator_closure.denominator_id=expected.denominator_id
       AND denominator_closure.execution_authority_id=$1
), parameter_expected AS (
    SELECT denominator.id AS denominator_id,item.id AS item_id
      FROM coverage_denominators denominator
      JOIN coverage_denominator_items item ON item.denominator_id=denominator.id
      JOIN candidate_expected parent
        ON parent.denominator_id=denominator.parent_denominator_id
       AND parent.item_id=denominator.parent_denominator_item_id
     WHERE denominator.execution_authority_id=$1
       AND denominator.sealed_at IS NOT NULL
       AND item.expected_capability='enumeration.parameter'
), parameter_terminal AS (
    SELECT DISTINCT expected.item_id
      FROM parameter_expected expected
      JOIN enumeration_endpoint_parameter_assessments assessment
        ON assessment.denominator_id=expected.denominator_id
       AND assessment.denominator_item_id=expected.item_id
       AND assessment.execution_authority_id=$1
      JOIN capability_execution_receipt_inputs input
        ON input.id=assessment.terminal_receipt_input_id
       AND input.receipt_id=assessment.terminal_receipt_id
       AND input.sealed_at IS NOT NULL
      JOIN enumeration_receipt_input_census_seals census
        ON census.receipt_id=input.receipt_id
       AND census.denominator_id=expected.denominator_id
       AND census.execution_authority_id=$1
      JOIN enumeration_endpoint_occurrence_evidence evidence
        ON evidence.parameter_assessment_id=assessment.id
       AND evidence.parameter_assessment_execution_authority_id=$1
       AND evidence.evidence_execution_authority_id=$1
       AND evidence.evidence_role='parameter'
     WHERE assessment.parameter_outcome<>'found' OR EXISTS (
         SELECT 1 FROM enumeration_endpoint_occurrence_parameters parameter
          WHERE parameter.assessment_id=assessment.id
     )
), missing AS (
    (SELECT item_id FROM js_expected EXCEPT SELECT item_id FROM js_terminal)
    UNION ALL
    (SELECT item_id FROM candidate_expected EXCEPT SELECT item_id FROM candidate_terminal)
    UNION ALL
    (SELECT item_id FROM parameter_expected EXCEPT SELECT item_id FROM parameter_terminal)
)
SELECT (SELECT COUNT(*) FROM js_expected)::BIGINT AS js_expected,
       (SELECT COUNT(*) FROM js_terminal)::BIGINT AS js_terminal,
       (SELECT COUNT(*) FROM candidate_expected)::BIGINT AS candidate_expected,
       (SELECT COUNT(*) FROM candidate_terminal)::BIGINT AS candidate_terminal,
       (SELECT COUNT(*) FROM parameter_expected)::BIGINT AS parameter_expected,
       (SELECT COUNT(*) FROM parameter_terminal)::BIGINT AS parameter_terminal,
       (SELECT COUNT(*) FROM missing)::BIGINT AS missing
"#;

#[derive(Debug, Clone, FromRow)]
struct EnumerationSourceRoot {
    source_execution_authority_id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    project_path_at_freeze: String,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_kind: String,
    stage_run_unit_id: Uuid,
    contract: String,
    input_manifest_hash: String,
    denominator_hash: String,
    member_count: i64,
    member_set_hash: String,
    enumeration_analysis_contract: String,
    tool_truth_contract: String,
}

#[derive(Debug, Clone, FromRow, PartialEq, Eq)]
struct EnumerationDenominatorItem {
    id: Uuid,
    ordinal: i32,
    input_key: String,
    target_id: Uuid,
    exact_asset: String,
    technique: String,
    expected_capability: String,
    member_hash: String,
}

#[derive(Debug, Clone, FromRow)]
struct EnumerationWorkerAuthorityRow {
    id: Uuid,
    operation_id: Uuid,
    project_scope_id: Uuid,
    project_path_at_freeze: String,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    authority_hash: String,
    stage_run_unit_id: Option<Uuid>,
    worker_run_id: Option<Uuid>,
    worker_attempt_epoch: Option<i64>,
    lease_token: Option<Uuid>,
    source_tool_call_id: Option<Uuid>,
}

impl EnumerationWorkerAuthorityRow {
    fn as_ref(&self) -> ToolTruthExecutionAuthorityRef {
        ToolTruthExecutionAuthorityRef {
            id: self.id,
            operation_id: self.operation_id,
            project_scope_id: self.project_scope_id,
            project_path_at_freeze: self.project_path_at_freeze.clone(),
            scope_snapshot_id: self.scope_snapshot_id,
            organization_id: self.organization_id,
            stage_execution_id: self.stage_execution_id,
            authority_hash: self.authority_hash.clone(),
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct FrozenEnumerationContract {
    enumeration_analysis_contract: String,
    tool_truth_contract: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsAnalysisDescriptorWrite {
    pub id: Uuid,
    pub stable_descriptor_request_id: Uuid,
    pub manifest_url: String,
    pub page_url: String,
    pub document_url: Option<String>,
    pub chunk_ordinal: i32,
    pub source_map_url: Option<String>,
    pub script_sha256: Option<String>,
    pub descriptor_metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateDescriptorWrite {
    pub id: Uuid,
    pub stable_candidate_request_id: Uuid,
    pub js_analysis_item_id: Option<Uuid>,
    pub source_anchor: String,
    pub callsite_fingerprint: String,
    pub capture_event_id: Uuid,
    pub capture_attempt_ordinal: i32,
    pub captured_at: DateTime<Utc>,
    pub event_fingerprint: String,
    pub duplicate_ordinal: i32,
    pub resolution_input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EndpointOccurrenceWrite {
    pub id: Uuid,
    pub stable_occurrence_request_id: Uuid,
    pub candidate_input_id: Uuid,
    pub capture_event_id: Uuid,
    pub source_target_id: Uuid,
    pub source_web_origin_id: Uuid,
    pub resolved_target_id: Option<Uuid>,
    pub resolved_web_origin_id: Option<Uuid>,
    pub parent_occurrence_id: Option<Uuid>,
    pub source_url: String,
    pub document_url: Option<String>,
    pub script_url: Option<String>,
    pub script_sha256: Option<String>,
    pub source_span: serde_json::Value,
    pub initiator_url: Option<String>,
    pub initiator_status: String,
    pub initiator_line: Option<i32>,
    pub initiator_column: Option<i32>,
    pub cdp_request_id_hash: Option<String>,
    pub protocol: String,
    pub method: String,
    pub graphql_operation_name: Option<String>,
    pub websocket_subprotocol: Option<String>,
    pub raw_expression: Option<String>,
    pub receiver_kind: Option<String>,
    pub observation_kind: String,
    pub inference_level: String,
    pub resolution_status: String,
    pub scope_decision: String,
    pub candidate_classification: String,
    pub canonical_request_url: Option<String>,
    pub display_url: Option<String>,
    pub resolution_reason: String,
    pub resolution_base_facts: serde_json::Value,
    pub resolution_candidates: serde_json::Value,
    pub resolution_chain: serde_json::Value,
    pub route_kind: String,
    pub route_template: Option<String>,
    pub request_sent: bool,
    pub request_schema: serde_json::Value,
    pub redaction_metadata: serde_json::Value,
    pub request_body_length: Option<i64>,
    pub runtime_sample_url: Option<String>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedEndpointOccurrence {
    pub id: Uuid,
    pub candidate_input_id: Uuid,
    pub execution_authority_id: Uuid,
    pub operation_id: Uuid,
    pub source_target_id: Uuid,
    pub source_web_origin_id: Uuid,
    pub resolved_target_id: Option<Uuid>,
    pub resolved_web_origin_id: Option<Uuid>,
    pub protocol: String,
    pub method: String,
    pub observation_kind: String,
    pub inference_level: String,
    pub resolution_status: String,
    pub scope_decision: String,
    pub candidate_classification: String,
    pub route_kind: String,
    pub promotion_eligible: bool,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParameterAssessmentWrite {
    pub id: Uuid,
    pub occurrence_id: Uuid,
    pub outcome: String,
    pub reason_code: String,
    pub parameters: Vec<OccurrenceParameterWrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OccurrenceParameterWrite {
    pub id: Uuid,
    pub name: String,
    pub location: String,
    pub value_type: String,
    pub requirement: String,
    pub confidence: f32,
    pub source_anchor_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointGroupProjectionSummary {
    pub groups_created: u64,
    pub occurrence_links_created: u64,
    pub api_links_created: u64,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationLaneCommitReceiptRow {
    pub id: Uuid,
    pub stable_commit_request_id: Uuid,
    pub execution_authority_id: Uuid,
    pub lane: String,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub target_id: Uuid,
    pub exact_origin: String,
    pub artifact_sha256: String,
    pub dependency_receipt_ids: Vec<Uuid>,
    pub evidence_audit_ids: Vec<i64>,
    pub script_denominator_id: Option<Uuid>,
    pub candidate_denominator_ids: Vec<Uuid>,
    pub parameter_denominator_ids: Vec<Uuid>,
    pub resolution_occurrence_id: Option<Uuid>,
    pub resolution_terminal_receipt_id: Option<Uuid>,
    pub resolution_terminal_receipt_input_id: Option<Uuid>,
    pub terminal_disposition: String,
    pub entity_set_sha256: String,
    pub denominator_set_sha256: String,
    pub receipt_set_sha256: String,
    pub closure_graph_sha256: String,
    pub script_count: i64,
    pub candidate_count: i64,
    pub occurrence_count: i64,
    pub parameter_assessment_count: i64,
    pub parameter_fact_count: i64,
    pub unresolved_count: i64,
    pub missing: i64,
    pub group_count: i64,
    pub occurrence_link_count: i64,
    pub api_link_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealEnumerationLaneCommitReceipt {
    pub stable_commit_request_id: Uuid,
    pub lane: String,
    pub target_id: Uuid,
    pub exact_origin: String,
    pub artifact_sha256: String,
    pub dependency_receipt_ids: Vec<Uuid>,
    pub evidence_audit_ids: Vec<i64>,
    pub script_denominator_id: Option<Uuid>,
    pub candidate_denominator_ids: Vec<Uuid>,
    pub parameter_denominator_ids: Vec<Uuid>,
    pub resolution_occurrence_id: Option<Uuid>,
    pub resolution_terminal_receipt_id: Option<Uuid>,
    pub resolution_terminal_receipt_input_id: Option<Uuid>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnumerationResolutionCloseoutRow {
    pub id: Uuid,
    pub stable_closeout_request_id: Uuid,
    pub execution_authority_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub assigned_work_item_id: Uuid,
    pub worker_run_id: Uuid,
    pub source_tool_call_id: Uuid,
    pub worker_attempt_epoch: i64,
    pub lease_token: Uuid,
    pub parent_occurrence_id: Uuid,
    pub producer_lane_receipt_id: Uuid,
    pub terminal_state: String,
    pub reason_code: String,
    pub suggestion_ids: Vec<Uuid>,
    pub terminal_receipt_id: Uuid,
    pub terminal_receipt_input_id: Uuid,
    pub evidence_set_sha256: String,
    pub closeout_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealEnumerationResolutionCloseout {
    pub stable_closeout_request_id: Uuid,
    pub assigned_work_item_id: Uuid,
    pub worker_fence: EnumerationWorkerFence,
    pub parent_occurrence_id: Uuid,
    pub producer_lane_receipt_id: Uuid,
    pub terminal_state: String,
    pub reason_code: String,
    pub suggestion_ids: Vec<Uuid>,
    pub terminal_receipt_id: Uuid,
    pub terminal_receipt_input_id: Uuid,
}

#[derive(Debug, FromRow)]
struct ProjectableGroup {
    id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    resolved_target_id: Uuid,
    resolved_web_origin_id: Uuid,
    protocol: String,
    method: String,
    route_template: String,
    replay_url: String,
}

#[derive(Debug, FromRow)]
struct ProjectableParameter {
    name: String,
    location: String,
    value_type: String,
    requirement: String,
}

const ENUMERATION_DENOMINATOR_COLUMNS: &str =
    "id,stable_seal_request_id,execution_authority_id,contract,input_manifest_hash,member_count,member_set_hash,denominator_hash,sealed_at";

/// Create or load the only valid Enumeration V2 authority root for one live
/// worker/tool fence. The root member census is copied from an already sealed
/// host-owned StageTeamUnit root while both sets are locked; callers cannot
/// provide member ids, hashes, targets, capabilities, or owner tuples.
pub async fn seal_enumeration_worker_authority_root(
    pool: &PgPool,
    command: &SealEnumerationWorkerAuthorityRoot,
) -> Result<EnumerationWorkerAuthorityRoot> {
    let mut tx = pool.begin().await?;
    let result = seal_enumeration_worker_authority_root_in_connection(&mut tx, command).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn seal_enumeration_worker_authority_root_in_connection(
    conn: &mut PgConnection,
    command: &SealEnumerationWorkerAuthorityRoot,
) -> Result<EnumerationWorkerAuthorityRoot> {
    if command.stable_authority_request_id.is_nil()
        || command.stable_root_request_id.is_nil()
        || command.source_root_denominator_id.is_nil()
        || command.worker_fence.worker_run_id.is_nil()
        || command.worker_fence.worker_attempt_epoch < 0
        || command.worker_fence.lease_token.is_nil()
        || command.worker_fence.source_tool_call_id.is_nil()
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let source = sqlx::query_as::<_, EnumerationSourceRoot>(
        r#"SELECT a.id AS source_execution_authority_id,
                  d.operation_id,d.project_scope_id,d.project_path_at_freeze,
                  d.scope_snapshot_id,d.organization_id,d.stage_execution_id,d.stage_kind,
                  a.stage_run_unit_id,d.contract,d.input_manifest_hash,d.denominator_hash,
                  d.member_count,d.member_set_hash,o.enumeration_analysis_contract,
                  o.tool_truth_contract
             FROM coverage_denominators d
             JOIN tool_truth_execution_authorities a ON a.id=d.execution_authority_id
             JOIN operation_state o ON o.operation_id=d.operation_id
            WHERE d.id=$1 AND d.denominator_kind='root' AND d.sealed_at IS NOT NULL
              AND d.member_count>0 AND a.execution_owner_kind='host_stage'
              AND a.execution_source_kind='stage_unit' AND a.stage_run_unit_id IS NOT NULL
            FOR SHARE OF d,a,o"#,
    )
    .bind(command.source_root_denominator_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
    if !matches!(
        source.enumeration_analysis_contract.as_str(),
        "agent_team_v2_shadow" | "agent_team_v2"
    ) || source.contract != source.tool_truth_contract
        || (source.enumeration_analysis_contract == "agent_team_v2"
            && source.tool_truth_contract != "receipt_v1")
    {
        return Err(fail(ENUMERATION_RECEIPT_V1_REQUIRED));
    }
    let live_fence: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM stage_worker_runs worker
               JOIN tool_calls call ON call.id=$5
                AND call.worker_run_id=worker.id
                AND call.operation_id=worker.operation_id
                AND call.stage_execution_id=worker.stage_execution_id
                AND call.stage_run_unit_id=worker.stage_run_unit_id
                AND call.organization_id=worker.organization_id
                AND call.attempt_epoch=worker.attempt_epoch
                AND call.lease_token=worker.lease_token
              WHERE worker.id=$1 AND worker.stage_run_unit_id=$2
                AND worker.attempt_epoch=$3 AND worker.lease_token=$4
                AND worker.operation_id=$6 AND worker.stage_execution_id=$7
                AND worker.organization_id=$8
                AND worker.active_tool_call_id=$5
                AND worker.status IN ('running','waiting_background')
                AND worker.lease_expires_at>statement_timestamp()
                AND call.status IN ('received','running')
           )"#,
    )
    .bind(command.worker_fence.worker_run_id)
    .bind(source.stage_run_unit_id)
    .bind(command.worker_fence.worker_attempt_epoch)
    .bind(command.worker_fence.lease_token)
    .bind(command.worker_fence.source_tool_call_id)
    .bind(source.operation_id)
    .bind(source.stage_execution_id)
    .bind(source.organization_id)
    .fetch_one(&mut *conn)
    .await?;
    if !live_fence {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }

    let authority_id = Uuid::new_v5(
        &command.stable_authority_request_id,
        b"enumeration-worker-tool-authority-v1",
    );
    sqlx::query(
        r#"INSERT INTO tool_truth_execution_authorities(
               id,stable_authority_request_id,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_source_kind,stage_run_unit_id,
               execution_owner_kind,worker_run_id,worker_attempt_epoch,lease_token,
               source_tool_call_id,authority_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'stage_unit',$10,'worker_tool',
                  $11,$12,$13,$14,$15)
           ON CONFLICT(operation_id,stable_authority_request_id) DO NOTHING"#,
    )
    .bind(authority_id)
    .bind(command.stable_authority_request_id)
    .bind(source.operation_id)
    .bind(source.project_scope_id)
    .bind(&source.project_path_at_freeze)
    .bind(source.scope_snapshot_id)
    .bind(source.organization_id)
    .bind(source.stage_execution_id)
    .bind(&source.stage_kind)
    .bind(source.stage_run_unit_id)
    .bind(command.worker_fence.worker_run_id)
    .bind(command.worker_fence.worker_attempt_epoch)
    .bind(command.worker_fence.lease_token)
    .bind(command.worker_fence.source_tool_call_id)
    // The BEFORE INSERT trigger derives the immutable authority hash from the
    // full worker fence. Supply an already-valid hash-shaped value here so the
    // row also remains valid if trigger execution is inspected independently.
    .bind(&source.denominator_hash)
    .execute(&mut *conn)
    .await?;
    let authority = sqlx::query_as::<_, EnumerationWorkerAuthorityRow>(
        r#"SELECT id,operation_id,project_scope_id,project_path_at_freeze,
                  scope_snapshot_id,organization_id,stage_execution_id,authority_hash,
                  stage_run_unit_id,worker_run_id,worker_attempt_epoch,lease_token,
                  source_tool_call_id
             FROM tool_truth_execution_authorities
            WHERE operation_id=$1 AND stable_authority_request_id=$2
              AND execution_owner_kind='worker_tool' FOR SHARE"#,
    )
    .bind(source.operation_id)
    .bind(command.stable_authority_request_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
    if authority.id != authority_id
        || authority.stage_run_unit_id != Some(source.stage_run_unit_id)
        || authority.worker_run_id != Some(command.worker_fence.worker_run_id)
        || authority.worker_attempt_epoch != Some(command.worker_fence.worker_attempt_epoch)
        || authority.lease_token != Some(command.worker_fence.lease_token)
        || authority.source_tool_call_id != Some(command.worker_fence.source_tool_call_id)
    {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }

    if let Some(existing) = sqlx::query_as::<_, CoverageDenominatorRow>(&format!(
        "SELECT {ENUMERATION_DENOMINATOR_COLUMNS} FROM coverage_denominators WHERE execution_authority_id=$1 AND stable_seal_request_id=$2 FOR SHARE"
    ))
    .bind(authority.id)
    .bind(command.stable_root_request_id)
    .fetch_optional(&mut *conn)
    .await?
    {
        let exact_bridge: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM enumeration_worker_authority_roots
                    WHERE operation_id=$1 AND stable_root_request_id=$2
                      AND source_root_denominator_id=$3
                      AND source_execution_authority_id=$4
                      AND source_denominator_hash=$5
                      AND worker_root_denominator_id=$6
                      AND worker_execution_authority_id=$7
               )"#,
        )
        .bind(source.operation_id)
        .bind(command.stable_root_request_id)
        .bind(command.source_root_denominator_id)
        .bind(source.source_execution_authority_id)
        .bind(&source.denominator_hash)
        .bind(existing.id)
        .bind(authority.id)
        .fetch_one(&mut *conn)
        .await?;
        if !exact_bridge {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        return Ok(EnumerationWorkerAuthorityRoot {
            authority: authority.as_ref(),
            root_denominator: existing,
        });
    }

    let source_items = sqlx::query_as::<_, EnumerationDenominatorItem>(
        r#"SELECT id,ordinal,input_key,target_id,exact_asset,technique,
                  expected_capability,member_hash
             FROM coverage_denominator_items
            WHERE denominator_id=$1 ORDER BY ordinal FOR SHARE"#,
    )
    .bind(command.source_root_denominator_id)
    .fetch_all(&mut *conn)
    .await?;
    if i64::try_from(source_items.len()).ok() != Some(source.member_count) {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    let root_denominator_id = Uuid::new_v5(
        &command.stable_root_request_id,
        b"enumeration-worker-root-denominator-v1",
    );
    let worker_denominator_hash: String = sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(jsonb_build_object(
               'execution_authority_hash',$1::TEXT,
               'input_manifest_hash',$2::TEXT,
               'contract',$3::TEXT,
               'denominator_kind','root'
           )::TEXT)"#,
    )
    .bind(&authority.authority_hash)
    .bind(&source.input_manifest_hash)
    .bind(&source.contract)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        r#"INSERT INTO coverage_denominators(
               id,stable_seal_request_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,denominator_kind,
               contract,input_manifest_hash,denominator_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,'root',$12,$13,$14)"#,
    )
    .bind(root_denominator_id)
    .bind(command.stable_root_request_id)
    .bind(authority.id)
    .bind(source.operation_id)
    .bind(source.project_scope_id)
    .bind(&source.project_path_at_freeze)
    .bind(source.scope_snapshot_id)
    .bind(source.organization_id)
    .bind(source.stage_execution_id)
    .bind(&source.stage_kind)
    .bind(&authority.authority_hash)
    .bind(&source.contract)
    .bind(&source.input_manifest_hash)
    .bind(&worker_denominator_hash)
    .execute(&mut *conn)
    .await?;
    for source_item in &source_items {
        sqlx::query(
            r#"INSERT INTO coverage_denominator_items(
                   id,denominator_id,execution_authority_id,denominator_hash,ordinal,
                   input_key,target_id,exact_asset,technique,expected_capability,member_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(Uuid::new_v5(
            &root_denominator_id,
            source_item.id.as_bytes(),
        ))
        .bind(root_denominator_id)
        .bind(authority.id)
        .bind(&worker_denominator_hash)
        .bind(source_item.ordinal)
        .bind(&source_item.input_key)
        .bind(source_item.target_id)
        .bind(&source_item.exact_asset)
        .bind(&source_item.technique)
        .bind(&source_item.expected_capability)
        .bind(&source_item.member_hash)
        .execute(&mut *conn)
        .await?;
    }
    let root_denominator = sqlx::query_as::<_, CoverageDenominatorRow>(&format!(
        "UPDATE coverage_denominators SET sealed_at=statement_timestamp() WHERE id=$1 RETURNING {ENUMERATION_DENOMINATOR_COLUMNS}"
    ))
    .bind(root_denominator_id)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        r#"INSERT INTO enumeration_worker_authority_roots(
               id,stable_root_request_id,source_root_denominator_id,
               source_execution_authority_id,source_denominator_hash,
               worker_root_denominator_id,worker_execution_authority_id,
               worker_denominator_hash,operation_id,project_scope_id,
               project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_run_unit_id,worker_run_id,
               worker_attempt_epoch,lease_token,source_tool_call_id,
               source_member_count,source_member_set_hash,root_seal_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
                  $19,$20,$21,$22)"#,
    )
    .bind(Uuid::new_v5(
        &root_denominator_id,
        b"enumeration-worker-root-seal-v1",
    ))
    .bind(command.stable_root_request_id)
    .bind(command.source_root_denominator_id)
    .bind(source.source_execution_authority_id)
    .bind(&source.denominator_hash)
    .bind(root_denominator_id)
    .bind(authority.id)
    .bind(&worker_denominator_hash)
    .bind(source.operation_id)
    .bind(source.project_scope_id)
    .bind(&source.project_path_at_freeze)
    .bind(source.scope_snapshot_id)
    .bind(source.organization_id)
    .bind(source.stage_execution_id)
    .bind(source.stage_run_unit_id)
    .bind(command.worker_fence.worker_run_id)
    .bind(command.worker_fence.worker_attempt_epoch)
    .bind(command.worker_fence.lease_token)
    .bind(command.worker_fence.source_tool_call_id)
    .bind(source.member_count)
    .bind(&source.member_set_hash)
    .bind(&worker_denominator_hash)
    .execute(&mut *conn)
    .await?;
    Ok(EnumerationWorkerAuthorityRoot {
        authority: authority.as_ref(),
        root_denominator,
    })
}

/// Seal one derived child beneath the worker root. An empty member set is a
/// first-class checked-empty denominator, not an unchecked/missing shortcut.
/// Hashes and ids are server-derived; the caller supplies only typed,
/// value-free logical members.
pub async fn seal_enumeration_derived_denominator(
    pool: &PgPool,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationDerivedDenominator,
) -> Result<CoverageDenominatorRow> {
    let mut tx = pool.begin().await?;
    let result =
        seal_enumeration_derived_denominator_in_connection(&mut tx, authority, command).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn seal_enumeration_derived_denominator_in_connection(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationDerivedDenominator,
) -> Result<CoverageDenominatorRow> {
    if command.stable_seal_request_id.is_nil()
        || command.parent_denominator_id.is_nil()
        || command.parent_denominator_item_id.is_nil()
        || command.derived_ordinal <= 0
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let mut input_keys = std::collections::BTreeSet::new();
    if command.items.iter().any(|item| {
        item.target_id.is_nil()
            || item.input_key.trim().is_empty()
            || item.exact_asset.trim().is_empty()
            || item.technique.trim().is_empty()
            || item.expected_capability.trim().is_empty()
            || !input_keys.insert(item.input_key.clone())
    }) {
        return Err(fail(ENUMERATION_VALUE_BEARING_METADATA));
    }
    lock_contract(conn, authority).await?;
    let (contract, parent_hash): (String, String) = sqlx::query_as(
        r#"SELECT parent.contract,parent.denominator_hash
             FROM coverage_denominators parent
             JOIN coverage_denominator_items item
               ON item.id=$2 AND item.denominator_id=parent.id
              AND item.execution_authority_id=parent.execution_authority_id
            WHERE parent.id=$1 AND parent.execution_authority_id=$3
              AND parent.sealed_at IS NOT NULL
              AND enumeration_denominator_has_worker_root(parent.id,parent.execution_authority_id)
            FOR SHARE OF parent,item"#,
    )
    .bind(command.parent_denominator_id)
    .bind(command.parent_denominator_item_id)
    .bind(authority.id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
    let target_ids = command.items.iter().map(|item| item.target_id).collect();
    lock_scoped_ids(conn, authority, "targets", target_ids).await?;

    let denominator_id = Uuid::new_v5(
        &command.stable_seal_request_id,
        b"enumeration-derived-denominator-v1",
    );
    let mut persisted_items = Vec::with_capacity(command.items.len());
    for (ordinal, item) in command.items.iter().enumerate() {
        let ordinal = i32::try_from(ordinal).map_err(|_| fail(ENUMERATION_MANIFEST_DRIFT))?;
        let member_hash = sha256_json(&serde_json::json!({
            "ordinal": ordinal,
            "input_key": item.input_key,
            "target_id": item.target_id,
            "exact_asset": item.exact_asset,
            "technique": item.technique,
            "expected_capability": item.expected_capability,
        }))?;
        persisted_items.push(EnumerationDenominatorItem {
            id: Uuid::new_v5(&denominator_id, item.input_key.as_bytes()),
            ordinal,
            input_key: item.input_key.clone(),
            target_id: item.target_id,
            exact_asset: item.exact_asset.clone(),
            technique: item.technique.clone(),
            expected_capability: item.expected_capability.clone(),
            member_hash,
        });
    }
    let input_manifest_hash = sha256_json(&serde_json::json!(persisted_items
        .iter()
        .map(|item| &item.member_hash)
        .collect::<Vec<_>>()))?;
    let denominator_hash: String = sqlx::query_scalar(
        r#"SELECT tool_truth_sha256(jsonb_build_object(
               'execution_authority_hash',$1::TEXT,
               'input_manifest_hash',$2::TEXT,
               'contract',$3::TEXT,
               'denominator_kind','derived_child',
               'parent_denominator_id',$4::UUID,
               'parent_denominator_item_id',$5::UUID,
               'parent_denominator_hash',$6::TEXT,
               'derived_ordinal',$7::INTEGER
           )::TEXT)"#,
    )
    .bind(&authority.authority_hash)
    .bind(&input_manifest_hash)
    .bind(&contract)
    .bind(command.parent_denominator_id)
    .bind(command.parent_denominator_item_id)
    .bind(parent_hash)
    .bind(command.derived_ordinal)
    .fetch_one(&mut *conn)
    .await?;
    if let Some(existing) = sqlx::query_as::<_, CoverageDenominatorRow>(&format!(
        "SELECT {ENUMERATION_DENOMINATOR_COLUMNS} FROM coverage_denominators WHERE execution_authority_id=$1 AND stable_seal_request_id=$2 FOR SHARE"
    ))
    .bind(authority.id)
    .bind(command.stable_seal_request_id)
    .fetch_optional(&mut *conn)
    .await?
    {
        let exact_items = sqlx::query_as::<_, EnumerationDenominatorItem>(
            r#"SELECT id,ordinal,input_key,target_id,exact_asset,technique,
                      expected_capability,member_hash
                 FROM coverage_denominator_items WHERE denominator_id=$1 ORDER BY ordinal"#,
        )
        .bind(existing.id)
        .fetch_all(&mut *conn)
        .await?;
        if existing.id != denominator_id
            || existing.input_manifest_hash != input_manifest_hash
            || existing.denominator_hash != denominator_hash
            || exact_items != persisted_items
        {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        return Ok(existing);
    }
    sqlx::query(
        r#"INSERT INTO coverage_denominators(
               id,stable_seal_request_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_kind,execution_authority_hash,denominator_kind,
               parent_denominator_id,parent_denominator_item_id,derived_ordinal,
               contract,input_manifest_hash,denominator_hash)
           SELECT $1,$2,$3,a.operation_id,a.project_scope_id,a.project_path_at_freeze,
                  a.scope_snapshot_id,a.organization_id,a.stage_execution_id,a.stage_kind,
                  a.authority_hash,'derived_child',$4,$5,$6,$7,$8,$9
             FROM tool_truth_execution_authorities a WHERE a.id=$3"#,
    )
    .bind(denominator_id)
    .bind(command.stable_seal_request_id)
    .bind(authority.id)
    .bind(command.parent_denominator_id)
    .bind(command.parent_denominator_item_id)
    .bind(command.derived_ordinal)
    .bind(&contract)
    .bind(&input_manifest_hash)
    .bind(&denominator_hash)
    .execute(&mut *conn)
    .await?;
    for item in &persisted_items {
        sqlx::query(
            r#"INSERT INTO coverage_denominator_items(
                   id,denominator_id,execution_authority_id,denominator_hash,ordinal,
                   input_key,target_id,exact_asset,technique,expected_capability,member_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(item.id)
        .bind(denominator_id)
        .bind(authority.id)
        .bind(&denominator_hash)
        .bind(item.ordinal)
        .bind(&item.input_key)
        .bind(item.target_id)
        .bind(&item.exact_asset)
        .bind(&item.technique)
        .bind(&item.expected_capability)
        .bind(&item.member_hash)
        .execute(&mut *conn)
        .await?;
    }
    let row = sqlx::query_as::<_, CoverageDenominatorRow>(&format!(
        "UPDATE coverage_denominators SET sealed_at=statement_timestamp() WHERE id=$1 RETURNING {ENUMERATION_DENOMINATOR_COLUMNS}"
    ))
    .bind(denominator_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// Seal the exact terminal input census for an already-begun generic Tool
/// Truth receipt. Every terminal input must carry normalized same-authority
/// evidence; the DB-generated census seal is what descriptor writers trust.
pub async fn seal_enumeration_terminal_receipt_inputs(
    pool: &PgPool,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationTerminalReceiptInputs,
) -> Result<Vec<CapabilityReceiptInputRef>> {
    let mut tx = pool.begin().await?;
    let result =
        seal_enumeration_terminal_receipt_inputs_in_connection(&mut tx, authority, command).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn seal_enumeration_terminal_receipt_inputs_in_connection(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationTerminalReceiptInputs,
) -> Result<Vec<CapabilityReceiptInputRef>> {
    if command.stable_seal_request_id.is_nil() || command.receipt_id.is_nil() {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    lock_contract(conn, authority).await?;
    let (denominator_id, capability, denominator_sealed_empty, denominator_member_count): (
        Uuid,
        String,
        bool,
        i64,
    ) = sqlx::query_as(
        r#"SELECT receipt.denominator_id,receipt.capability,
                  denominator.sealed_empty,denominator.member_count
             FROM capability_execution_receipts receipt
             JOIN coverage_denominators denominator ON denominator.id=receipt.denominator_id
            WHERE receipt.id=$1 AND receipt.execution_authority_id=$2
              AND denominator.execution_authority_id=$2 AND denominator.sealed_at IS NOT NULL
              AND enumeration_denominator_has_worker_root(denominator.id,$2)
            FOR SHARE OF receipt,denominator"#,
    )
    .bind(command.receipt_id)
    .bind(authority.id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
    let expected = sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT id,input_key FROM coverage_denominator_items
            WHERE denominator_id=$1 AND execution_authority_id=$2
              AND expected_capability=$3 ORDER BY ordinal FOR SHARE"#,
    )
    .bind(denominator_id)
    .bind(authority.id)
    .bind(&capability)
    .fetch_all(&mut *conn)
    .await?;
    let expected_ids = expected
        .iter()
        .map(|item| item.0)
        .collect::<std::collections::BTreeSet<_>>();
    let supplied_ids = command
        .inputs
        .iter()
        .map(|input| input.denominator_item_id)
        .collect::<std::collections::BTreeSet<_>>();
    if expected.len() != command.inputs.len()
        || i64::try_from(expected.len()).ok() != Some(denominator_member_count)
        || denominator_sealed_empty != expected.is_empty()
        || supplied_ids.len() != command.inputs.len()
        || expected_ids != supplied_ids
        || command
            .inputs
            .iter()
            .any(|input| input.evidence_authorities.is_empty())
    {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }

    if let Some((existing_stable_request_id,)) = sqlx::query_as::<_, (Uuid,)>(
        "SELECT stable_seal_request_id FROM enumeration_receipt_input_census_seals WHERE receipt_id=$1 FOR SHARE",
    )
    .bind(command.receipt_id)
    .fetch_optional(&mut *conn)
    .await?
    {
        if existing_stable_request_id != command.stable_seal_request_id {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        let refs = sqlx::query_as::<_, (Uuid, Uuid, Uuid, String)>(
            r#"SELECT input.id,input.denominator_id,input.denominator_item_id,input.input_key
                 FROM capability_execution_receipt_inputs input
                WHERE input.receipt_id=$1 AND input.execution_authority_id=$2
                ORDER BY input.input_key"#,
        )
        .bind(command.receipt_id)
        .bind(authority.id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|row| CapabilityReceiptInputRef {
            receipt_id: command.receipt_id,
            receipt_input_id: row.0,
            denominator_id: row.1,
            denominator_item_id: row.2,
            logical_input_key: row.3,
        })
        .collect();
        return Ok(refs);
    }

    let expected_by_id = expected
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut refs = Vec::with_capacity(command.inputs.len());
    for input in &command.inputs {
        let input_key = expected_by_id
            .get(&input.denominator_item_id)
            .ok_or_else(|| fail(ENUMERATION_MANIFEST_DRIFT))?;
        let (attempt_state, landing_state, observation_state, coverage_extent, gap_reason) =
            match &input.outcome {
                EnumerationTerminalInputOutcome::Found => {
                    ("succeeded", "committed", "found", "complete", "none")
                }
                EnumerationTerminalInputOutcome::CheckedEmpty => {
                    ("succeeded", "committed", "no_match", "complete", "none")
                }
                EnumerationTerminalInputOutcome::NotApplicable => (
                    "succeeded",
                    "committed",
                    "not_applicable",
                    "complete",
                    "none",
                ),
                EnumerationTerminalInputOutcome::UnresolvedExhausted {
                    coverage_gap_reason,
                } => {
                    if !matches!(
                        coverage_gap_reason.as_str(),
                        "transport"
                            | "tool_failure"
                            | "parser_reject"
                            | "budget_exhausted"
                            | "unsupported"
                            | "policy_blocked"
                            | "source_unavailable"
                    ) {
                        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
                    }
                    (
                        "exhausted",
                        "failed",
                        "indeterminate",
                        "none",
                        coverage_gap_reason.as_str(),
                    )
                }
            };
        let input_id = Uuid::new_v5(&command.receipt_id, input_key.as_bytes());
        sqlx::query(
            r#"INSERT INTO capability_execution_receipt_inputs(
                   id,receipt_id,denominator_id,denominator_item_id,execution_authority_id,
                   input_key,attempt_state,landing_state,observation_state,coverage_extent,
                   coverage_gap_reason)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(input_id)
        .bind(command.receipt_id)
        .bind(denominator_id)
        .bind(input.denominator_item_id)
        .bind(authority.id)
        .bind(input_key)
        .bind(attempt_state)
        .bind(landing_state)
        .bind(observation_state)
        .bind(coverage_extent)
        .bind(gap_reason)
        .execute(&mut *conn)
        .await?;
        let mut seen_evidence = std::collections::BTreeSet::new();
        for (ordinal, evidence) in input.evidence_authorities.iter().enumerate() {
            if !matches!(
                evidence.role.as_str(),
                "discovery" | "resolution" | "parameter"
            ) || !seen_evidence.insert(evidence.id)
            {
                return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
            }
            let exact_evidence: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                       SELECT 1 FROM tool_truth_evidence_authorities
                        WHERE id=$1 AND execution_authority_id=$2 AND authority_hash=$3
                   )"#,
            )
            .bind(evidence.id)
            .bind(authority.id)
            .bind(&evidence.authority_hash)
            .fetch_one(&mut *conn)
            .await?;
            if !exact_evidence {
                return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
            }
            let member_hash = sha256_json(&serde_json::json!({
                "input_id": input_id,
                "evidence_authority_id": evidence.id,
                "authority_hash": evidence.authority_hash,
                "role": evidence.role,
            }))?;
            sqlx::query(
                r#"INSERT INTO capability_execution_input_evidence_members(
                       id,input_id,receipt_id,denominator_item_id,execution_authority_id,
                       evidence_authority_id,ordinal,member_hash)
                   VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
            )
            .bind(Uuid::new_v5(&input_id, evidence.id.as_bytes()))
            .bind(input_id)
            .bind(command.receipt_id)
            .bind(input.denominator_item_id)
            .bind(authority.id)
            .bind(evidence.id)
            .bind(i32::try_from(ordinal).map_err(|_| fail(ENUMERATION_MANIFEST_DRIFT))?)
            .bind(member_hash)
            .execute(&mut *conn)
            .await?;
        }
        sqlx::query(
            "UPDATE capability_execution_receipt_inputs SET sealed_at=statement_timestamp() WHERE id=$1",
        )
        .bind(input_id)
        .execute(&mut *conn)
        .await?;
        refs.push(CapabilityReceiptInputRef {
            receipt_id: command.receipt_id,
            receipt_input_id: input_id,
            denominator_id,
            denominator_item_id: input.denominator_item_id,
            logical_input_key: input_key.clone(),
        });
    }
    sqlx::query(
        r#"INSERT INTO enumeration_receipt_input_census_seals(
               id,stable_seal_request_id,receipt_id,denominator_id,
               execution_authority_id,input_count,input_set_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7)"#,
    )
    .bind(Uuid::new_v5(
        &command.stable_seal_request_id,
        b"enumeration-terminal-input-census-v1",
    ))
    .bind(command.stable_seal_request_id)
    .bind(command.receipt_id)
    .bind(denominator_id)
    .bind(authority.id)
    .bind(i64::try_from(refs.len()).map_err(|_| fail(ENUMERATION_MANIFEST_DRIFT))?)
    .bind(&authority.authority_hash)
    .execute(&mut *conn)
    .await?;
    refs.sort_by(|left, right| left.logical_input_key.cmp(&right.logical_input_key));
    Ok(refs)
}

async fn lock_contract(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
) -> Result<FrozenEnumerationContract> {
    // Every entity writer and the final lane seal share this transaction lock.
    // Row locks alone cannot exclude a concurrent phantom descriptor,
    // occurrence or evidence row after the exact-set hash is computed.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("enumeration-authority:{}", authority.id))
        .execute(&mut *conn)
        .await?;
    let row = sqlx::query_as::<_, FrozenEnumerationContract>(
        r#"SELECT o.enumeration_analysis_contract,o.tool_truth_contract
             FROM operation_state o
             JOIN tool_truth_execution_authorities a
               ON a.operation_id=o.operation_id
            WHERE a.id=$1 AND a.operation_id=$2 AND a.project_scope_id=$3
              AND a.project_path_at_freeze=$4 AND a.scope_snapshot_id=$5
              AND a.organization_id=$6 AND a.stage_execution_id=$7
              AND a.authority_hash=$8
            FOR SHARE OF o,a"#,
    )
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(&authority.authority_hash)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
    match row.enumeration_analysis_contract.as_str() {
        "legacy_v1" => return Err(fail(ENUMERATION_V2_WRITER_DISABLED)),
        "agent_team_v2_shadow"
            if !matches!(row.tool_truth_contract.as_str(), "shadow_v1" | "receipt_v1") =>
        {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
        "agent_team_v2" if row.tool_truth_contract != "receipt_v1" => {
            return Err(fail(ENUMERATION_RECEIPT_V1_REQUIRED));
        }
        "agent_team_v2_shadow" | "agent_team_v2" => {}
        _ => return Err(fail(ENUMERATION_AUTHORITY_MISMATCH)),
    }
    Ok(row)
}

/// Serialize every Browser/JsApi/Parameter/Resolution/Coverage writer for one
/// exact Enumeration subject. Lane authorities are intentionally distinct, so
/// an authority-only advisory lock cannot prevent a Coverage snapshot racing a
/// late producer/projection write.
#[allow(clippy::too_many_arguments)]
pub async fn lock_enumeration_subject_identity(
    conn: &mut PgConnection,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    target_id: Uuid,
    exact_origin: &str,
) -> Result<()> {
    if operation_id.is_nil()
        || organization_id.is_nil()
        || stage_execution_id.is_nil()
        || stage_run_unit_id.is_nil()
        || target_id.is_nil()
        || !url_is_sanitized(exact_origin)
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let subject_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1
                 FROM operation_state operation
                 JOIN stage_run_units unit
                   ON unit.operation_id=operation.operation_id
                  AND unit.id=$4
                  AND unit.stage_execution_id=$3
                  AND unit.organization_id=$2
                  AND unit.stage_kind='enumeration'
                 JOIN operation_org_scope_snapshots snapshot
                   ON snapshot.id=unit.scope_snapshot_id
                  AND snapshot.operation_id=unit.operation_id
                  AND snapshot.project_scope_id=operation.project_scope_id
                 JOIN targets target
                   ON target.id=$5 AND target.organization_id=$2
                  AND target.project_path=snapshot.project_path_at_freeze
                 JOIN web_origins origin
                   ON origin.organization_id=$2
                  AND origin.project_path=snapshot.project_path_at_freeze
                  AND origin.origin=$6
                WHERE operation.operation_id=$1
                  AND operation.current_stage='enumeration'
                  AND operation.enumeration_analysis_contract IN (
                      'agent_team_v2_shadow','agent_team_v2'
                  )
                  AND snapshot.sealed_at IS NOT NULL
           )"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(target_id)
    .bind(exact_origin)
    .fetch_one(&mut *conn)
    .await?;
    if !subject_exists {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "enumeration-subject:{operation_id}:{organization_id}:{stage_execution_id}:{stage_run_unit_id}:{target_id}:{exact_origin}"
        ))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub async fn lock_enumeration_lane_subject(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    target_id: Uuid,
    exact_origin: &str,
) -> Result<Uuid> {
    lock_contract(conn, authority).await?;
    if target_id.is_nil() || !url_is_sanitized(exact_origin) {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let stage_run_unit_id: Uuid = sqlx::query_scalar(
        r#"SELECT authority.stage_run_unit_id
              FROM tool_truth_execution_authorities authority
              JOIN enumeration_worker_authority_roots root
                ON root.worker_execution_authority_id=authority.id
              JOIN web_origins origin
                ON origin.organization_id=authority.organization_id
               AND origin.project_path=authority.project_path_at_freeze
               AND origin.origin=$3
             WHERE authority.id=$1 AND authority.operation_id=$4
               AND authority.organization_id=$5
               AND authority.stage_execution_id=$6
               AND authority.stage_run_unit_id IS NOT NULL
               AND enumeration_worker_root_has_exact_origin(authority.id,$2,origin.id)
             FOR SHARE OF authority,root,origin"#,
    )
    .bind(authority.id)
    .bind(target_id)
    .bind(exact_origin)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "enumeration-subject:{}:{}:{}:{}:{}:{}",
            authority.operation_id,
            authority.organization_id,
            authority.stage_execution_id,
            stage_run_unit_id,
            target_id,
            exact_origin
        ))
        .execute(&mut *conn)
        .await?;
    Ok(stage_run_unit_id)
}

/// Normalize already-booked producer audit rows beneath the exact worker/tool
/// execution authority. The audit rows themselves were created while the tool
/// task-local fence was active; the database trigger revalidates that immutable
/// producer envelope before accepting either binding.
pub async fn bind_enumeration_evidence_authorities(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    audit_ids: &[i64],
    role: &str,
) -> Result<Vec<EvidenceAuthorityRef>> {
    lock_contract(conn, authority).await?;
    if !enumeration_evidence_role_is_valid(role) || audit_ids.is_empty() {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let mut audit_ids = audit_ids.to_vec();
    audit_ids.sort_unstable();
    audit_ids.dedup();
    if audit_ids.iter().any(|audit_id| *audit_id <= 0) {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }

    let placeholder_hash = sha256_json(&serde_json::json!({"server_recomputes": true}))?;
    let mut refs = Vec::with_capacity(audit_ids.len());
    for audit_id in audit_ids {
        let classifications = sqlx::query_as::<_, (i64,)>(
            r#"SELECT id FROM evidence_classifications
                WHERE evidence_audit_id=$1 AND valid_to IS NULL
                ORDER BY id FOR SHARE"#,
        )
        .bind(audit_id)
        .fetch_all(&mut *conn)
        .await?;
        if classifications.len() != 1 {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
        let classification_id = classifications[0].0;
        let binding_id = Uuid::new_v5(
            &authority.id,
            format!("enumeration-evidence-production:{audit_id}").as_bytes(),
        );
        sqlx::query(
            r#"INSERT INTO tool_truth_evidence_production_bindings(
                   id,execution_authority_id,operation_id,project_scope_id,
                   project_path_at_freeze,scope_snapshot_id,organization_id,
                   stage_execution_id,stage_kind,execution_authority_hash,
                   evidence_audit_id,evidence_classification_id,production_binding_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,'enumeration',$9,$10,$11,$12)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(binding_id)
        .bind(authority.id)
        .bind(authority.operation_id)
        .bind(authority.project_scope_id)
        .bind(&authority.project_path_at_freeze)
        .bind(authority.scope_snapshot_id)
        .bind(authority.organization_id)
        .bind(authority.stage_execution_id)
        .bind(&authority.authority_hash)
        .bind(audit_id)
        .bind(classification_id)
        .bind(&placeholder_hash)
        .execute(&mut *conn)
        .await?;
        let exact_binding: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM tool_truth_evidence_production_bindings
                    WHERE id=$1 AND execution_authority_id=$2
                      AND evidence_audit_id=$3 AND evidence_classification_id=$4
               )"#,
        )
        .bind(binding_id)
        .bind(authority.id)
        .bind(audit_id)
        .bind(classification_id)
        .fetch_one(&mut *conn)
        .await?;
        if !exact_binding {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }

        let evidence_id = Uuid::new_v5(
            &authority.id,
            format!("enumeration-evidence-authority:{audit_id}").as_bytes(),
        );
        sqlx::query(
            r#"INSERT INTO tool_truth_evidence_authorities(
                   id,production_binding_id,execution_authority_id,operation_id,
                   project_scope_id,project_path_at_freeze,scope_snapshot_id,
                   organization_id,stage_execution_id,stage_kind,execution_authority_hash,
                   evidence_audit_id,evidence_classification_id,audit_row_hash,
                   classification_row_hash,evidence_chain_hash,authority_hash)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'enumeration',$10,$11,$12,$13,$13,$13,$13)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(evidence_id)
        .bind(binding_id)
        .bind(authority.id)
        .bind(authority.operation_id)
        .bind(authority.project_scope_id)
        .bind(&authority.project_path_at_freeze)
        .bind(authority.scope_snapshot_id)
        .bind(authority.organization_id)
        .bind(authority.stage_execution_id)
        .bind(&authority.authority_hash)
        .bind(audit_id)
        .bind(classification_id)
        .bind(&placeholder_hash)
        .execute(&mut *conn)
        .await?;
        let authority_hash: Option<String> = sqlx::query_scalar(
            r#"SELECT authority_hash FROM tool_truth_evidence_authorities
                WHERE id=$1 AND production_binding_id=$2 AND execution_authority_id=$3
                  AND evidence_audit_id=$4 AND evidence_classification_id=$5"#,
        )
        .bind(evidence_id)
        .bind(binding_id)
        .bind(authority.id)
        .bind(audit_id)
        .bind(classification_id)
        .fetch_optional(&mut *conn)
        .await?;
        refs.push(EvidenceAuthorityRef {
            id: evidence_id,
            authority_hash: authority_hash.ok_or_else(|| fail(ENUMERATION_MANIFEST_DRIFT))?,
            role: role.to_string(),
        });
    }
    Ok(refs)
}

fn enumeration_evidence_role_is_valid(role: &str) -> bool {
    matches!(role, "discovery" | "resolution" | "parameter" | "coverage")
}

fn url_is_sanitized(url: &str) -> bool {
    let trimmed = url.trim();
    !trimmed.is_empty()
        && !trimmed.contains('?')
        && !trimmed.contains('#')
        && !trimmed
            .split_once("://")
            .is_some_and(|(_, authority_and_path)| {
                authority_and_path
                    .split('/')
                    .next()
                    .is_some_and(|authority| authority.contains('@'))
            })
}

fn display_url_is_value_free(url: &str) -> bool {
    let Some((base, query)) = url.split_once('?') else {
        return url_is_sanitized(url);
    };
    !query.is_empty()
        && !query.contains('=')
        && !query.contains('#')
        && url_is_sanitized(base)
        && query.split('&').all(|name| {
            !name.is_empty()
                && name.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "_.~-".contains(character)
                })
        })
}

const VALUE_FREE_METADATA_KEYS: &[&str] = &[
    "kind",
    "name",
    "type",
    "value_type",
    "location",
    "requirement",
    "required",
    "confidence",
    "source_anchor",
    "source_anchor_id",
    "source_anchor_ids",
    "applies_to",
    "base_kind",
    "base_url",
    "url",
    "method",
    "protocol",
    "status",
    "reason_code",
    "reason",
    "start_byte",
    "end_byte",
    "start_line",
    "start_column",
    "end_line",
    "end_column",
    "artifact_id",
    "artifact_sha256",
    "ordinal",
    "shape_hash",
    "schema_hash",
    "length",
    "body_length",
    "header_count",
    "field_count",
    "redacted",
    "present",
    "fields",
    "field_names",
    "properties",
    "items",
    "source_urls",
    "discovered_from",
    "document_bases",
    "duplicate_of",
    "chunk_name",
    "source_map_status",
    "capture_kind",
    "compatibility_version",
    "policy_version",
    "schema_version",
    "step",
    "selected",
    "selected_url",
    "receiver",
    "binding_id",
    "base_path",
    "resolved_path",
    "disposition",
    "candidate_id",
    "source_file",
    "callee",
    "client_kind",
    "content_type",
    "query",
    "body",
    "form",
    "header",
    "graphql_variables",
    "path",
    "request_id_hash",
    "initiator_status",
    "has_body",
    "facts",
    "document_base",
    "html_base",
    "app_base",
    "router_base",
    "client_base",
    "bundler_base",
    "candidates",
];

fn json_is_value_free(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().all(|(key, value)| {
            VALUE_FREE_METADATA_KEYS.contains(&key.as_str()) && json_is_value_free(value)
        }),
        serde_json::Value::Array(values) => values.iter().all(json_is_value_free),
        serde_json::Value::String(value) => {
            let lower = value.to_ascii_lowercase();
            value.len() <= 4_096
                && !value.chars().any(char::is_control)
                && ![
                    "authorization:",
                    "authorization=",
                    "cookie:",
                    "cookie=",
                    "password=",
                    "secret=",
                    "token=",
                    "api_key=",
                    "api-key=",
                    "bearer ",
                ]
                .iter()
                .any(|needle| lower.contains(needle))
                && !value.split_once("://").is_some_and(|(_, rest)| {
                    rest.split('/').next().is_some_and(|v| v.contains('@'))
                })
                && (!value.contains("://") || (!value.contains('?') && !value.contains('#')))
        }
        _ => true,
    }
}

fn json_object_has_only_keys(value: &serde_json::Value, allowed: &[&str]) -> bool {
    value.as_object().is_some_and(|map| {
        map.keys().all(|key| allowed.contains(&key.as_str())) && json_is_value_free(value)
    })
}

fn json_urls_are_sanitized(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) if value.starts_with('/') || value.contains("://") => {
            url_is_sanitized(value)
        }
        serde_json::Value::Array(values) => values.iter().all(json_urls_are_sanitized),
        serde_json::Value::Object(map) => map.values().all(json_urls_are_sanitized),
        _ => true,
    }
}

fn text_is_sanitized_anchor(value: &str) -> bool {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    !value.is_empty()
        && value.len() <= 2_048
        && !value
            .chars()
            .any(|character| matches!(character, '\n' | '\r' | '\0'))
        && ![
            "authorization:",
            "cookie:",
            "password=",
            "secret=",
            "token=",
            "api_key=",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
}

async fn lock_scoped_ids(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    table: &'static str,
    mut ids: Vec<Uuid>,
) -> Result<()> {
    ids.sort_unstable();
    ids.dedup();
    let sql = format!(
        "SELECT id FROM {table} WHERE id=$1 AND organization_id=$2 AND project_path=$3 FOR SHARE"
    );
    for id in ids {
        let locked: Option<Uuid> = sqlx::query_scalar(&sql)
            .bind(id)
            .bind(authority.organization_id)
            .bind(&authority.project_path_at_freeze)
            .fetch_optional(&mut *conn)
            .await?;
        if locked.is_none() {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
    }
    Ok(())
}

/// Persist a denominator-bound JS analysis descriptor. Completion is later
/// attached through the one-shot receipt-input CAS; there is no domain status.
pub async fn persist_js_analysis_descriptor(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    input: &CapabilityReceiptInputRef,
    draft: &JsAnalysisDescriptorWrite,
) -> Result<Uuid> {
    lock_contract(conn, authority).await?;
    if !url_is_sanitized(&draft.manifest_url)
        || !url_is_sanitized(&draft.page_url)
        || draft
            .document_url
            .as_deref()
            .is_some_and(|url| !url_is_sanitized(url))
        || !json_object_has_only_keys(
            &draft.descriptor_metadata,
            &[
                "source_urls",
                "discovered_from",
                "document_bases",
                "duplicate_of",
                "chunk_name",
                "source_map_status",
                "capture_kind",
                "compatibility_version",
            ],
        )
    {
        return Err(fail(ENUMERATION_VALUE_BEARING_METADATA));
    }
    let id: Option<Uuid> = sqlx::query_scalar(
        r#"INSERT INTO enumeration_js_analysis_items(
               id,stable_descriptor_request_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,denominator_id,denominator_item_id,manifest_url,page_url,
               document_url,chunk_ordinal,source_map_url,script_sha256,descriptor_metadata)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
           ON CONFLICT(execution_authority_id,stable_descriptor_request_id) DO NOTHING
           RETURNING id"#,
    )
    .bind(draft.id)
    .bind(draft.stable_descriptor_request_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(input.denominator_id)
    .bind(input.denominator_item_id)
    .bind(&draft.manifest_url)
    .bind(&draft.page_url)
    .bind(&draft.document_url)
    .bind(draft.chunk_ordinal)
    .bind(&draft.source_map_url)
    .bind(&draft.script_sha256)
    .bind(&draft.descriptor_metadata)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = id {
        return Ok(id);
    }
    let existing_id: Uuid = sqlx::query_scalar(
        r#"SELECT id FROM enumeration_js_analysis_items
            WHERE execution_authority_id=$1 AND stable_descriptor_request_id=$2
              AND denominator_id=$3 AND denominator_item_id=$4
              AND manifest_url=$5 AND page_url=$6
              AND document_url IS NOT DISTINCT FROM $7
              AND chunk_ordinal=$8 AND source_map_url IS NOT DISTINCT FROM $9
              AND script_sha256 IS NOT DISTINCT FROM $10 AND descriptor_metadata=$11"#,
    )
    .bind(authority.id)
    .bind(draft.stable_descriptor_request_id)
    .bind(input.denominator_id)
    .bind(input.denominator_item_id)
    .bind(&draft.manifest_url)
    .bind(&draft.page_url)
    .bind(&draft.document_url)
    .bind(draft.chunk_ordinal)
    .bind(&draft.source_map_url)
    .bind(&draft.script_sha256)
    .bind(&draft.descriptor_metadata)
    .fetch_one(&mut *conn)
    .await?;
    Ok(existing_id)
}

/// Bind the generic terminal receipt exactly once. The DB trigger is the CAS
/// authority and rejects every later update/delete.
pub async fn bind_js_analysis_terminal_receipt(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    descriptor_id: Uuid,
    input: &CapabilityReceiptInputRef,
) -> Result<()> {
    lock_contract(conn, authority).await?;
    let result = sqlx::query(
        r#"UPDATE enumeration_js_analysis_items
              SET terminal_receipt_id=$3,terminal_receipt_input_id=$4,
                  terminal_bound_at=statement_timestamp(),row_version=1
            WHERE id=$1 AND execution_authority_id=$2
              AND denominator_id=$5 AND denominator_item_id=$6
              AND terminal_receipt_input_id IS NULL AND row_version=0"#,
    )
    .bind(descriptor_id)
    .bind(authority.id)
    .bind(input.receipt_id)
    .bind(input.receipt_input_id)
    .bind(input.denominator_id)
    .bind(input.denominator_item_id)
    .execute(&mut *conn)
    .await?;
    if result.rows_affected() != 1 {
        let exact_replay: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM enumeration_js_analysis_items
                    WHERE id=$1 AND execution_authority_id=$2
                      AND denominator_id=$3 AND denominator_item_id=$4
                      AND terminal_receipt_id=$5 AND terminal_receipt_input_id=$6
                      AND terminal_bound_at IS NOT NULL AND row_version=1
               )"#,
        )
        .bind(descriptor_id)
        .bind(authority.id)
        .bind(input.denominator_id)
        .bind(input.denominator_item_id)
        .bind(input.receipt_id)
        .bind(input.receipt_input_id)
        .fetch_one(&mut *conn)
        .await?;
        if !exact_replay {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
    }
    Ok(())
}

pub async fn persist_candidate_descriptor(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    input: &CapabilityReceiptInputRef,
    draft: &CandidateDescriptorWrite,
) -> Result<Uuid> {
    lock_contract(conn, authority).await?;
    if !text_is_sanitized_anchor(&draft.source_anchor)
        || !text_is_sanitized_anchor(&draft.resolution_input)
    {
        return Err(fail(ENUMERATION_VALUE_BEARING_METADATA));
    }
    let id: Option<Uuid> = sqlx::query_scalar(
        r#"INSERT INTO enumeration_endpoint_candidate_inputs(
               id,stable_candidate_request_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,denominator_id,denominator_item_id,terminal_receipt_id,
               terminal_receipt_input_id,js_analysis_item_id,logical_input_key,source_anchor,
               callsite_fingerprint,event_fingerprint,duplicate_ordinal,resolution_input)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)
           ON CONFLICT(execution_authority_id,stable_candidate_request_id) DO NOTHING
           RETURNING id"#,
    )
    .bind(draft.id)
    .bind(draft.stable_candidate_request_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(input.denominator_id)
    .bind(input.denominator_item_id)
    .bind(input.receipt_id)
    .bind(input.receipt_input_id)
    .bind(draft.js_analysis_item_id)
    .bind(&input.logical_input_key)
    .bind(&draft.source_anchor)
    .bind(&draft.callsite_fingerprint)
    .bind(&draft.event_fingerprint)
    .bind(draft.duplicate_ordinal)
    .bind(&draft.resolution_input)
    .fetch_optional(&mut *conn)
    .await?;
    let candidate_id = if let Some(id) = id {
        id
    } else {
        let exact_replay: Option<Uuid> = sqlx::query_scalar(
            r#"SELECT id FROM enumeration_endpoint_candidate_inputs
            WHERE id=$1 AND execution_authority_id=$2 AND stable_candidate_request_id=$3
              AND denominator_id=$4 AND denominator_item_id=$5
              AND terminal_receipt_id=$6 AND terminal_receipt_input_id=$7
              AND js_analysis_item_id IS NOT DISTINCT FROM $8
              AND logical_input_key=$9 AND source_anchor=$10 AND callsite_fingerprint=$11
              AND event_fingerprint=$12 AND duplicate_ordinal=$13 AND resolution_input=$14"#,
        )
        .bind(draft.id)
        .bind(authority.id)
        .bind(draft.stable_candidate_request_id)
        .bind(input.denominator_id)
        .bind(input.denominator_item_id)
        .bind(input.receipt_id)
        .bind(input.receipt_input_id)
        .bind(draft.js_analysis_item_id)
        .bind(&input.logical_input_key)
        .bind(&draft.source_anchor)
        .bind(&draft.callsite_fingerprint)
        .bind(&draft.event_fingerprint)
        .bind(draft.duplicate_ordinal)
        .bind(&draft.resolution_input)
        .fetch_optional(&mut *conn)
        .await?;
        exact_replay.ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?
    };

    let capture_event_id: Option<Uuid> = sqlx::query_scalar(
        r#"INSERT INTO enumeration_endpoint_candidate_capture_events(
               capture_event_id,candidate_input_id,execution_authority_id,
               capture_attempt_ordinal,event_fingerprint,captured_at)
           VALUES($1,$2,$3,$4,$5,$6)
           ON CONFLICT DO NOTHING
           RETURNING capture_event_id"#,
    )
    .bind(draft.capture_event_id)
    .bind(candidate_id)
    .bind(authority.id)
    .bind(draft.capture_attempt_ordinal)
    .bind(&draft.event_fingerprint)
    .bind(draft.captured_at)
    .fetch_optional(&mut *conn)
    .await?;
    if capture_event_id.is_none() {
        let exact_replay: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM enumeration_endpoint_candidate_capture_events
                    WHERE capture_event_id=$1 AND candidate_input_id=$2
                      AND execution_authority_id=$3 AND capture_attempt_ordinal=$4
                      AND event_fingerprint=$5 AND captured_at=$6
               )"#,
        )
        .bind(draft.capture_event_id)
        .bind(candidate_id)
        .bind(authority.id)
        .bind(draft.capture_attempt_ordinal)
        .bind(&draft.event_fingerprint)
        .bind(draft.captured_at)
        .fetch_one(&mut *conn)
        .await?;
        if !exact_replay {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
    }
    Ok(candidate_id)
}

/// Persist one immutable endpoint occurrence and its normalized evidence refs.
/// This function never inserts or updates `api_endpoints`.
pub async fn persist_endpoint_occurrence(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    candidate: &CapabilityReceiptInputRef,
    draft: &EndpointOccurrenceWrite,
    evidence_authorities: &[EvidenceAuthorityRef],
) -> Result<PersistedEndpointOccurrence> {
    lock_contract(conn, authority).await?;
    for url in [
        Some(draft.source_url.as_str()),
        draft.document_url.as_deref(),
        draft.script_url.as_deref(),
        draft.initiator_url.as_deref(),
        draft.canonical_request_url.as_deref(),
        draft.runtime_sample_url.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !url_is_sanitized(url) {
            return Err(fail(ENUMERATION_VALUE_BEARING_METADATA));
        }
    }
    if draft
        .display_url
        .as_deref()
        .is_some_and(|url| !display_url_is_value_free(url))
        || !json_object_has_only_keys(
            &draft.source_span,
            &[
                "start_byte",
                "end_byte",
                "start_line",
                "start_column",
                "end_line",
                "end_column",
                "artifact_id",
                "artifact_sha256",
                "status",
                "reason_code",
            ],
        )
        || !json_object_has_only_keys(
            &draft.resolution_base_facts,
            &[
                "facts",
                "document_base",
                "html_base",
                "app_base",
                "router_base",
                "client_base",
                "bundler_base",
                "candidates",
                "selected_url",
            ],
        )
        || !json_is_value_free(&draft.resolution_candidates)
        || !json_is_value_free(&draft.resolution_chain)
        || !json_object_has_only_keys(
            &draft.request_schema,
            &[
                "query",
                "body",
                "form",
                "header",
                "path",
                "graphql_variables",
                "content_type",
                "schema_version",
                "fields",
            ],
        )
        || !json_object_has_only_keys(
            &draft.redaction_metadata,
            &[
                "redacted",
                "body_length",
                "header_count",
                "field_count",
                "schema_hash",
                "policy_version",
            ],
        )
        || !json_urls_are_sanitized(&draft.resolution_candidates)
        || draft
            .raw_expression
            .as_deref()
            .is_some_and(|value| !text_is_sanitized_anchor(value))
        || draft
            .receiver_kind
            .as_deref()
            .is_some_and(|value| !text_is_sanitized_anchor(value))
        || draft
            .websocket_subprotocol
            .as_deref()
            .is_some_and(|value| !text_is_sanitized_anchor(value))
        || !text_is_sanitized_anchor(&draft.resolution_reason)
    {
        return Err(fail(ENUMERATION_VALUE_BEARING_METADATA));
    }

    let candidate_is_exact: Option<i32> = sqlx::query_scalar(
        r#"SELECT 1 FROM enumeration_endpoint_candidate_inputs candidate
            JOIN capability_execution_receipt_inputs receipt_input
              ON receipt_input.id=candidate.terminal_receipt_input_id
             AND receipt_input.receipt_id=candidate.terminal_receipt_id
             AND receipt_input.denominator_item_id=candidate.denominator_item_id
             AND receipt_input.execution_authority_id=candidate.execution_authority_id
            JOIN enumeration_endpoint_candidate_capture_events capture
              ON capture.capture_event_id=$8
             AND capture.candidate_input_id=candidate.id
             AND capture.execution_authority_id=candidate.execution_authority_id
           WHERE candidate.id=$1 AND candidate.execution_authority_id=$2
             AND candidate.terminal_receipt_id=$3 AND candidate.terminal_receipt_input_id=$4
             AND candidate.denominator_id=$5 AND candidate.denominator_item_id=$6
             AND candidate.logical_input_key=$7
             AND receipt_input.sealed_at IS NOT NULL
           FOR SHARE OF candidate,receipt_input"#,
    )
    .bind(draft.candidate_input_id)
    .bind(authority.id)
    .bind(candidate.receipt_id)
    .bind(candidate.receipt_input_id)
    .bind(candidate.denominator_id)
    .bind(candidate.denominator_item_id)
    .bind(&candidate.logical_input_key)
    .bind(draft.capture_event_id)
    .fetch_optional(&mut *conn)
    .await?;
    if candidate_is_exact.is_none() {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    for evidence in evidence_authorities {
        if !matches!(evidence.role.as_str(), "discovery" | "resolution") {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
        let evidence_execution_authority_id: Option<Uuid> = sqlx::query_scalar(
            r#"SELECT evidence.execution_authority_id
                  FROM tool_truth_evidence_authorities evidence
                  JOIN tool_truth_execution_authorities evidence_authority
                    ON evidence_authority.id=evidence.execution_authority_id
                  JOIN tool_truth_execution_authorities occurrence_authority
                    ON occurrence_authority.id=$2
                 WHERE evidence.id=$1 AND evidence.authority_hash=$3
                   AND evidence_authority.operation_id=occurrence_authority.operation_id
                   AND evidence_authority.project_scope_id=occurrence_authority.project_scope_id
                   AND evidence_authority.scope_snapshot_id=occurrence_authority.scope_snapshot_id
                   AND evidence_authority.organization_id=occurrence_authority.organization_id
                   AND evidence_authority.stage_execution_id=occurrence_authority.stage_execution_id
                   AND evidence_authority.stage_run_unit_id
                       IS NOT DISTINCT FROM occurrence_authority.stage_run_unit_id
                   AND ($4<>'discovery' OR evidence.execution_authority_id=$2)
                 FOR SHARE OF evidence,evidence_authority,occurrence_authority"#,
        )
        .bind(evidence.id)
        .bind(authority.id)
        .bind(&evidence.authority_hash)
        .bind(&evidence.role)
        .fetch_optional(&mut *conn)
        .await?;
        if evidence_execution_authority_id.is_none() {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
    }

    let mut target_ids = vec![draft.source_target_id];
    target_ids.extend(draft.resolved_target_id);
    lock_scoped_ids(conn, authority, "targets", target_ids).await?;
    let mut origin_ids = vec![draft.source_web_origin_id];
    origin_ids.extend(draft.resolved_web_origin_id);
    lock_scoped_ids(conn, authority, "web_origins", origin_ids).await?;

    let row = sqlx::query_as::<_, PersistedEndpointOccurrence>(
        r#"INSERT INTO enumeration_endpoint_occurrences(
               id,stable_occurrence_request_id,candidate_input_id,initial_capture_event_id,
               execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,terminal_receipt_id,terminal_receipt_input_id,
               source_target_id,source_web_origin_id,resolved_target_id,resolved_web_origin_id,
               parent_occurrence_id,source_url,document_url,script_url,script_sha256,source_span,
               initiator_url,initiator_status,initiator_line,initiator_column,cdp_request_id_hash,
               protocol,method,graphql_operation_name,websocket_subprotocol,raw_expression,receiver_kind,
               observation_kind,inference_level,resolution_status,scope_decision,candidate_classification,
               canonical_request_url,display_url,resolution_reason,resolution_base_facts,
               resolution_candidates,resolution_chain,route_kind,route_template,request_sent,
               request_schema,redaction_metadata,request_body_length,runtime_sample_url,observed_at)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
                  $19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,
                  $36,$37,$38,$39,$40,$41,$42,$43,$44,$45,$46,$47,$48,$49,$50,$51,$52,$53)
           ON CONFLICT(execution_authority_id,stable_occurrence_request_id) DO NOTHING
           RETURNING id,stable_occurrence_request_id,candidate_input_id,execution_authority_id,operation_id,
                     source_target_id,source_web_origin_id,resolved_target_id,resolved_web_origin_id,
                     protocol,method,observation_kind,inference_level,resolution_status,
                     scope_decision,candidate_classification,route_kind,promotion_eligible,observed_at"#,
    )
    .bind(draft.id)
    .bind(draft.stable_occurrence_request_id)
    .bind(draft.candidate_input_id)
    .bind(draft.capture_event_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(candidate.receipt_id)
    .bind(candidate.receipt_input_id)
    .bind(draft.source_target_id)
    .bind(draft.source_web_origin_id)
    .bind(draft.resolved_target_id)
    .bind(draft.resolved_web_origin_id)
    .bind(draft.parent_occurrence_id)
    .bind(&draft.source_url)
    .bind(&draft.document_url)
    .bind(&draft.script_url)
    .bind(&draft.script_sha256)
    .bind(&draft.source_span)
    .bind(&draft.initiator_url)
    .bind(&draft.initiator_status)
    .bind(draft.initiator_line)
    .bind(draft.initiator_column)
    .bind(&draft.cdp_request_id_hash)
    .bind(&draft.protocol)
    .bind(draft.method.to_ascii_uppercase())
    .bind(&draft.graphql_operation_name)
    .bind(&draft.websocket_subprotocol)
    .bind(&draft.raw_expression)
    .bind(&draft.receiver_kind)
    .bind(&draft.observation_kind)
    .bind(&draft.inference_level)
    .bind(&draft.resolution_status)
    .bind(&draft.scope_decision)
    .bind(&draft.candidate_classification)
    .bind(&draft.canonical_request_url)
    .bind(&draft.display_url)
    .bind(&draft.resolution_reason)
    .bind(&draft.resolution_base_facts)
    .bind(&draft.resolution_candidates)
    .bind(&draft.resolution_chain)
    .bind(&draft.route_kind)
    .bind(&draft.route_template)
    .bind(draft.request_sent)
    .bind(&draft.request_schema)
    .bind(&draft.redaction_metadata)
    .bind(draft.request_body_length)
    .bind(&draft.runtime_sample_url)
    .bind(draft.observed_at)
    .fetch_optional(&mut *conn)
    .await?;

    let row = if let Some(row) = row {
        row
    } else {
        sqlx::query_as::<_, PersistedEndpointOccurrence>(
            r#"SELECT id,stable_occurrence_request_id,candidate_input_id,
                      execution_authority_id,operation_id,source_target_id,
                      source_web_origin_id,resolved_target_id,resolved_web_origin_id,
                      protocol,method,observation_kind,inference_level,resolution_status,
                      scope_decision,candidate_classification,route_kind,promotion_eligible,observed_at
                 FROM enumeration_endpoint_occurrences
                WHERE id=$1 AND stable_occurrence_request_id=$2
                  AND candidate_input_id=$3 AND execution_authority_id=$4
                  AND source_target_id=$5 AND source_web_origin_id=$6
                  AND resolved_target_id IS NOT DISTINCT FROM $7
                  AND resolved_web_origin_id IS NOT DISTINCT FROM $8
                  AND protocol=$9 AND method=$10 AND observation_kind=$11
                  AND inference_level=$12 AND resolution_status=$13
                  AND scope_decision=$14 AND candidate_classification=$15
                  AND route_kind=$16 AND observed_at=$17"#,
        )
        .bind(draft.id)
        .bind(draft.stable_occurrence_request_id)
        .bind(draft.candidate_input_id)
        .bind(authority.id)
        .bind(draft.source_target_id)
        .bind(draft.source_web_origin_id)
        .bind(draft.resolved_target_id)
        .bind(draft.resolved_web_origin_id)
        .bind(&draft.protocol)
        .bind(draft.method.to_ascii_uppercase())
        .bind(&draft.observation_kind)
        .bind(&draft.inference_level)
        .bind(&draft.resolution_status)
        .bind(&draft.scope_decision)
        .bind(&draft.candidate_classification)
        .bind(&draft.route_kind)
        .bind(draft.observed_at)
        .fetch_one(&mut *conn)
        .await?
    };

    sqlx::query(
        r#"INSERT INTO enumeration_endpoint_occurrence_capture_events(
               occurrence_id,candidate_input_id,execution_authority_id,capture_event_id)
           VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING"#,
    )
    .bind(row.id)
    .bind(draft.candidate_input_id)
    .bind(authority.id)
    .bind(draft.capture_event_id)
    .execute(&mut *conn)
    .await?;

    for evidence in evidence_authorities {
        let evidence_execution_authority_id: Uuid = sqlx::query_scalar(
            r#"SELECT execution_authority_id FROM tool_truth_evidence_authorities
                WHERE id=$1 AND authority_hash=$2 FOR SHARE"#,
        )
        .bind(evidence.id)
        .bind(&evidence.authority_hash)
        .fetch_one(&mut *conn)
        .await?;
        sqlx::query(
            r#"INSERT INTO enumeration_endpoint_occurrence_evidence(
                   occurrence_id,occurrence_execution_authority_id,
                   evidence_execution_authority_id,operation_id,project_scope_id,
                   scope_snapshot_id,organization_id,tool_truth_evidence_authority_id,
                   authority_hash,evidence_role)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT DO NOTHING"#,
        )
        .bind(row.id)
        .bind(authority.id)
        .bind(evidence_execution_authority_id)
        .bind(authority.operation_id)
        .bind(authority.project_scope_id)
        .bind(authority.scope_snapshot_id)
        .bind(authority.organization_id)
        .bind(evidence.id)
        .bind(&evidence.authority_hash)
        .bind(&evidence.role)
        .execute(&mut *conn)
        .await?;
    }
    Ok(row)
}

pub async fn persist_parameter_assessment(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    input: &CapabilityReceiptInputRef,
    draft: &ParameterAssessmentWrite,
) -> Result<Uuid> {
    lock_contract(conn, authority).await?;
    if !text_is_sanitized_anchor(&draft.reason_code)
        || draft.parameters.iter().any(|parameter| {
            !text_is_sanitized_anchor(&parameter.name)
                || parameter.source_anchor_ids.is_empty()
                || parameter
                    .source_anchor_ids
                    .iter()
                    .any(|anchor| !text_is_sanitized_anchor(anchor))
                || parameter
                    .source_anchor_ids
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
        })
    {
        return Err(fail(ENUMERATION_VALUE_BEARING_METADATA));
    }
    let occurrence_execution_authority_id: Uuid = sqlx::query_scalar(
        r#"SELECT occurrence.execution_authority_id
              FROM enumeration_endpoint_occurrences occurrence
              JOIN tool_truth_execution_authorities discovery_authority
                ON discovery_authority.id=occurrence.execution_authority_id
              JOIN tool_truth_execution_authorities parameter_authority
                ON parameter_authority.id=$2
             WHERE occurrence.id=$1
               AND occurrence.operation_id=$3
               AND occurrence.project_scope_id=$4
               AND occurrence.project_path_at_freeze=$5
               AND occurrence.scope_snapshot_id=$6
               AND occurrence.organization_id=$7
               AND occurrence.stage_execution_id=$8
               AND discovery_authority.stage_run_unit_id IS NOT NULL
               AND discovery_authority.stage_run_unit_id
                   =parameter_authority.stage_run_unit_id
               AND occurrence.execution_authority_id<>parameter_authority.id
               AND enumeration_worker_root_has_exact_origin(
                   parameter_authority.id,
                   occurrence.source_target_id,
                   occurrence.source_web_origin_id
               )
             FOR SHARE OF occurrence,discovery_authority,parameter_authority"#,
    )
    .bind(draft.occurrence_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
    let assessment_id: Option<Uuid> = sqlx::query_scalar(
        r#"INSERT INTO enumeration_endpoint_parameter_assessments(
               id,occurrence_id,occurrence_execution_authority_id,execution_authority_id,
               operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,stage_execution_id,denominator_id,denominator_item_id,
               terminal_receipt_id,terminal_receipt_input_id,parameter_outcome,reason_code)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)
           ON CONFLICT(occurrence_id,denominator_item_id) DO NOTHING RETURNING id"#,
    )
    .bind(draft.id)
    .bind(draft.occurrence_id)
    .bind(occurrence_execution_authority_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(input.denominator_id)
    .bind(input.denominator_item_id)
    .bind(input.receipt_id)
    .bind(input.receipt_input_id)
    .bind(&draft.outcome)
    .bind(&draft.reason_code)
    .fetch_optional(&mut *conn)
    .await?;
    let assessment_id = if let Some(assessment_id) = assessment_id {
        assessment_id
    } else {
        sqlx::query_scalar(
            r#"SELECT id FROM enumeration_endpoint_parameter_assessments
                WHERE id=$1 AND occurrence_id=$2
                  AND occurrence_execution_authority_id=$3 AND execution_authority_id=$4
                  AND operation_id=$5 AND project_scope_id=$6
                  AND project_path_at_freeze=$7 AND scope_snapshot_id=$8
                  AND organization_id=$9 AND stage_execution_id=$10
                  AND denominator_id=$11 AND denominator_item_id=$12
                  AND terminal_receipt_id=$13 AND terminal_receipt_input_id=$14
                  AND parameter_outcome=$15 AND reason_code=$16"#,
        )
        .bind(draft.id)
        .bind(draft.occurrence_id)
        .bind(occurrence_execution_authority_id)
        .bind(authority.id)
        .bind(authority.operation_id)
        .bind(authority.project_scope_id)
        .bind(&authority.project_path_at_freeze)
        .bind(authority.scope_snapshot_id)
        .bind(authority.organization_id)
        .bind(authority.stage_execution_id)
        .bind(input.denominator_id)
        .bind(input.denominator_item_id)
        .bind(input.receipt_id)
        .bind(input.receipt_input_id)
        .bind(&draft.outcome)
        .bind(&draft.reason_code)
        .fetch_one(&mut *conn)
        .await?
    };
    for parameter in &draft.parameters {
        let primary_source_anchor = parameter
            .source_anchor_ids
            .first()
            .ok_or_else(|| fail(ENUMERATION_MANIFEST_DRIFT))?;
        let inserted = sqlx::query(
            r#"INSERT INTO enumeration_endpoint_occurrence_parameters(
                   id,assessment_id,assessment_execution_authority_id,name,location,value_type,
                   requirement,confidence,source_anchor)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
               ON CONFLICT(assessment_id,location,name) DO NOTHING"#,
        )
        .bind(parameter.id)
        .bind(assessment_id)
        .bind(authority.id)
        .bind(&parameter.name)
        .bind(&parameter.location)
        .bind(&parameter.value_type)
        .bind(&parameter.requirement)
        .bind(parameter.confidence)
        .bind(primary_source_anchor)
        .execute(&mut *conn)
        .await?;
        if inserted.rows_affected() == 0 {
            let exact_replay: bool = sqlx::query_scalar(
                r#"SELECT EXISTS(
                       SELECT 1 FROM enumeration_endpoint_occurrence_parameters
                        WHERE id=$1 AND assessment_id=$2
                          AND assessment_execution_authority_id=$3
                          AND name=$4 AND location=$5 AND value_type=$6
                          AND requirement=$7 AND confidence=$8 AND source_anchor=$9
                   )"#,
            )
            .bind(parameter.id)
            .bind(assessment_id)
            .bind(authority.id)
            .bind(&parameter.name)
            .bind(&parameter.location)
            .bind(&parameter.value_type)
            .bind(&parameter.requirement)
            .bind(parameter.confidence)
            .bind(primary_source_anchor)
            .fetch_one(&mut *conn)
            .await?;
            if !exact_replay {
                return Err(fail(ENUMERATION_MANIFEST_DRIFT));
            }
        }
        for (ordinal, source_anchor) in parameter.source_anchor_ids.iter().enumerate() {
            let ordinal = i32::try_from(ordinal).map_err(|_| fail(ENUMERATION_MANIFEST_DRIFT))?;
            let inserted = sqlx::query(
                r#"INSERT INTO enumeration_endpoint_occurrence_parameter_source_anchors(
                       parameter_id,assessment_id,assessment_execution_authority_id,
                       anchor_ordinal,source_anchor)
                   VALUES($1,$2,$3,$4,$5)
                   ON CONFLICT(parameter_id,anchor_ordinal) DO NOTHING"#,
            )
            .bind(parameter.id)
            .bind(assessment_id)
            .bind(authority.id)
            .bind(ordinal)
            .bind(source_anchor)
            .execute(&mut *conn)
            .await?;
            if inserted.rows_affected() == 0 {
                let exact_replay: bool = sqlx::query_scalar(
                    r#"SELECT EXISTS(
                           SELECT 1
                             FROM enumeration_endpoint_occurrence_parameter_source_anchors
                            WHERE parameter_id=$1 AND assessment_id=$2
                              AND assessment_execution_authority_id=$3
                              AND anchor_ordinal=$4 AND source_anchor=$5
                       )"#,
                )
                .bind(parameter.id)
                .bind(assessment_id)
                .bind(authority.id)
                .bind(ordinal)
                .bind(source_anchor)
                .fetch_one(&mut *conn)
                .await?;
                if !exact_replay {
                    return Err(fail(ENUMERATION_MANIFEST_DRIFT));
                }
            }
        }
    }
    Ok(assessment_id)
}

/// Attach normalized parameter evidence to an assessment before its creating
/// transaction commits. The deferred assessment-shape trigger rejects a
/// terminal assessment that has parameters but no same-authority evidence.
pub async fn bind_parameter_assessment_evidence(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    assessment_id: Uuid,
    evidence_authorities: &[EvidenceAuthorityRef],
) -> Result<()> {
    lock_contract(conn, authority).await?;
    if assessment_id.is_nil() || evidence_authorities.is_empty() {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let mut seen = std::collections::BTreeSet::new();
    if evidence_authorities
        .iter()
        .any(|evidence| evidence.role != "parameter" || !seen.insert(evidence.id))
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let (occurrence_id, occurrence_execution_authority_id): (Uuid, Uuid) = sqlx::query_as(
        r#"SELECT occurrence_id,occurrence_execution_authority_id
              FROM enumeration_endpoint_parameter_assessments
             WHERE id=$1 AND execution_authority_id=$2
               AND operation_id=$3 AND project_scope_id=$4
               AND project_path_at_freeze=$5 AND scope_snapshot_id=$6
               AND organization_id=$7 AND stage_execution_id=$8
             FOR SHARE"#,
    )
    .bind(assessment_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;

    for evidence in evidence_authorities {
        let exact_evidence: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM tool_truth_evidence_authorities
                    WHERE id=$1 AND execution_authority_id=$2 AND authority_hash=$3
               )"#,
        )
        .bind(evidence.id)
        .bind(authority.id)
        .bind(&evidence.authority_hash)
        .fetch_one(&mut *conn)
        .await?;
        if !exact_evidence {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
        sqlx::query(
            r#"INSERT INTO enumeration_endpoint_occurrence_evidence(
                   occurrence_id,occurrence_execution_authority_id,
                   evidence_execution_authority_id,operation_id,project_scope_id,
                   scope_snapshot_id,organization_id,tool_truth_evidence_authority_id,
                   authority_hash,evidence_role,parameter_assessment_id,
                   parameter_assessment_execution_authority_id)
               VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'parameter',$10,$3)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(occurrence_id)
        .bind(occurrence_execution_authority_id)
        .bind(authority.id)
        .bind(authority.operation_id)
        .bind(authority.project_scope_id)
        .bind(authority.scope_snapshot_id)
        .bind(authority.organization_id)
        .bind(evidence.id)
        .bind(&evidence.authority_hash)
        .bind(assessment_id)
        .execute(&mut *conn)
        .await?;
        let exact_link: bool = sqlx::query_scalar(
            r#"SELECT EXISTS(
                   SELECT 1 FROM enumeration_endpoint_occurrence_evidence
                    WHERE occurrence_id=$1 AND occurrence_execution_authority_id=$2
                      AND evidence_execution_authority_id=$3
                      AND tool_truth_evidence_authority_id=$4 AND authority_hash=$5
                      AND evidence_role='parameter' AND parameter_assessment_id=$6
                      AND parameter_assessment_execution_authority_id=$3
               )"#,
        )
        .bind(occurrence_id)
        .bind(occurrence_execution_authority_id)
        .bind(authority.id)
        .bind(evidence.id)
        .bind(&evidence.authority_hash)
        .bind(assessment_id)
        .fetch_one(&mut *conn)
        .await?;
        if !exact_link {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
    }
    Ok(())
}

/// Seal the exact terminal occurrence set for one candidate. Disposition,
/// count and hash are always recomputed by the database trigger; the caller
/// supplies only stable request identity and an optional exhausted resolution
/// receipt input when the derived disposition requires one.
pub async fn seal_enumeration_candidate_closure(
    pool: &PgPool,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationCandidateClosure,
) -> Result<EnumerationCandidateClosureRow> {
    let mut tx = pool.begin().await?;
    let result =
        seal_enumeration_candidate_closure_in_connection(&mut tx, authority, command).await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn seal_enumeration_candidate_closure_in_connection(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationCandidateClosure,
) -> Result<EnumerationCandidateClosureRow> {
    if command.stable_closure_request_id.is_nil() || command.candidate_input_id.is_nil() {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    lock_contract(conn, authority).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "enumeration-candidate-closure:{}:{}",
            authority.id, command.candidate_input_id
        ))
        .execute(&mut *conn)
        .await?;
    let (terminal_receipt_id, terminal_receipt_input_id): (Uuid, Uuid) = sqlx::query_as(
        r#"SELECT terminal_receipt_id,terminal_receipt_input_id
              FROM enumeration_endpoint_candidate_inputs
             WHERE id=$1 AND execution_authority_id=$2
             FOR UPDATE"#,
    )
    .bind(command.candidate_input_id)
    .bind(authority.id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
    let resolution = if let Some(input) = &command.resolution_terminal_input {
        if input.receipt_id.is_nil()
            || input.receipt_input_id.is_nil()
            || input.denominator_id.is_nil()
            || input.denominator_item_id.is_nil()
            || input.logical_input_key.trim().is_empty()
        {
            return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
        }
        let resolution_execution_authority_id: Uuid = sqlx::query_scalar(
            r#"SELECT execution_authority_id
                  FROM capability_execution_receipt_inputs
                 WHERE id=$1 AND receipt_id=$2 AND denominator_id=$3
                   AND denominator_item_id=$4 AND input_key=$5
                   AND sealed_at IS NOT NULL
                 FOR SHARE"#,
        )
        .bind(input.receipt_input_id)
        .bind(input.receipt_id)
        .bind(input.denominator_id)
        .bind(input.denominator_item_id)
        .bind(&input.logical_input_key)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))?;
        Some((
            resolution_execution_authority_id,
            input.receipt_id,
            input.receipt_input_id,
        ))
    } else {
        None
    };
    let closure_id = Uuid::new_v5(
        &command.stable_closure_request_id,
        b"enumeration-candidate-closure-v1",
    );
    let existing = sqlx::query_as::<_, EnumerationCandidateClosureRow>(
        r#"SELECT id,stable_closure_request_id,candidate_input_id,execution_authority_id,
                  terminal_receipt_id,terminal_receipt_input_id,
                  resolution_execution_authority_id,resolution_terminal_receipt_id,
                  resolution_terminal_receipt_input_id,terminal_disposition,
                  occurrence_count,occurrence_set_hash
             FROM enumeration_endpoint_candidate_closure_receipts
            WHERE stable_closure_request_id=$1 OR candidate_input_id=$2
            ORDER BY id FOR SHARE"#,
    )
    .bind(command.stable_closure_request_id)
    .bind(command.candidate_input_id)
    .fetch_all(&mut *conn)
    .await?;
    if !existing.is_empty() {
        if existing.len() != 1 {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        let row = existing
            .into_iter()
            .next()
            .expect("one checked closure row");
        let exact_resolution = resolution
            .map(|value| (Some(value.0), Some(value.1), Some(value.2)))
            .unwrap_or((None, None, None));
        if row.id != closure_id
            || row.stable_closure_request_id != command.stable_closure_request_id
            || row.candidate_input_id != command.candidate_input_id
            || row.execution_authority_id != authority.id
            || row.terminal_receipt_id != terminal_receipt_id
            || row.terminal_receipt_input_id != terminal_receipt_input_id
            || row.resolution_execution_authority_id != exact_resolution.0
            || row.resolution_terminal_receipt_id != exact_resolution.1
            || row.resolution_terminal_receipt_input_id != exact_resolution.2
        {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        return Ok(row);
    }
    let resolution = resolution
        .map(|value| (Some(value.0), Some(value.1), Some(value.2)))
        .unwrap_or((None, None, None));
    let row = sqlx::query_as::<_, EnumerationCandidateClosureRow>(
        r#"INSERT INTO enumeration_endpoint_candidate_closure_receipts(
               id,stable_closure_request_id,candidate_input_id,execution_authority_id,
               terminal_receipt_id,terminal_receipt_input_id,
               resolution_execution_authority_id,resolution_terminal_receipt_id,
               resolution_terminal_receipt_input_id,terminal_disposition,
               occurrence_count,occurrence_set_hash)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'not_applicable',1,$10)
           RETURNING id,stable_closure_request_id,candidate_input_id,execution_authority_id,
                     terminal_receipt_id,terminal_receipt_input_id,
                     resolution_execution_authority_id,resolution_terminal_receipt_id,
                     resolution_terminal_receipt_input_id,terminal_disposition,
                     occurrence_count,occurrence_set_hash"#,
    )
    .bind(closure_id)
    .bind(command.stable_closure_request_id)
    .bind(command.candidate_input_id)
    .bind(authority.id)
    .bind(terminal_receipt_id)
    .bind(terminal_receipt_input_id)
    .bind(resolution.0)
    .bind(resolution.1)
    .bind(resolution.2)
    .bind(sha256_json(
        &serde_json::json!({"server_recomputes": true}),
    )?)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// Seal the exact candidate-member closure for one derived denominator. The
/// DB requires one candidate and one candidate-closure receipt per member and
/// computes the final member count/hash while all children are share-locked.
pub async fn seal_enumeration_candidate_denominator_closure(
    pool: &PgPool,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationCandidateDenominatorClosure,
) -> Result<EnumerationCandidateDenominatorClosureRow> {
    let mut tx = pool.begin().await?;
    let result =
        seal_enumeration_candidate_denominator_closure_in_connection(&mut tx, authority, command)
            .await?;
    tx.commit().await?;
    Ok(result)
}

pub async fn seal_enumeration_candidate_denominator_closure_in_connection(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationCandidateDenominatorClosure,
) -> Result<EnumerationCandidateDenominatorClosureRow> {
    if command.stable_closure_request_id.is_nil() || command.denominator_id.is_nil() {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    lock_contract(conn, authority).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "enumeration-candidate-denominator-closure:{}:{}",
            authority.id, command.denominator_id
        ))
        .execute(&mut *conn)
        .await?;
    let denominator_exists: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM coverage_denominators
                WHERE id=$1 AND execution_authority_id=$2
                  AND denominator_kind='derived_child' AND sealed_at IS NOT NULL
                  AND enumeration_denominator_has_worker_root(id,execution_authority_id)
           )"#,
    )
    .bind(command.denominator_id)
    .bind(authority.id)
    .fetch_one(&mut *conn)
    .await?;
    if !denominator_exists {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let closure_id = Uuid::new_v5(
        &command.stable_closure_request_id,
        b"enumeration-candidate-denominator-closure-v1",
    );
    let existing = sqlx::query_as::<_, EnumerationCandidateDenominatorClosureRow>(
        r#"SELECT id,stable_closure_request_id,denominator_id,execution_authority_id,
                  member_count,member_set_hash
             FROM enumeration_endpoint_candidate_denominator_closure_receipts
            WHERE stable_closure_request_id=$1 OR denominator_id=$2
            ORDER BY id FOR SHARE"#,
    )
    .bind(command.stable_closure_request_id)
    .bind(command.denominator_id)
    .fetch_all(&mut *conn)
    .await?;
    if !existing.is_empty() {
        if existing.len() != 1 {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        let row = existing
            .into_iter()
            .next()
            .expect("one checked closure row");
        if row.id != closure_id
            || row.stable_closure_request_id != command.stable_closure_request_id
            || row.denominator_id != command.denominator_id
            || row.execution_authority_id != authority.id
        {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        return Ok(row);
    }
    let row = sqlx::query_as::<_, EnumerationCandidateDenominatorClosureRow>(
        r#"INSERT INTO enumeration_endpoint_candidate_denominator_closure_receipts(
               id,stable_closure_request_id,denominator_id,execution_authority_id,
               member_count,member_set_hash)
           VALUES($1,$2,$3,$4,0,$5)
           RETURNING id,stable_closure_request_id,denominator_id,execution_authority_id,
                     member_count,member_set_hash"#,
    )
    .bind(closure_id)
    .bind(command.stable_closure_request_id)
    .bind(command.denominator_id)
    .bind(authority.id)
    .bind(&authority.authority_hash)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row)
}

/// Deterministic non-vacuous reducer used by Enumeration Gate integration.
/// A caller must additionally require [`EnumerationTerminalCoverage::is_complete_non_vacuous`].
pub async fn reduce_enumeration_terminal_coverage(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
) -> Result<EnumerationTerminalCoverage> {
    lock_contract(conn, authority).await?;
    Ok(
        sqlx::query_as::<_, EnumerationTerminalCoverage>(ENUMERATION_TERMINAL_COVERAGE_REDUCER_SQL)
            .bind(authority.id)
            .fetch_one(&mut *conn)
            .await?,
    )
}

/// Production-only group reducer and compatibility projector.
pub async fn project_endpoint_groups(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    browser_receipt_id: Uuid,
    js_api_receipt_id: Uuid,
) -> Result<EndpointGroupProjectionSummary> {
    let contract = lock_contract(conn, authority).await?;
    if contract.enumeration_analysis_contract == "agent_team_v2_shadow" {
        return Err(fail(ENUMERATION_SHADOW_PROJECTION_FORBIDDEN));
    }
    if contract.enumeration_analysis_contract != "agent_team_v2"
        || contract.tool_truth_contract != "receipt_v1"
    {
        return Err(fail(ENUMERATION_RECEIPT_V1_REQUIRED));
    }
    let parameter_lane_already_sealed: bool = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM enumeration_lane_commit_receipts
                WHERE execution_authority_id=$1 AND lane='parameter'
           )"#,
    )
    .bind(authority.id)
    .fetch_one(&mut *conn)
    .await?;
    if parameter_lane_already_sealed {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    if browser_receipt_id.is_nil()
        || js_api_receipt_id.is_nil()
        || browser_receipt_id == js_api_receipt_id
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let named_receipts = sqlx::query_as::<_, (String, Uuid, Uuid, String)>(
        r#"SELECT receipt.lane,receipt.execution_authority_id,
                  receipt.target_id,receipt.exact_origin
              FROM enumeration_lane_commit_receipts receipt
              JOIN tool_truth_execution_authorities parameter ON parameter.id=$2
             WHERE receipt.id=ANY($1)
               AND receipt.operation_id=parameter.operation_id
               AND receipt.project_scope_id=parameter.project_scope_id
               AND receipt.project_path_at_freeze=parameter.project_path_at_freeze
               AND receipt.scope_snapshot_id=parameter.scope_snapshot_id
               AND receipt.organization_id=parameter.organization_id
               AND receipt.stage_execution_id=parameter.stage_execution_id
               AND receipt.stage_run_unit_id=parameter.stage_run_unit_id
               AND receipt.missing=0
               AND receipt.lane IN ('browser','js_api')
             ORDER BY receipt.lane
             FOR SHARE OF receipt,parameter"#,
    )
    .bind(vec![browser_receipt_id, js_api_receipt_id])
    .bind(authority.id)
    .fetch_all(&mut *conn)
    .await?;
    if named_receipts.len() != 2
        || named_receipts[0].0 != "browser"
        || named_receipts[1].0 != "js_api"
        || named_receipts[0].2 != named_receipts[1].2
        || named_receipts[0].3 != named_receipts[1].3
        || named_receipts.iter().any(|row| row.1 == authority.id)
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    lock_enumeration_lane_subject(conn, authority, named_receipts[0].2, &named_receipts[0].3)
        .await?;
    let mut occurrence_execution_authority_ids =
        named_receipts.iter().map(|row| row.1).collect::<Vec<_>>();
    occurrence_execution_authority_ids.sort_unstable();

    let groups_created = sqlx::query(
        r#"INSERT INTO enumeration_endpoint_groups(
               id,operation_id,project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,resolved_target_id,resolved_web_origin_id,protocol,method,
               route_kind,route_template,graphql_operation_name,group_identity_hash)
           SELECT uuid_generate_v4(),o.operation_id,o.project_scope_id,o.project_path_at_freeze,
                  o.scope_snapshot_id,o.organization_id,o.resolved_target_id,o.resolved_web_origin_id,
                  o.protocol,upper(o.method),o.route_kind,
                  CASE WHEN o.route_kind='exact' THEN o.canonical_request_url ELSE o.route_template END,
                  COALESCE(o.graphql_operation_name,''),
                  tool_truth_sha256(jsonb_build_object(
                      'operation_id',o.operation_id,'origin',o.resolved_web_origin_id,
                      'protocol',o.protocol,'method',upper(o.method),'route_kind',o.route_kind,
                      'route',CASE WHEN o.route_kind='exact' THEN o.canonical_request_url ELSE o.route_template END,
                      'graphql_operation_name',COALESCE(o.graphql_operation_name,'')
                  )::TEXT)
             FROM enumeration_endpoint_occurrences o
            WHERE o.operation_id=$1 AND o.execution_authority_id=ANY($2)
              AND o.promotion_eligible
              AND o.protocol IN ('http','https','websocket','graphql')
              AND o.route_kind IN ('exact','template')
           ON CONFLICT(operation_id,resolved_web_origin_id,protocol,method,route_kind,
                       route_template,graphql_operation_name) DO NOTHING"#,
    )
    .bind(authority.operation_id)
    .bind(&occurrence_execution_authority_ids)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    let direct_links = sqlx::query(
        r#"INSERT INTO enumeration_endpoint_occurrence_group_links(
               occurrence_id,group_id,operation_id,scope_snapshot_id,organization_id,match_kind)
           SELECT o.id,g.id,o.operation_id,o.scope_snapshot_id,o.organization_id,
                  CASE WHEN o.route_kind='exact' THEN 'exact' ELSE 'unique_template' END
             FROM enumeration_endpoint_occurrences o
             JOIN enumeration_endpoint_groups g
               ON g.operation_id=o.operation_id AND g.resolved_web_origin_id=o.resolved_web_origin_id
              AND g.protocol=o.protocol AND g.method=upper(o.method) AND g.route_kind=o.route_kind
              AND g.route_template=CASE WHEN o.route_kind='exact' THEN o.canonical_request_url ELSE o.route_template END
              AND g.graphql_operation_name=COALESCE(o.graphql_operation_name,'')
            WHERE o.operation_id=$1 AND o.execution_authority_id=ANY($2)
              AND o.promotion_eligible
           ON CONFLICT DO NOTHING"#,
    )
    .bind(authority.operation_id)
    .bind(&occurrence_execution_authority_ids)
    .execute(&mut *conn)
    .await?
    .rows_affected();

    // `unique_template_matches` is intentionally server-derived. Ambiguous
    // runtime samples remain linked only to their exact group and surface the
    // deterministic `route_match_ambiguous` reason to closure diagnostics.
    let unique_template_matches = sqlx::query(
        r#"WITH unique_template_matches AS (
               SELECT runtime.id AS occurrence_id,
                      (array_agg(template_group.id ORDER BY template_group.id))[1] AS group_id
                 FROM enumeration_endpoint_occurrences runtime
                 JOIN enumeration_endpoint_groups template_group
                   ON template_group.operation_id=runtime.operation_id
                  AND template_group.resolved_web_origin_id=runtime.resolved_web_origin_id
                  AND template_group.protocol=runtime.protocol
                  AND template_group.method=upper(runtime.method)
                  AND template_group.route_kind='template'
                  AND enumeration_route_template_matches(
                      template_group.route_template,runtime.runtime_sample_url
                  )
                WHERE runtime.operation_id=$1 AND runtime.execution_authority_id=ANY($2)
                  AND runtime.observation_kind='runtime_request'
                  AND runtime.runtime_sample_url IS NOT NULL
                GROUP BY runtime.id
               HAVING COUNT(*) = 1
           )
           INSERT INTO enumeration_endpoint_occurrence_group_links(
               occurrence_id,group_id,operation_id,scope_snapshot_id,organization_id,match_kind)
           SELECT o.id,m.group_id,o.operation_id,o.scope_snapshot_id,o.organization_id,'unique_template'
             FROM unique_template_matches m
             JOIN enumeration_endpoint_occurrences o ON o.id=m.occurrence_id
           ON CONFLICT DO NOTHING"#,
    )
    .bind(authority.operation_id)
    .bind(&occurrence_execution_authority_ids)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let _route_match_ambiguous = "route_match_ambiguous";

    let projectable = sqlx::query_as::<_, ProjectableGroup>(
        r#"SELECT g.id,g.operation_id,g.organization_id,g.resolved_target_id,
                  g.resolved_web_origin_id,g.protocol,g.method,g.route_template,
                  CASE WHEN g.route_kind='exact' THEN g.route_template
                       ELSE COALESCE(
                           min(o.runtime_sample_url) FILTER (WHERE o.runtime_sample_url IS NOT NULL),
                           g.route_template
                       )
                  END AS replay_url
             FROM enumeration_endpoint_groups g
             JOIN enumeration_endpoint_occurrence_group_links l ON l.group_id=g.id
             JOIN enumeration_endpoint_occurrences o ON o.id=l.occurrence_id
            WHERE g.operation_id=$1 AND g.protocol IN ('http','https')
              AND o.execution_authority_id=ANY($2)
            GROUP BY g.id
            ORDER BY g.id"#,
    )
    .bind(authority.operation_id)
    .bind(&occurrence_execution_authority_ids)
    .fetch_all(&mut *conn)
    .await?;

    let mut api_links_created = 0;
    for group in projectable {
        if !matches!(group.protocol.as_str(), "http" | "https") {
            let _websocket_group_not_projectable = "websocket_group_not_projectable";
            continue;
        }
        let parameters = sqlx::query_as::<_, ProjectableParameter>(
            r#"SELECT DISTINCT p.name,p.location,p.value_type,p.requirement
                 FROM enumeration_endpoint_occurrence_group_links l
                 JOIN enumeration_endpoint_parameter_assessments a ON a.occurrence_id=l.occurrence_id
                 JOIN enumeration_endpoint_occurrence_parameters p ON p.assessment_id=a.id
                WHERE l.group_id=$1 AND a.parameter_outcome='found'
                ORDER BY p.location,p.name"#,
        )
        .bind(group.id)
        .fetch_all(&mut *conn)
        .await?;
        let legacy_params = serde_json::Value::Array(
            parameters
                .iter()
                .map(|parameter| {
                    serde_json::json!({
                        "name": parameter.name,
                        "location": parameter.location,
                        "type": parameter.value_type,
                        "required": parameter.requirement == "required",
                    })
                })
                .collect(),
        );
        // `api_endpoints` is a legacy compatibility table whose identity is
        // global to the Target + URL + method. A previous operation may have
        // linked and sealed the same row. Never mutate that shared payload
        // from an authoritative v2 reducer; current parameters and provenance
        // live in the operation-owned observation/parameter graph below.
        let inserted_endpoint_id: Option<Uuid> = sqlx::query_scalar(
            r#"INSERT INTO api_endpoints(
                   target_id,project_path,url,method,path,params,headers,source,risk_level)
               VALUES($1,$2,$3,$4,$5,$6,'{}'::jsonb,'occurrence_v2_aggregate','unknown')
               ON CONFLICT(target_id,url,method) DO NOTHING
               RETURNING id"#,
        )
        .bind(group.resolved_target_id)
        .bind(&authority.project_path_at_freeze)
        .bind(&group.replay_url)
        .bind(&group.method)
        .bind(&group.route_template)
        .bind(&legacy_params)
        .fetch_optional(&mut *conn)
        .await?;
        let (endpoint_id, endpoint_source) = if let Some(endpoint_id) = inserted_endpoint_id {
            (endpoint_id, "occurrence_v2_aggregate".to_string())
        } else {
            sqlx::query_as(
                r#"SELECT id,source FROM api_endpoints
                    WHERE target_id=$1 AND project_path=$2 AND url=$3 AND method=$4
                    FOR UPDATE"#,
            )
            .bind(group.resolved_target_id)
            .bind(&authority.project_path_at_freeze)
            .bind(&group.replay_url)
            .bind(&group.method)
            .fetch_one(&mut *conn)
            .await?
        };
        if endpoint_source != "occurrence_v2_aggregate" {
            // A pre-v2 browser/JS row may be promoted only while it is not yet
            // linked to any operation projection. Once linked, its payload is
            // shared immutable compatibility state; current-operation truth
            // must stay exclusively in the observation graph.
            let promoted: Option<Uuid> = sqlx::query_scalar(
                r#"UPDATE api_endpoints endpoint
                      SET params=(
                              SELECT COALESCE(jsonb_agg(DISTINCT parameter),'[]'::jsonb)
                                FROM jsonb_array_elements(
                                    CASE WHEN jsonb_typeof(endpoint.params)='array'
                                         THEN endpoint.params ELSE '[]'::jsonb END
                                    || $2::jsonb
                                ) AS parameter
                          ),
                          source='occurrence_v2_aggregate',updated_at=NOW()
                    WHERE endpoint.id=$1
                      AND NOT EXISTS(
                          SELECT 1 FROM enumeration_endpoint_group_api_links link
                           WHERE link.endpoint_id=endpoint.id
                      )
                    RETURNING endpoint.id"#,
            )
            .bind(endpoint_id)
            .bind(&legacy_params)
            .fetch_optional(&mut *conn)
            .await?;
            if promoted != Some(endpoint_id) {
                return Err(fail(ENUMERATION_MANIFEST_DRIFT));
            }
        }
        let observation_id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO enumeration_endpoint_observations(
                   operation_id,organization_id,target_id,web_origin_id,endpoint_id,
                   project_path,source)
               VALUES($1,$2,$3,$4,$5,$6,'occurrence_v2_aggregate')
               ON CONFLICT(operation_id,web_origin_id,endpoint_id)
               DO UPDATE SET source='occurrence_v2_aggregate'
               RETURNING id"#,
        )
        .bind(group.operation_id)
        .bind(group.organization_id)
        .bind(group.resolved_target_id)
        .bind(group.resolved_web_origin_id)
        .bind(endpoint_id)
        .bind(&authority.project_path_at_freeze)
        .fetch_one(&mut *conn)
        .await?;
        api_links_created += sqlx::query(
            r#"INSERT INTO enumeration_endpoint_group_api_links(
                   group_id,operation_id,endpoint_id,endpoint_observation_id,projection_source)
               VALUES($1,$2,$3,$4,'occurrence_v2_aggregate') ON CONFLICT DO NOTHING"#,
        )
        .bind(group.id)
        .bind(group.operation_id)
        .bind(endpoint_id)
        .bind(observation_id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
        for parameter in parameters {
            sqlx::query(
                r#"INSERT INTO enumeration_endpoint_parameters(
                       endpoint_observation_id,name,location,value_type,required,source)
                   VALUES($1,$2,$3,$4,$5,'occurrence_v2_aggregate')
                   ON CONFLICT(endpoint_observation_id,location,name) DO UPDATE
                     SET value_type=EXCLUDED.value_type,
                         required=(enumeration_endpoint_parameters.required OR EXCLUDED.required),
                         source='occurrence_v2_aggregate',updated_at=NOW()"#,
            )
            .bind(observation_id)
            .bind(parameter.name)
            .bind(parameter.location)
            .bind(parameter.value_type)
            .bind(parameter.requirement == "required")
            .execute(&mut *conn)
            .await?;
        }
    }

    Ok(EndpointGroupProjectionSummary {
        groups_created,
        occurrence_links_created: direct_links + unique_template_matches,
        api_links_created,
    })
}

/// Closure diagnostic: a sealed candidate denominator member without a
/// terminal occurrence is not `checked_empty` and must block closure.
pub async fn count_candidate_without_terminal_occurrence(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
) -> Result<i64> {
    lock_contract(conn, authority).await?;
    let candidate_without_terminal_occurrence = sqlx::query_scalar(
        r#"SELECT COUNT(*)::BIGINT
             FROM enumeration_endpoint_candidate_inputs c
             LEFT JOIN enumeration_endpoint_occurrences o ON o.candidate_input_id=c.id
            WHERE c.execution_authority_id=$1 AND o.id IS NULL"#,
    )
    .bind(authority.id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(candidate_without_terminal_occurrence)
}

#[allow(dead_code)]
async fn enumeration_projection_census(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
) -> Result<(i64, i64, i64)> {
    lock_contract(conn, authority).await?;
    Ok(sqlx::query_as(
        r#"SELECT
               COUNT(DISTINCT link.group_id)::BIGINT AS group_count,
               COUNT(DISTINCT (link.occurrence_id,link.group_id))::BIGINT AS occurrence_link_count,
               COUNT(DISTINCT api.group_id)::BIGINT AS api_link_count
             FROM enumeration_endpoint_occurrences occurrence
             LEFT JOIN enumeration_endpoint_occurrence_group_links link
               ON link.occurrence_id=occurrence.id
             LEFT JOIN enumeration_endpoint_group_api_links api
               ON api.group_id=link.group_id AND api.operation_id=occurrence.operation_id
            WHERE occurrence.execution_authority_id=$1"#,
    )
    .bind(authority.id)
    .fetch_one(&mut *conn)
    .await?)
}

const ENUMERATION_RESOLUTION_CLOSEOUT_COLUMNS: &str =
    "id,stable_closeout_request_id,execution_authority_id,operation_id,organization_id,stage_execution_id,stage_run_unit_id,assigned_work_item_id,worker_run_id,source_tool_call_id,worker_attempt_epoch,lease_token,parent_occurrence_id,producer_lane_receipt_id,terminal_state,reason_code,suggestion_ids,terminal_receipt_id,terminal_receipt_input_id,evidence_set_sha256,closeout_sha256";

/// Seal the server-observed terminal state of one bounded Resolution work
/// item. Advisory suggestions remain children of this receipt and never mutate
/// or replace the producer-owned occurrence.
pub async fn seal_enumeration_resolution_closeout(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationResolutionCloseout,
) -> Result<(EnumerationResolutionCloseoutRow, bool)> {
    if command.stable_closeout_request_id.is_nil()
        || command.assigned_work_item_id.is_nil()
        || command.parent_occurrence_id.is_nil()
        || command.producer_lane_receipt_id.is_nil()
        || command.terminal_receipt_id.is_nil()
        || command.terminal_receipt_input_id.is_nil()
        || command.worker_fence.worker_run_id.is_nil()
        || command.worker_fence.source_tool_call_id.is_nil()
        || command.worker_fence.lease_token.is_nil()
        || command.worker_fence.worker_attempt_epoch < 0
        || command.reason_code.trim().is_empty()
        || !matches!(
            command.terminal_state.as_str(),
            "advisory_residual" | "budget_exhausted" | "unsupported"
        )
    {
        return Err(fail(ENUMERATION_RESOLUTION_CLOSEOUT_INPUT_INVALID));
    }
    let mut suggestion_ids = command.suggestion_ids.clone();
    suggestion_ids.sort_unstable();
    suggestion_ids.dedup();
    if suggestion_ids.len() != command.suggestion_ids.len()
        || suggestion_ids.iter().any(Uuid::is_nil)
        || (command.terminal_state == "advisory_residual" && suggestion_ids.is_empty())
    {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    let (target_id, exact_origin, producer_execution_authority_id): (Uuid, String, Uuid) =
        sqlx::query_as(
            r#"SELECT producer.target_id,producer.exact_origin,
                      producer.execution_authority_id
                 FROM enumeration_lane_commit_receipts producer
                 JOIN enumeration_endpoint_occurrences occurrence
                   ON occurrence.id=$2
                  AND occurrence.execution_authority_id=producer.execution_authority_id
                WHERE producer.id=$1 AND producer.lane IN ('browser','js_api')
                  AND producer.operation_id=$3 AND producer.organization_id=$4
                  AND producer.stage_execution_id=$5
                  AND occurrence.resolution_status IN ('ambiguous','unresolved')
                  AND occurrence.scope_decision='in_scope'
                  AND occurrence.candidate_classification='endpoint'
                FOR SHARE OF producer,occurrence"#,
        )
        .bind(command.producer_lane_receipt_id)
        .bind(command.parent_occurrence_id)
        .bind(authority.operation_id)
        .bind(authority.organization_id)
        .bind(authority.stage_execution_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| fail(ENUMERATION_RESOLUTION_CLOSEOUT_PRODUCER_MISMATCH))?;
    if producer_execution_authority_id == authority.id {
        return Err(fail(ENUMERATION_RESOLUTION_CLOSEOUT_PRODUCER_MISMATCH));
    }
    let stage_run_unit_id =
        lock_enumeration_lane_subject(conn, authority, target_id, &exact_origin).await?;
    let live_stage_run_unit_id = assert_live_worker_tool_fence(conn, authority)
        .await
        .map_err(|_| fail(ENUMERATION_RESOLUTION_CLOSEOUT_WORKER_FENCE_MISMATCH))?;
    if stage_run_unit_id != live_stage_run_unit_id {
        return Err(fail(ENUMERATION_RESOLUTION_CLOSEOUT_UNIT_MISMATCH));
    }
    let closeout_id = Uuid::new_v5(
        &command.stable_closeout_request_id,
        b"enumeration-resolution-closeout-v1",
    );
    let mut existing = sqlx::query_as::<_, EnumerationResolutionCloseoutRow>(&format!(
        "SELECT {ENUMERATION_RESOLUTION_CLOSEOUT_COLUMNS} FROM enumeration_resolution_closeout_receipts WHERE stable_closeout_request_id=$1 OR parent_occurrence_id=$2 ORDER BY id FOR SHARE"
    ))
    .bind(command.stable_closeout_request_id)
    .bind(command.parent_occurrence_id)
    .fetch_all(&mut *conn)
    .await?;
    if !existing.is_empty() {
        if existing.len() != 1 {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        let existing = existing.remove(0);
        if existing.id != closeout_id
            || existing.stable_closeout_request_id != command.stable_closeout_request_id
            || existing.execution_authority_id != authority.id
            || existing.operation_id != authority.operation_id
            || existing.organization_id != authority.organization_id
            || existing.stage_execution_id != authority.stage_execution_id
            || existing.stage_run_unit_id != stage_run_unit_id
            || existing.assigned_work_item_id != command.assigned_work_item_id
            || existing.worker_run_id != command.worker_fence.worker_run_id
            || existing.source_tool_call_id != command.worker_fence.source_tool_call_id
            || existing.worker_attempt_epoch != command.worker_fence.worker_attempt_epoch
            || existing.lease_token != command.worker_fence.lease_token
            || existing.parent_occurrence_id != command.parent_occurrence_id
            || existing.producer_lane_receipt_id != command.producer_lane_receipt_id
            || existing.terminal_state != command.terminal_state
            || existing.reason_code != command.reason_code
            || existing.suggestion_ids != suggestion_ids
            || existing.terminal_receipt_id != command.terminal_receipt_id
            || existing.terminal_receipt_input_id != command.terminal_receipt_input_id
            || !existing.evidence_set_sha256.starts_with("sha256:")
            || !existing.closeout_sha256.starts_with("sha256:")
        {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        return Ok((existing, true));
    }
    let placeholder_hash = sha256_json(&serde_json::json!({"server_recomputes": true}))?;
    let inserted = sqlx::query(
        r#"INSERT INTO enumeration_resolution_closeout_receipts(
               id,stable_closeout_request_id,execution_authority_id,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,organization_id,
               stage_execution_id,stage_run_unit_id,assigned_work_item_id,worker_run_id,
               source_tool_call_id,worker_attempt_epoch,lease_token,parent_occurrence_id,
               producer_lane_receipt_id,terminal_state,reason_code,suggestion_ids,
               terminal_receipt_id,terminal_receipt_input_id,evidence_set_sha256,closeout_sha256)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,
                  $20,$21,$22,$23,$23)
           ON CONFLICT(stable_closeout_request_id) DO NOTHING"#,
    )
    .bind(closeout_id)
    .bind(command.stable_closeout_request_id)
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(command.assigned_work_item_id)
    .bind(command.worker_fence.worker_run_id)
    .bind(command.worker_fence.source_tool_call_id)
    .bind(command.worker_fence.worker_attempt_epoch)
    .bind(command.worker_fence.lease_token)
    .bind(command.parent_occurrence_id)
    .bind(command.producer_lane_receipt_id)
    .bind(&command.terminal_state)
    .bind(&command.reason_code)
    .bind(&suggestion_ids)
    .bind(command.terminal_receipt_id)
    .bind(command.terminal_receipt_input_id)
    .bind(&placeholder_hash)
    .execute(&mut *conn)
    .await?
    .rows_affected()
        == 1;
    let row = sqlx::query_as::<_, EnumerationResolutionCloseoutRow>(&format!(
        "SELECT {ENUMERATION_RESOLUTION_CLOSEOUT_COLUMNS} FROM enumeration_resolution_closeout_receipts WHERE stable_closeout_request_id=$1 FOR SHARE"
    ))
    .bind(command.stable_closeout_request_id)
    .fetch_one(&mut *conn)
    .await?;
    if row.id != closeout_id {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    Ok((row, !inserted))
}

async fn assert_live_worker_tool_fence(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
) -> Result<Uuid> {
    let stage_run_unit_id: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT a.stage_run_unit_id
              FROM tool_truth_execution_authorities a
              JOIN stage_worker_runs worker ON worker.id=a.worker_run_id
              JOIN tool_calls call ON call.id=a.source_tool_call_id
             WHERE a.id=$1 AND a.operation_id=$2 AND a.organization_id=$3
               AND a.stage_execution_id=$4 AND a.authority_hash=$5
               AND a.execution_owner_kind='worker_tool'
               AND worker.id=a.worker_run_id
               AND worker.stage_run_unit_id=a.stage_run_unit_id
               AND worker.attempt_epoch=a.worker_attempt_epoch
               AND worker.lease_token=a.lease_token
               AND worker.active_tool_call_id=a.source_tool_call_id
               AND worker.status IN ('running','waiting_background')
               AND worker.lease_expires_at>statement_timestamp()
               AND call.worker_run_id=worker.id
               AND call.attempt_epoch=worker.attempt_epoch
               AND call.lease_token=worker.lease_token
               AND call.status IN ('received','running')
             FOR SHARE OF a,worker,call"#,
    )
    .bind(authority.id)
    .bind(authority.operation_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(&authority.authority_hash)
    .fetch_optional(&mut *conn)
    .await?;
    stage_run_unit_id.ok_or_else(|| fail(ENUMERATION_AUTHORITY_MISMATCH))
}

const ENUMERATION_LANE_COMMIT_RECEIPT_COLUMNS: &str =
    "id,stable_commit_request_id,execution_authority_id,lane,operation_id,organization_id,stage_execution_id,stage_run_unit_id,target_id,exact_origin,artifact_sha256,dependency_receipt_ids,evidence_audit_ids,script_denominator_id,candidate_denominator_ids,parameter_denominator_ids,resolution_occurrence_id,resolution_terminal_receipt_id,resolution_terminal_receipt_input_id,terminal_disposition,entity_set_sha256,denominator_set_sha256,receipt_set_sha256,(SELECT seal.closure_graph_sha256 FROM enumeration_lane_closure_graph_seals seal WHERE seal.lane_receipt_id=enumeration_lane_commit_receipts.id) AS closure_graph_sha256,script_count,candidate_count,occurrence_count,parameter_assessment_count,parameter_fact_count,unresolved_count,missing,group_count,occurrence_link_count,api_link_count";

fn lane_receipt_command_is_valid(command: &SealEnumerationLaneCommitReceipt) -> bool {
    !command.stable_commit_request_id.is_nil()
        && matches!(
            command.lane.as_str(),
            "browser" | "js_api" | "parameter" | "resolution" | "coverage"
        )
        && !command.target_id.is_nil()
        && url_is_sanitized(&command.exact_origin)
        && command.artifact_sha256.len() == 71
        && command.artifact_sha256.starts_with("sha256:")
        && command
            .candidate_denominator_ids
            .iter()
            .all(|id| !id.is_nil())
        && command
            .parameter_denominator_ids
            .iter()
            .all(|id| !id.is_nil())
}

fn lane_receipt_matches_command(
    row: &EnumerationLaneCommitReceiptRow,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationLaneCommitReceipt,
    stage_run_unit_id: Option<Uuid>,
    dependency_receipt_ids: &[Uuid],
    evidence_audit_ids: &[i64],
    candidate_denominator_ids: &[Uuid],
    parameter_denominator_ids: &[Uuid],
) -> bool {
    row.execution_authority_id == authority.id
        && row.lane == command.lane
        && row.operation_id == authority.operation_id
        && row.organization_id == authority.organization_id
        && row.stage_execution_id == authority.stage_execution_id
        && stage_run_unit_id.is_none_or(|id| row.stage_run_unit_id == id)
        && row.target_id == command.target_id
        && row.exact_origin == command.exact_origin
        && row.artifact_sha256 == command.artifact_sha256
        && row.dependency_receipt_ids == dependency_receipt_ids
        && row.evidence_audit_ids == evidence_audit_ids
        && row.script_denominator_id == command.script_denominator_id
        && row.candidate_denominator_ids == candidate_denominator_ids
        && row.parameter_denominator_ids == parameter_denominator_ids
        && row.resolution_occurrence_id == command.resolution_occurrence_id
        && row.resolution_terminal_receipt_id == command.resolution_terminal_receipt_id
        && row.resolution_terminal_receipt_input_id == command.resolution_terminal_receipt_input_id
        && row.missing == 0
        && row.script_count >= 0
        && row.candidate_count >= 0
        && row.occurrence_count >= 0
        && row.parameter_assessment_count >= 0
        && row.parameter_fact_count >= 0
        && row.unresolved_count >= 0
        && row.group_count >= 0
        && row.occurrence_link_count >= 0
        && row.api_link_count >= 0
        && matches!(
            row.terminal_disposition.as_str(),
            "found" | "checked_empty" | "terminal_with_residual"
        )
        && row.receipt_set_sha256.starts_with("sha256:")
        && row.entity_set_sha256.starts_with("sha256:")
        && row.denominator_set_sha256.starts_with("sha256:")
}

/// Seal one exact Browser/JSAPI/Parameter/Resolution/Coverage entity census.
/// A completed receipt is returned before checking the live fence, which is
/// the response-loss recovery path.  A first write still requires the exact
/// live worker/tool lease; no caller-supplied counts are accepted because the
/// database trigger derives every census from immutable entity rows.
pub async fn seal_enumeration_lane_commit_receipt(
    conn: &mut PgConnection,
    authority: &ToolTruthExecutionAuthorityRef,
    command: &SealEnumerationLaneCommitReceipt,
) -> Result<(EnumerationLaneCommitReceiptRow, bool)> {
    let contract = lock_contract(conn, authority).await?;
    if contract.enumeration_analysis_contract != "agent_team_v2"
        || !lane_receipt_command_is_valid(command)
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let subject_stage_run_unit_id =
        lock_enumeration_lane_subject(conn, authority, command.target_id, &command.exact_origin)
            .await?;
    let mut dependency_receipt_ids = command.dependency_receipt_ids.clone();
    dependency_receipt_ids.sort_unstable();
    dependency_receipt_ids.dedup();
    let mut evidence_audit_ids = command.evidence_audit_ids.clone();
    evidence_audit_ids.sort_unstable();
    evidence_audit_ids.dedup();
    let mut candidate_denominator_ids = command.candidate_denominator_ids.clone();
    candidate_denominator_ids.sort_unstable();
    candidate_denominator_ids.dedup();
    let mut parameter_denominator_ids = command.parameter_denominator_ids.clone();
    parameter_denominator_ids.sort_unstable();
    parameter_denominator_ids.dedup();
    if dependency_receipt_ids.iter().any(Uuid::is_nil)
        || evidence_audit_ids.is_empty()
        || evidence_audit_ids.iter().any(|id| *id <= 0)
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    if let Some(existing) = sqlx::query_as::<_, EnumerationLaneCommitReceiptRow>(&format!(
        "SELECT {ENUMERATION_LANE_COMMIT_RECEIPT_COLUMNS} FROM enumeration_lane_commit_receipts WHERE stable_commit_request_id=$1 FOR SHARE"
    ))
    .bind(command.stable_commit_request_id)
    .fetch_optional(&mut *conn)
    .await?
    {
        if !lane_receipt_matches_command(
            &existing,
            authority,
            command,
            None,
            &dependency_receipt_ids,
            &evidence_audit_ids,
            &candidate_denominator_ids,
            &parameter_denominator_ids,
        ) {
            return Err(fail(ENUMERATION_MANIFEST_DRIFT));
        }
        return Ok((existing, true));
    }

    let stage_run_unit_id = assert_live_worker_tool_fence(conn, authority).await?;
    if stage_run_unit_id != subject_stage_run_unit_id {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    lock_scoped_ids(conn, authority, "targets", vec![command.target_id]).await?;
    let normalized_evidence_count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::BIGINT FROM tool_truth_evidence_authorities
            WHERE execution_authority_id=$1 AND evidence_audit_id=ANY($2::BIGINT[])"#,
    )
    .bind(authority.id)
    .bind(&evidence_audit_ids)
    .fetch_one(&mut *conn)
    .await?;
    if usize::try_from(normalized_evidence_count).ok() != Some(evidence_audit_ids.len()) {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let receipt_id = Uuid::new_v5(
        &command.stable_commit_request_id,
        format!("enumeration-lane-commit-receipt-v2:{}", command.lane).as_bytes(),
    );
    let placeholder_hash = sha256_json(&serde_json::json!({"server_recomputes": true}))?;
    let inserted = sqlx::query(
        r#"INSERT INTO enumeration_lane_commit_receipts(
               id,stable_commit_request_id,execution_authority_id,lane,operation_id,
               project_scope_id,project_path_at_freeze,scope_snapshot_id,
               organization_id,stage_execution_id,stage_kind,execution_authority_hash,
               stage_run_unit_id,target_id,exact_origin,
               artifact_sha256,dependency_receipt_ids,evidence_audit_ids,
               script_denominator_id,candidate_denominator_ids,parameter_denominator_ids,
               resolution_occurrence_id,resolution_terminal_receipt_id,
               resolution_terminal_receipt_input_id,terminal_disposition,
               entity_set_sha256,denominator_set_sha256,receipt_set_sha256,
               script_count,candidate_count,occurrence_count,parameter_assessment_count,
               parameter_fact_count,unresolved_count,missing,group_count,
               occurrence_link_count,api_link_count)
           VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'enumeration',$11,$12,$13,$14,$15,$16,$17,
                  $18,$19,$20,$21,$22,$23,'checked_empty',$24,$24,$24,
                  0,0,0,0,0,0,0,0,0,0)
           ON CONFLICT(stable_commit_request_id) DO NOTHING"#,
    )
    .bind(receipt_id)
    .bind(command.stable_commit_request_id)
    .bind(authority.id)
    .bind(&command.lane)
    .bind(authority.operation_id)
    .bind(authority.project_scope_id)
    .bind(&authority.project_path_at_freeze)
    .bind(authority.scope_snapshot_id)
    .bind(authority.organization_id)
    .bind(authority.stage_execution_id)
    .bind(&authority.authority_hash)
    .bind(stage_run_unit_id)
    .bind(command.target_id)
    .bind(&command.exact_origin)
    .bind(&command.artifact_sha256)
    .bind(&dependency_receipt_ids)
    .bind(&evidence_audit_ids)
    .bind(command.script_denominator_id)
    .bind(&candidate_denominator_ids)
    .bind(&parameter_denominator_ids)
    .bind(command.resolution_occurrence_id)
    .bind(command.resolution_terminal_receipt_id)
    .bind(command.resolution_terminal_receipt_input_id)
    .bind(&placeholder_hash)
    .execute(&mut *conn)
    .await?
    .rows_affected()
        == 1;
    sqlx::query(
        r#"INSERT INTO enumeration_lane_closure_graph_seals(
               lane_receipt_id,closure_graph_sha256)
           VALUES($1,enumeration_compute_lane_closure_graph_sha256($1))
           ON CONFLICT(lane_receipt_id) DO NOTHING"#,
    )
    .bind(receipt_id)
    .execute(&mut *conn)
    .await?;
    let row = sqlx::query_as::<_, EnumerationLaneCommitReceiptRow>(&format!(
        "SELECT {ENUMERATION_LANE_COMMIT_RECEIPT_COLUMNS} FROM enumeration_lane_commit_receipts WHERE stable_commit_request_id=$1 FOR SHARE"
    ))
    .bind(command.stable_commit_request_id)
    .fetch_one(&mut *conn)
    .await?;
    if row.id != receipt_id
        || !lane_receipt_matches_command(
            &row,
            authority,
            command,
            Some(stage_run_unit_id),
            &dependency_receipt_ids,
            &evidence_audit_ids,
            &candidate_denominator_ids,
            &parameter_denominator_ids,
        )
    {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    Ok((row, !inserted))
}

/// Load one explicitly named immutable lane receipt.  Callers compare the
/// entire typed receipt; this function never selects a latest authority.
pub async fn load_enumeration_lane_commit_receipt(
    conn: &mut PgConnection,
    receipt_id: Uuid,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    target_id: Uuid,
    exact_origin: &str,
    lane: &str,
    execution_authority_id: Uuid,
    expected_receipt_set_sha256: &str,
) -> Result<EnumerationLaneCommitReceiptRow> {
    let rows = sqlx::query_as::<_, EnumerationLaneCommitReceiptRow>(&format!(
        "SELECT {ENUMERATION_LANE_COMMIT_RECEIPT_COLUMNS} FROM enumeration_lane_commit_receipts WHERE id=$1 AND operation_id=$2 AND organization_id=$3 AND stage_execution_id=$4 AND stage_run_unit_id=$5 AND target_id=$6 AND exact_origin=$7 AND lane=$8 AND execution_authority_id=$9 AND receipt_set_sha256=$10 FOR SHARE"
    ))
    .bind(receipt_id)
    .bind(operation_id)
    .bind(organization_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(target_id)
    .bind(exact_origin)
    .bind(lane)
    .bind(execution_authority_id)
    .bind(expected_receipt_set_sha256)
    .fetch_all(&mut *conn)
    .await?;
    if rows.len() != 1 {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let row = rows.into_iter().next().expect("one checked lane receipt");
    if row.missing != 0
        || !row.receipt_set_sha256.starts_with("sha256:")
        || !row.closure_graph_sha256.starts_with("sha256:")
    {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    Ok(row)
}

/// Response-loss recovery lookup.  It intentionally does not require the old
/// worker lease to remain live: callers must compare the complete immutable
/// row (including dependencies, denominators and hashes) before returning it.
/// A mismatched owner tuple is not treated as a cache miss because reusing a
/// stable request id for another subject is manifest drift.
pub async fn load_enumeration_lane_commit_receipt_by_stable_request(
    conn: &mut PgConnection,
    stable_commit_request_id: Uuid,
    lane: &str,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    target_id: Uuid,
    exact_origin: &str,
    artifact_sha256: &str,
) -> Result<Option<EnumerationLaneCommitReceiptRow>> {
    if stable_commit_request_id.is_nil()
        || !matches!(
            lane,
            "browser" | "js_api" | "parameter" | "resolution" | "coverage"
        )
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let existing = sqlx::query_as::<_, EnumerationLaneCommitReceiptRow>(&format!(
        "SELECT {ENUMERATION_LANE_COMMIT_RECEIPT_COLUMNS} FROM enumeration_lane_commit_receipts WHERE stable_commit_request_id=$1 FOR SHARE"
    ))
    .bind(stable_commit_request_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = existing else {
        return Ok(None);
    };
    if row.lane != lane
        || row.operation_id != operation_id
        || row.organization_id != organization_id
        || row.stage_execution_id != stage_execution_id
        || row.stage_run_unit_id != stage_run_unit_id
        || row.target_id != target_id
        || row.exact_origin != exact_origin
        || row.artifact_sha256 != artifact_sha256
        || row.missing != 0
        || !row.receipt_set_sha256.starts_with("sha256:")
        || !row.entity_set_sha256.starts_with("sha256:")
        || !row.denominator_set_sha256.starts_with("sha256:")
    {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    Ok(Some(row))
}

/// Exact-subject recovery for the narrow commit→WorkerOutput crash window.
/// Base lanes are unique per frozen Origin; Resolution is additionally keyed
/// by its immutable parent occurrence. No current/latest ordering is used.
#[allow(clippy::too_many_arguments)]
pub async fn recover_enumeration_lane_commit_receipt(
    conn: &mut PgConnection,
    operation_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    target_id: Uuid,
    exact_origin: &str,
    lane: &str,
    resolution_occurrence_id: Option<Uuid>,
) -> Result<Option<EnumerationLaneCommitReceiptRow>> {
    if operation_id.is_nil()
        || organization_id.is_nil()
        || stage_execution_id.is_nil()
        || stage_run_unit_id.is_nil()
        || target_id.is_nil()
        || !url_is_sanitized(exact_origin)
        || !matches!(
            lane,
            "browser" | "js_api" | "parameter" | "resolution" | "coverage"
        )
        || (lane == "resolution") != resolution_occurrence_id.is_some()
    {
        return Err(fail(ENUMERATION_AUTHORITY_MISMATCH));
    }
    let rows = sqlx::query_as::<_, EnumerationLaneCommitReceiptRow>(&format!(
        "SELECT {ENUMERATION_LANE_COMMIT_RECEIPT_COLUMNS} FROM enumeration_lane_commit_receipts WHERE operation_id=$1 AND organization_id=$2 AND stage_execution_id=$3 AND stage_run_unit_id=$4 AND target_id=$5 AND exact_origin=$6 AND lane=$7 AND resolution_occurrence_id IS NOT DISTINCT FROM $8 ORDER BY id FOR SHARE"
    ))
    .bind(operation_id)
    .bind(organization_id)
    .bind(stage_execution_id)
    .bind(stage_run_unit_id)
    .bind(target_id)
    .bind(exact_origin)
    .bind(lane)
    .bind(resolution_occurrence_id)
    .fetch_all(&mut *conn)
    .await?;
    if rows.len() > 1 {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    if row.missing != 0
        || !row.receipt_set_sha256.starts_with("sha256:")
        || !row.entity_set_sha256.starts_with("sha256:")
        || !row.denominator_set_sha256.starts_with("sha256:")
        || !row.closure_graph_sha256.starts_with("sha256:")
    {
        return Err(fail(ENUMERATION_MANIFEST_DRIFT));
    }
    Ok(Some(row))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_guard_rejects_values_and_credential_urls() {
        assert!(json_is_value_free(&serde_json::json!({"type":"string"})));
        assert!(!json_is_value_free(&serde_json::json!({"value":"secret"})));
        assert!(url_is_sanitized("https://example.test/api/items"));
        assert!(!url_is_sanitized("https://user:pass@example.test/api"));
        assert!(!url_is_sanitized("https://example.test/api?token=x"));
    }

    #[test]
    fn occurrence_writer_does_not_project_canonical_rows() {
        let source = include_str!("enumeration_endpoint_occurrences.rs");
        let writer = source
            .split("pub async fn persist_endpoint_occurrence")
            .nth(1)
            .expect("occurrence writer")
            .split("pub async fn persist_parameter_assessment")
            .next()
            .expect("writer body");
        assert!(!writer.contains("INSERT INTO api_endpoints"));
    }

    #[test]
    fn coverage_is_a_valid_derived_enumeration_evidence_role() {
        assert!(enumeration_evidence_role_is_valid("coverage"));
        assert!(!enumeration_evidence_role_is_valid("reporting"));
    }
}
