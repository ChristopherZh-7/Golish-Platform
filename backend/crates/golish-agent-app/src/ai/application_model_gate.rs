//! Application-layer composition for the dormant Application Understanding Gate.
//!
//! The public command carries only runtime fences and persisted identities. Gate
//! decisions, hashes, handoff ids and publication state are always server-owned.

use golish_agent_kit::harness::application_model_gate::{
    validate_application_model_gate_truth, ApplicationModelAuthorityKind,
    ApplicationModelGateBlock, ApplicationModelGateCode, ApplicationModelGateDisposition,
    ApplicationModelGateSnapshot, ApplicationModelInputDecisionTruth,
    ApplicationModelInputDisposition, ApplicationModelItemTruth, ApplicationModelTruthState,
};
use golish_db::repo::application_models::{
    self, recompute_gate_hashes, ApplicationModelGateMaterial, ApplicationModelStoreError,
    LoadApplicationModelGateMaterial, LockApplicationModelFinalizeAuthority,
    PublishApplicationModelCurrentRevision,
};
use golish_db::repo::canonical_fact_refs::CanonicalFactKey;
use golish_db::repo::runtime_memory_tx::{
    self, FinalizeStageTeamUnitRow, FinalizeUnitPassRow, FinalizedUnitPassRow,
    RuntimeMemoryStoreError, RuntimeMemoryTxFence,
};
use golish_db::repo::stage_run_units::StageRunUnitStatus;
use golish_db::repo::stage_teams;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationModelGatePrecheckEvaluation {
    ContentReady(Box<ApplicationModelGateSnapshot>),
    Blocked(ApplicationModelGateBlock),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizeApplicationModelGatePass {
    pub fence: RuntimeMemoryTxFence,
    pub deliverable_submission_id: Uuid,
    pub manifest_id: Uuid,
    pub expected_unit_row_version: i64,
    pub scope_hash: String,
}

#[derive(Debug, Clone)]
pub struct FinalizedApplicationModelGatePass {
    pub manifest_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub final_seal: FinalizedUnitPassRow,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub enum ApplicationModelFinalizationOutcome {
    Passed(Box<FinalizedApplicationModelGatePass>),
    Blocked(ApplicationModelGateBlock),
}

#[derive(Debug)]
pub enum ApplicationModelGateError {
    Store(ApplicationModelStoreError),
    Runtime(RuntimeMemoryStoreError),
}

impl std::fmt::Display for ApplicationModelGateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => {
                write!(formatter, "application model persistence failed: {error}")
            }
            Self::Runtime(error) => write!(formatter, "application model runtime failed: {error}"),
        }
    }
}

impl std::error::Error for ApplicationModelGateError {}

