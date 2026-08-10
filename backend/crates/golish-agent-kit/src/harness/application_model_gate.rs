//! Deterministic truth validation for the operation-frozen Application Understanding stage.
//!
//! This module deliberately has no repository or runtime dependencies.  It accepts
//! one frozen snapshot and returns either an exact PASS (`Ok(())`) or one stable
//! machine-readable block reason.  Runtime wiring and persistence are separate
//! slices so the gate contract can be tested before the stage is activated.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const APPLICATION_MODEL_SCHEMA_V1: &str = "application_model.v1";

fn is_lowercase_tagged_sha256(value: &str) -> bool {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelAuthorityKind {
    Model,
    TerminalNoInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelTruthState {
    Observed,
    Inferred,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelInputDisposition {
    Incorporated,
    Duplicate,
    NotRelevant,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelGateDisposition {
    Rework,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationModelGateCode {
    IdentityMismatch,
    ManifestDrift,
    InputCoverageIncomplete,
    SchemaInvalid,
    ObservedEvidenceMissing,
    TruthStateConflict,
    ForeignReference,
    ForbiddenToolActivity,
    ProducerBarrierOpen,
    ReplayDrift,
}

impl ApplicationModelGateCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IdentityMismatch => "APPLICATION_MODEL_IDENTITY_MISMATCH",
            Self::ManifestDrift => "APPLICATION_MODEL_MANIFEST_DRIFT",
            Self::InputCoverageIncomplete => "APPLICATION_MODEL_INPUT_COVERAGE_INCOMPLETE",
            Self::SchemaInvalid => "APPLICATION_MODEL_SCHEMA_INVALID",
            Self::ObservedEvidenceMissing => "APPLICATION_MODEL_OBSERVED_EVIDENCE_MISSING",
            Self::TruthStateConflict => "APPLICATION_MODEL_TRUTH_STATE_CONFLICT",
            Self::ForeignReference => "APPLICATION_MODEL_FOREIGN_REFERENCE",
            Self::ForbiddenToolActivity => "APPLICATION_MODEL_FORBIDDEN_TOOL_ACTIVITY",
            Self::ProducerBarrierOpen => "APPLICATION_MODEL_PRODUCER_BARRIER_OPEN",
            Self::ReplayDrift => "APPLICATION_MODEL_REPLAY_DRIFT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationModelGateBlock {
    pub code: ApplicationModelGateCode,
    pub disposition: ApplicationModelGateDisposition,
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationModelInputDecisionTruth {
    pub input_key: String,
    pub disposition: ApplicationModelInputDisposition,
    pub item_keys: Vec<String>,
    pub duplicate_input_key: Option<String>,
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationModelItemTruth {
    pub item_key: String,
    pub truth_state: ApplicationModelTruthState,
    pub source_input_keys: Vec<String>,
    pub evidence_ids: Vec<i64>,
    /// Evidence ids explicitly carrying the observation-proof role.
    /// Must be a subset of `evidence_ids` and is forbidden for inferred/unknown items.
    pub observed_evidence_ids: Vec<i64>,
    pub referenced_item_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicationModelGateSnapshot {
    pub authority_kind: ApplicationModelAuthorityKind,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub manifest_hash: String,
    pub expected_manifest_hash: String,
    pub schema_version: Option<String>,
    pub model_hash: Option<String>,
    pub expected_model_hash: Option<String>,
    pub replay_material_hash: String,
    pub expected_replay_material_hash: String,
    pub manifest_input_keys: Vec<String>,
    pub authorized_evidence_ids: Vec<i64>,
    pub decisions: Vec<ApplicationModelInputDecisionTruth>,
    pub items: Vec<ApplicationModelItemTruth>,
    pub foreign_reference_keys: Vec<String>,
    pub forbidden_activity_refs: Vec<String>,
    pub pending_producer_refs: Vec<String>,
}

pub fn validate_application_model_gate_truth(
    snapshot: &ApplicationModelGateSnapshot,
) -> Result<(), ApplicationModelGateBlock> {
    validate_identity(snapshot)?;

    if snapshot.manifest_hash != snapshot.expected_manifest_hash {
        return Err(block(
            ApplicationModelGateCode::ManifestDrift,
            ApplicationModelGateDisposition::Hold,
            [
                snapshot.manifest_hash.clone(),
                snapshot.expected_manifest_hash.clone(),
            ],
        ));
    }
    if snapshot.replay_material_hash != snapshot.expected_replay_material_hash {
        return Err(block(
            ApplicationModelGateCode::ReplayDrift,
            ApplicationModelGateDisposition::Hold,
            [
                snapshot.replay_material_hash.clone(),
                snapshot.expected_replay_material_hash.clone(),
            ],
        ));
    }
    if !snapshot.foreign_reference_keys.is_empty() {
        return Err(block(
            ApplicationModelGateCode::ForeignReference,
            ApplicationModelGateDisposition::Hold,
            snapshot.foreign_reference_keys.iter().cloned(),
        ));
    }
    if !snapshot.forbidden_activity_refs.is_empty() {
        return Err(block(
            ApplicationModelGateCode::ForbiddenToolActivity,
            ApplicationModelGateDisposition::Hold,
            snapshot.forbidden_activity_refs.iter().cloned(),
        ));
    }
    if !snapshot.pending_producer_refs.is_empty() {
        return Err(block(
            ApplicationModelGateCode::ProducerBarrierOpen,
            ApplicationModelGateDisposition::Hold,
            snapshot.pending_producer_refs.iter().cloned(),
        ));
    }

    match snapshot.authority_kind {
        ApplicationModelAuthorityKind::TerminalNoInput => validate_terminal_no_input(snapshot),
        ApplicationModelAuthorityKind::Model => validate_model(snapshot),
    }
}

fn validate_identity(
    snapshot: &ApplicationModelGateSnapshot,
) -> Result<(), ApplicationModelGateBlock> {
    let mut refs = Vec::new();
    for (name, id) in [
        ("operation_id", snapshot.operation_id),
        ("scope_snapshot_id", snapshot.scope_snapshot_id),
        ("stage_execution_id", snapshot.stage_execution_id),
        ("stage_run_unit_id", snapshot.stage_run_unit_id),
        ("organization_id", snapshot.organization_id),
    ] {
        if id.is_nil() {
            refs.push(name.to_string());
        }
    }
    for (name, value) in [
        ("manifest_hash", snapshot.manifest_hash.as_str()),
        (
            "expected_manifest_hash",
            snapshot.expected_manifest_hash.as_str(),
        ),
        (
            "replay_material_hash",
            snapshot.replay_material_hash.as_str(),
        ),
        (
            "expected_replay_material_hash",
            snapshot.expected_replay_material_hash.as_str(),
        ),
    ] {
        if !is_lowercase_tagged_sha256(value) {
            refs.push(name.to_string());
        }
    }
    if refs.is_empty() {
        Ok(())
    } else {
        Err(block(
            ApplicationModelGateCode::IdentityMismatch,
            ApplicationModelGateDisposition::Hold,
            refs,
        ))
    }
}

fn validate_terminal_no_input(
    snapshot: &ApplicationModelGateSnapshot,
) -> Result<(), ApplicationModelGateBlock> {
    let mut refs = Vec::new();
    if !snapshot.manifest_input_keys.is_empty() {
        refs.push("manifest_input_keys".to_string());
    }
    if !snapshot.authorized_evidence_ids.is_empty() {
        refs.push("authorized_evidence_ids".to_string());
    }
    if !snapshot.decisions.is_empty() {
        refs.push("decisions".to_string());
    }
    if !snapshot.items.is_empty() {
        refs.push("items".to_string());
    }
    if snapshot.schema_version.is_some() {
        refs.push("schema_version".to_string());
    }
    if snapshot.model_hash.is_some() {
        refs.push("model_hash".to_string());
    }
    if snapshot.expected_model_hash.is_some() {
        refs.push("expected_model_hash".to_string());
    }
    if refs.is_empty() {
        Ok(())
    } else {
        Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            refs,
        ))
    }
}

fn validate_model(
    snapshot: &ApplicationModelGateSnapshot,
) -> Result<(), ApplicationModelGateBlock> {
    validate_model_envelope(snapshot)?;

    let manifest = unique_non_empty_keys(&snapshot.manifest_input_keys, "manifest_input_keys")?;
    let items = unique_items(&snapshot.items)?;
    let decisions = unique_decisions(&snapshot.decisions)?;

    let decision_keys: BTreeSet<&str> = decisions.keys().copied().collect();
    let missing: Vec<String> = manifest
        .difference(&decision_keys)
        .map(|key| (*key).to_string())
        .collect();
    let foreign: Vec<String> = decision_keys
        .difference(&manifest)
        .map(|key| (*key).to_string())
        .collect();
    if !missing.is_empty() || !foreign.is_empty() {
        return Err(block(
            ApplicationModelGateCode::InputCoverageIncomplete,
            ApplicationModelGateDisposition::Rework,
            missing.into_iter().chain(foreign),
        ));
    }

    for decision in &snapshot.decisions {
        validate_decision(decision, &manifest, &items)?;
    }
    let incorporated_items: BTreeSet<&str> = snapshot
        .decisions
        .iter()
        .filter(|decision| decision.disposition == ApplicationModelInputDisposition::Incorporated)
        .flat_map(|decision| decision.item_keys.iter().map(|key| key.trim()))
        .collect();
    let orphan_items: Vec<String> = items
        .keys()
        .filter(|key| !incorporated_items.contains(**key))
        .map(|key| (*key).to_string())
        .collect();
    if !orphan_items.is_empty() {
        return Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            orphan_items,
        ));
    }

    let authorized_evidence: BTreeSet<i64> =
        snapshot.authorized_evidence_ids.iter().copied().collect();
    if authorized_evidence.len() != snapshot.authorized_evidence_ids.len() {
        return Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            ["authorized_evidence_ids".to_string()],
        ));
    }

    for item in &snapshot.items {
        validate_item(item, &manifest, &items, &authorized_evidence)?;
    }
    if authorized_evidence.iter().any(|id| *id <= 0) {
        return Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            ["authorized_evidence_ids".to_string()],
        ));
    }

    Ok(())
}

