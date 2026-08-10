//! Server-owned hierarchical Application Understanding runtime composition.
//!
//! One frozen organization maps to one Unit and one static TeamPlan. Safe,
//! typed projections are analyzed by tool-free child work items; the unique
//! `leader:primary` synthesizer alone may submit the organization Application
//! Model through the deterministic gate.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use golish_agent_kit::harness::application_model_gate::{
    validate_application_model_gate_truth, ApplicationModelAuthorityKind,
    ApplicationModelGateBlock, ApplicationModelGateCode, ApplicationModelGateDisposition,
    ApplicationModelGateSnapshot, ApplicationModelInputDecisionTruth,
    ApplicationModelInputDisposition, ApplicationModelItemTruth, ApplicationModelTruthState,
};
use golish_db::repo::application_models::{
    self, ApplicationModelAuthorityKindRow, ApplicationModelEvidenceRoleRow,
    ApplicationModelInputDecisionSeed, ApplicationModelInputDispositionRow,
    ApplicationModelItemEvidenceSeed, ApplicationModelItemSeed, ApplicationModelManifestInputRow,
    ApplicationModelManifestRow, ApplicationModelStoreError, ApplicationModelTruthStateRow,
    DeriveApplicationModelManifestSeed, LoadApplicationModelGateMaterial,
    LoadStandaloneApplicationModelSubmission, ProposeApplicationModelRevision,
};
use golish_db::repo::runtime_memory_tx;
use golish_db::repo::runtime_memory_tx::RuntimeMemoryTxFence;
use golish_db::repo::stage_deliverable_submissions::{
    self, NewStageDeliverableSubmission, StageDeliverableSubmissionError,
};
use golish_db::repo::tool_calls::{self, RuntimeToolIdentity};
use golish_db::repo::{stage_run_units, stage_teams, stage_worker_runs};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::application_model_gate::{
    finalize_application_model_gate_pass, ApplicationModelFinalizationOutcome,
    ApplicationModelGateError, FinalizeApplicationModelGatePass, FinalizedApplicationModelGatePass,
};
use super::application_understanding_projection::{
    build_application_work_item_projections, load_application_projection_source,
    ProjectedApplicationWorkItem,
};

#[derive(Debug, Clone)]
pub struct RunApplicationUnderstandingUnit {
    pub session_id: Uuid,
    pub fence: RuntimeMemoryTxFence,
    pub expected_unit_row_version: i64,
    pub scope_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationModelProducerInput {
    pub manifest_id: Uuid,
    pub organization_id: Uuid,
    pub inputs: Vec<ApplicationModelManifestInputRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationModelProposalDraft {
    pub structured_model: Value,
    pub decisions: Vec<ApplicationModelInputDecisionSeed>,
    pub items: Vec<ApplicationModelItemSeed>,
}

#[async_trait::async_trait]
pub trait ApplicationModelProposalProducer: Send + Sync {
    async fn produce(
        &self,
        input: ApplicationModelProducerInput,
    ) -> Result<ApplicationModelProposalDraft, ApplicationUnderstandingRuntimeError>;
}

/// Product-active composition root for the formal Application Understanding
/// stage. It owns only a PostgreSQL handle; the one-shot model callback is
/// supplied by the orchestrator and has no tool surface.
pub struct PgApplicationUnderstandingStageRuntime {
    pool: Arc<PgPool>,
}

impl PgApplicationUnderstandingStageRuntime {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self { pool }
    }
}

fn producer_error(code: impl Into<String>) -> ApplicationUnderstandingRuntimeError {
    ApplicationUnderstandingRuntimeError::Producer { code: code.into() }
}

fn classify_application_model_producer_error(error: &anyhow::Error) -> &'static str {
    error
        .downcast_ref::<golish_agent_kit::task_orchestrator::ApplicationModelProducerFailure>()
        .map(|failure| failure.code())
        .unwrap_or("application_model_producer_failed")
}

fn proposal_from_contract(
    proposal: golish_agent_kit::task_orchestrator::ApplicationModelProposalContract,
) -> ApplicationModelProposalDraft {
    let mut decisions = proposal
        .decisions
        .into_iter()
        .map(|decision| {
            let disposition = match decision.disposition {
                golish_agent_kit::task_orchestrator::ApplicationModelInputDispositionContract::Incorporated => ApplicationModelInputDispositionRow::Incorporated,
                golish_agent_kit::task_orchestrator::ApplicationModelInputDispositionContract::Duplicate => ApplicationModelInputDispositionRow::Duplicate,
                golish_agent_kit::task_orchestrator::ApplicationModelInputDispositionContract::NotRelevant => ApplicationModelInputDispositionRow::NotRelevant,
                golish_agent_kit::task_orchestrator::ApplicationModelInputDispositionContract::Unknown => ApplicationModelInputDispositionRow::Unknown,
            };
            let mut item_keys = decision.item_keys;
            item_keys.sort();
            ApplicationModelInputDecisionSeed {
                input_key: decision.input_key,
                disposition,
                item_keys,
                duplicate_input_key: decision.duplicate_input_key,
                reason_code: decision.reason_code,
            }
        })
        .collect::<Vec<_>>();
    decisions.sort_by(|left, right| left.input_key.cmp(&right.input_key));

    let mut items = proposal
        .items
        .into_iter()
        .map(|item| {
            let truth_state = match item.truth_state {
                golish_agent_kit::task_orchestrator::ApplicationModelTruthStateContract::Observed => ApplicationModelTruthStateRow::Observed,
                golish_agent_kit::task_orchestrator::ApplicationModelTruthStateContract::Inferred => ApplicationModelTruthStateRow::Inferred,
                golish_agent_kit::task_orchestrator::ApplicationModelTruthStateContract::Unknown => ApplicationModelTruthStateRow::Unknown,
            };
            let mut source_input_keys = item.source_input_keys;
            source_input_keys.sort();
            let mut referenced_item_keys = item.referenced_item_keys;
            referenced_item_keys.sort();
            let mut evidence = item
                .evidence
                .into_iter()
                .map(|evidence| {
                    let role = match evidence.role {
                        golish_agent_kit::task_orchestrator::ApplicationModelEvidenceRoleContract::Observation => ApplicationModelEvidenceRoleRow::Observation,
                        golish_agent_kit::task_orchestrator::ApplicationModelEvidenceRoleContract::Support => ApplicationModelEvidenceRoleRow::Support,
                    };
                    ApplicationModelItemEvidenceSeed {
                        evidence_id: evidence.evidence_id,
                        role,
                    }
                })
                .collect::<Vec<_>>();
            evidence.sort_by_key(|entry| entry.evidence_id);
            ApplicationModelItemSeed {
                item_key: item.item_key,
                item_kind: item.item_kind,
                truth_state,
                source_input_keys,
                referenced_item_keys,
                payload: item.payload,
                evidence,
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.item_key.cmp(&right.item_key));
    ApplicationModelProposalDraft {
        structured_model: proposal.structured_model,
        decisions,
        items,
    }
}

fn application_team_output_blocks_child_barrier(
    required_work_item_ids: &BTreeSet<Uuid>,
    work_item_id: Uuid,
    business_disposition: &str,
) -> bool {
    required_work_item_ids.contains(&work_item_id) && business_disposition == "blocked"
}

#[allow(dead_code)] // Legacy direct-Unit replay/test adapter; fresh stages use static TeamPlans.
struct ToolFreeApplicationModelProducer<'a> {
    executor: &'a dyn golish_agent_kit::task_orchestrator::ApplicationModelProducer,
}

#[async_trait::async_trait]
impl ApplicationModelProposalProducer for ToolFreeApplicationModelProducer<'_> {
    async fn produce(
        &self,
        input: ApplicationModelProducerInput,
    ) -> Result<ApplicationModelProposalDraft, ApplicationUnderstandingRuntimeError> {
        let contract = self
            .executor
            .produce_application_model(
                golish_agent_kit::task_orchestrator::ApplicationModelProducerInputContract {
                    manifest_id: input.manifest_id,
                    organization_id: input.organization_id,
                    inputs: input
                        .inputs
                        .into_iter()
                        .map(|source| {
                            golish_agent_kit::task_orchestrator::ApplicationModelProducerSourceContract {
                                input_key: source.input_key,
                                input_kind: source.input_kind,
                                source_kind: source.source_kind,
                                source_id: source.source_id,
                                source_version: source.source_version,
                                source_payload: source.source_payload,
                                evidence_ids: source.evidence_ids,
                            }
                        })
                        .collect(),
                },
            )
            .await
            .map_err(|error| {
                producer_error(classify_application_model_producer_error(&error))
            })?;
        Ok(proposal_from_contract(contract))
    }
}

#[derive(Debug)]
pub enum ApplicationUnderstandingRuntimeError {
    Producer { code: String },
    Store(ApplicationModelStoreError),
    Runtime(runtime_memory_tx::RuntimeMemoryStoreError),
    Submission(StageDeliverableSubmissionError),
    Gate(ApplicationModelGateError),
    Database(golish_db::DbError),
    Sqlx(sqlx::Error),
}

impl std::fmt::Display for ApplicationUnderstandingRuntimeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Producer { code } => {
                write!(formatter, "application model producer failed: {code}")
            }
            Self::Store(error) => {
                write!(formatter, "application model persistence failed: {error}")
            }
            Self::Runtime(error) => write!(formatter, "application model runtime failed: {error}"),
            Self::Submission(error) => {
                write!(formatter, "application model submission failed: {error}")
            }
            Self::Gate(error) => write!(formatter, "application model gate failed: {error}"),
            Self::Database(error) => {
                write!(formatter, "application model database failed: {error}")
            }
            Self::Sqlx(error) => write!(formatter, "application model SQL failed: {error}"),
        }
    }
}

impl std::error::Error for ApplicationUnderstandingRuntimeError {}

impl From<ApplicationModelStoreError> for ApplicationUnderstandingRuntimeError {
    fn from(error: ApplicationModelStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<runtime_memory_tx::RuntimeMemoryStoreError> for ApplicationUnderstandingRuntimeError {
    fn from(error: runtime_memory_tx::RuntimeMemoryStoreError) -> Self {
        Self::Runtime(error)
    }
}

impl From<StageDeliverableSubmissionError> for ApplicationUnderstandingRuntimeError {
    fn from(error: StageDeliverableSubmissionError) -> Self {
        Self::Submission(error)
    }
}

impl From<ApplicationModelGateError> for ApplicationUnderstandingRuntimeError {
    fn from(error: ApplicationModelGateError) -> Self {
        Self::Gate(error)
    }
}

impl From<golish_db::DbError> for ApplicationUnderstandingRuntimeError {
    fn from(error: golish_db::DbError) -> Self {
        Self::Database(error)
    }
}

impl From<sqlx::Error> for ApplicationUnderstandingRuntimeError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sqlx(error)
    }
}

#[derive(Debug, Clone)]
pub enum ApplicationUnderstandingRuntimeOutcome {
    Passed(Box<FinalizedApplicationModelGatePass>),
    Blocked(ApplicationModelGateBlock),
}

#[derive(Debug, sqlx::FromRow)]
struct RuntimeOwner {
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    unit_status: String,
    worker_status: String,
    active_tool_call_id: Option<Uuid>,
    lease_live: bool,
    session_id: Uuid,
    unit_generation: i32,
    worker_generation: i32,
    lease_token: Option<Uuid>,
    attempt_epoch: i64,
    checkpoint_version: i64,
}

fn application_understanding_runtime_owner_is_active(owner: &RuntimeOwner) -> bool {
    matches!(
        owner.unit_status.as_str(),
        "queued" | "running" | "gate_blocked"
    ) && owner.worker_status == "running"
        && owner.unit_generation >= 0
        && owner.worker_generation >= 0
}

fn block(
    code: ApplicationModelGateCode,
    disposition: ApplicationModelGateDisposition,
    refs: impl IntoIterator<Item = String>,
) -> ApplicationUnderstandingRuntimeOutcome {
    ApplicationUnderstandingRuntimeOutcome::Blocked(ApplicationModelGateBlock {
        code,
        disposition,
        refs: refs
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    })
}

async fn resolve_runtime_owner(
    pool: &PgPool,
    input: &RunApplicationUnderstandingUnit,
) -> Result<Result<RuntimeOwner, ApplicationUnderstandingRuntimeOutcome>, sqlx::Error> {
    let owner = sqlx::query_as::<_, RuntimeOwner>(
        r#"SELECT unit.scope_snapshot_id,unit.organization_id,
                  unit.status AS unit_status,worker.status AS worker_status,
                  worker.active_tool_call_id,
                  COALESCE(worker.lease_expires_at > NOW(),FALSE) AS lease_live,
                  task.session_id,unit.generation AS unit_generation,
                  worker.worker_generation,worker.lease_token,
                  worker.attempt_epoch,worker.checkpoint_version
             FROM stage_run_units AS unit
             JOIN stage_worker_runs AS worker
               ON worker.stage_run_unit_id=unit.id
              AND worker.operation_id=unit.operation_id
              AND worker.stage_execution_id=unit.stage_execution_id
              AND worker.organization_id=unit.organization_id
             JOIN tasks AS task ON task.id=unit.operation_id
            WHERE unit.id=$1 AND unit.operation_id=$2
              AND unit.stage_execution_id=$3
              AND unit.stage_kind='application_understanding'
              AND worker.id=$4"#,
    )
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.worker_run_id)
    .fetch_optional(pool)
    .await?;
    let Some(owner) = owner else {
        return Ok(Err(block(
            ApplicationModelGateCode::IdentityMismatch,
            ApplicationModelGateDisposition::Hold,
            ["application_understanding_runtime_fence".to_string()],
        )));
    };
    if owner.session_id != input.session_id {
        return Ok(Err(block(
            ApplicationModelGateCode::IdentityMismatch,
            ApplicationModelGateDisposition::Hold,
            ["application_understanding_runtime_session".to_string()],
        )));
    }
    let replaying = owner.unit_status == "passed" && owner.worker_status == "passed";
    let active_state = application_understanding_runtime_owner_is_active(&owner);
    let fence_matches = owner.lease_token == Some(input.fence.lease_token)
        && owner.attempt_epoch == input.fence.attempt_epoch
        && owner.checkpoint_version == input.fence.expected_checkpoint_version
        && owner.lease_live;
    if !replaying && active_state && !fence_matches {
        return Ok(Err(block(
            ApplicationModelGateCode::IdentityMismatch,
            ApplicationModelGateDisposition::Hold,
            ["application_understanding_runtime_fence".to_string()],
        )));
    }
    if !replaying && (!active_state || owner.active_tool_call_id.is_some()) {
        return Ok(Err(block(
            ApplicationModelGateCode::ProducerBarrierOpen,
            ApplicationModelGateDisposition::Hold,
            ["application_understanding_runtime_not_runnable".to_string()],
        )));
    }
    Ok(Ok(owner))
}