impl From<ApplicationModelStoreError> for ApplicationModelGateError {
    fn from(error: ApplicationModelStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<RuntimeMemoryStoreError> for ApplicationModelGateError {
    fn from(error: RuntimeMemoryStoreError) -> Self {
        Self::Runtime(error)
    }
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

fn sha256_json(value: &Value) -> String {
    Sha256::digest(canonical_json(value).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn adapter_block(
    code: ApplicationModelGateCode,
    disposition: ApplicationModelGateDisposition,
    refs: impl IntoIterator<Item = String>,
) -> ApplicationModelGateBlock {
    ApplicationModelGateBlock {
        code,
        disposition,
        refs: refs
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
    }
}

fn schema_mapping_block(reference: impl Into<String>) -> ApplicationModelGateBlock {
    adapter_block(
        ApplicationModelGateCode::SchemaInvalid,
        ApplicationModelGateDisposition::Rework,
        [reference.into()],
    )
}

fn runtime_authority_block(error: &RuntimeMemoryStoreError) -> Option<ApplicationModelGateBlock> {
    let (code, reference) = match error {
        RuntimeMemoryStoreError::StaleVersion {
            entity,
            expected,
            actual,
        } => (
            ApplicationModelGateCode::IdentityMismatch,
            format!("runtime_stale:{entity}:{expected}:{actual}"),
        ),
        RuntimeMemoryStoreError::LeaseLost {
            worker_run_id,
            attempt_epoch,
        } => (
            ApplicationModelGateCode::IdentityMismatch,
            format!("runtime_lease_lost:{worker_run_id}:{attempt_epoch}"),
        ),
        RuntimeMemoryStoreError::IdentityMismatch { code }
        | RuntimeMemoryStoreError::Conflict { code } => (
            ApplicationModelGateCode::ReplayDrift,
            format!("runtime_authority:{code}"),
        ),
        RuntimeMemoryStoreError::Missing { entity } => (
            ApplicationModelGateCode::IdentityMismatch,
            format!("runtime_missing:{entity}"),
        ),
        RuntimeMemoryStoreError::InvalidContractTransition { .. }
        | RuntimeMemoryStoreError::Sqlx(_)
        | RuntimeMemoryStoreError::Repository(_) => return None,
    };
    Some(adapter_block(
        code,
        ApplicationModelGateDisposition::Hold,
        [reference],
    ))
}

/// Convert persisted rows into the DB-free Gate DTO. Expected hashes are
/// reconstructed from relational content and never copied from hash columns.
pub fn build_application_model_gate_snapshot(
    material: &ApplicationModelGateMaterial,
) -> Result<ApplicationModelGateSnapshot, ApplicationModelGateBlock> {
    let manifest = &material.manifest;
    let authority_kind = match manifest.authority_kind.as_str() {
        "model" => ApplicationModelAuthorityKind::Model,
        "terminal_no_input" => ApplicationModelAuthorityKind::TerminalNoInput,
        _ => {
            return Err(adapter_block(
                ApplicationModelGateCode::IdentityMismatch,
                ApplicationModelGateDisposition::Hold,
                ["authority_kind".to_string()],
            ));
        }
    };
    let expected_hashes = recompute_gate_hashes(material);
    let mut foreign_refs = Vec::new();
    if manifest.stage_kind != "application_understanding"
        || manifest.row_version != 0
        || manifest.input_count != i32::try_from(material.inputs.len()).unwrap_or(i32::MAX)
    {
        foreign_refs.push(format!("manifest:{}", manifest.id));
    }
    for (expected_ordinal, input) in material.inputs.iter().enumerate() {
        if input.ordinal != i32::try_from(expected_ordinal).unwrap_or(i32::MAX)
            || input.source_id != input.source_handoff_id.to_string()
        {
            foreign_refs.push(format!("input:{}", input.input_key));
        }
    }

    let mut decisions = Vec::with_capacity(material.decisions.len());
    for decision in &material.decisions {
        if decision.manifest_id != manifest.id {
            foreign_refs.push(format!("decision:{}", decision.input_key));
        }
        let disposition = match decision.disposition.as_str() {
            "incorporated" => ApplicationModelInputDisposition::Incorporated,
            "duplicate" => ApplicationModelInputDisposition::Duplicate,
            "not_relevant" => ApplicationModelInputDisposition::NotRelevant,
            "unknown" => ApplicationModelInputDisposition::Unknown,
            _ => {
                return Err(schema_mapping_block(format!(
                    "decision:{}",
                    decision.input_key
                )))
            }
        };
        decisions.push(ApplicationModelInputDecisionTruth {
            input_key: decision.input_key.clone(),
            disposition,
            item_keys: decision.item_keys.clone(),
            duplicate_input_key: decision.duplicate_input_key.clone(),
            reason_code: decision.reason_code.clone(),
        });
    }

    let evidence_by_item = material.item_evidence.iter().fold(
        BTreeMap::<&str, Vec<_>>::new(),
        |mut grouped, evidence| {
            grouped
                .entry(evidence.item_key.as_str())
                .or_default()
                .push(evidence);
            grouped
        },
    );
    let item_keys = material
        .items
        .iter()
        .map(|item| item.item_key.as_str())
        .collect::<BTreeSet<_>>();
    for evidence in &material.item_evidence {
        if evidence.manifest_id != manifest.id || !item_keys.contains(evidence.item_key.as_str()) {
            foreign_refs.push(format!(
                "item_evidence:{}:{}",
                evidence.item_key, evidence.evidence_id
            ));
        }
        if !matches!(evidence.role.as_str(), "observation" | "support") {
            return Err(schema_mapping_block(format!(
                "item_evidence_role:{}:{}",
                evidence.item_key, evidence.evidence_id
            )));
        }
    }
    let mut items = Vec::with_capacity(material.items.len());
    for item in &material.items {
        if item.manifest_id != manifest.id {
            foreign_refs.push(format!("item:{}", item.item_key));
        }
        let truth_state = match item.truth_state.as_str() {
            "observed" => ApplicationModelTruthState::Observed,
            "inferred" => ApplicationModelTruthState::Inferred,
            "unknown" => ApplicationModelTruthState::Unknown,
            _ => return Err(schema_mapping_block(format!("item:{}", item.item_key))),
        };
        let item_evidence = evidence_by_item
            .get(item.item_key.as_str())
            .cloned()
            .unwrap_or_default();
        items.push(ApplicationModelItemTruth {
            item_key: item.item_key.clone(),
            truth_state,
            source_input_keys: item.source_input_keys.clone(),
            evidence_ids: item_evidence
                .iter()
                .map(|evidence| evidence.evidence_id)
                .collect(),
            observed_evidence_ids: item_evidence
                .iter()
                .filter(|evidence| evidence.role == "observation")
                .map(|evidence| evidence.evidence_id)
                .collect(),
            referenced_item_keys: item.referenced_item_keys.clone(),
        });
    }

    let revision = material.revision.as_ref();
    if let Some(revision) = revision {
        let identity_matches = revision.manifest_id == manifest.id
            && revision.operation_id == manifest.operation_id
            && revision.scope_snapshot_id == manifest.scope_snapshot_id
            && revision.stage_execution_id == manifest.stage_execution_id
            && revision.stage_run_unit_id == manifest.stage_run_unit_id
            && revision.organization_id == manifest.organization_id
            && revision.stage_kind == "application_understanding"
            && matches!(
                (
                    revision.status.as_str(),
                    revision.row_version,
                    revision.finalized_at
                ),
                ("proposed", 0, None) | ("final", 1, Some(_))
            );
        if !identity_matches
            || material
                .decisions
                .iter()
                .any(|row| row.revision_id != revision.id)
            || material
                .items
                .iter()
                .any(|row| row.revision_id != revision.id)
            || material
                .item_evidence
                .iter()
                .any(|row| row.revision_id != revision.id)
        {
            foreign_refs.push(format!("revision:{}", revision.id));
        }
    }
    let authorized_evidence_ids = material
        .inputs
        .iter()
        .flat_map(|input| input.evidence_ids.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(ApplicationModelGateSnapshot {
        authority_kind,
        operation_id: manifest.operation_id,
        scope_snapshot_id: manifest.scope_snapshot_id,
        stage_execution_id: manifest.stage_execution_id,
        stage_run_unit_id: manifest.stage_run_unit_id,
        organization_id: manifest.organization_id,
        manifest_hash: manifest.manifest_hash.clone(),
        expected_manifest_hash: expected_hashes.manifest_hash,
        schema_version: revision.map(|row| row.schema_version.clone()),
        model_hash: revision.map(|row| row.model_hash.clone()),
        expected_model_hash: expected_hashes.model_hash,
        replay_material_hash: revision
            .map(|row| row.replay_material_hash.clone())
            .unwrap_or_else(|| manifest.replay_material_hash.clone()),
        expected_replay_material_hash: expected_hashes.replay_material_hash,
        manifest_input_keys: material
            .inputs
            .iter()
            .map(|input| input.input_key.clone())
            .collect(),
        authorized_evidence_ids,
        decisions,
        items,
        foreign_reference_keys: foreign_refs,
        forbidden_activity_refs: material.forbidden_activity_refs.clone(),
        pending_producer_refs: material.pending_producer_refs.clone(),
    })
}

/// Run a non-authoritative content precheck.
///
/// This deliberately cannot return `Passed`: pending producers and forbidden
/// tools are serialized only by `finalize_application_model_gate_pass` in its
/// publication transaction.
pub async fn evaluate_application_model_content_precheck(
    pool: &PgPool,
    input: &LoadApplicationModelGateMaterial,
) -> Result<ApplicationModelGatePrecheckEvaluation, ApplicationModelGateError> {
    let material = match golish_db::repo::application_models::load_gate_material(pool, input).await
    {
        Ok(material) => material,
        Err(ApplicationModelStoreError::InvalidInput { code }) => {
            return Ok(ApplicationModelGatePrecheckEvaluation::Blocked(
                adapter_block(
                    ApplicationModelGateCode::SchemaInvalid,
                    ApplicationModelGateDisposition::Rework,
                    [code.to_string()],
                ),
            ));
        }
        Err(ApplicationModelStoreError::IdentityMismatch { code }) => {
            return Ok(ApplicationModelGatePrecheckEvaluation::Blocked(
                adapter_block(
                    ApplicationModelGateCode::IdentityMismatch,
                    ApplicationModelGateDisposition::Hold,
                    [code.to_string()],
                ),
            ));
        }
        Err(ApplicationModelStoreError::ReplayConflict { code }) => {
            return Ok(ApplicationModelGatePrecheckEvaluation::Blocked(
                adapter_block(
                    ApplicationModelGateCode::ReplayDrift,
                    ApplicationModelGateDisposition::Hold,
                    [code.to_string()],
                ),
            ));
        }
        Err(error @ ApplicationModelStoreError::Sqlx(_)) => return Err(error.into()),
    };
    let snapshot = match build_application_model_gate_snapshot(&material) {
        Ok(snapshot) => snapshot,
        Err(block) => return Ok(ApplicationModelGatePrecheckEvaluation::Blocked(block)),
    };
    match validate_application_model_gate_truth(&snapshot) {
        Ok(()) => Ok(ApplicationModelGatePrecheckEvaluation::ContentReady(
            Box::new(snapshot),
        )),
        Err(block) => Ok(ApplicationModelGatePrecheckEvaluation::Blocked(block)),
    }
}

pub async fn finalize_application_model_gate_pass(
    pool: &PgPool,
    input: &FinalizeApplicationModelGatePass,
) -> Result<ApplicationModelFinalizationOutcome, ApplicationModelGateError> {
    let gate_identity = match application_models::resolve_gate_identity(
        pool,
        input.manifest_id,
        &input.fence,
    )
    .await
    {
        Ok(identity) => identity,
        Err(ApplicationModelStoreError::IdentityMismatch { code }) => {
            return Ok(ApplicationModelFinalizationOutcome::Blocked(adapter_block(
                ApplicationModelGateCode::IdentityMismatch,
                ApplicationModelGateDisposition::Hold,
                [code.to_string()],
            )));
        }
        Err(error) => return Err(error.into()),
    };
    let mut tx = pool
        .begin()
        .await
        .map_err(ApplicationModelStoreError::from)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        .execute(&mut *tx)
        .await
        .map_err(ApplicationModelStoreError::from)?;
    let barrier = match application_models::lock_finalize_authority_with_transaction(
        &mut tx,
        &LockApplicationModelFinalizeAuthority {
            gate: gate_identity.clone(),
            fence: input.fence.clone(),
            deliverable_submission_id: input.deliverable_submission_id,
        },
    )
    .await
    {
        Ok(barrier) => barrier,
        Err(ApplicationModelStoreError::InvalidInput { code }) => {
            tx.rollback()
                .await
                .map_err(ApplicationModelStoreError::from)?;
            return Ok(ApplicationModelFinalizationOutcome::Blocked(adapter_block(
                ApplicationModelGateCode::SchemaInvalid,
                ApplicationModelGateDisposition::Rework,
                [code.to_string()],
            )));
        }
        Err(ApplicationModelStoreError::IdentityMismatch { code }) => {
            tx.rollback()
                .await
                .map_err(ApplicationModelStoreError::from)?;
            return Ok(ApplicationModelFinalizationOutcome::Blocked(adapter_block(
                ApplicationModelGateCode::IdentityMismatch,
                ApplicationModelGateDisposition::Hold,
                [code.to_string()],
            )));
        }
        Err(ApplicationModelStoreError::ReplayConflict { code }) => {
            tx.rollback()
                .await
                .map_err(ApplicationModelStoreError::from)?;
            return Ok(ApplicationModelFinalizationOutcome::Blocked(adapter_block(
                ApplicationModelGateCode::ReplayDrift,
                ApplicationModelGateDisposition::Hold,
                [code.to_string()],
            )));
        }
        Err(error @ ApplicationModelStoreError::Sqlx(_)) => return Err(error.into()),
    };
    let mut material = match application_models::load_gate_material_with_transaction(
        &mut tx,
        &gate_identity,
    )
    .await
    {
        Ok(material) => material,
        Err(ApplicationModelStoreError::InvalidInput { code }) => {
            tx.rollback()
                .await
                .map_err(ApplicationModelStoreError::from)?;
            return Ok(ApplicationModelFinalizationOutcome::Blocked(adapter_block(
                ApplicationModelGateCode::SchemaInvalid,
                ApplicationModelGateDisposition::Rework,
                [code.to_string()],
            )));
        }
        Err(ApplicationModelStoreError::IdentityMismatch { code }) => {
            tx.rollback()
                .await
                .map_err(ApplicationModelStoreError::from)?;
            return Ok(ApplicationModelFinalizationOutcome::Blocked(adapter_block(
                ApplicationModelGateCode::IdentityMismatch,
                ApplicationModelGateDisposition::Hold,
                [code.to_string()],
            )));
        }
        Err(ApplicationModelStoreError::ReplayConflict { code }) => {
            tx.rollback()
                .await
                .map_err(ApplicationModelStoreError::from)?;
            return Ok(ApplicationModelFinalizationOutcome::Blocked(adapter_block(
                ApplicationModelGateCode::ReplayDrift,
                ApplicationModelGateDisposition::Hold,
                [code.to_string()],
            )));
        }
        Err(error @ ApplicationModelStoreError::Sqlx(_)) => return Err(error.into()),
    };
    material.forbidden_activity_refs = barrier.forbidden_activity_refs;
    material.pending_producer_refs = barrier.pending_producer_refs;
    let snapshot = match build_application_model_gate_snapshot(&material) {
        Ok(snapshot) => snapshot,
        Err(block) => {
            tx.rollback()
                .await
                .map_err(ApplicationModelStoreError::from)?;
            return Ok(ApplicationModelFinalizationOutcome::Blocked(block));
        }
    };
    if let Err(block) = validate_application_model_gate_truth(&snapshot) {
        tx.rollback()
            .await
            .map_err(ApplicationModelStoreError::from)?;
        return Ok(ApplicationModelFinalizationOutcome::Blocked(block));
    }
    let existing =
        application_models::load_current_revision_with_transaction(&mut tx, input.manifest_id)
            .await?;
    let revision_id = material.revision.as_ref().map(|revision| revision.id);
    if existing.is_none() {
        if let Some(revision_id) = revision_id {
            application_models::transition_revision_to_final_with_transaction(
                &mut tx,
                input.manifest_id,
                revision_id,
            )
            .await?;
        }
    }
    let authority_kind = material.manifest.authority_kind.clone();
    let manifest_hash = material.manifest.manifest_hash.clone();
    let model_hash = material
        .revision
        .as_ref()
        .map(|revision| revision.model_hash.clone());
    let replay_material_hash = material
        .revision
        .as_ref()
        .map(|revision| revision.replay_material_hash.clone())
        .unwrap_or_else(|| material.manifest.replay_material_hash.clone());
    let typed_claims = vec![serde_json::json!({
        "kind": "application_model_authority",
        "payload": {
            "authority_kind": authority_kind,
            "manifest_id": input.manifest_id,
            "revision_id": revision_id,
            "manifest_hash": manifest_hash,
            "model_hash": model_hash,
            "replay_material_hash": replay_material_hash,
            "deliverable_submission_id": input.deliverable_submission_id,
        }
    })];
    let coverage_watermark = serde_json::json!({
        "schema_version": "application_model_coverage.v1",
        "manifest_id": input.manifest_id,
        "revision_id": revision_id,
        "input_count": material.inputs.len(),
        "decision_count": material.decisions.len(),
        "item_count": material.items.len(),
        "manifest_hash": manifest_hash,
        "model_hash": model_hash,
        "replay_material_hash": replay_material_hash,
    });
    let terminal_checkpoint = serde_json::json!({
        "schema_version": "application_model_terminal.v1",
        "manifest_id": input.manifest_id,
        "revision_id": revision_id,
        "manifest_hash": manifest_hash,
        "model_hash": model_hash,
        "replay_material_hash": replay_material_hash,
        "deliverable_submission_id": input.deliverable_submission_id,
    });
    let canonical_fact_keys = revision_id
        .map(|revision_id| vec![CanonicalFactKey::ApplicationModelRevision { revision_id }])
        .unwrap_or_default();
    let mut evidence_ids = snapshot.authorized_evidence_ids.clone();
    evidence_ids.extend(
        material
            .item_evidence
            .iter()
            .map(|evidence| evidence.evidence_id),
    );
    evidence_ids.sort_unstable();
    evidence_ids.dedup();
    let deterministic_gate_details = serde_json::json!({
        "code": "APPLICATION_MODEL_GATE_PASS",
        "authority_kind": authority_kind,
        "manifest_id": input.manifest_id,
        "revision_id": revision_id,
        "manifest_hash": manifest_hash,
        "model_hash": model_hash,
        "replay_material_hash": replay_material_hash,
    });
    let seal_material = serde_json::json!({
        "canonical_fact_keys": canonical_fact_keys,
        "typed_claims": typed_claims,
        "coverage_watermark": coverage_watermark,
        "evidence_ids": evidence_ids,
        "terminal_checkpoint": terminal_checkpoint,
        "deterministic_gate_details": deterministic_gate_details,
        "candidate_acceptance": Value::Null,
    });
    let gate_decision = serde_json::json!({
        "outcome": "pass",
        "operation_id": input.fence.operation_id,
        "stage_execution_id": input.fence.stage_execution_id,
        "stage_run_unit_id": input.fence.stage_run_unit_id,
        "deliverable_submission_id": input.deliverable_submission_id,
        "scope_hash": input.scope_hash,
        "details": deterministic_gate_details,
        "seal_material_sha256": sha256_json(&seal_material),
    });
    let gate_decision_hash = sha256_json(&gate_decision);
    let final_seal_input = FinalizeUnitPassRow {
        fence: input.fence.clone(),
        deliverable_submission_id: input.deliverable_submission_id,
        expected_unit_status: StageRunUnitStatus::Running,
        expected_unit_row_version: input.expected_unit_row_version,
        scope_hash: input.scope_hash.clone(),
        gate_decision,
        gate_decision_hash: gate_decision_hash.clone(),
        aggregate_pass_token_hash: None,
        canonical_fact_keys,
        typed_claims,
        coverage_watermark,
        evidence_ids,
        terminal_checkpoint,
        candidate_acceptance: None,
    };
    let team_authority = sqlx::query_as::<_, (Uuid, i64, Uuid)>(
        r#"SELECT plan.id,plan.dispatch_epoch,item.id
             FROM stage_team_plans AS plan
             JOIN stage_worker_runs AS worker
               ON worker.id=plan.final_submitter_worker_run_id
              AND worker.work_item_id IS NOT NULL
             JOIN stage_work_items AS item
               ON item.id=worker.work_item_id AND item.team_plan_id=plan.id
            WHERE plan.stage_run_unit_id=$1 AND plan.operation_id=$2
              AND plan.stage_execution_id=$3 AND worker.id=$4
              AND item.stable_key='leader:primary'
            FOR SHARE OF plan,item"#,
    )
    .bind(input.fence.stage_run_unit_id)
    .bind(input.fence.operation_id)
    .bind(input.fence.stage_execution_id)
    .bind(input.fence.worker_run_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(ApplicationModelStoreError::from)?;
    let final_seal_result =
        if let Some((plan_id, dispatch_epoch, aggregator_work_item_id)) = team_authority {
            let barrier = stage_teams::load_barrier_with_connection(&mut tx, plan_id).await?;
            runtime_memory_tx::finalize_stage_team_unit_with_transaction(
                &mut tx,
                &FinalizeStageTeamUnitRow {
                    stage_team_plan_id: plan_id,
                    aggregator_work_item_id,
                    expected_dispatch_epoch: dispatch_epoch,
                    expected_manifest_hash: barrier.manifest_hash,
                    final_seal: final_seal_input,
                },
            )
            .await
            .map(|team| team.finalized)
        } else {
            runtime_memory_tx::finalize_unit_pass_with_transaction(&mut tx, &final_seal_input).await
        };
    let final_seal = match final_seal_result {
        Ok(final_seal) => final_seal,
        Err(error) => {
            let Some(block) = runtime_authority_block(&error) else {
                return Err(error.into());
            };
            tx.rollback()
                .await
                .map_err(ApplicationModelStoreError::from)?;
            return Ok(ApplicationModelFinalizationOutcome::Blocked(block));
        }
    };
    if let Some(current) = existing {
        let exact = current.revision_id == revision_id
            && current.authority_kind == authority_kind
            && current.stage_handoff_id == final_seal.handoff.id
            && current.deliverable_submission_id == input.deliverable_submission_id
            && current.manifest_hash == manifest_hash
            && current.model_hash == model_hash
            && current.replay_material_hash == replay_material_hash
            && current.gate_decision_hash == format!("sha256:{gate_decision_hash}");
        if !exact {
            return Err(ApplicationModelStoreError::ReplayConflict {
                code: "application_model_current_replay_drift",
            }
            .into());
        }
    } else {
        application_models::insert_current_revision_with_transaction(
            &mut tx,
            &PublishApplicationModelCurrentRevision {
                manifest_id: input.manifest_id,
                revision_id,
                authority_kind,
                stage_handoff_id: final_seal.handoff.id,
                deliverable_submission_id: input.deliverable_submission_id,
                manifest_hash,
                model_hash,
                replay_material_hash,
                gate_decision_hash: format!("sha256:{gate_decision_hash}"),
            },
        )
        .await?;
    }
    tx.commit()
        .await
        .map_err(ApplicationModelStoreError::from)?;
    Ok(ApplicationModelFinalizationOutcome::Passed(Box::new(
        FinalizedApplicationModelGatePass {
            manifest_id: input.manifest_id,
            revision_id,
            replayed: final_seal.replayed,
            final_seal,
        },
    )))
}
