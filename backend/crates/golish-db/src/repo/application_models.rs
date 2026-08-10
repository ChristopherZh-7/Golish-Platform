use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const MANIFEST_NAMESPACE_LABEL: &[u8] = b"application-model-manifest:v1";
const APPLICATION_MODEL_PREDECESSOR_STAGES: [&str; 4] = [
    "target_intel",
    "external_attack_surface",
    "enumeration",
    "vuln_triage",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationModelAuthorityKindRow {
    Model,
    TerminalNoInput,
}

impl ApplicationModelAuthorityKindRow {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::TerminalNoInput => "terminal_no_input",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationModelManifestInputSeed {
    pub input_key: String,
    pub input_kind: String,
    pub source_handoff_id: Uuid,
    pub source_kind: String,
    pub source_id: String,
    pub source_version: i64,
    pub source_payload: Value,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SeedApplicationModelManifest {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub authority_kind: ApplicationModelAuthorityKindRow,
    pub inputs: Vec<ApplicationModelManifestInputSeed>,
}

/// Trusted owner identity for the operation-frozen runtime manifest. The caller cannot
/// supply source rows: the repository derives the complete current predecessor
/// denominator for this exact operation/snapshot/organization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeriveApplicationModelManifestSeed {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ApplicationModelManifestRow {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub stage_kind: String,
    pub authority_kind: String,
    pub input_count: i32,
    pub manifest_hash: String,
    pub replay_material_hash: String,
    pub row_version: i64,
    pub frozen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededApplicationModelManifest {
    pub manifest: ApplicationModelManifestRow,
    pub replayed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ApplicationModelStoreError {
    #[error("application model invalid input: {code}")]
    InvalidInput { code: &'static str },
    #[error("application model identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("application model replay conflict: {code}")]
    ReplayConflict { code: &'static str },
    #[error("application model SQL failure: {0}")]
    Sqlx(#[from] sqlx::Error),
}

impl ApplicationModelStoreError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput { code }
            | Self::IdentityMismatch { code }
            | Self::ReplayConflict { code } => code,
            Self::Sqlx(_) => "application_model_sql_failure",
        }
    }
}

pub type ApplicationModelStoreResult<T> = Result<T, ApplicationModelStoreError>;

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct ApplicationModelManifestInputRow {
    pub input_key: String,
    pub ordinal: i32,
    pub input_kind: String,
    pub source_handoff_id: Uuid,
    pub source_kind: String,
    pub source_id: String,
    pub source_version: i64,
    pub source_payload: Value,
    pub source_payload_hash: String,
    pub evidence_ids: Vec<i64>,
}

fn tagged_sha256_json(value: &Value) -> String {
    format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(value)
    )
}

fn application_model_predecessor_stage_kinds() -> Vec<String> {
    APPLICATION_MODEL_PREDECESSOR_STAGES
        .iter()
        .map(|stage| (*stage).to_string())
        .collect()
}

fn map_handoff_resolution_error(
    error: super::runtime_memory_tx::RuntimeMemoryStoreError,
) -> ApplicationModelStoreError {
    match error {
        super::runtime_memory_tx::RuntimeMemoryStoreError::Sqlx(error) => {
            ApplicationModelStoreError::Sqlx(error)
        }
        _ => ApplicationModelStoreError::IdentityMismatch {
            code: "manifest_source_authority_resolution_failed",
        },
    }
}

fn input_is_well_formed(input: &ApplicationModelManifestInputSeed) -> bool {
    !input.input_key.trim().is_empty()
        && input.input_key.len() <= 256
        && !input.input_kind.trim().is_empty()
        && input.input_kind.len() <= 64
        && !input.source_kind.trim().is_empty()
        && input.source_kind.len() <= 64
        && !input.source_id.trim().is_empty()
        && input.source_id.len() <= 512
        && input.source_version > 0
        && matches!(input.source_payload, Value::Object(_) | Value::Array(_))
        && input.evidence_ids.iter().all(|id| *id > 0)
        && input.evidence_ids.windows(2).all(|pair| pair[0] < pair[1])
}

fn canonical_inputs(
    seed: &SeedApplicationModelManifest,
) -> ApplicationModelStoreResult<Vec<ApplicationModelManifestInputSeed>> {
    let expected_shape = match seed.authority_kind {
        ApplicationModelAuthorityKindRow::Model => !seed.inputs.is_empty(),
        ApplicationModelAuthorityKindRow::TerminalNoInput => seed.inputs.is_empty(),
    };
    if !expected_shape {
        return Err(ApplicationModelStoreError::InvalidInput {
            code: "authority_input_shape_mismatch",
        });
    }
    if seed.inputs.len() > 10_000 || seed.inputs.iter().any(|input| !input_is_well_formed(input)) {
        return Err(ApplicationModelStoreError::InvalidInput {
            code: "manifest_input_invalid",
        });
    }
    let mut inputs = seed.inputs.clone();
    inputs.sort_by(|left, right| left.input_key.cmp(&right.input_key));
    if inputs
        .windows(2)
        .any(|pair| pair[0].input_key == pair[1].input_key)
    {
        return Err(ApplicationModelStoreError::InvalidInput {
            code: "manifest_input_key_duplicate",
        });
    }
    Ok(inputs)
}

async fn lock_manifest_owner(
    tx: &mut Transaction<'_, Postgres>,
    seed: &SeedApplicationModelManifest,
) -> ApplicationModelStoreResult<()> {
    let operation_exists = sqlx::query_scalar::<_, Uuid>(
        "SELECT operation_id FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(seed.operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .is_some();
    if !operation_exists {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "manifest_operation_missing",
        });
    }
    let owner_exists = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT unit.id
             FROM stage_run_units AS unit
             JOIN operation_org_scope_snapshots AS scope
               ON scope.id=unit.scope_snapshot_id
              AND scope.operation_id=unit.operation_id
            WHERE unit.id=$1
              AND unit.operation_id=$2
              AND unit.scope_snapshot_id=$3
              AND unit.stage_execution_id=$4
              AND unit.organization_id=$5
              AND unit.stage_kind='application_understanding'
              AND (
                    unit.status IN ('queued','running','gate_blocked')
                    OR (
                        unit.status='passed'
                        AND EXISTS (
                            SELECT 1 FROM application_model_manifests AS existing
                             WHERE existing.stage_run_unit_id=unit.id
                        )
                    )
                  )
              AND scope.sealed_at IS NOT NULL
            FOR UPDATE OF unit"#,
    )
    .bind(seed.stage_run_unit_id)
    .bind(seed.operation_id)
    .bind(seed.scope_snapshot_id)
    .bind(seed.stage_execution_id)
    .bind(seed.organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .is_some();
    if !owner_exists {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "manifest_owner_not_active_application_understanding_unit",
        });
    }
    Ok(())
}

async fn validate_complete_source_denominator(
    tx: &mut Transaction<'_, Postgres>,
    seed: &SeedApplicationModelManifest,
    inputs: &[ApplicationModelManifestInputSeed],
) -> ApplicationModelStoreResult<()> {
    let source_stage_kinds = application_model_predecessor_stage_kinds();
    let expected_handoff_ids =
        super::stage_handoffs::list_latest_final_sealed_for_sources_with_connection(
            tx,
            seed.operation_id,
            seed.organization_id,
            &source_stage_kinds,
        )
        .await
        .map_err(map_handoff_resolution_error)?
        .into_iter()
        .filter(|handoff| handoff.scope_snapshot_id == seed.scope_snapshot_id)
        .map(|handoff| handoff.id)
        .collect::<Vec<_>>();
    let mut actual_handoff_ids = inputs
        .iter()
        .map(|input| input.source_handoff_id)
        .collect::<Vec<_>>();
    actual_handoff_ids.sort_unstable();
    let mut expected_handoff_ids_sorted = expected_handoff_ids;
    expected_handoff_ids_sorted.sort_unstable();
    if actual_handoff_ids != expected_handoff_ids_sorted {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "manifest_source_denominator_mismatch",
        });
    }
    Ok(())
}

async fn validate_source_authority(
    tx: &mut Transaction<'_, Postgres>,
    seed: &SeedApplicationModelManifest,
    input: &ApplicationModelManifestInputSeed,
) -> ApplicationModelStoreResult<String> {
    let rows = super::stage_handoffs::list_latest_final_sealed_for_sources_with_connection(
        tx,
        seed.operation_id,
        seed.organization_id,
        std::slice::from_ref(&input.source_kind),
    )
    .await
    .map_err(map_handoff_resolution_error)?;
    let [row] = rows.as_slice() else {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "source_handoff_missing",
        });
    };
    let payload_hash = tagged_sha256_json(&input.source_payload);
    let evidence_is_subset = input
        .evidence_ids
        .iter()
        .all(|evidence_id| row.evidence_ids.binary_search(evidence_id).is_ok());
    if row.operation_id != seed.operation_id
        || row.scope_snapshot_id != seed.scope_snapshot_id
        || row.organization_id != seed.organization_id
        || row.from_stage_kind != input.source_kind
        || row.id != input.source_handoff_id
        || input.source_id != input.source_handoff_id.to_string()
        || i64::from(row.schema_version) != input.source_version
        || row.payload != input.source_payload
        || format!("sha256:{}", row.payload_sha256) != payload_hash
        || !evidence_is_subset
    {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "source_handoff_authority_mismatch",
        });
    }
    Ok(payload_hash)
}

fn manifest_material(
    seed: &SeedApplicationModelManifest,
    manifest_id: Uuid,
    inputs: &[ApplicationModelManifestInputSeed],
    source_hashes: &[String],
) -> Value {
    serde_json::json!({
        "schema_version": "application_model_manifest.v1",
        "manifest_id": manifest_id,
        "operation_id": seed.operation_id,
        "scope_snapshot_id": seed.scope_snapshot_id,
        "stage_execution_id": seed.stage_execution_id,
        "stage_run_unit_id": seed.stage_run_unit_id,
        "organization_id": seed.organization_id,
        "authority_kind": seed.authority_kind.as_str(),
        "inputs": inputs.iter().zip(source_hashes).enumerate().map(|(ordinal, (input, source_payload_hash))| {
            serde_json::json!({
                "input_key": input.input_key,
                "ordinal": ordinal,
                "input_kind": input.input_kind,
                "source_handoff_id": input.source_handoff_id,
                "source_kind": input.source_kind,
                "source_id": input.source_id,
                "source_version": input.source_version,
                "source_payload_hash": source_payload_hash,
                "evidence_ids": input.evidence_ids,
            })
        }).collect::<Vec<_>>(),
    })
}

async fn load_manifest_inputs(
    tx: &mut Transaction<'_, Postgres>,
    manifest_id: Uuid,
) -> ApplicationModelStoreResult<Vec<ApplicationModelManifestInputRow>> {
    Ok(sqlx::query_as::<_, ApplicationModelManifestInputRow>(
        r#"SELECT input_key,ordinal,input_kind,source_handoff_id,source_kind,
                  source_id,source_version,source_payload,source_payload_hash,evidence_ids
             FROM application_model_manifest_inputs
            WHERE manifest_id=$1
            ORDER BY ordinal"#,
    )
    .bind(manifest_id)
    .fetch_all(&mut **tx)
    .await?)
}