fn validate_model_envelope(
    snapshot: &ApplicationModelGateSnapshot,
) -> Result<(), ApplicationModelGateBlock> {
    let mut refs = Vec::new();
    if snapshot.manifest_input_keys.is_empty() {
        refs.push("manifest_input_keys".to_string());
    }
    if snapshot.schema_version.as_deref() != Some(APPLICATION_MODEL_SCHEMA_V1) {
        refs.push("schema_version".to_string());
    }
    let model_hash = snapshot.model_hash.as_deref().unwrap_or_default();
    let expected_model_hash = snapshot.expected_model_hash.as_deref().unwrap_or_default();
    let model_hash_is_valid = is_lowercase_tagged_sha256(model_hash);
    let expected_model_hash_is_valid = is_lowercase_tagged_sha256(expected_model_hash);
    if !model_hash_is_valid {
        refs.push("model_hash".to_string());
    }
    if !expected_model_hash_is_valid {
        refs.push("expected_model_hash".to_string());
    }
    if model_hash_is_valid && expected_model_hash_is_valid && model_hash != expected_model_hash {
        return Err(block(
            ApplicationModelGateCode::ReplayDrift,
            ApplicationModelGateDisposition::Hold,
            [model_hash.to_string(), expected_model_hash.to_string()],
        ));
    }
    if refs.is_empty() {
        Ok(())
    } else {
        Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            refs,
        ))
    }
}