async fn runtime_producer_barrier_refs(
    pool: &PgPool,
    input: &RunApplicationUnderstandingUnit,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT ref FROM (
               SELECT 'worker:' || worker.id::TEXT AS ref
                 FROM stage_worker_runs AS worker
                WHERE worker.operation_id=$1
                  AND worker.stage_execution_id=$2
                  AND worker.stage_run_unit_id=$3
                  AND worker.id<>$4
                  AND worker.status NOT IN ('passed','failed','exhausted','superseded')
               UNION ALL
               SELECT 'work_item:' || item.id::TEXT AS ref
                 FROM stage_work_items AS item
                WHERE item.operation_id=$1
                  AND item.stage_execution_id=$2
                  AND item.stage_run_unit_id=$3
                  AND item.id IS DISTINCT FROM (
                        SELECT current_worker.work_item_id
                          FROM stage_worker_runs AS current_worker
                         WHERE current_worker.id=$4
                      )
                  AND item.status NOT IN ('completed','exhausted','superseded')
           ) AS barriers ORDER BY ref"#,
    )
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.worker_run_id)
    .fetch_all(pool)
    .await
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("JSON string serializes"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("JSON key serializes"),
                        canonical_json(&object[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn tagged_json_hash(value: &Value) -> String {
    format!("sha256:{}", sha256_hex(&canonical_json(value)))
}

fn decision_truth(seed: &ApplicationModelInputDecisionSeed) -> ApplicationModelInputDecisionTruth {
    ApplicationModelInputDecisionTruth {
        input_key: seed.input_key.clone(),
        disposition: match seed.disposition {
            ApplicationModelInputDispositionRow::Incorporated => {
                ApplicationModelInputDisposition::Incorporated
            }
            ApplicationModelInputDispositionRow::Duplicate => {
                ApplicationModelInputDisposition::Duplicate
            }
            ApplicationModelInputDispositionRow::NotRelevant => {
                ApplicationModelInputDisposition::NotRelevant
            }
            ApplicationModelInputDispositionRow::Unknown => {
                ApplicationModelInputDisposition::Unknown
            }
        },
        item_keys: seed.item_keys.clone(),
        duplicate_input_key: seed.duplicate_input_key.clone(),
        reason_code: seed.reason_code.clone(),
    }
}

fn item_truth(seed: &ApplicationModelItemSeed) -> ApplicationModelItemTruth {
    ApplicationModelItemTruth {
        item_key: seed.item_key.clone(),
        truth_state: match seed.truth_state {
            ApplicationModelTruthStateRow::Observed => ApplicationModelTruthState::Observed,
            ApplicationModelTruthStateRow::Inferred => ApplicationModelTruthState::Inferred,
            ApplicationModelTruthStateRow::Unknown => ApplicationModelTruthState::Unknown,
        },
        source_input_keys: seed.source_input_keys.clone(),
        evidence_ids: seed
            .evidence
            .iter()
            .map(|evidence| evidence.evidence_id)
            .collect(),
        observed_evidence_ids: seed
            .evidence
            .iter()
            .filter(|evidence| {
                evidence.role == application_models::ApplicationModelEvidenceRoleRow::Observation
            })
            .map(|evidence| evidence.evidence_id)
            .collect(),
        referenced_item_keys: seed.referenced_item_keys.clone(),
    }
}

fn validate_draft(
    manifest: &ApplicationModelManifestRow,
    inputs: &[ApplicationModelManifestInputRow],
    draft: &ApplicationModelProposalDraft,
) -> Result<(), ApplicationModelGateBlock> {
    if application_models::validate_proposal_content_shape(
        &draft.structured_model,
        &draft.decisions,
        &draft.items,
    )
    .is_err()
    {
        return Err(ApplicationModelGateBlock {
            code: ApplicationModelGateCode::SchemaInvalid,
            disposition: ApplicationModelGateDisposition::Rework,
            refs: vec!["proposed_revision_shape_invalid".to_string()],
        });
    }
    let proposed_organization_id = draft
        .structured_model
        .get("organization_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    if proposed_organization_id != Some(manifest.organization_id) {
        return Err(ApplicationModelGateBlock {
            code: ApplicationModelGateCode::IdentityMismatch,
            disposition: ApplicationModelGateDisposition::Rework,
            refs: vec!["proposed_model_organization_mismatch".to_string()],
        });
    }
    let authorized_evidence_ids = inputs
        .iter()
        .flat_map(|input| input.evidence_ids.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let model_hash = tagged_json_hash(&draft.structured_model);
    validate_application_model_gate_truth(&ApplicationModelGateSnapshot {
        authority_kind: ApplicationModelAuthorityKind::Model,
        operation_id: manifest.operation_id,
        scope_snapshot_id: manifest.scope_snapshot_id,
        stage_execution_id: manifest.stage_execution_id,
        stage_run_unit_id: manifest.stage_run_unit_id,
        organization_id: manifest.organization_id,
        manifest_hash: manifest.manifest_hash.clone(),
        expected_manifest_hash: manifest.manifest_hash.clone(),
        schema_version: Some("application_model.v1".to_string()),
        model_hash: Some(model_hash.clone()),
        expected_model_hash: Some(model_hash),
        replay_material_hash: manifest.replay_material_hash.clone(),
        expected_replay_material_hash: manifest.replay_material_hash.clone(),
        manifest_input_keys: inputs.iter().map(|input| input.input_key.clone()).collect(),
        authorized_evidence_ids,
        decisions: draft.decisions.iter().map(decision_truth).collect(),
        items: draft.items.iter().map(item_truth).collect(),
        foreign_reference_keys: Vec::new(),
        forbidden_activity_refs: Vec::new(),
        pending_producer_refs: Vec::new(),
    })
}

fn proposal_content_material(
    manifest: &ApplicationModelManifestRow,
    draft: &ApplicationModelProposalDraft,
) -> Value {
    json!({
        "schema_version": "application_model_proposal_content.v1",
        "manifest_id": manifest.id,
        "structured_model": draft.structured_model,
        "decisions": draft.decisions.iter().map(|decision| json!({
            "input_key": decision.input_key,
            "disposition": decision.disposition.as_str(),
            "item_keys": decision.item_keys,
            "duplicate_input_key": decision.duplicate_input_key,
            "reason_code": decision.reason_code,
        })).collect::<Vec<_>>(),
        "items": draft.items.iter().map(|item| json!({
            "item_key": item.item_key,
            "item_kind": item.item_kind,
            "truth_state": item.truth_state.as_str(),
            "source_input_keys": item.source_input_keys,
            "referenced_item_keys": item.referenced_item_keys,
            "payload": item.payload,
            "evidence": item.evidence.iter().map(|evidence: &ApplicationModelItemEvidenceSeed| json!({
                "evidence_id": evidence.evidence_id,
                "role": evidence.role.as_str(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn proposal_payload(
    manifest: &ApplicationModelManifestRow,
    draft: Option<&ApplicationModelProposalDraft>,
) -> Value {
    let Some(draft) = draft else {
        return json!({
            "stage_id": "application_understanding",
            "stage_run_id": manifest.stage_execution_id,
            "schema_version": 1,
            "manifest_id": manifest.id,
            "authority_kind": "terminal_no_input",
        });
    };
    json!({
        "stage_id": "application_understanding",
        "stage_run_id": manifest.stage_execution_id,
        "schema_version": 1,
        "manifest_id": manifest.id,
        "authority_kind": "model",
        "proposal_material_hash": tagged_json_hash(&proposal_content_material(manifest, draft)),
        "decision_count": draft.decisions.len(),
        "item_count": draft.items.len(),
    })
}

fn legacy_proposal_payload(
    manifest: &ApplicationModelManifestRow,
    draft: &ApplicationModelProposalDraft,
) -> Value {
    let mut payload = proposal_content_material(manifest, draft);
    let object = payload
        .as_object_mut()
        .expect("proposal content material is an object");
    object.remove("schema_version");
    object.insert(
        "stage_id".to_string(),
        Value::String("application_understanding".to_string()),
    );
    object.insert(
        "stage_run_id".to_string(),
        json!(manifest.stage_execution_id),
    );
    object.insert("schema_version".to_string(), json!(1));
    payload
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StandaloneApplicationModelDecisionPayload {
    input_key: String,
    disposition: String,
    item_keys: Vec<String>,
    duplicate_input_key: Option<String>,
    reason_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StandaloneApplicationModelEvidencePayload {
    evidence_id: i64,
    role: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StandaloneApplicationModelItemPayload {
    item_key: String,
    item_kind: String,
    truth_state: String,
    source_input_keys: Vec<String>,
    referenced_item_keys: Vec<String>,
    payload: Value,
    evidence: Vec<StandaloneApplicationModelEvidencePayload>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StandaloneApplicationModelPayload {
    stage_id: String,
    stage_run_id: Uuid,
    schema_version: i32,
    manifest_id: Uuid,
    structured_model: Value,
    decisions: Vec<StandaloneApplicationModelDecisionPayload>,
    items: Vec<StandaloneApplicationModelItemPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StandaloneTerminalNoInputPayload {
    stage_id: String,
    stage_run_id: Uuid,
    schema_version: i32,
    manifest_id: Uuid,
    authority_kind: String,
}

fn draft_from_standalone_payload(
    manifest: &ApplicationModelManifestRow,
    payload: &Value,
) -> Option<Option<ApplicationModelProposalDraft>> {
    if manifest.authority_kind == ApplicationModelAuthorityKindRow::TerminalNoInput.as_str() {
        let terminal =
            serde_json::from_value::<StandaloneTerminalNoInputPayload>(payload.clone()).ok()?;
        if terminal.stage_id != "application_understanding"
            || terminal.stage_run_id != manifest.stage_execution_id
            || terminal.schema_version != 1
            || terminal.manifest_id != manifest.id
            || terminal.authority_kind != "terminal_no_input"
        {
            return None;
        }
        return (proposal_payload(manifest, None) == *payload).then_some(None);
    }
    if manifest.authority_kind != ApplicationModelAuthorityKindRow::Model.as_str() {
        return None;
    }
    let recovered =
        serde_json::from_value::<StandaloneApplicationModelPayload>(payload.clone()).ok()?;
    if recovered.stage_id != "application_understanding"
        || recovered.stage_run_id != manifest.stage_execution_id
        || recovered.schema_version != 1
        || recovered.manifest_id != manifest.id
    {
        return None;
    }
    let decisions = recovered
        .decisions
        .into_iter()
        .map(|decision| {
            let disposition = match decision.disposition.as_str() {
                "incorporated" => ApplicationModelInputDispositionRow::Incorporated,
                "duplicate" => ApplicationModelInputDispositionRow::Duplicate,
                "not_relevant" => ApplicationModelInputDispositionRow::NotRelevant,
                "unknown" => ApplicationModelInputDispositionRow::Unknown,
                _ => return None,
            };
            Some(ApplicationModelInputDecisionSeed {
                input_key: decision.input_key,
                disposition,
                item_keys: decision.item_keys,
                duplicate_input_key: decision.duplicate_input_key,
                reason_code: decision.reason_code,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let items = recovered
        .items
        .into_iter()
        .map(|item| {
            let truth_state = match item.truth_state.as_str() {
                "observed" => ApplicationModelTruthStateRow::Observed,
                "inferred" => ApplicationModelTruthStateRow::Inferred,
                "unknown" => ApplicationModelTruthStateRow::Unknown,
                _ => return None,
            };
            let evidence = item
                .evidence
                .into_iter()
                .map(|evidence| {
                    let role = match evidence.role.as_str() {
                        "observation" => ApplicationModelEvidenceRoleRow::Observation,
                        "support" => ApplicationModelEvidenceRoleRow::Support,
                        _ => return None,
                    };
                    Some(ApplicationModelItemEvidenceSeed {
                        evidence_id: evidence.evidence_id,
                        role,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(ApplicationModelItemSeed {
                item_key: item.item_key,
                item_kind: item.item_kind,
                truth_state,
                source_input_keys: item.source_input_keys,
                referenced_item_keys: item.referenced_item_keys,
                payload: item.payload,
                evidence,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let draft = ApplicationModelProposalDraft {
        structured_model: recovered.structured_model,
        decisions,
        items,
    };
    (legacy_proposal_payload(manifest, &draft) == *payload).then_some(Some(draft))
}

fn draft_from_persisted_proposal(
    material: &application_models::ApplicationModelGateMaterial,
) -> Option<ApplicationModelProposalDraft> {
    let revision = material.revision.as_ref()?;
    if revision.status != "proposed"
        || revision.manifest_id != material.manifest.id
        || revision.operation_id != material.manifest.operation_id
        || revision.scope_snapshot_id != material.manifest.scope_snapshot_id
        || revision.stage_execution_id != material.manifest.stage_execution_id
        || revision.stage_run_unit_id != material.manifest.stage_run_unit_id
        || revision.organization_id != material.manifest.organization_id
    {
        return None;
    }
    let decisions = material
        .decisions
        .iter()
        .map(|decision| {
            if decision.revision_id != revision.id || decision.manifest_id != revision.manifest_id {
                return None;
            }
            let disposition = match decision.disposition.as_str() {
                "incorporated" => ApplicationModelInputDispositionRow::Incorporated,
                "duplicate" => ApplicationModelInputDispositionRow::Duplicate,
                "not_relevant" => ApplicationModelInputDispositionRow::NotRelevant,
                "unknown" => ApplicationModelInputDispositionRow::Unknown,
                _ => return None,
            };
            Some(ApplicationModelInputDecisionSeed {
                input_key: decision.input_key.clone(),
                disposition,
                item_keys: decision.item_keys.clone(),
                duplicate_input_key: decision.duplicate_input_key.clone(),
                reason_code: decision.reason_code.clone(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let mut evidence_by_item = BTreeMap::<String, Vec<ApplicationModelItemEvidenceSeed>>::new();
    for evidence in &material.item_evidence {
        if evidence.revision_id != revision.id || evidence.manifest_id != revision.manifest_id {
            return None;
        }
        let role = match evidence.role.as_str() {
            "observation" => ApplicationModelEvidenceRoleRow::Observation,
            "support" => ApplicationModelEvidenceRoleRow::Support,
            _ => return None,
        };
        evidence_by_item
            .entry(evidence.item_key.clone())
            .or_default()
            .push(ApplicationModelItemEvidenceSeed {
                evidence_id: evidence.evidence_id,
                role,
            });
    }
    let items = material
        .items
        .iter()
        .map(|item| {
            if item.revision_id != revision.id || item.manifest_id != revision.manifest_id {
                return None;
            }
            let truth_state = match item.truth_state.as_str() {
                "observed" => ApplicationModelTruthStateRow::Observed,
                "inferred" => ApplicationModelTruthStateRow::Inferred,
                "unknown" => ApplicationModelTruthStateRow::Unknown,
                _ => return None,
            };
            Some(ApplicationModelItemSeed {
                item_key: item.item_key.clone(),
                item_kind: item.item_kind.clone(),
                truth_state,
                source_input_keys: item.source_input_keys.clone(),
                referenced_item_keys: item.referenced_item_keys.clone(),
                payload: item.payload.clone(),
                evidence: evidence_by_item.remove(&item.item_key).unwrap_or_default(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    if !evidence_by_item.is_empty() {
        return None;
    }
    Some(ApplicationModelProposalDraft {
        structured_model: revision.structured_model.clone(),
        decisions,
        items,
    })
}

async fn record_submission(
    pool: &PgPool,
    input: &RunApplicationUnderstandingUnit,
    owner: &RuntimeOwner,
    manifest: &ApplicationModelManifestRow,
    payload: &Value,
) -> Result<Uuid, ApplicationUnderstandingRuntimeError> {
    let request_id = format!(
        "application-understanding-submit:{}:{}:{}",
        input.fence.worker_run_id, input.fence.attempt_epoch, manifest.id
    );
    let runtime = RuntimeToolIdentity {
        operation_id: input.fence.operation_id,
        stage_execution_id: input.fence.stage_execution_id,
        stage_run_unit_id: Some(input.fence.stage_run_unit_id),
        worker_run_id: Some(input.fence.worker_run_id),
        organization_id: Some(owner.organization_id),
        attempt_epoch: Some(input.fence.attempt_epoch),
        lease_token: Some(input.fence.lease_token),
    };
    let tool_call_record_id = tool_calls::record_tracked_start(
        pool,
        &request_id,
        input.session_id,
        Some(input.fence.operation_id),
        None,
        "submit_stage_deliverable",
        &json!({"manifest_id": manifest.id, "server_owned": true}),
        Some(&runtime),
    )
    .await?;
    if let Err(error) =
        runtime_memory_tx::begin_worker_tool(pool, &input.fence, tool_call_record_id).await
    {
        let _ = tool_calls::record_tracked_finish(
            pool,
            tool_call_record_id,
            input.session_id,
            "failed",
            &error.to_string(),
            0,
        )
        .await;
        return Err(error.into());
    }
    let canonical_payload_json = canonical_json(payload);
    let submission = stage_deliverable_submissions::insert(
        pool,
        &NewStageDeliverableSubmission {
            operation_id: input.fence.operation_id,
            stage_execution_id: input.fence.stage_execution_id,
            stage_run_unit_id: Some(input.fence.stage_run_unit_id),
            worker_run_id: Some(input.fence.worker_run_id),
            organization_id: Some(owner.organization_id),
            tool_call_record_id,
            tool_request_id: request_id,
            stage_kind: "application_understanding".to_string(),
            attempt_epoch: Some(input.fence.attempt_epoch),
            lease_token: Some(input.fence.lease_token),
            payload_sha256: sha256_hex(&canonical_payload_json),
            canonical_payload_json,
        },
    )
    .await;
    let submission = match submission {
        Ok(submission) => submission,
        Err(error) => {
            let _ = runtime_memory_tx::finish_worker_tool(pool, &input.fence, tool_call_record_id)
                .await;
            let _ = tool_calls::record_tracked_finish(
                pool,
                tool_call_record_id,
                input.session_id,
                "failed",
                &error.to_string(),
                0,
            )
            .await;
            return Err(error.into());
        }
    };
    runtime_memory_tx::finish_worker_tool(pool, &input.fence, tool_call_record_id).await?;
    tool_calls::record_tracked_finish(
        pool,
        tool_call_record_id,
        input.session_id,
        "finished",
        &canonical_json(&json!({
            "accepted": true,
            "deliverable_submission_id": submission.id,
        })),
        0,
    )
    .await?;
    Ok(submission.id)
}

async fn finalize_submission(
    pool: &PgPool,
    input: &RunApplicationUnderstandingUnit,
    manifest_id: Uuid,
    deliverable_submission_id: Uuid,
) -> Result<ApplicationUnderstandingRuntimeOutcome, ApplicationUnderstandingRuntimeError> {
    match finalize_application_model_gate_pass(
        pool,
        &FinalizeApplicationModelGatePass {
            fence: input.fence.clone(),
            deliverable_submission_id,
            manifest_id,
            expected_unit_row_version: input.expected_unit_row_version,
            scope_hash: input.scope_hash.clone(),
        },
    )
    .await?
    {
        ApplicationModelFinalizationOutcome::Passed(pass) => {
            Ok(ApplicationUnderstandingRuntimeOutcome::Passed(pass))
        }
        ApplicationModelFinalizationOutcome::Blocked(block) => {
            Ok(ApplicationUnderstandingRuntimeOutcome::Blocked(block))
        }
    }
}

pub async fn run_application_understanding_unit<P>(
    pool: &PgPool,
    input: &RunApplicationUnderstandingUnit,
    producer: &P,
) -> Result<ApplicationUnderstandingRuntimeOutcome, ApplicationUnderstandingRuntimeError>
where
    P: ApplicationModelProposalProducer,
{
    let owner = match resolve_runtime_owner(pool, input).await? {
        Ok(owner) => owner,
        Err(outcome) => return Ok(outcome),
    };
    let seeded = application_models::seed_manifest_from_current_predecessors(
        pool,
        &DeriveApplicationModelManifestSeed {
            operation_id: input.fence.operation_id,
            scope_snapshot_id: owner.scope_snapshot_id,
            stage_execution_id: input.fence.stage_execution_id,
            stage_run_unit_id: input.fence.stage_run_unit_id,
            organization_id: owner.organization_id,
        },
    )
    .await?;
    if let Some(deliverable_submission_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT deliverable_submission_id FROM application_model_current_revisions WHERE manifest_id=$1",
    )
    .bind(seeded.manifest.id)
    .fetch_optional(pool)
    .await?
    {
        return finalize_submission(pool, input, seeded.manifest.id, deliverable_submission_id)
            .await;
    }
    if let Some(deliverable_submission_id) = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT revision.source_submission_id
             FROM application_model_revisions AS revision
             JOIN stage_deliverable_submissions AS submission
               ON submission.id=revision.source_submission_id
              AND submission.operation_id=$2
              AND submission.stage_execution_id=$3
              AND submission.stage_run_unit_id=$4
              AND submission.worker_run_id=$5
              AND submission.attempt_epoch=$6
              AND submission.lease_token=$7
            WHERE revision.manifest_id=$1 AND revision.status='proposed'"#,
    )
    .bind(seeded.manifest.id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.worker_run_id)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.lease_token)
    .fetch_optional(pool)
    .await?
    {
        return finalize_submission(pool, input, seeded.manifest.id, deliverable_submission_id)
            .await;
    }
    let producer_barrier_refs = runtime_producer_barrier_refs(pool, input).await?;
    if !producer_barrier_refs.is_empty() {
        return Ok(block(
            ApplicationModelGateCode::ProducerBarrierOpen,
            ApplicationModelGateDisposition::Hold,
            producer_barrier_refs,
        ));
    }
    let gate_material = application_models::load_gate_material(
        pool,
        &LoadApplicationModelGateMaterial {
            manifest_id: seeded.manifest.id,
            operation_id: input.fence.operation_id,
            scope_snapshot_id: owner.scope_snapshot_id,
            stage_execution_id: input.fence.stage_execution_id,
            stage_run_unit_id: input.fence.stage_run_unit_id,
            organization_id: owner.organization_id,
        },
    )
    .await?;
    let standalone_submission = if gate_material.revision.is_none() {
        application_models::load_standalone_submission(
            pool,
            &LoadStandaloneApplicationModelSubmission {
                manifest_id: seeded.manifest.id,
                operation_id: input.fence.operation_id,
                scope_snapshot_id: owner.scope_snapshot_id,
                stage_execution_id: input.fence.stage_execution_id,
                stage_run_unit_id: input.fence.stage_run_unit_id,
                organization_id: owner.organization_id,
                worker_run_id: input.fence.worker_run_id,
                attempt_epoch: input.fence.attempt_epoch,
                lease_token: input.fence.lease_token,
            },
        )
        .await?
    } else {
        None
    };
    if !gate_material.forbidden_activity_refs.is_empty() {
        return Ok(block(
            ApplicationModelGateCode::ForbiddenToolActivity,
            ApplicationModelGateDisposition::Hold,
            gate_material.forbidden_activity_refs,
        ));
    }
    if !gate_material.pending_producer_refs.is_empty() {
        return Ok(block(
            ApplicationModelGateCode::ProducerBarrierOpen,
            ApplicationModelGateDisposition::Hold,
            gate_material.pending_producer_refs,
        ));
    }
    let (draft, recovered_submission_id) = if let Some(standalone) = standalone_submission {
        let submission_ref = format!(
            "standalone_submission:{}",
            standalone.deliverable_submission_id
        );
        if !standalone.recoverable_by_current_fence {
            return Ok(block(
                ApplicationModelGateCode::ProducerBarrierOpen,
                ApplicationModelGateDisposition::Hold,
                [
                    "application_model_submission_outcome_unknown".to_string(),
                    submission_ref,
                    format!("tool_status:{}", standalone.tool_status),
                ],
            ));
        }
        let draft = match draft_from_standalone_payload(&seeded.manifest, &standalone.payload) {
            Some(draft) => draft,
            None if seeded.manifest.authority_kind
                == ApplicationModelAuthorityKindRow::Model.as_str() =>
            {
                let reproduced = producer
                    .produce(ApplicationModelProducerInput {
                        manifest_id: seeded.manifest.id,
                        organization_id: owner.organization_id,
                        inputs: gate_material.inputs.clone(),
                    })
                    .await?;
                if proposal_payload(&seeded.manifest, Some(&reproduced)) != standalone.payload {
                    return Ok(block(
                        ApplicationModelGateCode::ProducerBarrierOpen,
                        ApplicationModelGateDisposition::Hold,
                        [
                            "application_model_standalone_payload_invalid".to_string(),
                            submission_ref,
                        ],
                    ));
                }
                Some(reproduced)
            }
            None => {
                return Ok(block(
                    ApplicationModelGateCode::ProducerBarrierOpen,
                    ApplicationModelGateDisposition::Hold,
                    [
                        "application_model_standalone_payload_invalid".to_string(),
                        submission_ref,
                    ],
                ));
            }
        };
        (
            draft,
            (!standalone.requires_reauthorization).then_some(standalone.deliverable_submission_id),
        )
    } else if let Some(draft) = draft_from_persisted_proposal(&gate_material) {
        (Some(draft), None)
    } else if gate_material.revision.is_some() {
        return Ok(block(
            ApplicationModelGateCode::ReplayDrift,
            ApplicationModelGateDisposition::Hold,
            ["application_model_persisted_proposal_invalid".to_string()],
        ));
    } else {
        let draft = match seeded.manifest.authority_kind.as_str() {
            "terminal_no_input" => None,
            "model" => Some(
                producer
                    .produce(ApplicationModelProducerInput {
                        manifest_id: seeded.manifest.id,
                        organization_id: owner.organization_id,
                        inputs: gate_material.inputs.clone(),
                    })
                    .await?,
            ),
            _ => {
                return Ok(block(
                    ApplicationModelGateCode::SchemaInvalid,
                    ApplicationModelGateDisposition::Rework,
                    ["manifest_authority_kind".to_string()],
                ));
            }
        };
        (draft, None)
    };
    if let Some(draft) = draft.as_ref() {
        if let Err(gate_block) = validate_draft(&seeded.manifest, &gate_material.inputs, draft) {
            return Ok(ApplicationUnderstandingRuntimeOutcome::Blocked(gate_block));
        }
    }
    let deliverable_submission_id = if let Some(submission_id) = recovered_submission_id {
        submission_id
    } else {
        let payload = proposal_payload(&seeded.manifest, draft.as_ref());
        record_submission(pool, input, &owner, &seeded.manifest, &payload).await?
    };
    if let Some(draft) = &draft {
        application_models::propose_revision(
            pool,
            &ProposeApplicationModelRevision {
                manifest_id: seeded.manifest.id,
                operation_id: input.fence.operation_id,
                scope_snapshot_id: owner.scope_snapshot_id,
                stage_execution_id: input.fence.stage_execution_id,
                stage_run_unit_id: input.fence.stage_run_unit_id,
                organization_id: owner.organization_id,
                source_submission_id: deliverable_submission_id,
                structured_model: draft.structured_model.clone(),
                decisions: draft.decisions.clone(),
                items: draft.items.clone(),
            },
        )
        .await?;
    } else if seeded.manifest.authority_kind
        != ApplicationModelAuthorityKindRow::TerminalNoInput.as_str()
    {
        return Ok(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            ["manifest_authority_kind".to_string()],
        ));
    }
    finalize_submission(pool, input, seeded.manifest.id, deliverable_submission_id).await
}

#[allow(dead_code)] // Legacy planless Worker recovery compatibility.
fn unit_status(value: &str) -> Option<stage_run_units::StageRunUnitStatus> {
    stage_run_units::StageRunUnitStatus::ALL
        .into_iter()
        .find(|status| status.as_str() == value)
}

#[allow(dead_code)] // Legacy planless Worker recovery compatibility.
fn worker_status(value: &str) -> Option<stage_worker_runs::StageWorkerRunStatus> {
    stage_worker_runs::StageWorkerRunStatus::ALL
        .into_iter()
        .find(|status| status.as_str() == value)
}

#[allow(dead_code)] // Legacy planless Worker recovery compatibility.
async fn claim_or_resume_application_model_worker(
    pool: &PgPool,
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
    seeded: &runtime_memory_tx::SeededStageRuntimeRow,
) -> anyhow::Result<Option<(RunApplicationUnderstandingUnit, bool)>> {
    if seeded.unit.status == "passed" && seeded.worker.status == "passed" {
        return Ok(None);
    }
    anyhow::ensure!(
        matches!(
            seeded.unit.status.as_str(),
            "queued" | "running" | "gate_blocked"
        ),
        "APPLICATION_UNDERSTANDING_UNIT_NOT_RUNNABLE:{}:{}",
        seeded.unit.id,
        seeded.unit.status
    );
    anyhow::ensure!(
        seeded.worker.active_tool_call_id.is_none(),
        "APPLICATION_UNDERSTANDING_FORBIDDEN_ACTIVE_TOOL:{}",
        seeded.worker.id
    );

    let mut worker = seeded.worker.clone();
    if worker.status == "running"
        && worker
            .lease_expires_at
            .is_some_and(|expires_at| expires_at <= chrono::Utc::now())
    {
        let (disposition, reaped) = stage_worker_runs::reap_expired(pool, worker.id).await?;
        anyhow::ensure!(
            disposition == stage_worker_runs::ExpiredWorkerDisposition::Requeued,
            "APPLICATION_UNDERSTANDING_WORKER_RECOVERY_REQUIRED:{}",
            worker.id
        );
        worker = reaped;
    }

    anyhow::ensure!(
        worker.status != "running",
        "APPLICATION_UNDERSTANDING_WORKER_ALREADY_ACTIVE:{}",
        worker.id
    );
    let (unit, worker, newly_claimed) = {
        let expected_unit_status = unit_status(&seeded.unit.status)
            .ok_or_else(|| anyhow::anyhow!("APPLICATION_UNDERSTANDING_UNIT_STATUS_INVALID"))?;
        let expected_worker_status = worker_status(&worker.status)
            .ok_or_else(|| anyhow::anyhow!("APPLICATION_UNDERSTANDING_WORKER_STATUS_INVALID"))?;
        anyhow::ensure!(
            matches!(
                expected_worker_status,
                stage_worker_runs::StageWorkerRunStatus::Queued
                    | stage_worker_runs::StageWorkerRunStatus::GateBlocked
            ),
            "APPLICATION_UNDERSTANDING_WORKER_NOT_CLAIMABLE:{}:{}",
            worker.id,
            worker.status
        );
        let claimed = runtime_memory_tx::claim_worker_and_bind_chain(
            pool,
            &runtime_memory_tx::ClaimWorkerAndBindChainRow {
                operation_id: request.operation_id,
                stage_execution_id: request.stage_execution_id,
                stage_run_unit_id: seeded.unit.id,
                worker_run_id: worker.id,
                expected_unit_status,
                expected_unit_row_version: seeded.unit.row_version,
                expected_worker_status,
                expected_attempt_epoch: worker.attempt_epoch,
                session_id: request.session_id,
                subtask_id: None,
                agent: golish_db::models::AgentType::Pentester,
                model: None,
                provider: None,
                parent_chain_id: None,
                lease_owner: format!("application-understanding:{}", request.operation_id),
                lease_seconds: 300,
                initial_chain: json!([]),
                initial_checkpoint: json!({
                    "schema_version": 1,
                    "phase": "modeling",
                }),
            },
        )
        .await?;
        (claimed.unit, claimed.worker, true)
    };
    let lease_token = worker
        .lease_token
        .ok_or_else(|| anyhow::anyhow!("APPLICATION_UNDERSTANDING_WORKER_LEASE_MISSING"))?;
    Ok(Some((
        RunApplicationUnderstandingUnit {
            session_id: request.session_id,
            fence: RuntimeMemoryTxFence {
                operation_id: request.operation_id,
                stage_execution_id: request.stage_execution_id,
                stage_run_unit_id: unit.id,
                worker_run_id: worker.id,
                lease_token,
                attempt_epoch: worker.attempt_epoch,
                expected_checkpoint_version: worker.checkpoint_version,
            },
            expected_unit_row_version: unit.row_version,
            scope_hash: seeded.scope_hash.clone(),
        },
        newly_claimed,
    )))
}

async fn aggregate_application_model_stage(
    pool: &PgPool,
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
) -> anyhow::Result<(usize, usize)> {
    let rows = sqlx::query_as::<_, (Uuid, i64, i64, i64, i64, i64, i64, i64)>(
        r#"SELECT member.organization_id,
                  count(DISTINCT unit.id) FILTER (WHERE unit.status='passed') AS passed_units,
                  count(DISTINCT worker.id) FILTER (
                      WHERE worker.status NOT IN ('passed','failed','exhausted','superseded')
                  ) AS non_terminal_workers,
                  count(DISTINCT authority_worker.id) FILTER (
                      WHERE authority_worker.status='passed'
                        AND authority_worker.id=authority_submission.worker_run_id
                  ) AS passed_authority_workers,
                  count(DISTINCT manifest.id) AS manifests,
                  count(DISTINCT current_revision.manifest_id) FILTER (
                      WHERE current_revision.manifest_hash=manifest.manifest_hash
                        AND (
                            (manifest.authority_kind='terminal_no_input'
                             AND current_revision.authority_kind='terminal_no_input'
                             AND current_revision.revision_id IS NULL
                             AND current_revision.replay_material_hash=manifest.replay_material_hash)
                            OR
                            (manifest.authority_kind='model'
                             AND current_revision.authority_kind='model'
                             AND revision.status='final'
                             AND revision.id=current_revision.revision_id
                             AND revision.model_hash=current_revision.model_hash
                             AND revision.replay_material_hash=current_revision.replay_material_hash)
                        )
                  ) AS valid_authorities,
                  count(DISTINCT handoff.id) AS handoffs,
                  count(DISTINCT handoff.id) FILTER (
                      WHERE handoff.invalidated_at IS NULL
                        AND handoff.id=current_revision.stage_handoff_id
                        AND handoff.deliverable_submission_id=current_revision.deliverable_submission_id
                        AND current_revision.gate_decision_hash='sha256:' || handoff.unit_gate_decision_hash
                        AND handoff.source_stage_run_unit_id=unit.id
                  ) AS valid_handoffs
             FROM operation_org_scope_snapshots AS snapshot
             JOIN operation_org_scope_units AS member ON member.snapshot_id=snapshot.id
             LEFT JOIN stage_run_units AS unit
               ON unit.operation_id=snapshot.operation_id
              AND unit.scope_snapshot_id=snapshot.id
              AND unit.organization_id=member.organization_id
              AND unit.stage_execution_id=$2
              AND unit.stage_kind='application_understanding'
             LEFT JOIN stage_worker_runs AS worker
               ON worker.operation_id=unit.operation_id
              AND worker.stage_execution_id=unit.stage_execution_id
              AND worker.stage_run_unit_id=unit.id
              AND worker.organization_id=unit.organization_id
             LEFT JOIN application_model_manifests AS manifest
               ON manifest.operation_id=unit.operation_id
              AND manifest.scope_snapshot_id=unit.scope_snapshot_id
              AND manifest.stage_execution_id=unit.stage_execution_id
              AND manifest.stage_run_unit_id=unit.id
              AND manifest.organization_id=unit.organization_id
             LEFT JOIN application_model_current_revisions AS current_revision
               ON current_revision.manifest_id=manifest.id
             LEFT JOIN stage_deliverable_submissions AS authority_submission
               ON authority_submission.id=current_revision.deliverable_submission_id
              AND authority_submission.operation_id=unit.operation_id
              AND authority_submission.stage_execution_id=unit.stage_execution_id
              AND authority_submission.stage_run_unit_id=unit.id
              AND authority_submission.organization_id=unit.organization_id
              AND authority_submission.stage_kind='application_understanding'
             LEFT JOIN stage_worker_runs AS authority_worker
               ON authority_worker.id=authority_submission.worker_run_id
              AND authority_worker.operation_id=unit.operation_id
              AND authority_worker.stage_execution_id=unit.stage_execution_id
              AND authority_worker.stage_run_unit_id=unit.id
              AND authority_worker.organization_id=unit.organization_id
             LEFT JOIN application_model_revisions AS revision
               ON revision.id=current_revision.revision_id
              AND revision.manifest_id=manifest.id
             LEFT JOIN stage_handoffs AS handoff
               ON handoff.operation_id=unit.operation_id
              AND handoff.scope_snapshot_id=unit.scope_snapshot_id
              AND handoff.stage_execution_id=unit.stage_execution_id
              AND handoff.organization_id=unit.organization_id
              AND handoff.from_stage_kind='application_understanding'
            WHERE snapshot.operation_id=$1 AND snapshot.sealed_at IS NOT NULL
            GROUP BY member.organization_id
            ORDER BY member.organization_id"#,
    )
    .bind(request.operation_id)
    .bind(request.stage_execution_id)
    .fetch_all(pool)
    .await?;
    let invalid = rows
        .iter()
        .filter(
            |(_, units, non_terminal_workers, passed_authority_workers, manifests, authorities, handoffs, valid_handoffs)| {
            *units != 1
                || *non_terminal_workers != 0
                || *passed_authority_workers != 1
                || *manifests != 1
                || *authorities != 1
                || *handoffs != 1
                || *valid_handoffs != 1
        },
        )
        .map(
            |(organization_id, units, non_terminal_workers, passed_authority_workers, manifests, authorities, handoffs, valid_handoffs)| {
                format!(
                    "{organization_id}:unit={units},workers=non_terminal:{non_terminal_workers}/authority_passed:{passed_authority_workers},manifest={manifests},authority={authorities},handoffs={handoffs}/{valid_handoffs}"
                )
            },
        )
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !rows.is_empty() && invalid.is_empty(),
        "APPLICATION_UNDERSTANDING_AGGREGATE_INCOMPLETE:{invalid:?}"
    );
    Ok((rows.len(), rows.len()))
}

async fn validate_complete_predecessor_handoffs(
    pool: &PgPool,
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
) -> anyhow::Result<()> {
    let rows = sqlx::query_as::<_, (Uuid, i64)>(
        r#"SELECT member.organization_id,
                  count(handoff.id) FILTER (WHERE source_unit.id IS NOT NULL) AS handoff_rows
             FROM operation_org_scope_snapshots AS snapshot
             JOIN operation_org_scope_units AS member ON member.snapshot_id=snapshot.id
             LEFT JOIN stage_handoffs AS handoff
               ON handoff.operation_id=snapshot.operation_id
              AND handoff.scope_snapshot_id=snapshot.id
              AND handoff.organization_id=member.organization_id
              AND handoff.from_stage_kind='vuln_triage'
              AND handoff.invalidated_at IS NULL
             LEFT JOIN stage_run_units AS source_unit
               ON source_unit.id=handoff.source_stage_run_unit_id
              AND source_unit.operation_id=handoff.operation_id
              AND source_unit.stage_execution_id=handoff.stage_execution_id
              AND source_unit.organization_id=handoff.organization_id
              AND source_unit.stage_kind=handoff.from_stage_kind
              AND source_unit.status='passed'
            WHERE snapshot.operation_id=$1 AND snapshot.sealed_at IS NOT NULL
            GROUP BY member.organization_id
            ORDER BY member.organization_id"#,
    )
    .bind(request.operation_id)
    .fetch_all(pool)
    .await?;
    let mut incomplete = Vec::new();
    for (organization_id, direct_rows) in rows {
        let adopted_source_operation =
            golish_db::repo::operation_stage_forks::source_operation_for_stage(
                pool,
                request.operation_id,
                organization_id,
                "vuln_triage",
            )
            .await?;
        if !predecessor_handoff_authority_is_complete(direct_rows, adopted_source_operation) {
            incomplete.push(format!(
                "{organization_id}:direct={direct_rows},adopted_source={}",
                adopted_source_operation
                    .map(|operation_id| operation_id.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ));
        }
    }
    anyhow::ensure!(
        incomplete.is_empty(),
        "APPLICATION_UNDERSTANDING_PREDECESSOR_HANDOFF_INCOMPLETE:{:?}",
        incomplete
    );
    Ok(())
}

fn predecessor_handoff_authority_is_complete(
    direct_rows: i64,
    adopted_source_operation: Option<Uuid>,
) -> bool {
    matches!(
        (direct_rows, adopted_source_operation),
        (1, None) | (0, Some(_))
    )
}

#[allow(dead_code)] // Legacy planless Worker recovery compatibility.
async fn park_rework_attempt_if_unsubmitted(
    pool: &PgPool,
    command: &RunApplicationUnderstandingUnit,
    code: &str,
    refs: &[String],
) -> anyhow::Result<bool> {
    match runtime_memory_tx::block_application_understanding_attempt(
        pool,
        &runtime_memory_tx::BlockApplicationUnderstandingAttemptRow {
            fence: command.fence.clone(),
            expected_unit_row_version: command.expected_unit_row_version,
            code: code.to_string(),
            refs: refs.to_vec(),
        },
    )
    .await
    {
        Ok(_) => Ok(true),
        Err(runtime_memory_tx::RuntimeMemoryStoreError::Conflict {
            code: "application_understanding_block_after_submission_forbidden",
        }) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Clone)]
struct ApplicationTeamRuntime {
    unit: stage_run_units::StageRunUnitRow,
    plan: stage_teams::StageTeamPlanRow,
    work_items: Vec<stage_teams::StageWorkItemRow>,
    scope_hash: String,
}

enum EnsuredApplicationTeamRuntimes {
    Ready(Vec<ApplicationTeamRuntime>),
    LegacyReplaced(Uuid),
    ExhaustedRuntimeReplaced(Uuid),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicationWorkItemInputAuthority {
    schema: String,
    manifest_id: Uuid,
    manifest_hash: String,
    organization_id: Uuid,
    work_item_key: String,
    work_item_kind: String,
    projection_hash: String,
    projection: Value,
    source_input_keys: Vec<String>,
    evidence_ids: Vec<i64>,
}

fn application_manifest_input_refs(
    inputs: &[ApplicationModelManifestInputRow],
) -> Result<Vec<Value>, ApplicationUnderstandingRuntimeError> {
    inputs
        .iter()
        .map(|input| {
            let content_hash = input
                .source_payload_hash
                .strip_prefix("sha256:")
                .filter(|hash| {
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
                .ok_or_else(|| producer_error("application_model_manifest_hash_non_contract"))?;
            Ok(json!({
                "content_hash": content_hash,
                "evidence_ids": input.evidence_ids,
                "input_key": input.input_key,
                "input_kind": input.input_kind,
                "source_id": input.source_id,
                "source_version": input.source_version,
            }))
        })
        .collect()
}

fn application_team_seed(
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
    unit: &stage_run_units::StageRunUnitRow,
    manifest: &ApplicationModelManifestRow,
    inputs: &[ApplicationModelManifestInputRow],
    projections: &[ProjectedApplicationWorkItem],
) -> anyhow::Result<runtime_memory_tx::SeedStageTeamRuntimeRow> {
    let manifest_input_refs = application_manifest_input_refs(inputs)?;
    let mut work_items = projections
        .iter()
        .map(|projection| {
            let input_manifest = json!({
                "schema": "application_model_work_item_input.v1",
                "manifest_id": manifest.id,
                "manifest_hash": manifest.manifest_hash,
                "organization_id": manifest.organization_id,
                "work_item_key": projection.work_item_key,
                "work_item_kind": projection.work_item_kind,
                "projection_hash": projection.projection_hash,
                "projection": projection.projection,
                "source_input_keys": projection.source_input_keys,
                "evidence_ids": projection.evidence_ids,
            });
            runtime_memory_tx::StageWorkItemSeedRow {
                stable_key: projection.work_item_key.clone(),
                work_item_kind: projection.work_item_kind.clone(),
                role: "application_model_worker".to_string(),
                input_manifest_hash: tagged_json_hash(&input_manifest),
                input_manifest,
                conflict_key: None,
                priority: 0,
                required_for_barrier: true,
                is_aggregator: false,
                attempt_policy: json!({"max_attempts": 2}),
                budget: json!({
                    "completion_mode": "tool_free",
                    "max_output_items": 256,
                }),
                output_schema: "application_model_work_item_output.v1".to_string(),
                created_by: "server_seed".to_string(),
            }
        })
        .collect::<Vec<_>>();
    let expected_work_items = projections
        .iter()
        .map(|projection| {
            json!({
                "work_item_key": projection.work_item_key,
                "work_item_kind": projection.work_item_kind,
                "projection_hash": projection.projection_hash,
            })
        })
        .collect::<Vec<_>>();
    let leader_manifest = json!({
        "schema": "application_model_synthesis_input.v1",
        "manifest_id": manifest.id,
        "manifest_hash": manifest.manifest_hash,
        "organization_id": manifest.organization_id,
        "manifest_inputs": manifest_input_refs,
        "expected_work_items": expected_work_items,
    });
    work_items.push(runtime_memory_tx::StageWorkItemSeedRow {
        stable_key: "leader:primary".to_string(),
        work_item_kind: "application_model_synthesis".to_string(),
        role: "application_model_synthesizer".to_string(),
        input_manifest_hash: tagged_json_hash(&leader_manifest),
        input_manifest: leader_manifest,
        conflict_key: Some("stage_unit_finalizer".to_string()),
        priority: i32::MAX,
        required_for_barrier: false,
        is_aggregator: true,
        attempt_policy: json!({"max_attempts": 2}),
        budget: json!({
            "completion_mode": "tool_free",
            "max_output_items": 512,
        }),
        output_schema: "application_model_proposal.v1".to_string(),
        created_by: "server_seed".to_string(),
    });
    let plan_material = json!({
        "contract": "application_understanding_company_team.v1",
        "manifest_hash": manifest.manifest_hash,
        "organization_id": manifest.organization_id,
        "work_items": projections.iter().map(|projection| json!({
            "key": projection.work_item_key,
            "kind": projection.work_item_kind,
            "projection_hash": projection.projection_hash,
        })).collect::<Vec<_>>(),
    });
    let max_workers_total = i32::try_from(work_items.len())
        .map_err(|_| anyhow::anyhow!("APPLICATION_UNDERSTANDING_WORK_ITEM_COUNT_OVERFLOW"))?;
    Ok(runtime_memory_tx::SeedStageTeamRuntimeRow {
        base: runtime_memory_tx::SeedStageRuntimeRow {
            operation_id: request.operation_id,
            stage_execution_id: request.stage_execution_id,
            stage_kind: "application_understanding".to_string(),
            unit_generation: unit.generation,
            specialist: "application_understanding".to_string(),
            worker_generation: 0,
            work_item_kind: "application_model".to_string(),
            work_item_key: "application_understanding".to_string(),
            agent_path_prefix: "main>application_understanding".to_string(),
            organization_ids: Some(vec![unit.organization_id]),
        },
        plan: runtime_memory_tx::StageTeamPlanSeedRow {
            schema_version: 1,
            plan_version: 1,
            plan_hash: tagged_json_hash(&plan_material),
            leader_role: "application_model_synthesizer".to_string(),
            allowed_roles: vec![
                "application_model_synthesizer".to_string(),
                "application_model_worker".to_string(),
            ],
            aggregator_kind: "worker".to_string(),
            aggregator_role: Some("application_model_synthesizer".to_string()),
            max_workers_total,
            max_workers_active: max_workers_total.min(4),
            dynamic_requests_enabled: false,
            dynamic_request_policy: json!({
                "coordination_mode": "company_controller",
                "formulaic_worklist_executor": "application_model_v1",
                "static_work_items_only": true,
            }),
            final_submitter_kind: "worker".to_string(),
            created_from_stage_spec_hash: tagged_json_hash(&json!({
                "stage": "application_understanding",
                "runtime_contract": "application_understanding_company_team.v1",
            })),
        },
        work_items,
    })
}

async fn seed_one_application_team(
    pool: &PgPool,
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
    unit: &stage_run_units::StageRunUnitRow,
    scope_hash: &str,
) -> Result<ApplicationTeamRuntime, ApplicationUnderstandingRuntimeError> {
    let seeded_manifest = application_models::seed_manifest_from_current_predecessors(
        pool,
        &DeriveApplicationModelManifestSeed {
            operation_id: request.operation_id,
            scope_snapshot_id: unit.scope_snapshot_id,
            stage_execution_id: request.stage_execution_id,
            stage_run_unit_id: unit.id,
            organization_id: unit.organization_id,
        },
    )
    .await?;
    let gate_material = application_models::load_gate_material(
        pool,
        &LoadApplicationModelGateMaterial {
            manifest_id: seeded_manifest.manifest.id,
            operation_id: request.operation_id,
            scope_snapshot_id: unit.scope_snapshot_id,
            stage_execution_id: request.stage_execution_id,
            stage_run_unit_id: unit.id,
            organization_id: unit.organization_id,
        },
    )
    .await?;
    let projections = if seeded_manifest.manifest.authority_kind
        == ApplicationModelAuthorityKindRow::TerminalNoInput.as_str()
    {
        Vec::new()
    } else {
        let source = load_application_projection_source(
            pool,
            request.operation_id,
            &seeded_manifest.manifest,
            &gate_material.inputs,
        )
        .await
        .map_err(|error| {
            producer_error(format!("application_model_projection_failed:{error:#}"))
        })?;
        build_application_work_item_projections(&source).map_err(|error| {
            producer_error(format!("application_model_projection_failed:{error:#}"))
        })?
    };
    let team_seed = application_team_seed(
        request,
        unit,
        &seeded_manifest.manifest,
        &gate_material.inputs,
        &projections,
    )
    .map_err(|error| producer_error(format!("application_model_team_seed_invalid:{error:#}")))?;
    let mut seeded = runtime_memory_tx::seed_stage_team_runtime(pool, &team_seed).await?;
    if seeded.len() != 1 {
        return Err(producer_error("application_model_team_seed_cardinality"));
    }
    let seeded = seeded.remove(0);
    Ok(ApplicationTeamRuntime {
        unit: seeded.unit,
        plan: seeded.plan,
        work_items: seeded.work_items,
        scope_hash: scope_hash.to_string(),
    })
}

async fn recover_exhausted_application_team_runtime(
    pool: &PgPool,
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
) -> Result<Option<Uuid>, ApplicationUnderstandingRuntimeError> {
    const MAX_RESPONSE_NON_CONTRACT_RECOVERIES: u64 = 3;

    let Some((state_blob, Some(min_generation), Some(max_generation))) =
        sqlx::query_as::<_, (Value, Option<i32>, Option<i32>)>(
        r#"SELECT operation.state_blob,
                  (SELECT min(unit.generation) FROM stage_run_units unit
                    WHERE unit.operation_id=$1 AND unit.stage_execution_id=$2),
                  (SELECT max(unit.generation) FROM stage_run_units unit
                    WHERE unit.operation_id=$1 AND unit.stage_execution_id=$2)
             FROM operation_state AS operation
            WHERE operation.operation_id=$1
              AND operation.superseded_by IS NULL
              AND operation.runtime_memory_contract='v2_only'
              AND operation.application_model_contract='application_model_v1'
              AND operation.current_stage='application_understanding'
              AND EXISTS (
                    SELECT 1 FROM stage_runs execution
                     WHERE execution.id=$2 AND execution.operation_id=$1
                       AND execution.stage_kind='application_understanding'
                       AND execution.status='started'
                  )
              AND 1=(
                    SELECT count(*) FROM stage_runs execution
                     WHERE execution.operation_id=$1 AND execution.status='started'
                  )
              AND EXISTS (
                    SELECT 1 FROM stage_run_units unit
                     WHERE unit.operation_id=$1 AND unit.stage_execution_id=$2
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM stage_run_units unit
                     WHERE unit.operation_id=$1 AND unit.stage_execution_id=$2
                       AND (unit.stage_kind<>'application_understanding'
                            OR unit.status<>'gate_blocked'
                            OR unit.generation NOT IN (0,1,2))
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM stage_team_plans plan
                     WHERE plan.operation_id=$1 AND plan.stage_execution_id=$2
                       AND plan.requests_closed_at IS NULL
                  )
              AND EXISTS (
                    SELECT 1 FROM stage_work_items item
                     WHERE item.operation_id=$1 AND item.stage_execution_id=$2
                       AND item.required_for_barrier AND item.status='exhausted'
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM stage_work_items item
                     WHERE item.operation_id=$1 AND item.stage_execution_id=$2
                       AND item.required_for_barrier
                       AND item.status NOT IN ('completed','exhausted')
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM stage_work_items item
                     WHERE item.operation_id=$1 AND item.stage_execution_id=$2
                       AND item.required_for_barrier AND item.status='exhausted'
                       AND (
                            1<>(SELECT count(*) FROM stage_worker_outputs output
                                 WHERE output.work_item_id=item.id)
                            OR NOT EXISTS (
                                SELECT 1 FROM stage_worker_outputs output
                                 WHERE output.work_item_id=item.id
                                   AND output.business_disposition='blocked'
                                   AND 'STAGE_TEAM_PRODUCER_ATTEMPTS_EXHAUSTED'=ANY(output.blocker_codes)
                            )
                            OR COALESCE((item.attempt_policy->>'max_attempts')::BIGINT,0)<>
                               (SELECT count(*) FROM stage_worker_runs worker
                                 WHERE worker.work_item_id=item.id)
                            OR EXISTS (
                                SELECT 1 FROM stage_worker_runs worker
                                 WHERE worker.work_item_id=item.id
                                   AND (worker.status<>'failed'
                                        OR worker.checkpoint #>> '{stage_team_execution_failure,code}'
                                           <>'application_model_response_non_contract'
                                        OR worker.lease_token IS NOT NULL
                                        OR worker.active_tool_call_id IS NOT NULL)
                            )
                       )
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM stage_work_items item
                     WHERE item.operation_id=$1 AND item.stage_execution_id=$2
                       AND item.stable_key='leader:primary'
                       AND item.status<>'superseded'
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM stage_worker_runs worker
                     WHERE worker.operation_id=$1 AND worker.stage_execution_id=$2
                       AND (worker.status IN ('queued','running','retry_pending','recovery_required')
                            OR worker.lease_token IS NOT NULL
                            OR worker.active_tool_call_id IS NOT NULL)
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM stage_deliverable_submissions submission
                     WHERE submission.operation_id=$1 AND submission.stage_execution_id=$2
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM stage_handoffs handoff
                     WHERE handoff.operation_id=$1 AND handoff.stage_execution_id=$2
                  )
              AND NOT EXISTS (
                    SELECT 1 FROM application_model_revisions revision
                     WHERE revision.operation_id=$1 AND revision.stage_execution_id=$2
                  )
              AND NOT EXISTS (
                    SELECT 1
                      FROM application_model_current_revisions current_revision
                      JOIN application_model_manifests manifest
                        ON manifest.id=current_revision.manifest_id
                     WHERE manifest.operation_id=$1 AND manifest.stage_execution_id=$2
                  )"#,
    )
    .bind(request.operation_id)
    .bind(request.stage_execution_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    if min_generation != max_generation {
        return Ok(None);
    }
    let prior_recovery_count = match min_generation {
        0 => {
            if state_blob
                .get("application_understanding_response_non_contract_recovery")
                .is_some()
            {
                return Ok(None);
            }
            0
        }
        1 | 2 => {
            let reset = state_blob.get("runtime_v2_dev_reset");
            let source_stage_execution_id = reset
                .and_then(|marker| marker.get("superseded_stage_execution_ids"))
                .and_then(Value::as_array)
                .and_then(|ids| ids.as_slice().first().filter(|_| ids.len() == 1))
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok());
            let exact_reset_lineage = reset
                .and_then(|marker| marker.get("selected_stage"))
                .and_then(Value::as_str)
                == Some("application_understanding")
                && reset
                    .and_then(|marker| marker.get("replacement_stage_execution_id"))
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                    == Some(request.stage_execution_id)
                && source_stage_execution_id.is_some();
            if !exact_reset_lineage {
                return Ok(None);
            }
            match state_blob.get("application_understanding_response_non_contract_recovery") {
                Some(marker)
                    if marker.get("schema_version").and_then(Value::as_u64) == Some(1)
                        && marker.get("recovery_count").and_then(Value::as_u64)
                            == Some(min_generation as u64)
                        && marker
                            .get("max_recoveries")
                            .and_then(Value::as_u64)
                            .is_some_and(|max_recoveries| {
                                (2..=MAX_RESPONSE_NON_CONTRACT_RECOVERIES).contains(&max_recoveries)
                                    && max_recoveries >= min_generation as u64
                            })
                        && marker.get("facts_purged").and_then(Value::as_bool) == Some(false)
                        && marker
                            .get("source_stage_execution_id")
                            .and_then(Value::as_str)
                            .and_then(|value| Uuid::parse_str(value).ok())
                            == source_stage_execution_id
                        && marker
                            .get("replacement_stage_execution_id")
                            .and_then(Value::as_str)
                            .and_then(|value| Uuid::parse_str(value).ok())
                            == Some(request.stage_execution_id)
                        && marker.get("source_unit_generation").and_then(Value::as_i64)
                            == Some(i64::from(min_generation - 1))
                        && marker
                            .get("replacement_unit_generation")
                            .and_then(Value::as_i64)
                            == Some(i64::from(min_generation)) =>
                {
                    min_generation as u64
                }
                None if min_generation == 1
                    && source_stage_execution_id.is_some_and(|source| {
                        Uuid::new_v5(
                            &source,
                            b"application-understanding-response-non-contract-recovery:v1",
                        ) == request.stage_execution_id
                    }) =>
                {
                    1
                }
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    if prior_recovery_count >= MAX_RESPONSE_NON_CONTRACT_RECOVERIES {
        return Ok(None);
    }
    let recovery_count = prior_recovery_count + 1;

    let replacement_stage_execution_id = Uuid::new_v5(
        &request.stage_execution_id,
        format!("application-understanding-response-non-contract-recovery:v{recovery_count}")
            .as_bytes(),
    );
    runtime_memory_tx::recover_exhausted_application_understanding_checkpoint(
        pool,
        &runtime_memory_tx::RecoverExhaustedApplicationUnderstandingCheckpointRow {
            operation_id: request.operation_id,
            expected_stage_execution_id: request.stage_execution_id,
            replacement_stage_execution_id,
            recovery_count: i32::try_from(recovery_count).expect("bounded recovery count fits i32"),
        },
    )
    .await?;
    tracing::warn!(
        target: "harness::application_understanding",
        operation_id = %request.operation_id,
        source_stage_execution_id = %request.stage_execution_id,
        %replacement_stage_execution_id,
        recovery_count,
        "replaced exact response-non-contract exhausted Application Understanding runtime without purging facts"
    );
    Ok(Some(replacement_stage_execution_id))
}

async fn ensure_application_team_runtimes(
    pool: &PgPool,
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
) -> Result<EnsuredApplicationTeamRuntimes, ApplicationUnderstandingRuntimeError> {
    let legacy_direct_workers: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
             FROM stage_worker_runs AS worker
             JOIN stage_run_units AS unit ON unit.id=worker.stage_run_unit_id
             LEFT JOIN stage_team_plans AS plan ON plan.stage_run_unit_id=unit.id
            WHERE worker.operation_id=$1 AND worker.stage_execution_id=$2
              AND unit.stage_kind='application_understanding'
              AND plan.id IS NULL AND worker.work_item_id IS NULL"#,
    )
    .bind(request.operation_id)
    .bind(request.stage_execution_id)
    .fetch_one(pool)
    .await?;
    if legacy_direct_workers > 0 {
        let recovered = runtime_memory_tx::recover_legacy_direct_application_understanding_runtime(
            pool,
            &runtime_memory_tx::RecoverLegacyDirectApplicationUnderstandingRuntimeRow {
                operation_id: request.operation_id,
                expected_stage_execution_id: request.stage_execution_id,
            },
        )
        .await?;
        return Ok(EnsuredApplicationTeamRuntimes::LegacyReplaced(
            recovered.replacement_stage_execution_id,
        ));
    }

    if let Some(replacement_stage_execution_id) =
        recover_exhausted_application_team_runtime(pool, request).await?
    {
        return Ok(EnsuredApplicationTeamRuntimes::ExhaustedRuntimeReplaced(
            replacement_stage_execution_id,
        ));
    }

    let existing_plan_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM stage_team_plans WHERE operation_id=$1 AND stage_execution_id=$2",
    )
    .bind(request.operation_id)
    .bind(request.stage_execution_id)
    .fetch_one(pool)
    .await?;
    if existing_plan_count == 0 {
        runtime_memory_tx::prepare_application_understanding_units(
            pool,
            &runtime_memory_tx::PrepareApplicationUnderstandingUnitsRow {
                operation_id: request.operation_id,
                stage_execution_id: request.stage_execution_id,
            },
        )
        .await?;
    }
    let units =
        stage_run_units::list_for_execution(pool, request.operation_id, request.stage_execution_id)
            .await?;
    let mut teams = Vec::with_capacity(units.len());
    for mut unit in units {
        let scope_hash: String = sqlx::query_scalar(
            "SELECT scope_hash FROM operation_org_scope_snapshots WHERE id=$1 AND operation_id=$2",
        )
        .bind(unit.scope_snapshot_id)
        .bind(request.operation_id)
        .fetch_one(pool)
        .await?;
        if let Some(mut plan) = stage_teams::get_plan_for_unit_with_executor(pool, unit.id).await? {
            if unit.status == "gate_blocked"
                && runtime_memory_tx::recover_terminalized_application_model_finalizer(
                    pool,
                    &runtime_memory_tx::RecoverTerminalizedApplicationModelFinalizerRow {
                        operation_id: request.operation_id,
                        stage_execution_id: request.stage_execution_id,
                        stage_run_unit_id: unit.id,
                        stage_team_plan_id: plan.id,
                    },
                )
                .await?
            {
                tracing::warn!(
                    target: "harness::application_understanding",
                    operation_id = %request.operation_id,
                    stage_execution_id = %request.stage_execution_id,
                    stage_run_unit_id = %unit.id,
                    organization_id = %unit.organization_id,
                    stage_team_plan_id = %plan.id,
                    "requeued terminalized Application Model finalizer from exact finished submission"
                );
                unit = stage_run_units::get(pool, unit.id)
                    .await?
                    .ok_or_else(|| producer_error("application_model_recovered_unit_missing"))?;
                plan = stage_teams::get_plan_for_unit_with_executor(pool, unit.id)
                    .await?
                    .ok_or_else(|| producer_error("application_model_recovered_plan_missing"))?;
            }
            let work_items = stage_teams::list_work_items_with_executor(pool, plan.id).await?;
            teams.push(ApplicationTeamRuntime {
                unit,
                plan,
                work_items,
                scope_hash,
            });
        } else {
            teams.push(seed_one_application_team(pool, request, &unit, &scope_hash).await?);
        }
    }
    Ok(EnsuredApplicationTeamRuntimes::Ready(teams))
}

fn stage_team_claim_input(
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
    team: &ApplicationTeamRuntime,
    phase: &str,
) -> runtime_memory_tx::ClaimStageWorkItemRow {
    runtime_memory_tx::ClaimStageWorkItemRow {
        operation_id: request.operation_id,
        stage_execution_id: request.stage_execution_id,
        stage_run_unit_id: team.unit.id,
        stage_team_plan_id: team.plan.id,
        exact_work_item_id: None,
        lease_owner: format!(
            "application-understanding:{}:{}",
            request.operation_id, team.unit.organization_id
        ),
        lease_seconds: 300,
        session_id: request.session_id,
        subtask_id: None,
        agent: golish_db::models::AgentType::Pentester,
        model: None,
        provider: None,
        parent_chain_id: None,
        initial_chain: json!([]),
        initial_checkpoint: json!({
            "schema_version": 1,
            "phase": phase,
        }),
    }
}

fn claimed_fence_at_checkpoint(
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
    checkpoint_version: i64,
) -> Result<RuntimeMemoryTxFence, ApplicationUnderstandingRuntimeError> {
    let lease_token = claimed
        .worker
        .lease_token
        .ok_or_else(|| producer_error("application_model_worker_lease_missing"))?;
    if checkpoint_version < claimed.worker.checkpoint_version {
        return Err(producer_error(
            "application_model_agent_checkpoint_version_regressed",
        ));
    }
    Ok(RuntimeMemoryTxFence {
        operation_id: claimed.plan.operation_id,
        stage_execution_id: claimed.plan.stage_execution_id,
        stage_run_unit_id: claimed.plan.stage_run_unit_id,
        worker_run_id: claimed.worker.id,
        lease_token,
        attempt_epoch: claimed.worker.attempt_epoch,
        expected_checkpoint_version: checkpoint_version,
    })
}

fn application_model_agent_binding(
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
    lead: bool,
) -> Result<
    golish_agent_kit::task_orchestrator::ApplicationModelAgentBinding,
    ApplicationUnderstandingRuntimeError,
> {
    if claimed.plan.operation_id != request.operation_id
        || claimed.plan.stage_execution_id != request.stage_execution_id
        || claimed.unit.id != claimed.plan.stage_run_unit_id
        || claimed.unit.organization_id != claimed.plan.organization_id
        || claimed.work_item.team_plan_id != claimed.plan.id
        || claimed.work_item.stage_run_unit_id != claimed.plan.stage_run_unit_id
        || claimed.work_item.organization_id != claimed.plan.organization_id
        || claimed.worker.work_item_id != Some(claimed.work_item.id)
        || claimed.worker.stage_run_unit_id != claimed.plan.stage_run_unit_id
        || claimed.worker.organization_id != claimed.plan.organization_id
        || claimed.worker.message_chain_id != Some(claimed.message_chain_id)
    {
        return Err(producer_error(
            "application_model_agent_binding_identity_mismatch",
        ));
    }
    let lease_token = claimed
        .worker
        .lease_token
        .ok_or_else(|| producer_error("application_model_worker_lease_missing"))?;
    let lane = if lead { "lead" } else { "worker" };
    Ok(
        golish_agent_kit::task_orchestrator::ApplicationModelAgentBinding {
            operation_id: claimed.plan.operation_id,
            stage_execution_id: claimed.plan.stage_execution_id,
            stage_run_unit_id: claimed.plan.stage_run_unit_id,
            stage_team_plan_id: claimed.plan.id,
            work_item_id: claimed.work_item.id,
            worker_run_id: claimed.worker.id,
            organization_id: claimed.plan.organization_id,
            work_item_key: claimed.work_item.stable_key.clone(),
            work_item_kind: claimed.work_item.kind.clone(),
            work_item_role: claimed.work_item.role.clone(),
            lease_token,
            attempt_epoch: claimed.worker.attempt_epoch,
            session_id: request.session_id,
            message_chain_id: claimed.message_chain_id,
            checkpoint_version: claimed.worker.checkpoint_version,
            checkpoint_body: claimed.worker.checkpoint.clone(),
            parent_request_id: format!(
                "{}::team::{}::{lane}:{}",
                request.stage_run_parent_request_id,
                claimed.plan.organization_id,
                claimed.worker.id
            ),
        },
    )
}

fn validate_application_model_agent_checkpoint(
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
    checkpoint_version: i64,
    checkpoint_body: &Value,
) -> Result<(), ApplicationUnderstandingRuntimeError> {
    if checkpoint_version < claimed.worker.checkpoint_version
        || !(checkpoint_body.is_array() || checkpoint_body.is_object())
    {
        return Err(producer_error("application_model_agent_checkpoint_invalid"));
    }
    Ok(())
}

async fn application_model_agent_checkpoint_is_latest(
    pool: &PgPool,
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
    checkpoint_version: i64,
    checkpoint_body: &Value,
) -> Result<bool, ApplicationUnderstandingRuntimeError> {
    let Some(lease_token) = claimed.worker.lease_token else {
        return Ok(false);
    };
    Ok(sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM stage_worker_runs
                WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
                  AND stage_run_unit_id=$4 AND work_item_id=$5
                  AND organization_id=$6 AND lease_token=$7 AND attempt_epoch=$8
                  AND checkpoint_version=$9 AND checkpoint=$10 AND status='running'
                  AND active_tool_call_id IS NULL
           )"#,
    )
    .bind(claimed.worker.id)
    .bind(claimed.plan.operation_id)
    .bind(claimed.plan.stage_execution_id)
    .bind(claimed.plan.stage_run_unit_id)
    .bind(claimed.work_item.id)
    .bind(claimed.plan.organization_id)
    .bind(lease_token)
    .bind(claimed.worker.attempt_epoch)
    .bind(checkpoint_version)
    .bind(checkpoint_body)
    .fetch_one(pool)
    .await?)
}

async fn load_latest_application_model_agent_checkpoint(
    pool: &PgPool,
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
) -> Result<(i64, Value), ApplicationUnderstandingRuntimeError> {
    let lease_token = claimed
        .worker
        .lease_token
        .ok_or_else(|| producer_error("application_model_worker_lease_missing"))?;
    sqlx::query_as::<_, (i64, Value)>(
        r#"SELECT checkpoint_version,checkpoint FROM stage_worker_runs
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND work_item_id=$5
              AND organization_id=$6 AND lease_token=$7 AND attempt_epoch=$8
              AND status='running' AND active_tool_call_id IS NULL"#,
    )
    .bind(claimed.worker.id)
    .bind(claimed.plan.operation_id)
    .bind(claimed.plan.stage_execution_id)
    .bind(claimed.plan.stage_run_unit_id)
    .bind(claimed.work_item.id)
    .bind(claimed.plan.organization_id)
    .bind(lease_token)
    .bind(claimed.worker.attempt_epoch)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| producer_error("application_model_agent_latest_checkpoint_missing"))
}

fn application_work_item_input(
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
) -> Result<
    golish_agent_kit::task_orchestrator::ApplicationModelWorkItemInputContract,
    ApplicationUnderstandingRuntimeError,
> {
    let authority = claimed
        .work_item
        .input_refs
        .as_array()
        .filter(|refs| refs.len() == 1)
        .and_then(|refs| refs.first())
        .cloned()
        .ok_or_else(|| producer_error("application_model_work_item_input_missing"))?;
    let authority = serde_json::from_value::<ApplicationWorkItemInputAuthority>(authority)
        .map_err(|_| producer_error("application_model_work_item_input_non_contract"))?;
    if authority.schema != "application_model_work_item_input.v1"
        || authority.organization_id != claimed.plan.organization_id
        || authority.work_item_key != claimed.work_item.stable_key
        || authority.work_item_kind != claimed.work_item.kind
    {
        return Err(producer_error(
            "application_model_work_item_input_identity_mismatch",
        ));
    }
    let input = serde_json::from_value::<
        golish_agent_kit::task_orchestrator::ApplicationModelWorkItemInputContract,
    >(json!({
        "operation_id": claimed.plan.operation_id,
        "manifest_id": authority.manifest_id,
        "organization_id": authority.organization_id,
        "stage_run_unit_id": claimed.plan.stage_run_unit_id,
        "work_item_id": claimed.work_item.id,
        "work_item_key": authority.work_item_key,
        "work_item_kind": authority.work_item_kind,
        "projection_hash": authority.projection_hash,
        "projection": authority.projection,
    }))
    .map_err(|_| producer_error("application_model_work_item_input_non_contract"))?;
    let mut source_input_keys = input
        .projection
        .manifest_inputs
        .iter()
        .map(|reference| reference.input_key.clone())
        .collect::<Vec<_>>();
    source_input_keys.sort();
    source_input_keys.dedup();
    let mut evidence_ids = input
        .projection
        .manifest_inputs
        .iter()
        .flat_map(|reference| reference.evidence_ids.iter().copied())
        .collect::<Vec<_>>();
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    if !authority.manifest_hash.starts_with("sha256:")
        || authority.manifest_hash.len() != 71
        || authority.source_input_keys != source_input_keys
        || authority.evidence_ids != evidence_ids
    {
        return Err(producer_error(
            "application_model_work_item_input_authority_mismatch",
        ));
    }
    Ok(input)
}

async fn retry_application_work_item(
    pool: &PgPool,
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
    failure_code: &str,
    checkpoint_version: i64,
    checkpoint_body: Value,
) -> Result<bool, ApplicationUnderstandingRuntimeError> {
    validate_application_model_agent_checkpoint(claimed, checkpoint_version, &checkpoint_body)?;
    let retried = stage_teams::retry_stage_worker(
        pool,
        stage_teams::RetryStageWorkerRow {
            fence: claimed_fence_at_checkpoint(claimed, checkpoint_version)?,
            team_plan_id: claimed.plan.id,
            work_item_id: claimed.work_item.id,
            expected_work_item_row_version: claimed.work_item.row_version,
            failure_code: failure_code.to_string(),
            terminal_checkpoint: json!({
                "chain": checkpoint_body,
                "stage_team_execution_failure": {
                    "code": failure_code,
                    "schema_version": 1,
                    "stage": "application_understanding",
                },
            }),
        },
    )
    .await?;
    Ok(retried.retry_scheduled)
}

async fn retry_latest_application_work_item(
    pool: &PgPool,
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
    failure_code: &str,
) -> Result<bool, ApplicationUnderstandingRuntimeError> {
    let (checkpoint_version, checkpoint_body) =
        load_latest_application_model_agent_checkpoint(pool, claimed).await?;
    retry_application_work_item(
        pool,
        claimed,
        failure_code,
        checkpoint_version,
        checkpoint_body,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplicationModelAggregatorFailureDisposition {
    RetryScheduled,
    Exhausted,
}

impl ApplicationModelAggregatorFailureDisposition {
    const fn as_str(self) -> &'static str {
        match self {
            Self::RetryScheduled => "retry_scheduled",
            Self::Exhausted => "exhausted",
        }
    }
}

/// Consume one claimed synthesizer attempt without publishing model authority.
///
/// The ordinary producer retry helper intentionally rejects the Team
/// aggregator. This transaction therefore fences the exact final submitter,
/// samples its latest durable checkpoint, and either requeues the same
/// WorkerRun/message chain or terminalizes the WorkItem and Unit together.
async fn fail_application_model_aggregator_attempt(
    pool: &PgPool,
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
    failure_code: &str,
) -> Result<ApplicationModelAggregatorFailureDisposition, ApplicationUnderstandingRuntimeError> {
    let failure_code = failure_code.trim();
    if failure_code.is_empty() || failure_code.len() > 128 {
        return Err(producer_error(
            "application_model_synthesis_failure_code_invalid",
        ));
    }
    let lease_token = claimed
        .worker
        .lease_token
        .ok_or_else(|| producer_error("application_model_worker_lease_missing"))?;
    let mut tx = pool.begin().await?;
    let operation = sqlx::query_as::<_, (Option<Uuid>, String, String)>(
        "SELECT superseded_by,runtime_memory_contract,current_stage
           FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(claimed.plan.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| producer_error("application_model_synthesis_operation_missing"))?;
    if operation.0.is_some()
        || operation.1 != "v2_only"
        || operation.2 != "application_understanding"
    {
        return Err(producer_error(
            "application_model_synthesis_operation_authority_mismatch",
        ));
    }
    let unit = sqlx::query_as::<_, stage_run_units::StageRunUnitRow>(
        "SELECT * FROM stage_run_units
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3 FOR UPDATE",
    )
    .bind(claimed.unit.id)
    .bind(claimed.plan.operation_id)
    .bind(claimed.plan.stage_execution_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| producer_error("application_model_synthesis_unit_missing"))?;
    let plan = sqlx::query_as::<_, stage_teams::StageTeamPlanRow>(
        "SELECT * FROM stage_team_plans
          WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
            AND stage_run_unit_id=$4 FOR UPDATE",
    )
    .bind(claimed.plan.id)
    .bind(claimed.plan.operation_id)
    .bind(claimed.plan.stage_execution_id)
    .bind(claimed.plan.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| producer_error("application_model_synthesis_plan_missing"))?;
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        "SELECT * FROM stage_work_items WHERE id=$1 AND team_plan_id=$2 FOR UPDATE",
    )
    .bind(claimed.work_item.id)
    .bind(claimed.plan.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| producer_error("application_model_synthesis_work_item_missing"))?;
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        "SELECT * FROM stage_worker_runs WHERE id=$1 FOR UPDATE",
    )
    .bind(claimed.worker.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| producer_error("application_model_synthesis_worker_missing"))?;
    if unit.status != "running"
        || unit.organization_id != plan.organization_id
        || plan.requests_closed_at.is_none()
        || plan.final_submitter_kind != "worker"
        || plan.aggregator_kind != "worker"
        || plan.final_submitter_worker_run_id != Some(worker.id)
        || plan.aggregator_role.as_deref() != Some(item.role.as_str())
        || item.operation_id != plan.operation_id
        || item.stage_execution_id != plan.stage_execution_id
        || item.stage_run_unit_id != plan.stage_run_unit_id
        || item.organization_id != plan.organization_id
        || item.stable_key != "leader:primary"
        || item.required_for_barrier
        || item.status != "running"
        || item.row_version != claimed.work_item.row_version
        || worker.work_item_id != Some(item.id)
        || worker.stage_run_unit_id != plan.stage_run_unit_id
        || worker.organization_id != plan.organization_id
        || worker.message_chain_id != Some(claimed.message_chain_id)
        || worker.status != "running"
        || worker.lease_token != Some(lease_token)
        || worker.attempt_epoch != claimed.worker.attempt_epoch
        || worker.active_tool_call_id.is_some()
    {
        return Err(producer_error(
            "application_model_synthesis_failure_fence_mismatch",
        ));
    }
    if !(worker.checkpoint.is_array() || worker.checkpoint.is_object()) {
        return Err(producer_error(
            "application_model_synthesis_checkpoint_invalid",
        ));
    }
    let submission_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM stage_deliverable_submissions
          WHERE operation_id=$1 AND stage_execution_id=$2 AND stage_run_unit_id=$3
            AND worker_run_id=$4 AND attempt_epoch=$5 AND lease_token=$6",
    )
    .bind(plan.operation_id)
    .bind(plan.stage_execution_id)
    .bind(plan.stage_run_unit_id)
    .bind(worker.id)
    .bind(worker.attempt_epoch)
    .bind(lease_token)
    .fetch_one(&mut *tx)
    .await?;
    if submission_count != 0 {
        return Err(producer_error(
            "application_model_synthesis_failure_after_submission_forbidden",
        ));
    }
    let max_attempts = item
        .attempt_policy
        .get("max_attempts")
        .and_then(Value::as_i64)
        .filter(|max_attempts| (1..=32).contains(max_attempts))
        .ok_or_else(|| producer_error("application_model_synthesis_attempt_policy_invalid"))?;
    let attempts_used: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(GREATEST(attempt_epoch, 1)), 0)::BIGINT
           FROM stage_worker_runs WHERE work_item_id=$1",
    )
    .bind(item.id)
    .fetch_one(&mut *tx)
    .await?;
    let retry_scheduled = attempts_used < max_attempts;
    let next_worker_status = if retry_scheduled { "queued" } else { "failed" };
    let terminal_checkpoint = json!({
        "chain": worker.checkpoint,
        "stage_team_execution_failure": {
            "attempts_used": attempts_used,
            "code": failure_code,
            "max_attempts": max_attempts,
            "schema_version": 1,
            "stage": "application_understanding_synthesis",
        },
    });
    let worker = sqlx::query_as::<_, stage_worker_runs::StageWorkerRunRow>(
        r#"UPDATE stage_worker_runs
              SET status=$9,checkpoint=$10,checkpoint_version=checkpoint_version+1,
                  lease_token=NULL,lease_owner=NULL,lease_acquired_at=NULL,
                  lease_expires_at=NULL,heartbeat_at=NULL,
                  terminal_at=CASE WHEN $9='queued' THEN NULL ELSE NOW() END,
                  updated_at=NOW()
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND work_item_id=$5 AND lease_token=$6
              AND attempt_epoch=$7 AND checkpoint_version=$8 AND status='running'
              AND active_tool_call_id IS NULL
            RETURNING *"#,
    )
    .bind(worker.id)
    .bind(worker.operation_id)
    .bind(worker.stage_execution_id)
    .bind(worker.stage_run_unit_id)
    .bind(item.id)
    .bind(lease_token)
    .bind(worker.attempt_epoch)
    .bind(worker.checkpoint_version)
    .bind(next_worker_status)
    .bind(&terminal_checkpoint)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| producer_error("application_model_synthesis_failure_lease_lost"))?;
    let next_item_status = if retry_scheduled {
        "retry_pending"
    } else {
        "exhausted"
    };
    let item = sqlx::query_as::<_, stage_teams::StageWorkItemRow>(
        r#"UPDATE stage_work_items
              SET status=$4,row_version=row_version+1,
                  terminal_at=CASE WHEN $4='exhausted' THEN NOW() ELSE NULL END,
                  updated_at=NOW()
            WHERE id=$1 AND team_plan_id=$2 AND status='running' AND row_version=$3
            RETURNING *"#,
    )
    .bind(item.id)
    .bind(plan.id)
    .bind(item.row_version)
    .bind(next_item_status)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| producer_error("application_model_synthesis_failure_item_stale"))?;
    if retry_scheduled {
        let queued = sqlx::query(
            "UPDATE stage_work_items
                SET status='queued',row_version=row_version+1,terminal_at=NULL,updated_at=NOW()
              WHERE id=$1 AND team_plan_id=$2 AND status='retry_pending' AND row_version=$3",
        )
        .bind(item.id)
        .bind(plan.id)
        .bind(item.row_version)
        .execute(&mut *tx)
        .await?;
        if queued.rows_affected() != 1 {
            return Err(producer_error(
                "application_model_synthesis_failure_requeue_stale",
            ));
        }
    } else {
        stage_run_units::transition_cas(
            &mut *tx,
            unit.id,
            unit.operation_id,
            unit.stage_execution_id,
            unit.organization_id,
            stage_run_units::StageRunUnitStatus::Running,
            unit.row_version,
            stage_run_units::StageRunUnitStatus::GateBlocked,
            Some(&terminal_checkpoint),
        )
        .await?;
    }
    debug_assert_eq!(worker.message_chain_id, Some(claimed.message_chain_id));
    tx.commit().await?;
    Ok(if retry_scheduled {
        ApplicationModelAggregatorFailureDisposition::RetryScheduled
    } else {
        ApplicationModelAggregatorFailureDisposition::Exhausted
    })
}

async fn execute_application_team_children(
    pool: &PgPool,
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
    team: &ApplicationTeamRuntime,
    runner: &dyn golish_agent_kit::task_orchestrator::ApplicationModelAgentRunner,
) -> Result<Option<(String, Vec<String>)>, ApplicationUnderstandingRuntimeError> {
    let mut exhausted_refs = Vec::new();
    loop {
        let Some(claimed) = runtime_memory_tx::claim_stage_work_item(
            pool,
            &stage_team_claim_input(request, team, "application_model_work_item"),
        )
        .await?
        else {
            break;
        };
        let input = match application_work_item_input(&claimed) {
            Ok(input) => input,
            Err(_) => {
                let failure_code = "application_model_work_item_input_invalid";
                if retry_latest_application_work_item(pool, &claimed, failure_code).await? {
                    continue;
                }
                exhausted_refs.extend([
                    format!("organization:{}", team.unit.organization_id),
                    format!("work_item:{}", claimed.work_item.stable_key),
                    format!("failure_code:{failure_code}"),
                ]);
                continue;
            }
        };
        tracing::info!(
            target: "harness::application_understanding",
            operation_id = %request.operation_id,
            stage_execution_id = %request.stage_execution_id,
            organization_id = %team.unit.organization_id,
            team_plan_id = %team.plan.id,
            work_item_id = %claimed.work_item.id,
            work_item_key = %claimed.work_item.stable_key,
            work_item_kind = %claimed.work_item.kind,
            projection_hash = %input.projection_hash,
            "claimed closed Application Understanding Modeler work item"
        );
        let binding = match application_model_agent_binding(request, &claimed, false) {
            Ok(binding) => binding,
            Err(_) => {
                let failure_code = "application_model_agent_binding_invalid";
                if retry_latest_application_work_item(pool, &claimed, failure_code).await? {
                    continue;
                }
                exhausted_refs.extend([
                    format!("organization:{}", team.unit.organization_id),
                    format!("work_item:{}", claimed.work_item.stable_key),
                    format!("failure_code:{failure_code}"),
                ]);
                continue;
            }
        };
        let attempt = match runner.run_work_item(binding, input.clone()).await {
            Ok(attempt) => attempt,
            Err(error) => {
                let failure_code = classify_application_model_producer_error(&error);
                if retry_latest_application_work_item(pool, &claimed, failure_code).await? {
                    continue;
                }
                exhausted_refs.extend([
                    format!("organization:{}", team.unit.organization_id),
                    format!("work_item:{}", claimed.work_item.stable_key),
                    format!("failure_code:{failure_code}"),
                ]);
                continue;
            }
        };
        let (checkpoint_version, checkpoint_body) =
            load_latest_application_model_agent_checkpoint(pool, &claimed).await?;
        if validate_application_model_agent_checkpoint(
            &claimed,
            attempt.checkpoint_version,
            &attempt.checkpoint_body,
        )
        .is_err()
            || attempt.checkpoint_version != checkpoint_version
            || attempt.checkpoint_body != checkpoint_body
        {
            let failure_code = "application_model_agent_checkpoint_invalid";
            if retry_application_work_item(
                pool,
                &claimed,
                failure_code,
                checkpoint_version,
                checkpoint_body,
            )
            .await?
            {
                continue;
            }
            exhausted_refs.extend([
                format!("organization:{}", team.unit.organization_id),
                format!("work_item:{}", claimed.work_item.stable_key),
                format!("failure_code:{failure_code}"),
            ]);
            continue;
        }
        let output = match attempt.outcome {
            golish_agent_kit::task_orchestrator::ApplicationModelAgentOutcome::Completed(
                output,
            ) => output,
            golish_agent_kit::task_orchestrator::ApplicationModelAgentOutcome::Failed(failure) => {
                let failure_code = failure.code();
                if retry_application_work_item(
                    pool,
                    &claimed,
                    failure_code,
                    checkpoint_version,
                    checkpoint_body,
                )
                .await?
                {
                    continue;
                }
                exhausted_refs.extend([
                    format!("organization:{}", team.unit.organization_id),
                    format!("work_item:{}", claimed.work_item.stable_key),
                    format!("failure_code:{failure_code}"),
                ]);
                continue;
            }
        };
        if let Err(error) = output.validate_against(&input) {
            let failure_code = error.code();
            if retry_application_work_item(
                pool,
                &claimed,
                failure_code,
                checkpoint_version,
                checkpoint_body.clone(),
            )
            .await?
            {
                continue;
            }
            exhausted_refs.extend([
                format!("organization:{}", team.unit.organization_id),
                format!("work_item:{}", claimed.work_item.stable_key),
                format!("failure_code:{failure_code}"),
            ]);
            continue;
        }
        let canonical_output = serde_json::to_value(&output)
            .map_err(|_| producer_error("application_model_work_item_output_serialize"))?;
        let mut evidence_ids = output
            .items
            .iter()
            .flat_map(|item| item.evidence.iter().map(|evidence| evidence.evidence_id))
            .collect::<Vec<_>>();
        evidence_ids.sort_unstable();
        evidence_ids.dedup();
        if evidence_ids.is_empty() {
            let failure_code = "application_model_work_item_evidence_missing";
            if retry_application_work_item(
                pool,
                &claimed,
                failure_code,
                checkpoint_version,
                checkpoint_body.clone(),
            )
            .await?
            {
                continue;
            }
            exhausted_refs.extend([
                format!("organization:{}", team.unit.organization_id),
                format!("work_item:{}", claimed.work_item.stable_key),
                format!("failure_code:{failure_code}"),
            ]);
            continue;
        }
        let business_disposition = if output.items.is_empty() && output.unknowns.is_empty() {
            "checked_empty"
        } else {
            "found"
        };
        let evidence_watermark = evidence_ids.iter().copied().max();
        let checked_empty_cells = if business_disposition == "checked_empty" {
            json!([{
                "kind": "application_model_projection",
                "projection_hash": input.projection_hash,
                "work_item_key": claimed.work_item.stable_key,
            }])
        } else {
            json!([])
        };
        let mut completion = stage_teams::CompleteStageWorkerRow {
            fence: claimed_fence_at_checkpoint(&claimed, checkpoint_version)?,
            team_plan_id: claimed.plan.id,
            work_item_id: claimed.work_item.id,
            expected_work_item_row_version: claimed.work_item.row_version,
            output_schema: claimed.work_item.output_schema.clone(),
            business_disposition: business_disposition.to_string(),
            canonical_output,
            canonical_fact_refs: json!([]),
            evidence_ids,
            checked_empty_cells,
            blocker_codes: Vec::new(),
            output_hash: String::new(),
            terminal_checkpoint: json!({
                "chain": checkpoint_body,
                "application_model_work_item_completion": {
                    "schema_version": 1,
                    "phase": "application_model_work_item_completed",
                    "work_item_id": claimed.work_item.id,
                    "projection_hash": input.projection_hash,
                },
            }),
            evidence_watermark,
        };
        completion.output_hash = stage_teams::canonical_stage_worker_output_hash(&completion);
        stage_teams::complete_stage_worker(pool, completion).await?;
    }
    if exhausted_refs.is_empty() {
        Ok(None)
    } else {
        exhausted_refs.sort();
        exhausted_refs.dedup();
        Ok(Some((
            "APPLICATION_MODEL_WORK_ITEM_ATTEMPTS_EXHAUSTED".to_string(),
            exhausted_refs,
        )))
    }
}

fn synthesis_input(
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
    team: &ApplicationTeamRuntime,
    manifest: &ApplicationModelManifestRow,
    inputs: &[ApplicationModelManifestInputRow],
    outputs: &[stage_teams::StageWorkerOutputRow],
) -> Result<
    golish_agent_kit::task_orchestrator::ApplicationModelSynthesisInputContract,
    ApplicationUnderstandingRuntimeError,
> {
    let mut producer_items = team
        .work_items
        .iter()
        .filter(|item| item.required_for_barrier)
        .collect::<Vec<_>>();
    producer_items.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
    let outputs_by_item = outputs
        .iter()
        .map(|output| (output.work_item_id, output))
        .collect::<std::collections::BTreeMap<_, _>>();
    let expected_work_items = producer_items
        .iter()
        .map(|item| {
            let authority = item
                .input_refs
                .as_array()
                .and_then(|refs| refs.first())
                .cloned()
                .and_then(|value| {
                    serde_json::from_value::<ApplicationWorkItemInputAuthority>(value).ok()
                })
                .ok_or_else(|| {
                    producer_error("application_model_synthesis_item_authority_missing")
                })?;
            Ok(json!({
                "work_item_id": item.id,
                "work_item_key": item.stable_key,
                "projection_hash": authority.projection_hash,
            }))
        })
        .collect::<Result<Vec<_>, ApplicationUnderstandingRuntimeError>>()?;
    let partial_outputs = producer_items
        .iter()
        .map(|item| {
            let output = outputs_by_item.get(&item.id).ok_or_else(|| {
                producer_error("application_model_synthesis_partial_output_missing")
            })?;
            if output.business_disposition == "blocked" {
                return Err(producer_error(
                    "application_model_synthesis_partial_output_blocked",
                ));
            }
            serde_json::from_value::<
                golish_agent_kit::task_orchestrator::ApplicationModelWorkItemOutputContract,
            >(output.canonical_output.clone())
            .map_err(|_| producer_error("application_model_synthesis_partial_output_invalid"))
        })
        .collect::<Result<Vec<_>, ApplicationUnderstandingRuntimeError>>()?;
    serde_json::from_value(json!({
        "operation_id": request.operation_id,
        "manifest_id": manifest.id,
        "organization_id": manifest.organization_id,
        "stage_run_unit_id": team.unit.id,
        "manifest_inputs": application_manifest_input_refs(inputs)?,
        "expected_work_items": expected_work_items,
        "partial_outputs": partial_outputs,
    }))
    .map_err(|_| producer_error("application_model_synthesis_input_non_contract"))
}

struct PreparedApplicationModelSynthesisProducer {
    proposal: Option<ApplicationModelProposalDraft>,
}

#[async_trait::async_trait]
impl ApplicationModelProposalProducer for PreparedApplicationModelSynthesisProducer {
    async fn produce(
        &self,
        _legacy_input: ApplicationModelProducerInput,
    ) -> Result<ApplicationModelProposalDraft, ApplicationUnderstandingRuntimeError> {
        self.proposal
            .clone()
            .ok_or_else(|| producer_error("application_model_synthesis_proposal_missing"))
    }
}

/// Park an exact Company Controller finalizer after it has durably submitted
/// its Application Model but deterministic publication did not finish.
///
/// The receipt is the boundary between producer work and closeout work. Once
/// it exists, a database/Gate failure must not consume synthesizer attempt
/// fuel or leave the WorkerRun live until a lease reaper misclassifies it as a
/// producer exhaustion. The same WorkerRun/message chain is requeued and the
/// persisted proposal is reauthorized under the next exact fence.
async fn park_application_model_finalizer_after_submitted_closeout_failure(
    pool: &PgPool,
    claimed: &runtime_memory_tx::ClaimedStageWorkItemRow,
    closed: &runtime_memory_tx::ClosedStageRequestEpochRow,
    failure_detail: &str,
) -> Result<bool, ApplicationUnderstandingRuntimeError> {
    let lease_token = claimed
        .worker
        .lease_token
        .ok_or_else(|| producer_error("application_model_worker_lease_missing"))?;
    let deliverable_submission_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT submission.id
             FROM stage_deliverable_submissions submission
             JOIN tool_calls tool ON tool.id=submission.tool_call_record_id
            WHERE submission.operation_id=$1
              AND submission.stage_execution_id=$2
              AND submission.stage_run_unit_id=$3
              AND submission.organization_id=$4
              AND submission.worker_run_id=$5
              AND submission.attempt_epoch=$6
              AND submission.lease_token=$7
              AND submission.stage_kind='application_understanding'
              AND tool.status='finished'
            ORDER BY submission.submitted_at DESC,submission.id DESC
            LIMIT 1"#,
    )
    .bind(claimed.plan.operation_id)
    .bind(claimed.plan.stage_execution_id)
    .bind(claimed.plan.stage_run_unit_id)
    .bind(claimed.plan.organization_id)
    .bind(claimed.worker.id)
    .bind(claimed.worker.attempt_epoch)
    .bind(lease_token)
    .fetch_optional(pool)
    .await?;
    let Some(deliverable_submission_id) = deliverable_submission_id else {
        return Ok(false);
    };
    let (checkpoint_version, checkpoint) =
        load_latest_application_model_agent_checkpoint(pool, claimed).await?;
    runtime_memory_tx::park_stage_team_finalizer_after_failure(
        pool,
        &runtime_memory_tx::ParkStageTeamFinalizerAfterFailureRow {
            fence: claimed_fence_at_checkpoint(claimed, checkpoint_version)?,
            stage_team_plan_id: claimed.plan.id,
            leader_work_item_id: claimed.work_item.id,
            deliverable_submission_id,
            expected_work_item_row_version: claimed.work_item.row_version,
            expected_dispatch_epoch: closed.plan.dispatch_epoch,
            expected_manifest_hash: closed.barrier.manifest_hash.clone(),
            checkpoint,
            failure_detail: failure_detail.chars().take(4096).collect(),
        },
    )
    .await?;
    tracing::warn!(
        target: "harness::application_understanding",
        operation_id = %claimed.plan.operation_id,
        stage_execution_id = %claimed.plan.stage_execution_id,
        stage_run_unit_id = %claimed.plan.stage_run_unit_id,
        organization_id = %claimed.plan.organization_id,
        worker_run_id = %claimed.worker.id,
        attempt_epoch = claimed.worker.attempt_epoch,
        deliverable_submission_id = %deliverable_submission_id,
        "parked submitted Application Model finalizer for exact closeout retry"
    );
    Ok(true)
}

async fn run_application_team(
    pool: &PgPool,
    request: &golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
    team: &ApplicationTeamRuntime,
    runner: &dyn golish_agent_kit::task_orchestrator::ApplicationModelAgentRunner,
) -> Result<Option<(String, Vec<String>)>, ApplicationUnderstandingRuntimeError> {
    if team.unit.status == "passed" {
        return Ok(None);
    }
    if team.unit.status == "gate_blocked" {
        return Ok(Some((
            "APPLICATION_MODEL_WORK_ITEM_BLOCKED".to_string(),
            vec![
                format!("organization:{}", team.unit.organization_id),
                format!("team_plan:{}", team.plan.id),
            ],
        )));
    }
    let child_block = execute_application_team_children(pool, request, team, runner).await?;
    let closed = runtime_memory_tx::close_stage_request_epoch(
        pool,
        &runtime_memory_tx::CloseStageRequestEpochRow {
            operation_id: request.operation_id,
            stage_execution_id: request.stage_execution_id,
            stage_run_unit_id: team.unit.id,
            stage_team_plan_id: team.plan.id,
            expected_dispatch_epoch: team.plan.dispatch_epoch,
            expected_plan_row_version: team.plan.row_version,
        },
    )
    .await?;
    if !closed.barrier.ready_to_finalize() {
        return Ok(Some((
            "APPLICATION_MODEL_WORK_ITEM_BARRIER_OPEN".to_string(),
            vec![
                format!("organization:{}", team.unit.organization_id),
                format!("team_plan:{}", team.plan.id),
                format!("manifest_hash:{}", closed.barrier.manifest_hash),
            ],
        )));
    }
    let outputs = stage_teams::list_outputs_with_executor(pool, team.plan.id).await?;
    let required_work_item_ids = team
        .work_items
        .iter()
        .filter(|item| item.required_for_barrier)
        .map(|item| item.id)
        .collect::<BTreeSet<_>>();
    if outputs.iter().any(|output| {
        application_team_output_blocks_child_barrier(
            &required_work_item_ids,
            output.work_item_id,
            &output.business_disposition,
        )
    }) {
        runtime_memory_tx::block_stage_team_unit(
            pool,
            &runtime_memory_tx::BlockStageTeamUnitRow {
                operation_id: request.operation_id,
                stage_execution_id: request.stage_execution_id,
                stage_run_unit_id: team.unit.id,
                stage_team_plan_id: team.plan.id,
                expected_dispatch_epoch: closed.plan.dispatch_epoch,
                expected_manifest_hash: closed.barrier.manifest_hash.clone(),
            },
        )
        .await?;
        return Ok(Some(child_block.unwrap_or_else(|| {
            (
                "APPLICATION_MODEL_WORK_ITEM_BLOCKED".to_string(),
                vec![
                    format!("organization:{}", team.unit.organization_id),
                    format!("team_plan:{}", team.plan.id),
                ],
            )
        })));
    }
    if let Some(block) = child_block {
        return Ok(Some(block));
    }
    let manifest_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM application_model_manifests WHERE stage_run_unit_id=$1",
    )
    .bind(team.unit.id)
    .fetch_one(pool)
    .await?;
    let gate_material = application_models::load_gate_material(
        pool,
        &LoadApplicationModelGateMaterial {
            manifest_id,
            operation_id: request.operation_id,
            scope_snapshot_id: team.unit.scope_snapshot_id,
            stage_execution_id: request.stage_execution_id,
            stage_run_unit_id: team.unit.id,
            organization_id: team.unit.organization_id,
        },
    )
    .await?;
    let synthesis = if gate_material.manifest.authority_kind
        == ApplicationModelAuthorityKindRow::TerminalNoInput.as_str()
    {
        None
    } else {
        Some(synthesis_input(
            request,
            team,
            &gate_material.manifest,
            &gate_material.inputs,
            &outputs,
        )?)
    };
    let (claimed, proposal, checkpoint_version) = loop {
        let claimed = runtime_memory_tx::claim_stage_aggregator(
            pool,
            &runtime_memory_tx::ClaimStageAggregatorRow {
                claim: stage_team_claim_input(request, team, "application_model_synthesis"),
                expected_dispatch_epoch: closed.plan.dispatch_epoch,
                expected_manifest_hash: closed.barrier.manifest_hash.clone(),
            },
        )
        .await?;
        let Some(synthesis) = synthesis.as_ref() else {
            break (claimed.clone(), None, claimed.worker.checkpoint_version);
        };
        let binding = match application_model_agent_binding(request, &claimed, true) {
            Ok(binding) => binding,
            Err(_) => {
                let disposition = fail_application_model_aggregator_attempt(
                    pool,
                    &claimed,
                    "application_model_agent_binding_invalid",
                )
                .await?;
                return Ok(Some((
                    "APPLICATION_MODEL_SYNTHESIS_INFRASTRUCTURE".to_string(),
                    vec![
                        format!("organization:{}", team.unit.organization_id),
                        "failure_code:application_model_agent_binding_invalid".to_string(),
                        format!("worker_run:{}", claimed.worker.id),
                        format!("attempt_disposition:{}", disposition.as_str()),
                    ],
                )));
            }
        };
        let attempt = match runner.run_synthesis(binding, synthesis.clone()).await {
            Ok(attempt) => attempt,
            Err(error) => {
                let failure_code = classify_application_model_producer_error(&error);
                let disposition =
                    fail_application_model_aggregator_attempt(pool, &claimed, failure_code).await?;
                if disposition == ApplicationModelAggregatorFailureDisposition::RetryScheduled {
                    continue;
                }
                return Ok(Some((
                    "APPLICATION_MODEL_SYNTHESIS_INFRASTRUCTURE".to_string(),
                    vec![
                        format!("organization:{}", team.unit.organization_id),
                        format!("failure_code:{failure_code}"),
                        format!("worker_run:{}", claimed.worker.id),
                        format!("attempt_disposition:{}", disposition.as_str()),
                    ],
                )));
            }
        };
        if validate_application_model_agent_checkpoint(
            &claimed,
            attempt.checkpoint_version,
            &attempt.checkpoint_body,
        )
        .is_err()
            || !application_model_agent_checkpoint_is_latest(
                pool,
                &claimed,
                attempt.checkpoint_version,
                &attempt.checkpoint_body,
            )
            .await?
        {
            let failure_code = "application_model_agent_checkpoint_invalid";
            let disposition =
                fail_application_model_aggregator_attempt(pool, &claimed, failure_code).await?;
            return Ok(Some((
                "APPLICATION_MODEL_SYNTHESIS_INFRASTRUCTURE".to_string(),
                vec![
                    format!("organization:{}", team.unit.organization_id),
                    format!("failure_code:{failure_code}"),
                    format!("worker_run:{}", claimed.worker.id),
                    format!("attempt_disposition:{}", disposition.as_str()),
                ],
            )));
        }
        let proposal = match attempt.outcome {
            golish_agent_kit::task_orchestrator::ApplicationModelAgentOutcome::Completed(
                proposal,
            ) => proposal,
            golish_agent_kit::task_orchestrator::ApplicationModelAgentOutcome::Failed(failure) => {
                let disposition =
                    fail_application_model_aggregator_attempt(pool, &claimed, failure.code())
                        .await?;
                if disposition == ApplicationModelAggregatorFailureDisposition::RetryScheduled {
                    continue;
                }
                return Ok(Some((
                    "APPLICATION_MODEL_SYNTHESIS_REWORK".to_string(),
                    vec![
                        format!("organization:{}", team.unit.organization_id),
                        format!("failure_code:{}", failure.code()),
                        format!("worker_run:{}", claimed.worker.id),
                        format!("attempt_disposition:{}", disposition.as_str()),
                    ],
                )));
            }
        };
        let proposal = serde_json::to_value(proposal).ok().and_then(|value| {
            serde_json::from_value::<
                golish_agent_kit::task_orchestrator::ApplicationModelProposalContract,
            >(value)
            .ok()
        });
        let Some(proposal) = proposal else {
            let failure_code = "application_model_response_non_contract";
            let disposition =
                fail_application_model_aggregator_attempt(pool, &claimed, failure_code).await?;
            if disposition == ApplicationModelAggregatorFailureDisposition::RetryScheduled {
                continue;
            }
            return Ok(Some((
                "APPLICATION_MODEL_SYNTHESIS_REWORK".to_string(),
                vec![
                    format!("organization:{}", team.unit.organization_id),
                    format!("failure_code:{failure_code}"),
                    format!("worker_run:{}", claimed.worker.id),
                    format!("attempt_disposition:{}", disposition.as_str()),
                ],
            )));
        };
        break (
            claimed,
            Some(proposal_from_contract(proposal)),
            attempt.checkpoint_version,
        );
    };
    let synthesis_producer = PreparedApplicationModelSynthesisProducer { proposal };
    let command = RunApplicationUnderstandingUnit {
        session_id: request.session_id,
        fence: claimed_fence_at_checkpoint(&claimed, checkpoint_version)?,
        expected_unit_row_version: claimed.unit.row_version,
        scope_hash: team.scope_hash.clone(),
    };
    match run_application_understanding_unit(pool, &command, &synthesis_producer).await {
        Ok(ApplicationUnderstandingRuntimeOutcome::Passed(_)) => Ok(None),
        Ok(ApplicationUnderstandingRuntimeOutcome::Blocked(block)) => {
            let code = block.code.as_str().to_string();
            let mut refs = block.refs;
            if !park_application_model_finalizer_after_submitted_closeout_failure(
                pool,
                &claimed,
                &closed,
                &format!("application model closeout blocked: {code}"),
            )
            .await?
            {
                let disposition = fail_application_model_aggregator_attempt(
                    pool,
                    &claimed,
                    "application_model_closeout_blocked_before_submission",
                )
                .await?;
                refs.push(format!("attempt_disposition:{}", disposition.as_str()));
            }
            refs.push(format!("organization:{}", team.unit.organization_id));
            Ok(Some((code, refs)))
        }
        Err(error) => {
            if !park_application_model_finalizer_after_submitted_closeout_failure(
                pool,
                &claimed,
                &closed,
                &error.to_string(),
            )
            .await?
            {
                fail_application_model_aggregator_attempt(
                    pool,
                    &claimed,
                    "application_model_closeout_failed_before_submission",
                )
                .await?;
            }
            Err(error)
        }
    }
}

#[async_trait::async_trait]
impl golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRuntime
    for PgApplicationUnderstandingStageRuntime
{
    async fn run(
        &self,
        request: golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageRequest,
        runner: &dyn golish_agent_kit::task_orchestrator::ApplicationModelAgentRunner,
    ) -> anyhow::Result<golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageOutcome>
    {
        let operation = sqlx::query_as::<_, (String, String, Option<Uuid>)>(
            r#"SELECT application_model_contract,current_stage,superseded_by
                 FROM operation_state WHERE operation_id=$1"#,
        )
        .bind(request.operation_id)
        .fetch_optional(&*self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("APPLICATION_UNDERSTANDING_OPERATION_MISSING"))?;
        anyhow::ensure!(
            operation.0 == "application_model_v1"
                && operation.1 == "application_understanding"
                && operation.2.is_none()
                && !request.stage_run_parent_request_id.trim().is_empty(),
            "APPLICATION_UNDERSTANDING_OPERATION_AUTHORITY_MISMATCH"
        );
        validate_complete_predecessor_handoffs(&self.pool, &request).await?;

        let teams = match ensure_application_team_runtimes(&self.pool, &request).await? {
            EnsuredApplicationTeamRuntimes::LegacyReplaced(replacement_stage_execution_id) => {
                return Ok(
                    golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageOutcome::Blocked {
                        code: "APPLICATION_UNDERSTANDING_RUNTIME_REPLACED_CONTINUE_REQUIRED"
                            .to_string(),
                        refs: vec![format!(
                            "replacement_stage_execution:{replacement_stage_execution_id}"
                        )],
                    },
                );
            }
            EnsuredApplicationTeamRuntimes::ExhaustedRuntimeReplaced(
                replacement_stage_execution_id,
            ) => {
                return Ok(
                    golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageOutcome::Blocked {
                        code: "APPLICATION_UNDERSTANDING_EXHAUSTED_RUNTIME_RECOVERED_CONTINUE_REQUIRED"
                            .to_string(),
                        refs: vec![format!(
                            "replacement_stage_execution:{replacement_stage_execution_id}"
                        )],
                    },
                );
            }
            EnsuredApplicationTeamRuntimes::Ready(teams) => teams,
        };
        anyhow::ensure!(
            !teams.is_empty(),
            "APPLICATION_UNDERSTANDING_FROZEN_SCOPE_EMPTY"
        );

        let mut company_blocks = Vec::new();
        for team in &teams {
            if let Some((code, mut refs)) =
                run_application_team(&self.pool, &request, team, runner).await?
            {
                refs.sort();
                refs.dedup();
                company_blocks.push((team.unit.organization_id, code, refs));
            }
        }
        if company_blocks.len() == 1 {
            let (_, code, refs) = company_blocks.remove(0);
            return Ok(
                golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageOutcome::Blocked {
                    code,
                    refs,
                },
            );
        }
        if !company_blocks.is_empty() {
            company_blocks.sort_by_key(|(organization_id, _, _)| *organization_id);
            let refs = company_blocks
                .into_iter()
                .flat_map(|(organization_id, code, refs)| {
                    std::iter::once(format!("company_block:{organization_id}:{code}")).chain(refs)
                })
                .collect::<Vec<_>>();
            return Ok(
                golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageOutcome::Blocked {
                    code: "APPLICATION_UNDERSTANDING_COMPANY_UNITS_BLOCKED".to_string(),
                    refs,
                },
            );
        }
        let (completed_units, total_units) =
            aggregate_application_model_stage(&self.pool, &request).await?;
        Ok(
            golish_agent_kit::task_orchestrator::ApplicationUnderstandingStageOutcome::Passed {
                completed_units,
                total_units,
            },
        )
    }
}

#[cfg(test)]
mod producer_error_tests {
    use super::*;
    use golish_agent_kit::task_orchestrator::ApplicationModelProducerFailure;

    #[test]
    fn application_model_producer_error_classification() {
        let typed = anyhow::Error::new(ApplicationModelProducerFailure::CompletionTransport);
        assert_eq!(
            classify_application_model_producer_error(&typed),
            "application_model_completion_transport_failed"
        );

        let opaque = anyhow::anyhow!("provider detail must not become a durable code");
        assert_eq!(
            classify_application_model_producer_error(&opaque),
            "application_model_producer_failed"
        );
    }

    #[test]
    fn predecessor_handoff_accepts_exactly_one_direct_or_adopted_authority() {
        let source_operation_id = Uuid::new_v4();
        assert!(predecessor_handoff_authority_is_complete(1, None));
        assert!(predecessor_handoff_authority_is_complete(
            0,
            Some(source_operation_id)
        ));
        assert!(!predecessor_handoff_authority_is_complete(0, None));
        assert!(!predecessor_handoff_authority_is_complete(
            1,
            Some(source_operation_id)
        ));
        assert!(!predecessor_handoff_authority_is_complete(2, None));
    }

    #[test]
    fn compact_application_model_submission_attests_large_relational_proposal() {
        let manifest = ApplicationModelManifestRow {
            id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            stage_execution_id: Uuid::new_v4(),
            stage_run_unit_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            stage_kind: "application_understanding".to_string(),
            authority_kind: "model".to_string(),
            input_count: 1,
            manifest_hash: format!("sha256:{}", "a".repeat(64)),
            replay_material_hash: format!("sha256:{}", "b".repeat(64)),
            row_version: 1,
            frozen_at: chrono::Utc::now(),
        };
        let items = (0..570)
            .map(|ordinal| ApplicationModelItemSeed {
                item_key: format!("item:{ordinal:04}"),
                item_kind: "technology".to_string(),
                truth_state: ApplicationModelTruthStateRow::Unknown,
                source_input_keys: vec!["input:one".to_string()],
                referenced_item_keys: Vec::new(),
                payload: json!({"summary": "x".repeat(1024)}),
                evidence: Vec::new(),
            })
            .collect::<Vec<_>>();
        let draft = ApplicationModelProposalDraft {
            structured_model: json!({"organization_id": manifest.organization_id}),
            decisions: vec![ApplicationModelInputDecisionSeed {
                input_key: "input:one".to_string(),
                disposition: ApplicationModelInputDispositionRow::Incorporated,
                item_keys: items.iter().map(|item| item.item_key.clone()).collect(),
                duplicate_input_key: None,
                reason_code: None,
            }],
            items,
        };

        let payload = proposal_payload(&manifest, Some(&draft));
        let canonical = canonical_json(&payload);
        assert!(canonical.len() < 256 * 1024);
        assert_eq!(payload["authority_kind"], "model");
        assert_eq!(payload["decision_count"], 1);
        assert_eq!(payload["item_count"], 570);
        assert!(payload["proposal_material_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:")));
        assert!(payload.get("structured_model").is_none());
        assert!(payload.get("decisions").is_none());
        assert!(payload.get("items").is_none());
    }

    #[test]
    fn only_required_application_model_outputs_block_the_child_barrier() {
        let required_id = Uuid::new_v4();
        let finalizer_id = Uuid::new_v4();
        let required = BTreeSet::from([required_id]);
        assert!(application_team_output_blocks_child_barrier(
            &required,
            required_id,
            "blocked",
        ));
        assert!(!application_team_output_blocks_child_barrier(
            &required,
            finalizer_id,
            "blocked",
        ));
        assert!(!application_team_output_blocks_child_barrier(
            &required,
            required_id,
            "found",
        ));
    }

    #[test]
    fn replacement_unit_generation_does_not_invalidate_exact_finalizer_lease() {
        let owner = RuntimeOwner {
            scope_snapshot_id: Uuid::new_v4(),
            organization_id: Uuid::new_v4(),
            unit_status: "running".to_string(),
            worker_status: "running".to_string(),
            active_tool_call_id: None,
            lease_live: true,
            session_id: Uuid::new_v4(),
            unit_generation: 1,
            worker_generation: 0,
            lease_token: Some(Uuid::new_v4()),
            attempt_epoch: 1,
            checkpoint_version: 2,
        };
        assert!(
            application_understanding_runtime_owner_is_active(&owner),
            "Worker generation is the per-WorkItem attempt ordinal, not the Unit generation",
        );
    }
}