/// Derive and freeze one complete Application Understanding input manifest
/// from current final-sealed predecessor Handoffs. A true empty result is the
/// only path to terminal-no-input authority; the existing seed path locks and
/// revalidates the same denominator before writing anything.
pub async fn seed_manifest_from_current_predecessors(
    pool: &PgPool,
    owner: &DeriveApplicationModelManifestSeed,
) -> ApplicationModelStoreResult<SeededApplicationModelManifest> {
    let source_stage_kinds = application_model_predecessor_stage_kinds();
    let mut connection = pool.acquire().await?;
    let rows = super::stage_handoffs::list_latest_final_sealed_for_sources_with_connection(
        &mut connection,
        owner.operation_id,
        owner.organization_id,
        &source_stage_kinds,
    )
    .await
    .map_err(map_handoff_resolution_error)?
    .into_iter()
    .filter(|handoff| handoff.scope_snapshot_id == owner.scope_snapshot_id)
    .collect::<Vec<_>>();
    drop(connection);
    let authority_kind = if rows.is_empty() {
        ApplicationModelAuthorityKindRow::TerminalNoInput
    } else {
        ApplicationModelAuthorityKindRow::Model
    };
    let inputs = rows
        .into_iter()
        .map(|row| ApplicationModelManifestInputSeed {
            input_key: format!("{}:{}", row.from_stage_kind, row.id),
            input_kind: row.from_stage_kind.clone(),
            source_handoff_id: row.id,
            source_kind: row.from_stage_kind,
            source_id: row.id.to_string(),
            source_version: i64::from(row.schema_version),
            source_payload: row.payload,
            evidence_ids: row.evidence_ids,
        })
        .collect();
    seed_manifest(
        pool,
        &SeedApplicationModelManifest {
            operation_id: owner.operation_id,
            scope_snapshot_id: owner.scope_snapshot_id,
            stage_execution_id: owner.stage_execution_id,
            stage_run_unit_id: owner.stage_run_unit_id,
            organization_id: owner.organization_id,
            authority_kind,
            inputs,
        },
    )
    .await
}