fn unique_non_empty_keys<'a>(
    keys: &'a [String],
    field: &str,
) -> Result<BTreeSet<&'a str>, ApplicationModelGateBlock> {
    let unique: BTreeSet<&str> = keys
        .iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
        .collect();
    if unique.len() == keys.len() {
        Ok(unique)
    } else {
        Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            [field.to_string()],
        ))
    }
}

fn unique_items(
    items: &[ApplicationModelItemTruth],
) -> Result<BTreeMap<&str, &ApplicationModelItemTruth>, ApplicationModelGateBlock> {
    let mut indexed = BTreeMap::new();
    for item in items {
        let key = item.item_key.trim();
        if key.is_empty() || indexed.insert(key, item).is_some() {
            return Err(block(
                ApplicationModelGateCode::SchemaInvalid,
                ApplicationModelGateDisposition::Rework,
                ["items".to_string()],
            ));
        }
    }
    Ok(indexed)
}

fn unique_decisions(
    decisions: &[ApplicationModelInputDecisionTruth],
) -> Result<BTreeMap<&str, &ApplicationModelInputDecisionTruth>, ApplicationModelGateBlock> {
    let mut indexed = BTreeMap::new();
    for decision in decisions {
        let key = decision.input_key.trim();
        if key.is_empty() || indexed.insert(key, decision).is_some() {
            return Err(block(
                ApplicationModelGateCode::InputCoverageIncomplete,
                ApplicationModelGateDisposition::Rework,
                [decision.input_key.clone()],
            ));
        }
    }
    Ok(indexed)
}