/// Freeze the exact, final-sealed inputs for one Application Understanding
/// Understanding unit. This function cannot publish a current revision,
/// StageHandoff, Finding, Candidate, or Gate result.
pub async fn seed_manifest(
    pool: &PgPool,
    seed: &SeedApplicationModelManifest,
) -> ApplicationModelStoreResult<SeededApplicationModelManifest> {
    let inputs = canonical_inputs(seed)?;
    let manifest_id = Uuid::new_v5(&seed.stage_run_unit_id, MANIFEST_NAMESPACE_LABEL);
    let mut tx = pool.begin().await?;
    lock_manifest_owner(&mut tx, seed).await?;
    validate_complete_source_denominator(&mut tx, seed, &inputs).await?;
    let mut source_hashes = Vec::with_capacity(inputs.len());
    for input in &inputs {
        source_hashes.push(validate_source_authority(&mut tx, seed, input).await?);
    }
    let material = manifest_material(seed, manifest_id, &inputs, &source_hashes);
    let manifest_hash = tagged_sha256_json(&material);
    let replay_material_hash = manifest_hash.clone();
    let existing = sqlx::query_as::<_, ApplicationModelManifestRow>(
        r#"SELECT id,operation_id,scope_snapshot_id,stage_execution_id,
                  stage_run_unit_id,organization_id,stage_kind,authority_kind,
                  input_count,manifest_hash,replay_material_hash,row_version,frozen_at
             FROM application_model_manifests
            WHERE stage_run_unit_id=$1
            FOR UPDATE"#,
    )
    .bind(seed.stage_run_unit_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(manifest) = existing {
        let persisted_inputs = load_manifest_inputs(&mut tx, manifest.id).await?;
        let inputs_match = persisted_inputs.len() == inputs.len()
            && persisted_inputs
                .iter()
                .zip(inputs.iter().zip(&source_hashes))
                .enumerate()
                .all(|(ordinal, (persisted, (input, source_hash)))| {
                    persisted.input_key == input.input_key
                        && persisted.ordinal == i32::try_from(ordinal).unwrap_or(i32::MAX)
                        && persisted.input_kind == input.input_kind
                        && persisted.source_handoff_id == input.source_handoff_id
                        && persisted.source_kind == input.source_kind
                        && persisted.source_id == input.source_id
                        && persisted.source_version == input.source_version
                        && persisted.source_payload == input.source_payload
                        && persisted.source_payload_hash == *source_hash
                        && persisted.evidence_ids == input.evidence_ids
                });
        let header_matches = manifest.id == manifest_id
            && manifest.operation_id == seed.operation_id
            && manifest.scope_snapshot_id == seed.scope_snapshot_id
            && manifest.stage_execution_id == seed.stage_execution_id
            && manifest.organization_id == seed.organization_id
            && manifest.authority_kind == seed.authority_kind.as_str()
            && manifest.input_count == i32::try_from(inputs.len()).unwrap_or(i32::MAX)
            && manifest.manifest_hash == manifest_hash
            && manifest.replay_material_hash == replay_material_hash;
        if !header_matches || !inputs_match {
            return Err(ApplicationModelStoreError::ReplayConflict {
                code: "manifest_replay_drift",
            });
        }
        tx.commit().await?;
        return Ok(SeededApplicationModelManifest {
            manifest,
            replayed: true,
        });
    }

    let manifest = sqlx::query_as::<_, ApplicationModelManifestRow>(
        r#"INSERT INTO application_model_manifests(
               id,operation_id,scope_snapshot_id,stage_execution_id,
               stage_run_unit_id,organization_id,stage_kind,authority_kind,
               input_count,manifest_hash,replay_material_hash
           ) VALUES($1,$2,$3,$4,$5,$6,'application_understanding',$7,$8,$9,$10)
           RETURNING id,operation_id,scope_snapshot_id,stage_execution_id,
                     stage_run_unit_id,organization_id,stage_kind,authority_kind,
                     input_count,manifest_hash,replay_material_hash,row_version,frozen_at"#,
    )
    .bind(manifest_id)
    .bind(seed.operation_id)
    .bind(seed.scope_snapshot_id)
    .bind(seed.stage_execution_id)
    .bind(seed.stage_run_unit_id)
    .bind(seed.organization_id)
    .bind(seed.authority_kind.as_str())
    .bind(
        i32::try_from(inputs.len()).map_err(|_| ApplicationModelStoreError::InvalidInput {
            code: "manifest_input_count_overflow",
        })?,
    )
    .bind(&manifest_hash)
    .bind(&replay_material_hash)
    .fetch_one(&mut *tx)
    .await?;
    for (ordinal, (input, source_payload_hash)) in inputs.iter().zip(&source_hashes).enumerate() {
        sqlx::query(
            r#"INSERT INTO application_model_manifest_inputs(
                   manifest_id,operation_id,scope_snapshot_id,stage_execution_id,
                   stage_run_unit_id,organization_id,input_key,ordinal,input_kind,
                   source_handoff_id,source_kind,source_id,source_version,source_payload,
                   source_payload_hash,evidence_ids
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16)"#,
        )
        .bind(manifest.id)
        .bind(seed.operation_id)
        .bind(seed.scope_snapshot_id)
        .bind(seed.stage_execution_id)
        .bind(seed.stage_run_unit_id)
        .bind(seed.organization_id)
        .bind(&input.input_key)
        .bind(
            i32::try_from(ordinal).map_err(|_| ApplicationModelStoreError::InvalidInput {
                code: "manifest_input_ordinal_overflow",
            })?,
        )
        .bind(&input.input_kind)
        .bind(input.source_handoff_id)
        .bind(&input.source_kind)
        .bind(&input.source_id)
        .bind(input.source_version)
        .bind(&input.source_payload)
        .bind(source_payload_hash)
        .bind(&input.evidence_ids)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(SeededApplicationModelManifest {
        manifest,
        replayed: false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationModelInputDispositionRow {
    Incorporated,
    Duplicate,
    NotRelevant,
    Unknown,
}

impl ApplicationModelInputDispositionRow {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Incorporated => "incorporated",
            Self::Duplicate => "duplicate",
            Self::NotRelevant => "not_relevant",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationModelTruthStateRow {
    Observed,
    Inferred,
    Unknown,
}

impl ApplicationModelTruthStateRow {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Inferred => "inferred",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationModelEvidenceRoleRow {
    Observation,
    Support,
}

impl ApplicationModelEvidenceRoleRow {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Support => "support",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationModelInputDecisionSeed {
    pub input_key: String,
    pub disposition: ApplicationModelInputDispositionRow,
    pub item_keys: Vec<String>,
    pub duplicate_input_key: Option<String>,
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationModelItemEvidenceSeed {
    pub evidence_id: i64,
    pub role: ApplicationModelEvidenceRoleRow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationModelItemSeed {
    pub item_key: String,
    pub item_kind: String,
    pub truth_state: ApplicationModelTruthStateRow,
    pub source_input_keys: Vec<String>,
    pub referenced_item_keys: Vec<String>,
    pub payload: Value,
    pub evidence: Vec<ApplicationModelItemEvidenceSeed>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProposeApplicationModelRevision {
    pub manifest_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub source_submission_id: Uuid,
    pub structured_model: Value,
    pub decisions: Vec<ApplicationModelInputDecisionSeed>,
    pub items: Vec<ApplicationModelItemSeed>,
}

/// Exact owner fence used to discover a durable Application Understanding
/// submission that was committed before its proposed revision.  The caller
/// cannot select a submission id; the repository derives the only standalone
/// receipt for this manifest/Unit and classifies whether it still belongs to
/// the currently live Worker attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadStandaloneApplicationModelSubmission {
    pub manifest_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub worker_run_id: Uuid,
    pub attempt_epoch: i64,
    pub lease_token: Uuid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StandaloneApplicationModelSubmissionRow {
    pub deliverable_submission_id: Uuid,
    pub payload: Value,
    pub payload_sha256: String,
    pub tool_status: String,
    pub recoverable_by_current_fence: bool,
    pub requires_reauthorization: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct StandaloneApplicationModelSubmissionQueryRow {
    deliverable_submission_id: Uuid,
    payload: Value,
    payload_sha256: String,
    tool_status: String,
    tool_result: Option<String>,
    recoverable_by_current_fence: bool,
    requires_reauthorization: bool,
}

/// Load the only durable submit receipt that is not yet referenced by either
/// a proposed/final revision or the current authority pointer.  A terminal
/// tool receipt plus the exact current Worker fence permits reconstruction
/// from the immutable canonical payload.  Every other shape remains visible
/// to the runtime as outcome-unknown and must HOLD rather than invoke the
/// model provider again.
pub async fn load_standalone_submission(
    pool: &PgPool,
    input: &LoadStandaloneApplicationModelSubmission,
) -> ApplicationModelStoreResult<Option<StandaloneApplicationModelSubmissionRow>> {
    let rows = sqlx::query_as::<_, StandaloneApplicationModelSubmissionQueryRow>(
        r#"SELECT submission.id AS deliverable_submission_id,
                  submission.payload,submission.payload_sha256,
                  tool.status::TEXT AS tool_status,tool.result AS tool_result,
                  (
                      tool.status='finished'
                      AND worker.id=$7
                      AND worker.status='running'
                      AND worker.active_tool_call_id IS NULL
                      AND worker.attempt_epoch=$8
                      AND worker.lease_token=$9
                      AND worker.lease_expires_at>NOW()
                      AND submission.worker_run_id=$7
                      AND submission.attempt_epoch IS NOT NULL
                      AND submission.attempt_epoch<=$8
                      AND submission.lease_token IS NOT NULL
                  ) AS recoverable_by_current_fence,
                  (
                      submission.attempt_epoch IS DISTINCT FROM $8
                      OR submission.lease_token IS DISTINCT FROM $9
                  ) AS requires_reauthorization
             FROM application_model_manifests AS manifest
             JOIN stage_deliverable_submissions AS submission
               ON submission.operation_id=manifest.operation_id
              AND submission.stage_execution_id=manifest.stage_execution_id
              AND submission.stage_run_unit_id=manifest.stage_run_unit_id
              AND submission.organization_id=manifest.organization_id
              AND submission.stage_kind='application_understanding'
             JOIN stage_run_units AS unit
               ON unit.id=manifest.stage_run_unit_id
              AND unit.operation_id=manifest.operation_id
              AND unit.scope_snapshot_id=manifest.scope_snapshot_id
              AND unit.stage_execution_id=manifest.stage_execution_id
              AND unit.organization_id=manifest.organization_id
              AND unit.stage_kind='application_understanding'
             JOIN stage_worker_runs AS worker
               ON worker.id=submission.worker_run_id
              AND worker.operation_id=submission.operation_id
              AND worker.stage_execution_id=submission.stage_execution_id
              AND worker.stage_run_unit_id=submission.stage_run_unit_id
              AND worker.organization_id=submission.organization_id
             JOIN tool_calls AS tool
               ON tool.id=submission.tool_call_record_id
              AND tool.call_id=submission.tool_request_id
              AND tool.name='submit_stage_deliverable'
              AND tool.operation_id=submission.operation_id
              AND tool.stage_execution_id=submission.stage_execution_id
              AND tool.stage_run_unit_id=submission.stage_run_unit_id
              AND tool.worker_run_id=submission.worker_run_id
              AND tool.organization_id=submission.organization_id
              AND tool.attempt_epoch=submission.attempt_epoch
              AND tool.lease_token=submission.lease_token
            WHERE manifest.id=$1
              AND manifest.operation_id=$2
              AND manifest.scope_snapshot_id=$3
              AND manifest.stage_execution_id=$4
              AND manifest.stage_run_unit_id=$5
              AND manifest.organization_id=$6
              AND NOT EXISTS (
                    SELECT 1 FROM application_model_revisions AS revision
                     WHERE revision.source_submission_id=submission.id
              )
              AND NOT EXISTS (
                    SELECT 1 FROM application_model_current_revisions AS current_revision
                     WHERE current_revision.deliverable_submission_id=submission.id
              )
            ORDER BY submission.submitted_at,submission.id
            LIMIT 2"#,
    )
    .bind(input.manifest_id)
    .bind(input.operation_id)
    .bind(input.scope_snapshot_id)
    .bind(input.stage_execution_id)
    .bind(input.stage_run_unit_id)
    .bind(input.organization_id)
    .bind(input.worker_run_id)
    .bind(input.attempt_epoch)
    .bind(input.lease_token)
    .fetch_all(pool)
    .await?;
    if rows.len() > 1 {
        return Err(ApplicationModelStoreError::ReplayConflict {
            code: "standalone_submission_ambiguous",
        });
    }
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    if row.payload_sha256 != super::operation_scope_decisions::sha256_json(&row.payload) {
        return Err(ApplicationModelStoreError::ReplayConflict {
            code: "standalone_submission_payload_hash_mismatch",
        });
    }
    let terminal_result_matches = row.tool_status == "finished"
        && row
            .tool_result
            .as_deref()
            .and_then(|result| serde_json::from_str::<Value>(result).ok())
            .is_some_and(|result| {
                result.get("accepted").and_then(Value::as_bool) == Some(true)
                    && result
                        .get("deliverable_submission_id")
                        .and_then(Value::as_str)
                        .and_then(|value| Uuid::parse_str(value).ok())
                        == Some(row.deliverable_submission_id)
            });
    Ok(Some(StandaloneApplicationModelSubmissionRow {
        deliverable_submission_id: row.deliverable_submission_id,
        payload: row.payload,
        payload_sha256: row.payload_sha256,
        tool_status: row.tool_status,
        recoverable_by_current_fence: row.recoverable_by_current_fence && terminal_result_matches,
        requires_reauthorization: row.requires_reauthorization,
    }))
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct ApplicationModelRevisionRow {
    pub id: Uuid,
    pub manifest_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
    pub revision_ordinal: i32,
    pub stage_kind: String,
    pub schema_version: String,
    pub status: String,
    pub structured_model: Value,
    pub model_hash: String,
    pub replay_material_hash: String,
    pub source_submission_id: Uuid,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub finalized_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ApplicationModelInputDecisionRow {
    pub revision_id: Uuid,
    pub manifest_id: Uuid,
    pub input_key: String,
    pub disposition: String,
    pub item_keys: Vec<String>,
    pub duplicate_input_key: Option<String>,
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct ApplicationModelItemRow {
    pub revision_id: Uuid,
    pub manifest_id: Uuid,
    pub item_key: String,
    pub ordinal: i32,
    pub item_kind: String,
    pub truth_state: String,
    pub source_input_keys: Vec<String>,
    pub referenced_item_keys: Vec<String>,
    pub payload: Value,
    pub payload_hash: String,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ApplicationModelItemEvidenceRow {
    pub revision_id: Uuid,
    pub manifest_id: Uuid,
    pub item_key: String,
    pub evidence_id: i64,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProposedApplicationModelRevision {
    pub revision: ApplicationModelRevisionRow,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadApplicationModelGateMaterial {
    pub manifest_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub organization_id: Uuid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationModelGateMaterial {
    pub manifest: ApplicationModelManifestRow,
    pub inputs: Vec<ApplicationModelManifestInputRow>,
    pub revision: Option<ApplicationModelRevisionRow>,
    pub decisions: Vec<ApplicationModelInputDecisionRow>,
    pub items: Vec<ApplicationModelItemRow>,
    pub item_evidence: Vec<ApplicationModelItemEvidenceRow>,
    pub forbidden_activity_refs: Vec<String>,
    pub pending_producer_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationModelExpectedGateHashes {
    pub manifest_hash: String,
    pub model_hash: Option<String>,
    pub replay_material_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockApplicationModelFinalizeAuthority {
    pub gate: LoadApplicationModelGateMaterial,
    pub fence: super::runtime_memory_tx::RuntimeMemoryTxFence,
    pub deliverable_submission_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationModelFinalizeBarrier {
    pub forbidden_activity_refs: Vec<String>,
    pub pending_producer_refs: Vec<String>,
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq, Eq)]
pub struct ApplicationModelCurrentRevisionRow {
    pub manifest_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub authority_kind: String,
    pub stage_handoff_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub manifest_hash: String,
    pub model_hash: Option<String>,
    pub replay_material_hash: String,
    pub gate_decision_hash: String,
    pub published_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishApplicationModelCurrentRevision {
    pub manifest_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub authority_kind: String,
    pub stage_handoff_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub manifest_hash: String,
    pub model_hash: Option<String>,
    pub replay_material_hash: String,
    pub gate_decision_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct FinalizeWorkerAuthorityRow {
    id: Uuid,
    status: String,
    attempt_epoch: i64,
    checkpoint_version: i64,
    lease_token: Option<Uuid>,
    lease_live: bool,
    active_tool_call_id: Option<Uuid>,
}

/// Rebuild the Gate hashes from relational content instead of trusting the
/// persisted hash columns. The same canonical shapes are used by the S1 write
/// paths, so the app adapter can compare stored authority with independently
/// reconstructed truth.
pub fn recompute_gate_hashes(
    material: &ApplicationModelGateMaterial,
) -> ApplicationModelExpectedGateHashes {
    let manifest = &material.manifest;
    let manifest_material = serde_json::json!({
        "schema_version": "application_model_manifest.v1",
        "manifest_id": manifest.id,
        "operation_id": manifest.operation_id,
        "scope_snapshot_id": manifest.scope_snapshot_id,
        "stage_execution_id": manifest.stage_execution_id,
        "stage_run_unit_id": manifest.stage_run_unit_id,
        "organization_id": manifest.organization_id,
        "authority_kind": manifest.authority_kind,
        "inputs": material.inputs.iter().map(|input| serde_json::json!({
            "input_key": input.input_key,
            "ordinal": input.ordinal,
            "input_kind": input.input_kind,
            "source_handoff_id": input.source_handoff_id,
            "source_kind": input.source_kind,
            "source_id": input.source_id,
            "source_version": input.source_version,
            "source_payload_hash": tagged_sha256_json(&input.source_payload),
            "evidence_ids": input.evidence_ids,
        })).collect::<Vec<_>>(),
    });
    let manifest_hash = tagged_sha256_json(&manifest_material);
    let Some(revision) = material.revision.as_ref() else {
        return ApplicationModelExpectedGateHashes {
            replay_material_hash: manifest_hash.clone(),
            manifest_hash,
            model_hash: None,
        };
    };
    let evidence_by_item = material.item_evidence.iter().fold(
        std::collections::BTreeMap::<&str, Vec<Value>>::new(),
        |mut grouped, evidence| {
            grouped
                .entry(evidence.item_key.as_str())
                .or_default()
                .push(serde_json::json!({
                    "evidence_id": evidence.evidence_id,
                    "role": evidence.role,
                }));
            grouped
        },
    );
    let revision_material = serde_json::json!({
        "schema_version": "application_model_revision.v1",
        "manifest_id": revision.manifest_id,
        "operation_id": revision.operation_id,
        "scope_snapshot_id": revision.scope_snapshot_id,
        "stage_execution_id": revision.stage_execution_id,
        "stage_run_unit_id": revision.stage_run_unit_id,
        "organization_id": revision.organization_id,
        "source_submission_id": revision.source_submission_id,
        "structured_model": revision.structured_model,
        "decisions": material.decisions.iter().map(|decision| serde_json::json!({
            "input_key": decision.input_key,
            "disposition": decision.disposition,
            "item_keys": decision.item_keys,
            "duplicate_input_key": decision.duplicate_input_key,
            "reason_code": decision.reason_code,
        })).collect::<Vec<_>>(),
        "items": material.items.iter().map(|item| serde_json::json!({
            "item_key": item.item_key,
            "item_kind": item.item_kind,
            "truth_state": item.truth_state,
            "source_input_keys": item.source_input_keys,
            "referenced_item_keys": item.referenced_item_keys,
            "payload": item.payload,
            "evidence": evidence_by_item.get(item.item_key.as_str()).cloned().unwrap_or_default(),
        })).collect::<Vec<_>>(),
    });
    ApplicationModelExpectedGateHashes {
        manifest_hash,
        model_hash: Some(tagged_sha256_json(&revision.structured_model)),
        replay_material_hash: tagged_sha256_json(&revision_material),
    }
}

#[derive(Debug, sqlx::FromRow)]
struct ProposalSubmissionAuthorityRow {
    payload: Value,
    worker_status: String,
    unit_status: String,
    worker_attempt_epoch: i64,
    submission_attempt_epoch: Option<i64>,
    worker_lease_token: Option<Uuid>,
    submission_lease_token: Option<Uuid>,
    active_tool_call_id: Option<Uuid>,
    lease_live: bool,
}

fn string_array_is_canonical(values: &[String], allow_empty: bool) -> bool {
    (allow_empty || !values.is_empty())
        && values
            .iter()
            .all(|value| !value.trim().is_empty() && value.len() <= 256)
        && values.windows(2).all(|pair| pair[0] < pair[1])
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicationModelV1Body {
    organization_id: Uuid,
    summary: String,
    technologies: Vec<String>,
    routes_and_pages: Vec<String>,
    api_surfaces: Vec<String>,
    roles_and_identities: Vec<String>,
    business_entities: Vec<String>,
    workflows: Vec<String>,
    state_transitions: Vec<String>,
    ownership_rules: Vec<String>,
    sensitive_operations: Vec<String>,
    trust_boundaries: Vec<String>,
    unknowns: Vec<String>,
}

fn validate_application_model_v1_body(
    structured_model: &Value,
    items: &[ApplicationModelItemSeed],
) -> Option<Uuid> {
    if serde_json::to_vec(structured_model).ok()?.len() > 262_144 {
        return None;
    }
    let model = serde_json::from_value::<ApplicationModelV1Body>(structured_model.clone()).ok()?;
    if model.summary.trim().is_empty() || model.summary.len() > 4096 {
        return None;
    }
    let collections = [
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
    let total_keys = collections.iter().map(|values| values.len()).sum::<usize>();
    if total_keys > 50_000 {
        return None;
    }
    let item_keys = items
        .iter()
        .map(|item| item.item_key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut model_keys = std::collections::HashSet::with_capacity(total_keys);
    if !collections.into_iter().flatten().all(|key| {
        !key.trim().is_empty()
            && key.len() <= 256
            && item_keys.contains(key.as_str())
            && model_keys.insert(key.as_str())
    }) {
        return None;
    }
    Some(model.organization_id)
}

fn proposal_content_shape_is_valid(
    structured_model: &Value,
    decisions: &[ApplicationModelInputDecisionSeed],
    items: &[ApplicationModelItemSeed],
) -> bool {
    if decisions.is_empty()
        || decisions.len() > 10_000
        || items.len() > 50_000
        || validate_application_model_v1_body(structured_model, items).is_none()
    {
        return false;
    }
    let decisions_valid = decisions.iter().all(|decision| {
        !decision.input_key.trim().is_empty()
            && string_array_is_canonical(&decision.item_keys, true)
            && match decision.disposition {
                ApplicationModelInputDispositionRow::Incorporated => {
                    !decision.item_keys.is_empty()
                        && decision.duplicate_input_key.is_none()
                        && decision.reason_code.is_none()
                }
                ApplicationModelInputDispositionRow::Duplicate => {
                    decision.item_keys.is_empty()
                        && decision.duplicate_input_key.as_ref().is_some_and(|value| {
                            !value.trim().is_empty() && value != &decision.input_key
                        })
                        && decision.reason_code.is_none()
                }
                ApplicationModelInputDispositionRow::NotRelevant
                | ApplicationModelInputDispositionRow::Unknown => {
                    decision.item_keys.is_empty()
                        && decision.duplicate_input_key.is_none()
                        && decision.reason_code.as_ref().is_some_and(|code| {
                            !code.is_empty()
                                && code.len() <= 64
                                && code.bytes().all(|byte| {
                                    byte.is_ascii_lowercase()
                                        || byte.is_ascii_digit()
                                        || byte == b'_'
                                })
                        })
                }
            }
    });
    let items_valid = items.iter().all(|item| {
        !item.item_key.trim().is_empty()
            && item.item_key.len() <= 256
            && !item.item_kind.trim().is_empty()
            && item.item_kind.len() <= 64
            && item.payload.is_object()
            && string_array_is_canonical(&item.source_input_keys, false)
            && string_array_is_canonical(&item.referenced_item_keys, true)
            && !item.referenced_item_keys.contains(&item.item_key)
            && item
                .evidence
                .iter()
                .all(|evidence| evidence.evidence_id > 0)
            && item
                .evidence
                .windows(2)
                .all(|pair| pair[0].evidence_id < pair[1].evidence_id)
            && match item.truth_state {
                ApplicationModelTruthStateRow::Observed => item
                    .evidence
                    .iter()
                    .any(|evidence| evidence.role == ApplicationModelEvidenceRoleRow::Observation),
                ApplicationModelTruthStateRow::Inferred
                | ApplicationModelTruthStateRow::Unknown => item
                    .evidence
                    .iter()
                    .all(|evidence| evidence.role != ApplicationModelEvidenceRoleRow::Observation),
            }
    });
    decisions_valid
        && items_valid
        && decisions
            .windows(2)
            .all(|pair| pair[0].input_key < pair[1].input_key)
        && items
            .windows(2)
            .all(|pair| pair[0].item_key < pair[1].item_key)
}

/// Perform the same canonical structural validation as `propose_revision`
/// before the formal runtime writes its immutable submit receipt.
pub fn validate_proposal_content_shape(
    structured_model: &Value,
    decisions: &[ApplicationModelInputDecisionSeed],
    items: &[ApplicationModelItemSeed],
) -> ApplicationModelStoreResult<()> {
    if proposal_content_shape_is_valid(structured_model, decisions, items) {
        Ok(())
    } else {
        Err(ApplicationModelStoreError::InvalidInput {
            code: "proposed_revision_shape_invalid",
        })
    }
}

fn proposal_shape_is_valid(proposal: &ProposeApplicationModelRevision) -> bool {
    proposal_content_shape_is_valid(
        &proposal.structured_model,
        &proposal.decisions,
        &proposal.items,
    ) && validate_application_model_v1_body(&proposal.structured_model, &proposal.items)
        == Some(proposal.organization_id)
}

fn proposal_content_material(proposal: &ProposeApplicationModelRevision) -> Value {
    serde_json::json!({
        "schema_version": "application_model_proposal_content.v1",
        "manifest_id": proposal.manifest_id,
        "structured_model": proposal.structured_model,
        "decisions": proposal_decisions_material(proposal),
        "items": proposal_items_material(proposal),
    })
}

fn compact_submission_matches_proposal(
    payload: &Value,
    proposal: &ProposeApplicationModelRevision,
) -> bool {
    let expected_hash = tagged_sha256_json(&proposal_content_material(proposal));
    payload.as_object().is_some_and(|object| object.len() == 8)
        && payload.get("stage_id").and_then(Value::as_str) == Some("application_understanding")
        && payload
            .get("stage_run_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            == Some(proposal.stage_execution_id)
        && payload.get("schema_version").and_then(Value::as_i64) == Some(1)
        && payload
            .get("manifest_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            == Some(proposal.manifest_id)
        && payload.get("authority_kind").and_then(Value::as_str) == Some("model")
        && payload
            .get("proposal_material_hash")
            .and_then(Value::as_str)
            == Some(expected_hash.as_str())
        && payload.get("decision_count").and_then(Value::as_u64)
            == u64::try_from(proposal.decisions.len()).ok()
        && payload.get("item_count").and_then(Value::as_u64)
            == u64::try_from(proposal.items.len()).ok()
}

fn proposal_material(proposal: &ProposeApplicationModelRevision) -> Value {
    serde_json::json!({
        "schema_version": "application_model_revision.v1",
        "manifest_id": proposal.manifest_id,
        "operation_id": proposal.operation_id,
        "scope_snapshot_id": proposal.scope_snapshot_id,
        "stage_execution_id": proposal.stage_execution_id,
        "stage_run_unit_id": proposal.stage_run_unit_id,
        "organization_id": proposal.organization_id,
        "source_submission_id": proposal.source_submission_id,
        "structured_model": proposal.structured_model,
        "decisions": proposal.decisions.iter().map(|decision| serde_json::json!({
            "input_key": decision.input_key,
            "disposition": decision.disposition.as_str(),
            "item_keys": decision.item_keys,
            "duplicate_input_key": decision.duplicate_input_key,
            "reason_code": decision.reason_code,
        })).collect::<Vec<_>>(),
        "items": proposal.items.iter().map(|item| serde_json::json!({
            "item_key": item.item_key,
            "item_kind": item.item_kind,
            "truth_state": item.truth_state.as_str(),
            "source_input_keys": item.source_input_keys,
            "referenced_item_keys": item.referenced_item_keys,
            "payload": item.payload,
            "evidence": item.evidence.iter().map(|evidence| serde_json::json!({
                "evidence_id": evidence.evidence_id,
                "role": evidence.role.as_str(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn proposal_decisions_material(proposal: &ProposeApplicationModelRevision) -> Vec<Value> {
    proposal
        .decisions
        .iter()
        .map(|decision| {
            serde_json::json!({
                "input_key": decision.input_key,
                "disposition": decision.disposition.as_str(),
                "item_keys": decision.item_keys,
                "duplicate_input_key": decision.duplicate_input_key,
                "reason_code": decision.reason_code,
            })
        })
        .collect()
}

fn proposal_items_material(proposal: &ProposeApplicationModelRevision) -> Vec<Value> {
    proposal
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "item_key": item.item_key,
                "item_kind": item.item_kind,
                "truth_state": item.truth_state.as_str(),
                "source_input_keys": item.source_input_keys,
                "referenced_item_keys": item.referenced_item_keys,
                "payload": item.payload,
                "evidence": item.evidence.iter().map(|evidence| serde_json::json!({
                    "evidence_id": evidence.evidence_id,
                    "role": evidence.role.as_str(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

const REVISION_COLUMNS: &str = r#"id,manifest_id,operation_id,scope_snapshot_id,
    stage_execution_id,stage_run_unit_id,organization_id,revision_ordinal,stage_kind,
    schema_version,status,structured_model,model_hash,replay_material_hash,
    source_submission_id,row_version,created_at,finalized_at"#;

async fn load_revision_decisions(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> ApplicationModelStoreResult<Vec<ApplicationModelInputDecisionRow>> {
    Ok(sqlx::query_as::<_, ApplicationModelInputDecisionRow>(
        r#"SELECT revision_id,manifest_id,input_key,disposition,item_keys,
                  duplicate_input_key,reason_code
             FROM application_model_input_decisions
            WHERE revision_id=$1 ORDER BY input_key"#,
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}

async fn load_revision_items(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> ApplicationModelStoreResult<Vec<ApplicationModelItemRow>> {
    Ok(sqlx::query_as::<_, ApplicationModelItemRow>(
        r#"SELECT revision_id,manifest_id,item_key,ordinal,item_kind,truth_state,
                  source_input_keys,referenced_item_keys,payload,payload_hash
             FROM application_model_items
            WHERE revision_id=$1 ORDER BY ordinal"#,
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}

async fn load_revision_evidence(
    tx: &mut Transaction<'_, Postgres>,
    revision_id: Uuid,
) -> ApplicationModelStoreResult<Vec<ApplicationModelItemEvidenceRow>> {
    Ok(sqlx::query_as::<_, ApplicationModelItemEvidenceRow>(
        r#"SELECT revision_id,manifest_id,item_key,evidence_id,role
             FROM application_model_item_evidence
            WHERE revision_id=$1 ORDER BY item_key,evidence_id"#,
    )
    .bind(revision_id)
    .fetch_all(&mut **tx)
    .await?)
}

async fn persisted_proposal_matches(
    tx: &mut Transaction<'_, Postgres>,
    revision: &ApplicationModelRevisionRow,
    proposal: &ProposeApplicationModelRevision,
) -> ApplicationModelStoreResult<bool> {
    let decisions = load_revision_decisions(tx, revision.id).await?;
    let items = load_revision_items(tx, revision.id).await?;
    let evidence = load_revision_evidence(tx, revision.id).await?;
    let decisions_match = decisions.len() == proposal.decisions.len()
        && decisions
            .iter()
            .zip(&proposal.decisions)
            .all(|(row, seed)| {
                row.manifest_id == proposal.manifest_id
                    && row.input_key == seed.input_key
                    && row.disposition == seed.disposition.as_str()
                    && row.item_keys == seed.item_keys
                    && row.duplicate_input_key == seed.duplicate_input_key
                    && row.reason_code == seed.reason_code
            });
    let items_match = items.len() == proposal.items.len()
        && items
            .iter()
            .zip(&proposal.items)
            .enumerate()
            .all(|(ordinal, (row, seed))| {
                row.manifest_id == proposal.manifest_id
                    && row.item_key == seed.item_key
                    && row.ordinal == i32::try_from(ordinal).unwrap_or(i32::MAX)
                    && row.item_kind == seed.item_kind
                    && row.truth_state == seed.truth_state.as_str()
                    && row.source_input_keys == seed.source_input_keys
                    && row.referenced_item_keys == seed.referenced_item_keys
                    && row.payload == seed.payload
                    && row.payload_hash == tagged_sha256_json(&seed.payload)
            });
    let expected_evidence = proposal
        .items
        .iter()
        .flat_map(|item| {
            item.evidence.iter().map(|item_evidence| {
                (
                    item.item_key.as_str(),
                    item_evidence.evidence_id,
                    item_evidence.role.as_str(),
                )
            })
        })
        .collect::<Vec<_>>();
    let evidence_match = evidence.len() == expected_evidence.len()
        && evidence
            .iter()
            .zip(expected_evidence)
            .all(|(row, expected)| {
                row.manifest_id == proposal.manifest_id
                    && row.item_key == expected.0
                    && row.evidence_id == expected.1
                    && row.role == expected.2
            });
    Ok(decisions_match && items_match && evidence_match)
}

/// Persist one proposed Application Model revision. The revision remains
/// non-authoritative until the later deterministic Gate finalizer publishes it.
pub async fn propose_revision(
    pool: &PgPool,
    proposal: &ProposeApplicationModelRevision,
) -> ApplicationModelStoreResult<ProposedApplicationModelRevision> {
    if !proposal_shape_is_valid(proposal) {
        return Err(ApplicationModelStoreError::InvalidInput {
            code: "proposed_revision_shape_invalid",
        });
    }
    let mut tx = pool.begin().await?;
    let manifest = sqlx::query_as::<_, ApplicationModelManifestRow>(
        r#"SELECT id,operation_id,scope_snapshot_id,stage_execution_id,
                  stage_run_unit_id,organization_id,stage_kind,authority_kind,
                  input_count,manifest_hash,replay_material_hash,row_version,frozen_at
             FROM application_model_manifests WHERE id=$1 FOR SHARE"#,
    )
    .bind(proposal.manifest_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApplicationModelStoreError::IdentityMismatch {
        code: "proposal_manifest_missing",
    })?;
    if manifest.operation_id != proposal.operation_id
        || manifest.scope_snapshot_id != proposal.scope_snapshot_id
        || manifest.stage_execution_id != proposal.stage_execution_id
        || manifest.stage_run_unit_id != proposal.stage_run_unit_id
        || manifest.organization_id != proposal.organization_id
        || manifest.authority_kind != "model"
    {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "proposal_manifest_owner_mismatch",
        });
    }
    let submission = sqlx::query_as::<_, ProposalSubmissionAuthorityRow>(
        r#"SELECT submission.payload,worker.status AS worker_status,
                  unit.status AS unit_status,
                  worker.attempt_epoch AS worker_attempt_epoch,
                  submission.attempt_epoch AS submission_attempt_epoch,
                  worker.lease_token AS worker_lease_token,
                  submission.lease_token AS submission_lease_token,
                  worker.active_tool_call_id,
                  worker.lease_expires_at > NOW() AS lease_live
             FROM stage_deliverable_submissions AS submission
             JOIN stage_worker_runs AS worker
               ON worker.id=submission.worker_run_id
              AND worker.operation_id=submission.operation_id
              AND worker.stage_execution_id=submission.stage_execution_id
              AND worker.stage_run_unit_id=submission.stage_run_unit_id
              AND worker.organization_id=submission.organization_id
             JOIN stage_run_units AS unit
               ON unit.id=submission.stage_run_unit_id
              AND unit.operation_id=submission.operation_id
              AND unit.stage_execution_id=submission.stage_execution_id
              AND unit.organization_id=submission.organization_id
              AND unit.stage_kind=submission.stage_kind
            WHERE submission.id=$1 AND submission.operation_id=$2
              AND submission.stage_execution_id=$3
              AND submission.stage_run_unit_id=$4
              AND submission.organization_id=$5
              AND submission.stage_kind='application_understanding'
            FOR SHARE OF submission,worker,unit"#,
    )
    .bind(proposal.source_submission_id)
    .bind(proposal.operation_id)
    .bind(proposal.stage_execution_id)
    .bind(proposal.stage_run_unit_id)
    .bind(proposal.organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApplicationModelStoreError::IdentityMismatch {
        code: "proposal_source_submission_mismatch",
    })?;
    let expected_decisions = proposal_decisions_material(proposal);
    let expected_items = proposal_items_material(proposal);
    let legacy_submission_matches = submission.payload.get("structured_model")
        == Some(&proposal.structured_model)
        && submission.payload.get("decisions") == Some(&Value::Array(expected_decisions))
        && submission.payload.get("items") == Some(&Value::Array(expected_items));
    if submission.worker_status != "running"
        || !matches!(
            submission.unit_status.as_str(),
            "queued" | "running" | "gate_blocked"
        )
        || submission.submission_attempt_epoch != Some(submission.worker_attempt_epoch)
        || submission.worker_lease_token.is_none()
        || submission.submission_lease_token != submission.worker_lease_token
        || submission.active_tool_call_id.is_some()
        || !submission.lease_live
        || submission
            .payload
            .get("manifest_id")
            .and_then(Value::as_str)
            .and_then(|value| Uuid::parse_str(value).ok())
            != Some(proposal.manifest_id)
        || !(legacy_submission_matches
            || compact_submission_matches_proposal(&submission.payload, proposal))
    {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "proposal_source_payload_mismatch",
        });
    }
    let manifest_inputs = load_manifest_inputs(&mut tx, manifest.id).await?;
    let expected_input_keys = manifest_inputs
        .iter()
        .map(|input| input.input_key.as_str())
        .collect::<Vec<_>>();
    let actual_input_keys = proposal
        .decisions
        .iter()
        .map(|decision| decision.input_key.as_str())
        .collect::<Vec<_>>();
    if expected_input_keys != actual_input_keys {
        return Err(ApplicationModelStoreError::InvalidInput {
            code: "proposal_input_decision_closure_mismatch",
        });
    }
    let allowed_evidence = manifest_inputs
        .iter()
        .flat_map(|input| input.evidence_ids.iter().copied())
        .collect::<std::collections::BTreeSet<_>>();
    if proposal
        .items
        .iter()
        .flat_map(|item| &item.evidence)
        .any(|evidence| !allowed_evidence.contains(&evidence.evidence_id))
    {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "proposal_evidence_outside_manifest_authority",
        });
    }
    let revision_id = Uuid::new_v5(&manifest.id, b"application-model-revision:1");
    let material = proposal_material(proposal);
    let replay_material_hash = tagged_sha256_json(&material);
    let model_hash = tagged_sha256_json(&proposal.structured_model);
    let select_existing_sql = format!(
        "SELECT {REVISION_COLUMNS} FROM application_model_revisions \
         WHERE manifest_id=$1 AND revision_ordinal=1 FOR UPDATE"
    );
    if let Some(revision) = sqlx::query_as::<_, ApplicationModelRevisionRow>(&select_existing_sql)
        .bind(manifest.id)
        .fetch_optional(&mut *tx)
        .await?
    {
        let source_submission_changed =
            revision.source_submission_id != proposal.source_submission_id;
        if revision.id != revision_id
            || revision.status != "proposed"
            || revision.structured_model != proposal.structured_model
            || revision.model_hash != model_hash
            || (!source_submission_changed && revision.replay_material_hash != replay_material_hash)
            || !persisted_proposal_matches(&mut tx, &revision, proposal).await?
        {
            return Err(ApplicationModelStoreError::ReplayConflict {
                code: "proposed_revision_replay_drift",
            });
        }
        let revision = if source_submission_changed {
            let reauthorize_sql = format!(
                "UPDATE application_model_revisions \
                    SET source_submission_id=$4,replay_material_hash=$5 \
                  WHERE id=$1 AND manifest_id=$2 AND status='proposed' \
                    AND row_version=0 AND source_submission_id=$3 \
                  RETURNING {REVISION_COLUMNS}"
            );
            sqlx::query_as::<_, ApplicationModelRevisionRow>(&reauthorize_sql)
                .bind(revision.id)
                .bind(revision.manifest_id)
                .bind(revision.source_submission_id)
                .bind(proposal.source_submission_id)
                .bind(&replay_material_hash)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(ApplicationModelStoreError::ReplayConflict {
                    code: "proposed_revision_submission_reauthorization_failed",
                })?
        } else {
            revision
        };
        tx.commit().await?;
        return Ok(ProposedApplicationModelRevision {
            revision,
            replayed: true,
        });
    }
    let insert_revision_sql = format!(
        r#"INSERT INTO application_model_revisions(
               id,manifest_id,operation_id,scope_snapshot_id,stage_execution_id,
               stage_run_unit_id,organization_id,revision_ordinal,stage_kind,
               schema_version,status,structured_model,model_hash,replay_material_hash,
               source_submission_id
           ) VALUES($1,$2,$3,$4,$5,$6,$7,1,'application_understanding',
                    'application_model.v1','building',$8,$9,$10,$11)
           RETURNING {REVISION_COLUMNS}"#
    );
    let building_revision = sqlx::query_as::<_, ApplicationModelRevisionRow>(&insert_revision_sql)
        .bind(revision_id)
        .bind(manifest.id)
        .bind(proposal.operation_id)
        .bind(proposal.scope_snapshot_id)
        .bind(proposal.stage_execution_id)
        .bind(proposal.stage_run_unit_id)
        .bind(proposal.organization_id)
        .bind(&proposal.structured_model)
        .bind(&model_hash)
        .bind(&replay_material_hash)
        .bind(proposal.source_submission_id)
        .fetch_one(&mut *tx)
        .await?;
    for decision in &proposal.decisions {
        sqlx::query(
            r#"INSERT INTO application_model_input_decisions(
                   revision_id,manifest_id,input_key,disposition,item_keys,
                   duplicate_input_key,reason_code
               ) VALUES($1,$2,$3,$4,$5,$6,$7)"#,
        )
        .bind(building_revision.id)
        .bind(manifest.id)
        .bind(&decision.input_key)
        .bind(decision.disposition.as_str())
        .bind(&decision.item_keys)
        .bind(&decision.duplicate_input_key)
        .bind(&decision.reason_code)
        .execute(&mut *tx)
        .await?;
    }
    for (ordinal, item) in proposal.items.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO application_model_items(
                   revision_id,manifest_id,item_key,ordinal,item_kind,truth_state,
                   source_input_keys,referenced_item_keys,payload,payload_hash
               ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)"#,
        )
        .bind(building_revision.id)
        .bind(manifest.id)
        .bind(&item.item_key)
        .bind(
            i32::try_from(ordinal).map_err(|_| ApplicationModelStoreError::InvalidInput {
                code: "proposal_item_ordinal_overflow",
            })?,
        )
        .bind(&item.item_kind)
        .bind(item.truth_state.as_str())
        .bind(&item.source_input_keys)
        .bind(&item.referenced_item_keys)
        .bind(&item.payload)
        .bind(tagged_sha256_json(&item.payload))
        .execute(&mut *tx)
        .await?;
        for evidence in &item.evidence {
            sqlx::query(
                r#"INSERT INTO application_model_item_evidence(
                       revision_id,manifest_id,item_key,evidence_id,role
                   ) VALUES($1,$2,$3,$4,$5)"#,
            )
            .bind(building_revision.id)
            .bind(manifest.id)
            .bind(&item.item_key)
            .bind(evidence.evidence_id)
            .bind(evidence.role.as_str())
            .execute(&mut *tx)
            .await?;
        }
    }
    let seal_revision_sql = format!(
        "UPDATE application_model_revisions SET status='proposed' \
         WHERE id=$1 AND status='building' RETURNING {REVISION_COLUMNS}"
    );
    let revision = sqlx::query_as::<_, ApplicationModelRevisionRow>(&seal_revision_sql)
        .bind(building_revision.id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ProposedApplicationModelRevision {
        revision,
        replayed: false,
    })
}

pub async fn load_current_revision_with_transaction(
    tx: &mut Transaction<'_, Postgres>,
    manifest_id: Uuid,
) -> ApplicationModelStoreResult<Option<ApplicationModelCurrentRevisionRow>> {
    Ok(sqlx::query_as::<_, ApplicationModelCurrentRevisionRow>(
        r#"SELECT manifest_id,revision_id,authority_kind,stage_handoff_id,
                  deliverable_submission_id,manifest_hash,model_hash,
                  replay_material_hash,gate_decision_hash,published_at
             FROM application_model_current_revisions
            WHERE manifest_id=$1
            FOR UPDATE"#,
    )
    .bind(manifest_id)
    .fetch_optional(&mut **tx)
    .await?)
}

pub async fn resolve_gate_identity(
    pool: &PgPool,
    manifest_id: Uuid,
    fence: &super::runtime_memory_tx::RuntimeMemoryTxFence,
) -> ApplicationModelStoreResult<LoadApplicationModelGateMaterial> {
    let manifest = sqlx::query_as::<_, ApplicationModelManifestRow>(
        r#"SELECT id,operation_id,scope_snapshot_id,stage_execution_id,
                  stage_run_unit_id,organization_id,stage_kind,authority_kind,
                  input_count,manifest_hash,replay_material_hash,row_version,frozen_at
             FROM application_model_manifests
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4"#,
    )
    .bind(manifest_id)
    .bind(fence.operation_id)
    .bind(fence.stage_execution_id)
    .bind(fence.stage_run_unit_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ApplicationModelStoreError::IdentityMismatch {
        code: "finalize_manifest_fence_mismatch",
    })?;
    Ok(LoadApplicationModelGateMaterial {
        manifest_id,
        operation_id: manifest.operation_id,
        scope_snapshot_id: manifest.scope_snapshot_id,
        stage_execution_id: manifest.stage_execution_id,
        stage_run_unit_id: manifest.stage_run_unit_id,
        organization_id: manifest.organization_id,
    })
}

pub async fn transition_revision_to_final_with_transaction(
    tx: &mut Transaction<'_, Postgres>,
    manifest_id: Uuid,
    revision_id: Uuid,
) -> ApplicationModelStoreResult<ApplicationModelRevisionRow> {
    let sql = format!(
        r#"UPDATE application_model_revisions
              SET status='final',row_version=1,finalized_at=transaction_timestamp()
            WHERE id=$1 AND manifest_id=$2 AND status='proposed'
              AND row_version=0 AND finalized_at IS NULL
            RETURNING {REVISION_COLUMNS}"#
    );
    sqlx::query_as::<_, ApplicationModelRevisionRow>(&sql)
        .bind(revision_id)
        .bind(manifest_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ApplicationModelStoreError::ReplayConflict {
            code: "final_revision_compare_and_swap_failed",
        })
}

pub async fn insert_current_revision_with_transaction(
    tx: &mut Transaction<'_, Postgres>,
    input: &PublishApplicationModelCurrentRevision,
) -> ApplicationModelStoreResult<ApplicationModelCurrentRevisionRow> {
    Ok(sqlx::query_as::<_, ApplicationModelCurrentRevisionRow>(
        r#"INSERT INTO application_model_current_revisions(
               manifest_id,revision_id,authority_kind,stage_handoff_id,
               deliverable_submission_id,manifest_hash,model_hash,
               replay_material_hash,gate_decision_hash
           ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
           RETURNING manifest_id,revision_id,authority_kind,stage_handoff_id,
                     deliverable_submission_id,manifest_hash,model_hash,
                     replay_material_hash,gate_decision_hash,published_at"#,
    )
    .bind(input.manifest_id)
    .bind(input.revision_id)
    .bind(&input.authority_kind)
    .bind(input.stage_handoff_id)
    .bind(input.deliverable_submission_id)
    .bind(&input.manifest_hash)
    .bind(&input.model_hash)
    .bind(&input.replay_material_hash)
    .bind(&input.gate_decision_hash)
    .fetch_one(&mut **tx)
    .await?)
}

/// Lock every mutable producer that can change an Application Model Gate result.
/// The caller must keep this transaction open through the standard runtime
/// final seal and current-pointer publication.
pub async fn lock_finalize_authority_with_transaction(
    tx: &mut Transaction<'_, Postgres>,
    input: &LockApplicationModelFinalizeAuthority,
) -> ApplicationModelStoreResult<ApplicationModelFinalizeBarrier> {
    if input.gate.operation_id != input.fence.operation_id
        || input.gate.stage_execution_id != input.fence.stage_execution_id
        || input.gate.stage_run_unit_id != input.fence.stage_run_unit_id
    {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "finalize_fence_identity_mismatch",
        });
    }
    let operation_exists = sqlx::query_scalar::<_, Uuid>(
        "SELECT operation_id FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(input.fence.operation_id)
    .fetch_optional(&mut **tx)
    .await?
    .is_some();
    if !operation_exists {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "finalize_operation_missing",
        });
    }
    let unit = sqlx::query_as::<_, (String, i32)>(
        r#"SELECT status,generation
             FROM stage_run_units
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND scope_snapshot_id=$4 AND organization_id=$5
              AND stage_kind='application_understanding'
            FOR UPDATE"#,
    )
    .bind(input.gate.stage_run_unit_id)
    .bind(input.gate.operation_id)
    .bind(input.gate.stage_execution_id)
    .bind(input.gate.scope_snapshot_id)
    .bind(input.gate.organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ApplicationModelStoreError::IdentityMismatch {
        code: "finalize_application_understanding_unit_mismatch",
    })?;
    if !matches!(unit.0.as_str(), "running" | "passed") {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "finalize_application_understanding_unit_not_running",
        });
    }
    let team_plan_id = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id
             FROM stage_team_plans
            WHERE operation_id=$1 AND stage_execution_id=$2
              AND stage_run_unit_id=$3 AND scope_snapshot_id=$4
              AND organization_id=$5 AND stage_kind='application_understanding'
            FOR UPDATE"#,
    )
    .bind(input.gate.operation_id)
    .bind(input.gate.stage_execution_id)
    .bind(input.gate.stage_run_unit_id)
    .bind(input.gate.scope_snapshot_id)
    .bind(input.gate.organization_id)
    .fetch_optional(&mut **tx)
    .await?;
    let team_final_submitter_work_item_id = if let Some(team_plan_id) = team_plan_id {
        Some(
            sqlx::query_scalar::<_, Uuid>(
                r#"SELECT item.id
                     FROM stage_team_plans AS plan
                     JOIN stage_worker_runs AS final_worker
                       ON final_worker.id=plan.final_submitter_worker_run_id
                      AND final_worker.operation_id=plan.operation_id
                      AND final_worker.stage_execution_id=plan.stage_execution_id
                      AND final_worker.stage_run_unit_id=plan.stage_run_unit_id
                      AND final_worker.organization_id=plan.organization_id
                     JOIN stage_work_items AS item
                       ON item.id=final_worker.work_item_id
                      AND item.team_plan_id=plan.id
                      AND item.operation_id=plan.operation_id
                      AND item.stage_execution_id=plan.stage_execution_id
                      AND item.stage_run_unit_id=plan.stage_run_unit_id
                      AND item.scope_snapshot_id=plan.scope_snapshot_id
                      AND item.organization_id=plan.organization_id
                    WHERE plan.id=$1
                      AND plan.requests_closed_at IS NOT NULL
                      AND plan.final_submitter_kind='worker'
                      AND plan.final_submitter_worker_run_id=$2
                      AND plan.aggregator_kind='worker'
                      AND plan.aggregator_role=plan.leader_role
                      AND plan.dynamic_request_policy->>'coordination_mode'='company_controller'
                      AND item.stable_key='leader:primary'
                      AND item.role=plan.leader_role
                      AND item.required_for_barrier=FALSE
                      AND item.conflict_key='stage_unit_finalizer'
                      AND item.status IN ('running','completed')
                    FOR UPDATE OF item"#,
            )
            .bind(team_plan_id)
            .bind(input.fence.worker_run_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(ApplicationModelStoreError::IdentityMismatch {
                code: "stage_team_finalizer_required",
            })?,
        )
    } else {
        None
    };
    let workers = sqlx::query_as::<_, FinalizeWorkerAuthorityRow>(
        r#"SELECT id,status,attempt_epoch,checkpoint_version,
                  lease_token,COALESCE(lease_expires_at > NOW(),FALSE) AS lease_live,
                  active_tool_call_id
             FROM stage_worker_runs
            WHERE operation_id=$1 AND stage_execution_id=$2
              AND stage_run_unit_id=$3 AND organization_id=$4
            ORDER BY id
            FOR UPDATE"#,
    )
    .bind(input.gate.operation_id)
    .bind(input.gate.stage_execution_id)
    .bind(input.gate.stage_run_unit_id)
    .bind(input.gate.organization_id)
    .fetch_all(&mut **tx)
    .await?;
    let worker = workers
        .iter()
        .find(|worker| worker.id == input.fence.worker_run_id)
        .ok_or(ApplicationModelStoreError::IdentityMismatch {
            code: "finalize_worker_missing",
        })?;
    let replaying = unit.0 == "passed" && worker.status == "passed";
    if !replaying
        && (worker.status != "running"
            || worker.attempt_epoch != input.fence.attempt_epoch
            || worker.checkpoint_version != input.fence.expected_checkpoint_version
            || worker.lease_token != Some(input.fence.lease_token)
            || !worker.lease_live)
    {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "finalize_worker_fence_mismatch",
        });
    }
    let submission_exists = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id
             FROM stage_deliverable_submissions
            WHERE id=$1 AND operation_id=$2 AND stage_execution_id=$3
              AND stage_run_unit_id=$4 AND worker_run_id=$5
              AND organization_id=$6 AND stage_kind='application_understanding'
              AND attempt_epoch=$7 AND lease_token=$8
            FOR UPDATE"#,
    )
    .bind(input.deliverable_submission_id)
    .bind(input.gate.operation_id)
    .bind(input.gate.stage_execution_id)
    .bind(input.gate.stage_run_unit_id)
    .bind(input.fence.worker_run_id)
    .bind(input.gate.organization_id)
    .bind(input.fence.attempt_epoch)
    .bind(input.fence.lease_token)
    .fetch_optional(&mut **tx)
    .await?
    .is_some();
    if !submission_exists {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "finalize_submission_fence_mismatch",
        });
    }
    let manifest_exists = sqlx::query_scalar::<_, Uuid>(
        r#"SELECT id FROM application_model_manifests
            WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
              AND stage_execution_id=$4 AND stage_run_unit_id=$5
              AND organization_id=$6
            FOR UPDATE"#,
    )
    .bind(input.gate.manifest_id)
    .bind(input.gate.operation_id)
    .bind(input.gate.scope_snapshot_id)
    .bind(input.gate.stage_execution_id)
    .bind(input.gate.stage_run_unit_id)
    .bind(input.gate.organization_id)
    .fetch_optional(&mut **tx)
    .await?
    .is_some();
    if !manifest_exists {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "finalize_manifest_mismatch",
        });
    }
    let revision_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM application_model_revisions WHERE manifest_id=$1 ORDER BY id FOR UPDATE",
    )
    .bind(input.gate.manifest_id)
    .fetch_all(&mut **tx)
    .await?;
    for revision_id in revision_ids {
        sqlx::query_scalar::<_, String>(
            "SELECT input_key FROM application_model_input_decisions \
             WHERE revision_id=$1 ORDER BY input_key FOR SHARE",
        )
        .bind(revision_id)
        .fetch_all(&mut **tx)
        .await?;
        sqlx::query_scalar::<_, String>(
            "SELECT item_key FROM application_model_items \
             WHERE revision_id=$1 ORDER BY item_key FOR SHARE",
        )
        .bind(revision_id)
        .fetch_all(&mut **tx)
        .await?;
        sqlx::query_as::<_, (String, i64)>(
            "SELECT item_key,evidence_id FROM application_model_item_evidence \
             WHERE revision_id=$1 ORDER BY item_key,evidence_id FOR SHARE",
        )
        .bind(revision_id)
        .fetch_all(&mut **tx)
        .await?;
    }
    sqlx::query_scalar::<_, Uuid>(
        r#"SELECT handoff.id
             FROM application_model_manifest_inputs AS input
             JOIN stage_handoffs AS handoff ON handoff.id=input.source_handoff_id
             JOIN stage_run_units AS source_unit
               ON source_unit.id=handoff.source_stage_run_unit_id
              AND source_unit.operation_id=handoff.operation_id
              AND source_unit.stage_execution_id=handoff.stage_execution_id
              AND source_unit.organization_id=handoff.organization_id
              AND source_unit.stage_kind=handoff.from_stage_kind
            WHERE input.manifest_id=$1
            ORDER BY handoff.id
            FOR SHARE OF handoff,source_unit"#,
    )
    .bind(input.gate.manifest_id)
    .fetch_all(&mut **tx)
    .await?;
    let forbidden_activity_refs = sqlx::query_as::<_, (Uuid, String, String)>(
        r#"SELECT tool.id,tool.name,tool.status::TEXT
             FROM tool_calls AS tool
            WHERE tool.operation_id=$1 AND tool.stage_execution_id=$2
              AND tool.stage_run_unit_id=$3
              AND tool.name NOT IN (
                  'submit_stage_deliverable',
                  'update_plan'
              )
              AND NOT (
                  tool.name='submit_result'
                  AND EXISTS (
                      SELECT 1
                        FROM stage_worker_runs AS producer
                        JOIN stage_work_items AS item
                          ON item.id=producer.work_item_id
                         AND item.operation_id=producer.operation_id
                         AND item.stage_execution_id=producer.stage_execution_id
                         AND item.stage_run_unit_id=producer.stage_run_unit_id
                         AND item.organization_id=producer.organization_id
                        JOIN stage_team_plans AS plan
                          ON plan.id=item.team_plan_id
                         AND plan.operation_id=item.operation_id
                         AND plan.stage_execution_id=item.stage_execution_id
                         AND plan.stage_run_unit_id=item.stage_run_unit_id
                         AND plan.organization_id=item.organization_id
                       WHERE producer.id=tool.worker_run_id
                         AND producer.operation_id=tool.operation_id
                         AND producer.stage_execution_id=tool.stage_execution_id
                         AND producer.stage_run_unit_id=tool.stage_run_unit_id
                         AND producer.organization_id=tool.organization_id
                         AND plan.stage_kind='application_understanding'
                         AND plan.dynamic_request_policy->>'formulaic_worklist_executor'=
                             'application_model_v1'
                         AND (
                             (item.role='application_model_worker'
                              AND item.output_schema='application_model_work_item_output.v1')
                             OR
                             (item.role='application_model_synthesizer'
                              AND item.output_schema='application_model_proposal.v1')
                         )
                  )
              )
            ORDER BY tool.id
            FOR SHARE OF tool"#,
    )
    .bind(input.gate.operation_id)
    .bind(input.gate.stage_execution_id)
    .bind(input.gate.stage_run_unit_id)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|(id, name, status)| format!("tool:{id}:{name}:{status}"))
    .collect();
    let mut pending_producer_refs = workers
        .iter()
        .filter(|candidate| {
            candidate.id != input.fence.worker_run_id
                && !matches!(
                    candidate.status.as_str(),
                    "passed" | "failed" | "exhausted" | "superseded"
                )
        })
        .map(|candidate| format!("worker:{}:{}", candidate.id, candidate.status))
        .collect::<Vec<_>>();
    if let Some(active_tool_call_id) = worker.active_tool_call_id {
        pending_producer_refs.push(format!("active_tool:{active_tool_call_id}"));
    }
    pending_producer_refs.extend(
        sqlx::query_as::<_, (Uuid, String, String)>(
            r#"SELECT id,name,status::TEXT
                 FROM tool_calls
                WHERE operation_id=$1 AND stage_execution_id=$2
                  AND stage_run_unit_id=$3
                  AND status IN ('received','running')
                ORDER BY id
                FOR SHARE"#,
        )
        .bind(input.gate.operation_id)
        .bind(input.gate.stage_execution_id)
        .bind(input.gate.stage_run_unit_id)
        .fetch_all(&mut **tx)
        .await?
        .into_iter()
        .map(|(id, name, status)| format!("tool:{id}:{name}:{status}")),
    );
    let pending_work_items = sqlx::query_as::<_, (Uuid, String)>(
        r#"SELECT id,status
             FROM stage_work_items
            WHERE operation_id=$1 AND stage_execution_id=$2
              AND stage_run_unit_id=$3
              AND ($4::UUID IS NULL OR id<>$4)
              AND status NOT IN ('completed','exhausted','superseded')
            ORDER BY id
            FOR SHARE"#,
    )
    .bind(input.gate.operation_id)
    .bind(input.gate.stage_execution_id)
    .bind(input.gate.stage_run_unit_id)
    .bind(team_final_submitter_work_item_id)
    .fetch_all(&mut **tx)
    .await?;
    pending_producer_refs.extend(
        pending_work_items
            .into_iter()
            .map(|(id, status)| format!("work_item:{id}:{status}")),
    );
    pending_producer_refs.sort();
    pending_producer_refs.dedup();
    Ok(ApplicationModelFinalizeBarrier {
        forbidden_activity_refs,
        pending_producer_refs,
    })
}

/// Load one identity-exact snapshot inside the caller-owned transaction.
///
/// The authoritative finalizer calls this only after locking every mutable
/// producer and keeps the same transaction open through publication.
pub async fn load_gate_material_with_transaction(
    tx: &mut Transaction<'_, Postgres>,
    input: &LoadApplicationModelGateMaterial,
) -> ApplicationModelStoreResult<ApplicationModelGateMaterial> {
    let manifest = sqlx::query_as::<_, ApplicationModelManifestRow>(
        r#"SELECT id,operation_id,scope_snapshot_id,stage_execution_id,
                  stage_run_unit_id,organization_id,stage_kind,authority_kind,
                  input_count,manifest_hash,replay_material_hash,row_version,frozen_at
             FROM application_model_manifests WHERE id=$1"#,
    )
    .bind(input.manifest_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ApplicationModelStoreError::IdentityMismatch {
        code: "gate_manifest_missing",
    })?;
    if manifest.operation_id != input.operation_id
        || manifest.scope_snapshot_id != input.scope_snapshot_id
        || manifest.stage_execution_id != input.stage_execution_id
        || manifest.stage_run_unit_id != input.stage_run_unit_id
        || manifest.organization_id != input.organization_id
    {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "gate_manifest_owner_mismatch",
        });
    }
    let inputs = load_manifest_inputs(tx, manifest.id).await?;
    let unit_is_active = sqlx::query_scalar::<_, bool>(
        r#"SELECT EXISTS(
               SELECT 1 FROM stage_run_units
                WHERE id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
                  AND stage_execution_id=$4 AND organization_id=$5
                  AND stage_kind='application_understanding'
                  AND status IN ('queued','running','gate_blocked','passed')
           )"#,
    )
    .bind(input.stage_run_unit_id)
    .bind(input.operation_id)
    .bind(input.scope_snapshot_id)
    .bind(input.stage_execution_id)
    .bind(input.organization_id)
    .fetch_one(&mut **tx)
    .await?;
    if !unit_is_active {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "gate_application_understanding_unit_not_active",
        });
    }
    let source_stage_kinds = inputs
        .iter()
        .map(|manifest_input| manifest_input.source_kind.clone())
        .collect::<Vec<_>>();
    let resolved_sources =
        super::stage_handoffs::list_latest_final_sealed_for_sources_with_connection(
            tx,
            input.operation_id,
            input.organization_id,
            &source_stage_kinds,
        )
        .await
        .map_err(map_handoff_resolution_error)?;
    for manifest_input in &inputs {
        let source_is_exact = resolved_sources.iter().any(|source| {
            manifest_input.source_id == manifest_input.source_handoff_id.to_string()
                && source.id == manifest_input.source_handoff_id
                && source.operation_id == input.operation_id
                && source.scope_snapshot_id == input.scope_snapshot_id
                && source.organization_id == input.organization_id
                && source.from_stage_kind == manifest_input.source_kind
                && i64::from(source.schema_version) == manifest_input.source_version
                && source.payload == manifest_input.source_payload
                && format!("sha256:{}", source.payload_sha256) == manifest_input.source_payload_hash
                && manifest_input
                    .evidence_ids
                    .iter()
                    .all(|evidence_id| source.evidence_ids.binary_search(evidence_id).is_ok())
        });
        if !source_is_exact {
            return Err(ApplicationModelStoreError::IdentityMismatch {
                code: "gate_source_handoff_authority_mismatch",
            });
        }
    }
    let revision_sql = format!(
        "SELECT {REVISION_COLUMNS} FROM application_model_revisions \
         WHERE manifest_id=$1 ORDER BY revision_ordinal DESC LIMIT 1"
    );
    let revision = sqlx::query_as::<_, ApplicationModelRevisionRow>(&revision_sql)
        .bind(manifest.id)
        .fetch_optional(&mut **tx)
        .await?;
    let revision_id = revision.as_ref().map(|row| row.id);
    let decisions = if let Some(revision_id) = revision_id {
        sqlx::query_as::<_, ApplicationModelInputDecisionRow>(
            r#"SELECT revision_id,manifest_id,input_key,disposition,item_keys,
                      duplicate_input_key,reason_code
                 FROM application_model_input_decisions
                WHERE revision_id=$1 ORDER BY input_key"#,
        )
        .bind(revision_id)
        .fetch_all(&mut **tx)
        .await?
    } else {
        Vec::new()
    };
    let items = if let Some(revision_id) = revision_id {
        sqlx::query_as::<_, ApplicationModelItemRow>(
            r#"SELECT revision_id,manifest_id,item_key,ordinal,item_kind,truth_state,
                      source_input_keys,referenced_item_keys,payload,payload_hash
                 FROM application_model_items
                WHERE revision_id=$1 ORDER BY ordinal"#,
        )
        .bind(revision_id)
        .fetch_all(&mut **tx)
        .await?
    } else {
        Vec::new()
    };
    let item_evidence = if let Some(revision_id) = revision_id {
        sqlx::query_as::<_, ApplicationModelItemEvidenceRow>(
            r#"SELECT revision_id,manifest_id,item_key,evidence_id,role
                 FROM application_model_item_evidence
                WHERE revision_id=$1 ORDER BY item_key,evidence_id"#,
        )
        .bind(revision_id)
        .fetch_all(&mut **tx)
        .await?
    } else {
        Vec::new()
    };
    Ok(ApplicationModelGateMaterial {
        manifest,
        inputs,
        revision,
        decisions,
        items,
        item_evidence,
        forbidden_activity_refs: Vec::new(),
        pending_producer_refs: Vec::new(),
    })
}

/// Load an immutable content snapshot for diagnostics and preflight checks.
/// This does not inspect the authoritative producer/tool barrier and therefore
/// must never be treated as a publication PASS.
pub async fn load_gate_material(
    pool: &PgPool,
    input: &LoadApplicationModelGateMaterial,
) -> ApplicationModelStoreResult<ApplicationModelGateMaterial> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let material = load_gate_material_with_transaction(&mut tx, input).await?;
    tx.commit().await?;
    Ok(material)
}

/// Exact final Application Model authority consumed by the strict
/// generation-zero Candidate entry.  The caller supplies only owner identity
/// plus the already-authoritative vuln handoff; revision/Handoff selection is
/// always server-derived from the immutable current pointer.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateApplicationModelContextRow {
    pub manifest_id: Uuid,
    pub revision_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub deliverable_submission_id: Uuid,
    pub stage_handoff_id: Uuid,
    pub manifest_hash: String,
    pub model_hash: String,
    pub replay_material_hash: String,
    pub gate_decision_hash: String,
    pub structured_model: Value,
    pub items: Vec<ApplicationModelItemRow>,
    pub adoption: Option<CandidateApplicationModelAdoptionRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateApplicationModelAdoptionRow {
    pub application_model_stage_fork_input_id: Uuid,
    pub application_model_stage_fork_input_hash: String,
    pub source_vuln_stage_fork_input_id: Uuid,
    pub source_vuln_stage_fork_input_hash: String,
    pub stage_fork_manifest_hash: String,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateApplicationModelHeaderRow {
    manifest_id: Uuid,
    revision_id: Uuid,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    stage_execution_id: Uuid,
    stage_run_unit_id: Uuid,
    deliverable_submission_id: Uuid,
    stage_handoff_id: Uuid,
    manifest_hash: String,
    model_hash: String,
    replay_material_hash: String,
    gate_decision_hash: String,
    structured_model: Value,
}

#[derive(Debug, sqlx::FromRow)]
struct CandidateApplicationModelAdoptionQueryRow {
    application_model_stage_fork_input_id: Uuid,
    application_model_stage_fork_input_hash: String,
    source_vuln_stage_fork_input_id: Uuid,
    source_vuln_stage_fork_input_hash: String,
    stage_fork_manifest_hash: String,
    source_operation_id: Uuid,
    source_scope_snapshot_id: Uuid,
    application_model_handoff_id: Uuid,
}

async fn resolve_candidate_application_model_adoption(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    source_vuln_handoff_id: Uuid,
) -> ApplicationModelStoreResult<Option<CandidateApplicationModelAdoptionQueryRow>> {
    let rows = sqlx::query_as::<_, CandidateApplicationModelAdoptionQueryRow>(
        r#"SELECT model_input.id AS application_model_stage_fork_input_id,
                  model_input.manifest_input_sha256 AS application_model_stage_fork_input_hash,
                  vuln_input.id AS source_vuln_stage_fork_input_id,
                  vuln_input.manifest_input_sha256 AS source_vuln_stage_fork_input_hash,
                  fork.manifest_sha256 AS stage_fork_manifest_hash,
                  model_input.source_operation_id,
                  model_input.source_scope_snapshot_id,
                  model_input.source_handoff_id AS application_model_handoff_id
             FROM operation_stage_fork_inputs AS model_input
             JOIN operation_stage_forks AS fork
               ON fork.operation_id=model_input.operation_id
              AND fork.source_operation_id=model_input.fork_source_operation_id
              AND fork.target_scope_snapshot_id=model_input.target_scope_snapshot_id
              AND fork.entry_stage='attack_candidate'
              AND 'application_understanding'=ANY(fork.adopted_stage_kinds)
             JOIN operation_stage_fork_inputs AS vuln_input
               ON vuln_input.operation_id=model_input.operation_id
              AND vuln_input.target_scope_snapshot_id=model_input.target_scope_snapshot_id
              AND vuln_input.organization_id=model_input.organization_id
              AND vuln_input.source_stage_kind='vuln_triage'
              AND vuln_input.source_handoff_id=$4
             JOIN operation_state AS source_operation
               ON source_operation.operation_id=model_input.source_operation_id
              AND source_operation.superseded_by IS NULL
            WHERE model_input.operation_id=$1
              AND model_input.target_scope_snapshot_id=$2
              AND model_input.organization_id=$3
              AND model_input.source_stage_kind='application_understanding'
            FOR SHARE OF model_input,fork,vuln_input,source_operation"#,
    )
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .bind(source_vuln_handoff_id)
    .fetch_all(&mut **tx)
    .await?;
    match rows.as_slice() {
        [] => {
            let is_candidate_fork: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM operation_stage_forks \
                 WHERE operation_id=$1 AND entry_stage='attack_candidate')",
            )
            .bind(operation_id)
            .fetch_one(&mut **tx)
            .await?;
            if is_candidate_fork {
                return Err(ApplicationModelStoreError::IdentityMismatch {
                    code: "candidate_application_model_fork_authority_missing",
                });
            }
            Ok(None)
        }
        [row] => Ok(Some(CandidateApplicationModelAdoptionQueryRow {
            application_model_stage_fork_input_id: row.application_model_stage_fork_input_id,
            application_model_stage_fork_input_hash: row
                .application_model_stage_fork_input_hash
                .clone(),
            source_vuln_stage_fork_input_id: row.source_vuln_stage_fork_input_id,
            source_vuln_stage_fork_input_hash: row.source_vuln_stage_fork_input_hash.clone(),
            stage_fork_manifest_hash: row.stage_fork_manifest_hash.clone(),
            source_operation_id: row.source_operation_id,
            source_scope_snapshot_id: row.source_scope_snapshot_id,
            application_model_handoff_id: row.application_model_handoff_id,
        })),
        _ => Err(ApplicationModelStoreError::IdentityMismatch {
            code: "candidate_application_model_fork_authority_ambiguous",
        }),
    }
}

pub async fn load_current_candidate_context_with_transaction(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    organization_id: Uuid,
    source_vuln_handoff_id: Uuid,
) -> ApplicationModelStoreResult<CandidateApplicationModelContextRow> {
    let adoption = resolve_candidate_application_model_adoption(
        tx,
        operation_id,
        scope_snapshot_id,
        organization_id,
        source_vuln_handoff_id,
    )
    .await?;
    let application_model_operation_id = adoption
        .as_ref()
        .map_or(operation_id, |row| row.source_operation_id);
    let application_model_scope_snapshot_id = adoption
        .as_ref()
        .map_or(scope_snapshot_id, |row| row.source_scope_snapshot_id);
    let application_model_handoff_id = adoption
        .as_ref()
        .map(|row| row.application_model_handoff_id);
    let header = sqlx::query_as::<_, CandidateApplicationModelHeaderRow>(
        r#"SELECT manifest.id AS manifest_id,revision.id AS revision_id,
                  manifest.operation_id,manifest.scope_snapshot_id,
                  manifest.organization_id,manifest.stage_execution_id,
                  manifest.stage_run_unit_id,
                  current_revision.deliverable_submission_id,
                  current_revision.stage_handoff_id,
                  current_revision.manifest_hash,
                  current_revision.model_hash,
                  current_revision.replay_material_hash,
                  current_revision.gate_decision_hash,
                  revision.structured_model
             FROM application_model_current_revisions AS current_revision
             JOIN application_model_manifests AS manifest
               ON manifest.id=current_revision.manifest_id
              AND manifest.authority_kind='model'
             JOIN application_model_revisions AS revision
               ON revision.id=current_revision.revision_id
              AND revision.manifest_id=manifest.id
              AND revision.operation_id=manifest.operation_id
              AND revision.scope_snapshot_id=manifest.scope_snapshot_id
              AND revision.stage_execution_id=manifest.stage_execution_id
              AND revision.stage_run_unit_id=manifest.stage_run_unit_id
              AND revision.organization_id=manifest.organization_id
              AND revision.status='final' AND revision.row_version=1
              AND revision.finalized_at IS NOT NULL
              AND revision.model_hash=current_revision.model_hash
              AND revision.replay_material_hash=current_revision.replay_material_hash
             JOIN stage_handoffs AS handoff
               ON handoff.id=current_revision.stage_handoff_id
              AND handoff.operation_id=manifest.operation_id
              AND handoff.scope_snapshot_id=manifest.scope_snapshot_id
              AND handoff.organization_id=manifest.organization_id
              AND handoff.stage_execution_id=manifest.stage_execution_id
              AND handoff.source_stage_run_unit_id=manifest.stage_run_unit_id
              AND handoff.deliverable_submission_id=
                    current_revision.deliverable_submission_id
              AND handoff.from_stage_kind='application_understanding'
              AND ('sha256:' || handoff.unit_gate_decision_hash)=
                    current_revision.gate_decision_hash
              AND handoff.invalidated_at IS NULL
             JOIN stage_run_units AS source_unit
               ON source_unit.id=manifest.stage_run_unit_id
              AND source_unit.operation_id=manifest.operation_id
              AND source_unit.stage_execution_id=manifest.stage_execution_id
              AND source_unit.scope_snapshot_id=manifest.scope_snapshot_id
              AND source_unit.organization_id=manifest.organization_id
              AND source_unit.stage_kind='application_understanding'
              AND source_unit.status='passed' AND source_unit.terminal_at IS NOT NULL
             JOIN stage_deliverable_submissions AS submission
               ON submission.id=current_revision.deliverable_submission_id
              AND submission.operation_id=manifest.operation_id
              AND submission.stage_execution_id=manifest.stage_execution_id
              AND submission.stage_run_unit_id=manifest.stage_run_unit_id
              AND submission.organization_id=manifest.organization_id
              AND submission.stage_kind='application_understanding'
            WHERE manifest.operation_id=$1
              AND manifest.scope_snapshot_id=$2
              AND manifest.organization_id=$3
              AND ($5::UUID IS NULL OR current_revision.stage_handoff_id=$5)
              AND current_revision.manifest_hash=manifest.manifest_hash
              AND EXISTS (
                    SELECT 1 FROM application_model_manifest_inputs AS input
                     WHERE input.manifest_id=manifest.id
                       AND input.operation_id=manifest.operation_id
                       AND input.scope_snapshot_id=manifest.scope_snapshot_id
                       AND input.organization_id=manifest.organization_id
                       AND input.source_handoff_id=$4
              )
              AND pg_column_size(revision.structured_model) <= 262144
            FOR SHARE OF current_revision,manifest,revision,handoff,source_unit,submission"#,
    )
    .bind(application_model_operation_id)
    .bind(application_model_scope_snapshot_id)
    .bind(organization_id)
    .bind(source_vuln_handoff_id)
    .bind(application_model_handoff_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ApplicationModelStoreError::IdentityMismatch {
        code: "candidate_application_model_current_authority_missing",
    })?;
    let items = sqlx::query_as::<_, ApplicationModelItemRow>(
        r#"SELECT revision_id,manifest_id,item_key,ordinal,item_kind,truth_state,
                  source_input_keys,referenced_item_keys,payload,payload_hash
             FROM application_model_items
            WHERE revision_id=$1 AND manifest_id=$2
            ORDER BY ordinal
            FOR SHARE"#,
    )
    .bind(header.revision_id)
    .bind(header.manifest_id)
    .fetch_all(&mut **tx)
    .await?;
    let item_payload_bytes = serde_json::to_vec(
        &items.iter().map(|item| &item.payload).collect::<Vec<_>>(),
    )
    .map_err(|_| ApplicationModelStoreError::InvalidInput {
        code: "candidate_application_model_context_serialization_failed",
    })?;
    if items.is_empty() || items.len() > 4096 || item_payload_bytes.len() > 524_288 {
        return Err(ApplicationModelStoreError::IdentityMismatch {
            code: "candidate_application_model_context_out_of_bounds",
        });
    }
    Ok(CandidateApplicationModelContextRow {
        manifest_id: header.manifest_id,
        revision_id: header.revision_id,
        operation_id: header.operation_id,
        scope_snapshot_id: header.scope_snapshot_id,
        organization_id: header.organization_id,
        stage_execution_id: header.stage_execution_id,
        stage_run_unit_id: header.stage_run_unit_id,
        deliverable_submission_id: header.deliverable_submission_id,
        stage_handoff_id: header.stage_handoff_id,
        manifest_hash: header.manifest_hash,
        model_hash: header.model_hash,
        replay_material_hash: header.replay_material_hash,
        gate_decision_hash: header.gate_decision_hash,
        structured_model: header.structured_model,
        items,
        adoption: adoption.map(|row| CandidateApplicationModelAdoptionRow {
            application_model_stage_fork_input_id: row.application_model_stage_fork_input_id,
            application_model_stage_fork_input_hash: row.application_model_stage_fork_input_hash,
            source_vuln_stage_fork_input_id: row.source_vuln_stage_fork_input_id,
            source_vuln_stage_fork_input_hash: row.source_vuln_stage_fork_input_hash,
            stage_fork_manifest_hash: row.stage_fork_manifest_hash,
        }),
    })
}

#[cfg(test)]
mod closed_schema_tests {
    use super::*;

    fn incorporated_decision() -> ApplicationModelInputDecisionSeed {
        ApplicationModelInputDecisionSeed {
            input_key: "vuln-handoff".to_string(),
            disposition: ApplicationModelInputDispositionRow::Incorporated,
            item_keys: vec!["workflow:order_read".to_string()],
            duplicate_input_key: None,
            reason_code: None,
        }
    }

    fn observed_item() -> ApplicationModelItemSeed {
        ApplicationModelItemSeed {
            item_key: "workflow:order_read".to_string(),
            item_kind: "workflow".to_string(),
            truth_state: ApplicationModelTruthStateRow::Observed,
            source_input_keys: vec!["vuln-handoff".to_string()],
            referenced_item_keys: Vec::new(),
            payload: serde_json::json!({"path": "/orders/{id}"}),
            evidence: vec![ApplicationModelItemEvidenceSeed {
                evidence_id: 1,
                role: ApplicationModelEvidenceRoleRow::Observation,
            }],
        }
    }

    fn valid_model() -> Value {
        serde_json::json!({
            "organization_id": Uuid::from_u128(0x706),
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
        })
    }

    #[test]
    fn proposal_validator_requires_closed_application_model_v1() {
        assert!(validate_proposal_content_shape(
            &valid_model(),
            &[incorporated_decision()],
            &[observed_item()],
        )
        .is_ok());

        let unknown_only = ApplicationModelInputDecisionSeed {
            input_key: "vuln-handoff".to_string(),
            disposition: ApplicationModelInputDispositionRow::Unknown,
            item_keys: Vec::new(),
            duplicate_input_key: None,
            reason_code: Some("insufficient_evidence".to_string()),
        };
        let unknown_only_model = serde_json::json!({
            "organization_id": Uuid::from_u128(0x706),
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
        });
        assert!(validate_proposal_content_shape(
            &unknown_only_model,
            std::slice::from_ref(&unknown_only),
            &[],
        )
        .is_ok());
        assert_eq!(
            validate_proposal_content_shape(&serde_json::json!({}), &[unknown_only], &[])
                .expect_err("an empty object is not application_model.v1")
                .code(),
            "proposed_revision_shape_invalid",
        );

        let mut unknown_field = valid_model();
        unknown_field["commentary"] = serde_json::json!("model prose");
        assert_eq!(
            validate_proposal_content_shape(
                &unknown_field,
                &[incorporated_decision()],
                &[observed_item()],
            )
            .expect_err("unknown model fields must fail closed")
            .code(),
            "proposed_revision_shape_invalid",
        );
    }

    #[test]
    fn proposal_validator_rejects_foreign_structured_model_organization() {
        let proposal = ProposeApplicationModelRevision {
            manifest_id: Uuid::from_u128(0x710),
            operation_id: Uuid::from_u128(0x711),
            scope_snapshot_id: Uuid::from_u128(0x712),
            stage_execution_id: Uuid::from_u128(0x713),
            stage_run_unit_id: Uuid::from_u128(0x714),
            organization_id: Uuid::from_u128(0x715),
            source_submission_id: Uuid::from_u128(0x716),
            structured_model: valid_model(),
            decisions: vec![incorporated_decision()],
            items: vec![observed_item()],
        };
        assert!(!proposal_shape_is_valid(&proposal));
    }
}