fn validate_decision(
    decision: &ApplicationModelInputDecisionTruth,
    manifest: &BTreeSet<&str>,
    items: &BTreeMap<&str, &ApplicationModelItemTruth>,
) -> Result<(), ApplicationModelGateBlock> {
    let invalid = match decision.disposition {
        ApplicationModelInputDisposition::Incorporated => {
            decision.item_keys.is_empty()
                || decision.duplicate_input_key.is_some()
                || decision.reason_code.is_some()
                || decision
                    .item_keys
                    .iter()
                    .any(|key| !items.contains_key(key.trim()))
        }
        ApplicationModelInputDisposition::Duplicate => {
            let duplicate = decision.duplicate_input_key.as_deref().unwrap_or_default();
            !decision.item_keys.is_empty()
                || decision.reason_code.is_some()
                || duplicate.is_empty()
                || duplicate == decision.input_key
                || !manifest.contains(duplicate)
        }
        ApplicationModelInputDisposition::NotRelevant
        | ApplicationModelInputDisposition::Unknown => {
            !decision.item_keys.is_empty()
                || decision.duplicate_input_key.is_some()
                || !decision
                    .reason_code
                    .as_deref()
                    .is_some_and(valid_reason_code)
        }
    };
    if invalid {
        return Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            [decision.input_key.clone()],
        ));
    }
    if decision.disposition == ApplicationModelInputDisposition::Incorporated
        && decision.item_keys.iter().any(|item_key| {
            items.get(item_key.trim()).is_some_and(|item| {
                !item
                    .source_input_keys
                    .iter()
                    .any(|source| source.trim() == decision.input_key.trim())
            })
        })
    {
        return Err(block(
            ApplicationModelGateCode::TruthStateConflict,
            ApplicationModelGateDisposition::Rework,
            [decision.input_key.clone()],
        ));
    }
    Ok(())
}

fn validate_item(
    item: &ApplicationModelItemTruth,
    manifest: &BTreeSet<&str>,
    items: &BTreeMap<&str, &ApplicationModelItemTruth>,
    authorized_evidence: &BTreeSet<i64>,
) -> Result<(), ApplicationModelGateBlock> {
    let sources: BTreeSet<&str> = item
        .source_input_keys
        .iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
        .collect();
    if sources.is_empty() {
        return Err(block(
            ApplicationModelGateCode::TruthStateConflict,
            ApplicationModelGateDisposition::Rework,
            [item.item_key.clone()],
        ));
    }
    if sources.len() != item.source_input_keys.len()
        || sources.iter().any(|key| !manifest.contains(key))
        || item
            .referenced_item_keys
            .iter()
            .any(|key| key == &item.item_key || !items.contains_key(key.trim()))
    {
        return Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            [item.item_key.clone()],
        ));
    }

    let evidence: BTreeSet<i64> = item.evidence_ids.iter().copied().collect();
    let observed_evidence: BTreeSet<i64> = item.observed_evidence_ids.iter().copied().collect();
    if evidence.len() != item.evidence_ids.len() {
        return Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            [item.item_key.clone()],
        ));
    }
    match item.truth_state {
        ApplicationModelTruthState::Observed => {
            if observed_evidence.is_empty()
                || observed_evidence.len() != item.observed_evidence_ids.len()
                || observed_evidence.iter().any(|id| *id <= 0)
            {
                return Err(block(
                    ApplicationModelGateCode::ObservedEvidenceMissing,
                    ApplicationModelGateDisposition::Rework,
                    [item.item_key.clone()],
                ));
            }
        }
        ApplicationModelTruthState::Inferred | ApplicationModelTruthState::Unknown => {
            if !observed_evidence.is_empty() {
                return Err(block(
                    ApplicationModelGateCode::TruthStateConflict,
                    ApplicationModelGateDisposition::Rework,
                    [item.item_key.clone()],
                ));
            }
        }
    }
    if observed_evidence.iter().any(|id| !evidence.contains(id)) {
        return Err(block(
            ApplicationModelGateCode::SchemaInvalid,
            ApplicationModelGateDisposition::Rework,
            [item.item_key.clone()],
        ));
    }
    if evidence.iter().any(|id| !authorized_evidence.contains(id)) {
        return Err(block(
            ApplicationModelGateCode::ForeignReference,
            ApplicationModelGateDisposition::Hold,
            item.evidence_ids.iter().map(ToString::to_string),
        ));
    }
    Ok(())
}

fn valid_reason_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn block(
    code: ApplicationModelGateCode,
    disposition: ApplicationModelGateDisposition,
    refs: impl IntoIterator<Item = String>,
) -> ApplicationModelGateBlock {
    let refs: BTreeSet<String> = refs
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect();
    ApplicationModelGateBlock {
        code,
        disposition,
        refs: refs.into_iter().collect(),
    }
}
