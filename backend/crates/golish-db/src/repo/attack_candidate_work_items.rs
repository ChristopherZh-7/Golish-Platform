//! Complete, per-WaveUnit Candidate reasoning manifest.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

use super::attack_candidate_seeds::{self, AttackCandidateSeedRow, NewAttackCandidateSeed};
use super::attack_waves;

pub const MAX_ATTACK_MANIFEST_ITEMS: usize = 100;
pub const MAX_ATTACK_WORK_ITEM_KEY_BYTES: usize = 256;
pub const MAX_ATTACK_TECHNIQUE_BYTES: usize = 128;
pub const MAX_ATTACK_OBSERVATION_BYTES: usize = 64 * 1024;
pub const MAX_ATTACK_OBSERVATION_EVIDENCE_IDS: usize = 64;
const FORMULAIC_TECHNIQUES: &[&str] = &[
    "WSTG-INPV-05",
    "WSTG-INPV-01",
    "WSTG-INPV-12",
    "WSTG-ATHZ-04",
    "WSTG-ATHN-02",
    "WSTG-SESS-02",
    "WSTG-CONF-05",
    "WSTG-CRYP-03",
    "WSTG-INFO",
    "GOLISH-NDAY",
];

#[derive(Debug, sqlx::FromRow)]
struct FormulaicHandoffAuthority {
    stage_execution_id: Uuid,
    source_stage_run_unit_id: Uuid,
    deliverable_submission_id: Uuid,
    scope_snapshot_id: Uuid,
    source_generation: i32,
    evidence_ids: Vec<i64>,
    coverage_watermark: serde_json::Value,
    gate_passed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct FormulaicOutcomeRow {
    asset: String,
    technique: String,
    outcome: String,
    source: Option<String>,
    query: Option<String>,
    result_count: Option<i32>,
    confidence: Option<f32>,
    evidence_ids: Vec<i64>,
    collected_at: DateTime<Utc>,
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    let hex = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn frozen_target_snapshot(asset: &str) -> (String, String, String) {
    let value = asset.trim().to_string();
    let target_type = if value.starts_with("http://") || value.starts_with("https://") {
        "url"
    } else if value.parse::<std::net::IpAddr>().is_ok() {
        "ip"
    } else if value.contains('/') {
        "cidr"
    } else if value.contains('.') {
        "domain"
    } else {
        "other"
    }
    .to_string();
    let identity_hash = sha256_prefixed(format!("{target_type}\u{0}{value}").as_bytes());
    (target_type, value, identity_hash)
}

fn watermark_usize(watermark: &serde_json::Value, key: &str) -> anyhow::Result<usize> {
    watermark
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("vuln_triage handoff watermark missing {key}"))
}

fn watermark_strings(watermark: &serde_json::Value, key: &str) -> anyhow::Result<BTreeSet<String>> {
    watermark
        .get(key)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("vuln_triage handoff watermark missing {key}"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| anyhow::anyhow!("vuln_triage handoff watermark has invalid {key}"))
        })
        .collect()
}

fn attest_formulaic_outcomes(
    organization_id: Uuid,
    authority: &FormulaicHandoffAuthority,
    outcomes: &[FormulaicOutcomeRow],
) -> anyhow::Result<()> {
    let watermark = &authority.coverage_watermark;
    anyhow::ensure!(
        watermark.get("kind").and_then(serde_json::Value::as_str)
            == Some("information_coverage_v1")
            && watermark.get("stage").and_then(serde_json::Value::as_str) == Some("vuln_triage")
            && watermark
                .get("organization_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == organization_id.to_string()),
        "vuln_triage handoff watermark identity mismatch"
    );
    for (flag, total, included) in [
        (
            "canonical_ref_truncated",
            "canonical_ref_total",
            "canonical_ref_included",
        ),
        (
            "evidence_id_truncated",
            "evidence_id_total",
            "evidence_id_included",
        ),
    ] {
        anyhow::ensure!(
            watermark.get(flag).and_then(serde_json::Value::as_bool) == Some(false)
                && watermark_usize(watermark, total)? == watermark_usize(watermark, included)?,
            "vuln_triage handoff is truncated and cannot seed an exact Candidate manifest"
        );
    }
    let terminal_cells = watermark_usize(watermark, "terminal_cells")?;
    anyhow::ensure!(
        terminal_cells == outcomes.len()
            && terminal_cells > 0
            && terminal_cells <= MAX_ATTACK_MANIFEST_ITEMS
            && watermark_usize(watermark, "canonical_ref_total")? == terminal_cells,
        "vuln_triage terminal-cell attestation mismatch"
    );
    let actual_assets = outcomes
        .iter()
        .map(|row| row.asset.clone())
        .collect::<BTreeSet<_>>();
    let actual_techniques = outcomes
        .iter()
        .map(|row| row.technique.clone())
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        watermark_strings(watermark, "assets")? == actual_assets
            && watermark_strings(watermark, "techniques")? == actual_techniques
            && actual_techniques
                == FORMULAIC_TECHNIQUES
                    .iter()
                    .map(|technique| (*technique).to_string())
                    .collect::<BTreeSet<_>>(),
        "vuln_triage asset/technique attestation mismatch"
    );
    let handoff_evidence = authority
        .evidence_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        authority.scope_snapshot_id != Uuid::nil()
            && authority.gate_passed_at <= Utc::now()
            && outcomes.iter().all(|row| {
                matches!(
                    row.outcome.as_str(),
                    "found" | "empty" | "blocked" | "not_applicable"
                ) && !row.evidence_ids.is_empty()
                    && row.evidence_ids.len() <= MAX_ATTACK_OBSERVATION_EVIDENCE_IDS
                    && row
                        .evidence_ids
                        .iter()
                        .all(|id| *id > 0 && handoff_evidence.contains(id))
            }),
        "vuln_triage canonical outcomes are not grounded by the exact handoff"
    );
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct AttackCandidateWorkItemRow {
    pub id: Uuid,
    pub seed_id: Uuid,
    pub wave_unit_id: Uuid,
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub organization_id: Uuid,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub work_item_key: String,
    pub decision_kind: Option<String>,
    pub candidate_id: Option<Uuid>,
    pub no_candidate_reason_code: Option<String>,
    pub no_candidate_detail: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub row_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct SeedAttackObservation {
    pub work_item_key: String,
    pub target_live_id: Option<Uuid>,
    pub target_type_at_time: String,
    pub target_value_at_time: String,
    pub target_identity_hash: String,
    pub technique: String,
    pub observation: serde_json::Value,
    pub observation_hash: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct SeedAttackWorkItems {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub observations: Vec<SeedAttackObservation>,
}

#[derive(Debug, Clone)]
pub struct SeededAttackWorkItem {
    pub seed: AttackCandidateSeedRow,
    pub work_item: AttackCandidateWorkItemRow,
}

#[derive(Debug, Clone)]
pub struct SeedAttackWorkItemsResult {
    pub items: Vec<SeededAttackWorkItem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateManifestItemRow {
    pub work_item: AttackCandidateWorkItemRow,
    pub technique: String,
    pub evidence_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateManifestRow {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub items: Vec<CandidateManifestItemRow>,
}

#[derive(Debug, sqlx::FromRow)]
struct FrozenEntryEvidenceAuthority {
    manifest_hash: Option<String>,
    manifest_count: Option<i32>,
    manifest_frozen_at: Option<DateTime<Utc>>,
    entry_consolidation_id: Option<Uuid>,
}

pub fn canonical_manifest_hash(manifest: &CandidateManifestRow) -> String {
    let projection = manifest
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "evidence_ids": item.evidence_ids,
                "target_identity_hash": item.work_item.target_identity_hash,
                "technique": item.technique,
                "work_item_id": item.work_item.id,
                "work_item_key": item.work_item.work_item_key,
            })
        })
        .collect::<Vec<_>>();
    format!(
        "sha256:{}",
        super::operation_scope_decisions::sha256_json(&serde_json::Value::Array(projection))
    )
}

const COLUMNS: &str = "id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,\
    organization_id,target_live_id,target_type_at_time,target_value_at_time,\
    target_identity_hash,work_item_key,decision_kind,candidate_id,no_candidate_reason_code,\
    no_candidate_detail,decided_at,row_version,created_at,updated_at";

fn invalid(message: &str) -> crate::DbError {
    crate::DbError::Other(anyhow::anyhow!(message.to_string()))
}

/// Seed the complete reasoning manifest for one exact frozen organization unit.
/// Natural-key conflicts are read back and compared, so a replay is idempotent
/// but cannot silently rewrite a frozen observation or target identity.
pub async fn seed_wave_work_items(
    tx: &mut Transaction<'_, Postgres>,
    command: SeedAttackWorkItems,
) -> crate::Result<SeedAttackWorkItemsResult> {
    if command.observations.is_empty() || command.observations.len() > MAX_ATTACK_MANIFEST_ITEMS {
        return Err(invalid("attack work-item manifest cannot be empty"));
    }
    let submitted_count = i32::try_from(command.observations.len())
        .map_err(|_| invalid("attack work-item manifest is too large"))?;
    let operation_contracts: Option<(String, String)> = sqlx::query_as(
        "SELECT runtime_memory_contract,attack_execution_contract
         FROM operation_state WHERE operation_id=$1 FOR UPDATE",
    )
    .bind(command.operation_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (_, attack_contract) = operation_contracts
        .ok_or_else(|| crate::DbError::NotFound("operation_state".to_string()))?;
    if attack_contract == "legacy" {
        return Err(invalid(
            "legacy operation cannot seed Candidate V2 work-items",
        ));
    }
    let wave = attack_waves::lock_wave(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
    )
    .await?;
    if submitted_count > wave.max_candidates_total {
        return Err(invalid(
            "attack work-item manifest exceeds its frozen Wave policy",
        ));
    }
    let wave_unit = attack_waves::lock_wave_unit(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
        command.wave_unit_id,
        command.organization_id,
    )
    .await?;
    if wave_unit.review_closed || wave_unit.verification_closed || wave_unit.terminal_at.is_some() {
        return Err(invalid(
            "closed WaveUnit cannot accept new reasoning work-items",
        ));
    }

    let mut items = Vec::with_capacity(command.observations.len());
    for observation in command.observations {
        if observation.work_item_key.trim().is_empty()
            || observation.work_item_key.len() > MAX_ATTACK_WORK_ITEM_KEY_BYTES
            || observation.target_type_at_time.trim().is_empty()
            || observation.target_value_at_time.trim().is_empty()
            || observation.target_identity_hash.trim().is_empty()
            || observation.technique.trim().is_empty()
            || observation.technique.len() > MAX_ATTACK_TECHNIQUE_BYTES
            || observation.observation_hash.trim().is_empty()
            || !observation.observation.is_object()
            || serde_json::to_vec(&observation.observation)?.len() > MAX_ATTACK_OBSERVATION_BYTES
            || observation.evidence_ids.is_empty()
            || observation.evidence_ids.len() > MAX_ATTACK_OBSERVATION_EVIDENCE_IDS
        {
            return Err(invalid("invalid or ungrounded attack observation"));
        }
        let seed = attack_candidate_seeds::insert_or_get_exact(
            tx,
            command.operation_id,
            command.scope_snapshot_id,
            command.wave_unit_id,
            command.organization_id,
            &NewAttackCandidateSeed {
                id: Uuid::new_v4(),
                target_live_id: observation.target_live_id,
                target_type_at_time: observation.target_type_at_time.clone(),
                target_value_at_time: observation.target_value_at_time.clone(),
                target_identity_hash: observation.target_identity_hash.clone(),
                technique: observation.technique,
                observation: observation.observation,
                observation_hash: observation.observation_hash,
            },
        )
        .await?;
        let work_item_id = Uuid::new_v4();
        let insert_sql = format!(
            "INSERT INTO attack_candidate_work_items(
                 id,seed_id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,
                 target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,
                 work_item_key)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT(wave_unit_id,work_item_key) DO NOTHING RETURNING {COLUMNS}"
        );
        let inserted = sqlx::query_as::<_, AttackCandidateWorkItemRow>(&insert_sql)
            .bind(work_item_id)
            .bind(seed.id)
            .bind(command.wave_unit_id)
            .bind(command.operation_id)
            .bind(command.scope_snapshot_id)
            .bind(command.organization_id)
            .bind(seed.target_live_id)
            .bind(&seed.target_type_at_time)
            .bind(&seed.target_value_at_time)
            .bind(&seed.target_identity_hash)
            .bind(&observation.work_item_key)
            .fetch_optional(&mut **tx)
            .await?;
        let work_item = if let Some(row) = inserted {
            row
        } else {
            let select_sql = format!(
                "SELECT {COLUMNS} FROM attack_candidate_work_items
                 WHERE wave_unit_id=$1 AND work_item_key=$2 FOR UPDATE"
            );
            sqlx::query_as::<_, AttackCandidateWorkItemRow>(&select_sql)
                .bind(command.wave_unit_id)
                .bind(&observation.work_item_key)
                .fetch_one(&mut **tx)
                .await?
        };
        if work_item.seed_id != seed.id
            || work_item.operation_id != command.operation_id
            || work_item.scope_snapshot_id != command.scope_snapshot_id
            || work_item.organization_id != command.organization_id
            || work_item.target_identity_hash != seed.target_identity_hash
        {
            return Err(invalid("attack work-item idempotency identity mismatch"));
        }
        let mut expected_evidence = observation.evidence_ids;
        expected_evidence.sort_unstable();
        let expected_len = expected_evidence.len();
        expected_evidence.dedup();
        if expected_evidence.len() != expected_len
            || expected_evidence
                .iter()
                .any(|evidence_id| *evidence_id <= 0)
        {
            return Err(invalid(
                "attack observation evidence must be unique and positive",
            ));
        }
        let existing_seed_evidence: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_seed_evidence
             WHERE seed_id=$1 AND role='observation' ORDER BY evidence_id",
        )
        .bind(seed.id)
        .fetch_all(&mut **tx)
        .await?;
        let existing_work_item_evidence: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_work_item_evidence
             WHERE work_item_id=$1 AND role='observation' ORDER BY evidence_id",
        )
        .bind(work_item.id)
        .fetch_all(&mut **tx)
        .await?;
        let replay = !existing_seed_evidence.is_empty() || !existing_work_item_evidence.is_empty();
        if replay
            && (existing_seed_evidence != expected_evidence
                || existing_work_item_evidence != expected_evidence)
        {
            return Err(invalid("attack observation evidence replay drift"));
        }
        for evidence_id in expected_evidence {
            sqlx::query(
                "INSERT INTO attack_candidate_seed_evidence(seed_id,evidence_id,role)
                 VALUES($1,$2,'observation') ON CONFLICT DO NOTHING",
            )
            .bind(seed.id)
            .bind(evidence_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO attack_candidate_work_item_evidence(work_item_id,evidence_id,role)
                 VALUES($1,$2,'observation') ON CONFLICT DO NOTHING",
            )
            .bind(work_item.id)
            .bind(evidence_id)
            .execute(&mut **tx)
            .await?;
        }
        items.push(SeededAttackWorkItem { seed, work_item });
    }
    let frozen_manifest = load_for_wave_unit_in_transaction(
        tx,
        command.operation_id,
        command.scope_snapshot_id,
        command.wave_run_id,
        command.wave_unit_id,
        command.organization_id,
    )
    .await?;
    if frozen_manifest.items.len() != submitted_count as usize {
        return Err(invalid(
            "attack manifest replay must provide the exact complete work-item set",
        ));
    }
    let manifest_hash = canonical_manifest_hash(&frozen_manifest);
    match (
        wave_unit.manifest_hash.as_deref(),
        wave_unit.manifest_count,
        wave_unit.manifest_frozen_at,
    ) {
        (Some(existing_hash), Some(existing_count), Some(_)) => {
            if existing_hash != manifest_hash || existing_count != submitted_count {
                return Err(invalid("attack manifest attestation replay drift"));
            }
        }
        (None, None, None) => {
            let frozen = sqlx::query(
                r#"UPDATE attack_wave_units
                      SET manifest_hash=$2,manifest_count=$3,manifest_frozen_at=NOW(),
                          row_version=row_version+1,updated_at=NOW()
                    WHERE id=$1 AND manifest_hash IS NULL
                      AND manifest_count IS NULL AND manifest_frozen_at IS NULL"#,
            )
            .bind(command.wave_unit_id)
            .bind(&manifest_hash)
            .bind(submitted_count)
            .execute(&mut **tx)
            .await?;
            if frozen.rows_affected() != 1 {
                return Err(invalid("attack manifest freeze CAS lost"));
            }
        }
        _ => return Err(invalid("attack manifest attestation is partially written")),
    }
    Ok(SeedAttackWorkItemsResult { items })
}

async fn load_for_wave_unit_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let sql = format!(
        "SELECT {COLUMNS} FROM attack_candidate_work_items
         WHERE wave_unit_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND organization_id=$4 ORDER BY work_item_key,id FOR UPDATE"
    );
    let work_items = sqlx::query_as::<_, AttackCandidateWorkItemRow>(&sql)
        .bind(wave_unit_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_all(&mut **tx)
        .await?;
    let mut items = Vec::with_capacity(work_items.len());
    for work_item in work_items {
        let technique: String = sqlx::query_scalar(
            "SELECT technique FROM attack_candidate_seeds WHERE id=$1 FOR SHARE",
        )
        .bind(work_item.seed_id)
        .fetch_one(&mut **tx)
        .await?;
        let evidence_ids = sqlx::query_scalar(
            r#"SELECT evidence_id FROM (
                   SELECT evidence_id FROM attack_candidate_seed_evidence WHERE seed_id=$1
                   UNION
                   SELECT evidence_id FROM attack_candidate_work_item_evidence
                    WHERE work_item_id=$2 AND role IN ('observation','support')
               ) evidence ORDER BY evidence_id"#,
        )
        .bind(work_item.seed_id)
        .bind(work_item.id)
        .fetch_all(&mut **tx)
        .await?;
        items.push(CandidateManifestItemRow {
            work_item,
            technique,
            evidence_ids,
        });
    }
    Ok(CandidateManifestRow {
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        items,
    })
}

/// Materialize and freeze the exact Candidate reasoning manifest for one
/// `attack_candidate` runtime Unit. The complete canonical `vuln_triage`
/// outcome set is re-read under the predecessor handoff/watermark locks; a
/// truncated or drifted handoff fails closed. Live targets are optional hints —
/// the frozen target type/value/hash remains authoritative after deletion.
pub async fn seed_from_final_vuln_triage_handoff(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let mut tx = pool.begin().await?;
    let unit: (Uuid, i32) = sqlx::query_as(
        r#"SELECT scope_snapshot_id,generation FROM stage_run_units
            WHERE id=$1 AND operation_id=$2 AND organization_id=$3
              AND stage_kind='attack_candidate'
            FOR SHARE"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(organization_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| invalid("attack_candidate StageRunUnit identity mismatch"))?;
    let authority = sqlx::query_as::<_, FormulaicHandoffAuthority>(
        r#"SELECT handoff.stage_execution_id,handoff.source_stage_run_unit_id,
                  handoff.deliverable_submission_id,handoff.scope_snapshot_id,
                  source_unit.generation AS source_generation,handoff.evidence_ids,
                  handoff.coverage_watermark,handoff.gate_passed_at
             FROM stage_handoffs AS handoff
             JOIN stage_run_units AS source_unit
               ON source_unit.id=handoff.source_stage_run_unit_id
              AND source_unit.operation_id=handoff.operation_id
              AND source_unit.stage_execution_id=handoff.stage_execution_id
              AND source_unit.scope_snapshot_id=handoff.scope_snapshot_id
              AND source_unit.organization_id=handoff.organization_id
              AND source_unit.stage_kind=handoff.from_stage_kind
            WHERE handoff.operation_id=$1 AND handoff.organization_id=$2
              AND handoff.scope_snapshot_id=$3
              AND handoff.from_stage_kind='vuln_triage'
              AND handoff.invalidated_at IS NULL
              AND source_unit.status='passed' AND source_unit.terminal_at IS NOT NULL
            ORDER BY handoff.gate_passed_at DESC,handoff.id DESC
            LIMIT 1 FOR SHARE OF handoff,source_unit"#,
    )
    .bind(operation_id)
    .bind(organization_id)
    .bind(unit.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| invalid("exact vuln_triage final handoff is unavailable"))?;
    if authority.scope_snapshot_id != unit.0
        || authority.source_generation < 0
        || unit.1 != 0
        || authority.evidence_ids.is_empty()
    {
        return Err(invalid(
            "initial Candidate Wave generation or vuln_triage handoff authority mismatch",
        ));
    }
    let ordinal: i32 = sqlx::query_scalar(
        "SELECT ordinal FROM operation_org_scope_units
         WHERE snapshot_id=$1 AND organization_id=$2",
    )
    .bind(unit.0)
    .bind(organization_id)
    .fetch_one(&mut *tx)
    .await?;
    let outcomes = sqlx::query_as::<_, FormulaicOutcomeRow>(
        r#"SELECT asset,technique,outcome,source,query,result_count,confidence,
                  evidence_ids,collected_at
             FROM technique_outcomes
            WHERE organization_id=$1 AND run_id=$2
              AND technique=ANY($3)
              AND outcome IN ('found','empty','blocked','not_applicable')
              AND collected_at IS NOT NULL AND collected_at<=$4
              AND updated_at<=$4
            ORDER BY asset,technique
            FOR SHARE"#,
    )
    .bind(organization_id)
    .bind(operation_id.to_string())
    .bind(FORMULAIC_TECHNIQUES)
    .bind(authority.gate_passed_at)
    .fetch_all(&mut *tx)
    .await?;
    attest_formulaic_outcomes(organization_id, &authority, &outcomes)
        .map_err(crate::DbError::Other)?;

    let mut observations = Vec::with_capacity(outcomes.len());
    for row in outcomes {
        let (target_type_at_time, target_value_at_time, target_identity_hash) =
            frozen_target_snapshot(&row.asset);
        let live_target: Option<(Uuid, String, String)> = sqlx::query_as(
            r#"SELECT id,target_type,value FROM targets
                WHERE organization_id=$1 AND project_path=(
                    SELECT project_path_at_freeze FROM operation_org_scope_snapshots
                     WHERE id=$2 AND operation_id=$3
                ) AND scope='in' AND value=$4
                ORDER BY id LIMIT 1 FOR SHARE"#,
        )
        .bind(organization_id)
        .bind(unit.0)
        .bind(operation_id)
        .bind(&target_value_at_time)
        .fetch_optional(&mut *tx)
        .await?;
        let target_live_id = live_target.and_then(|(id, target_type, value)| {
            let (live_type, _, live_hash) = frozen_target_snapshot(&value);
            (live_hash == target_identity_hash
                && live_type == target_type_at_time
                && !target_type.trim().is_empty())
            .then_some(id)
        });
        let mut evidence_ids = row.evidence_ids;
        evidence_ids.sort_unstable();
        let original_evidence_count = evidence_ids.len();
        evidence_ids.dedup();
        if evidence_ids.len() != original_evidence_count {
            return Err(invalid("formulaic outcome contains duplicate evidence ids"));
        }
        let observation = serde_json::json!({
            "asset": target_value_at_time,
            "collected_at": row.collected_at,
            "confidence": row.confidence,
            "outcome": row.outcome,
            "query": row.query,
            "result_count": row.result_count,
            "source": row.source,
        });
        let observation_hash = sha256_prefixed(serde_json::to_vec(&observation)?.as_slice());
        observations.push(SeedAttackObservation {
            work_item_key: format!("{}:{}:{}", target_identity_hash, row.technique, row.outcome),
            target_live_id,
            target_type_at_time,
            target_value_at_time,
            target_identity_hash,
            technique: row.technique,
            observation,
            observation_hash,
            evidence_ids,
        });
    }
    let wave_run_id = attack_waves::deterministic_initial_wave_run_id(operation_id, unit.1);
    let wave_unit_id =
        attack_waves::deterministic_initial_wave_unit_id(wave_run_id, organization_id);
    let (policy_snapshot, policy_hash) = attack_waves::deterministic_initial_policy()?;
    attack_waves::open_from_vuln_triage_handoff(
        &mut tx,
        &attack_waves::OpenAttackWaveUnit {
            wave_run_id,
            wave_unit_id,
            operation_id,
            scope_snapshot_id: unit.0,
            organization_id,
            entry_stage_execution_id: authority.stage_execution_id,
            entry_stage_run_unit_id: authority.source_stage_run_unit_id,
            entry_deliverable_submission_id: authority.deliverable_submission_id,
            generation: unit.1,
            ordinal,
            policy_snapshot,
            policy_hash,
            max_waves: 3,
            max_candidates_total: 100,
            max_chain_depth: 3,
            max_attempts_total: 200,
        },
    )
    .await?;
    seed_wave_work_items(
        &mut tx,
        SeedAttackWorkItems {
            operation_id,
            scope_snapshot_id: unit.0,
            wave_run_id,
            wave_unit_id,
            organization_id,
            observations,
        },
    )
    .await?;
    tx.commit().await?;
    load_for_wave_unit(
        pool,
        operation_id,
        unit.0,
        wave_run_id,
        wave_unit_id,
        organization_id,
    )
    .await
}

/// Load the exact manifest consumed by one current attack_candidate runtime
/// Unit. Unit generation is the server-owned bridge to the immutable WaveRun;
/// zero/multiple work is never collapsed into an "unavailable means empty"
/// result.
pub async fn load_for_runtime_unit(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    stage_run_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let identity: (Uuid, Uuid, Uuid) = sqlx::query_as(
        r#"SELECT run.scope_snapshot_id,run.id,wave_unit.id
             FROM stage_run_units AS decision_unit
             JOIN attack_wave_runs AS run
               ON run.operation_id=decision_unit.operation_id
              AND run.scope_snapshot_id=decision_unit.scope_snapshot_id
              AND run.generation=decision_unit.generation
             JOIN attack_wave_units AS wave_unit
               ON wave_unit.wave_run_id=run.id
              AND wave_unit.operation_id=run.operation_id
              AND wave_unit.scope_snapshot_id=run.scope_snapshot_id
              AND wave_unit.organization_id=decision_unit.organization_id
            WHERE decision_unit.id=$1
              AND decision_unit.operation_id=$2
              AND decision_unit.organization_id=$3
              AND decision_unit.stage_kind='attack_candidate'"#,
    )
    .bind(stage_run_unit_id)
    .bind(operation_id)
    .bind(organization_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| crate::DbError::NotFound("attack_candidate_manifest".to_string()))?;
    load_for_wave_unit(
        pool,
        operation_id,
        identity.0,
        identity.1,
        identity.2,
        organization_id,
    )
    .await
}

async fn load_manifest_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let sql = format!(
        "SELECT {COLUMNS} FROM attack_candidate_work_items
         WHERE wave_unit_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND organization_id=$4 ORDER BY work_item_key,id FOR SHARE"
    );
    let work_items = sqlx::query_as::<_, AttackCandidateWorkItemRow>(&sql)
        .bind(wave_unit_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_all(&mut *connection)
        .await?;
    let mut items = Vec::with_capacity(work_items.len());
    for work_item in work_items {
        let technique: String = sqlx::query_scalar(
            "SELECT technique FROM attack_candidate_seeds WHERE id=$1 FOR SHARE",
        )
        .bind(work_item.seed_id)
        .fetch_one(&mut *connection)
        .await?;
        let seed_evidence: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_seed_evidence
             WHERE seed_id=$1 AND role IN ('observation','support')
             ORDER BY evidence_id FOR SHARE",
        )
        .bind(work_item.seed_id)
        .fetch_all(&mut *connection)
        .await?;
        let work_item_evidence: Vec<i64> = sqlx::query_scalar(
            "SELECT evidence_id FROM attack_candidate_work_item_evidence
             WHERE work_item_id=$1 AND role IN ('observation','support')
             ORDER BY evidence_id FOR SHARE",
        )
        .bind(work_item.id)
        .fetch_all(&mut *connection)
        .await?;
        let evidence_ids = seed_evidence
            .into_iter()
            .chain(work_item_evidence)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        items.push(CandidateManifestItemRow {
            work_item,
            technique,
            evidence_ids,
        });
    }
    Ok(CandidateManifestRow {
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        items,
    })
}

/// Resolve the historical evidence that an `attack_candidate` final seal may
/// carry across its own Unit freshness boundary. Authority is the exact frozen
/// `vuln_triage` entry handoff plus the immutable manifest attestation; merely
/// belonging to the same operation or organization is insufficient.
///
/// This is connection-based so callers can keep the proof under the same
/// final-seal transaction and locks as Candidate acceptance.
pub async fn load_frozen_entry_evidence_ids_with_connection(
    connection: &mut PgConnection,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<Vec<i64>> {
    let authority = sqlx::query_as::<_, FrozenEntryEvidenceAuthority>(
        r#"SELECT wave_unit.manifest_hash,wave_unit.manifest_count,
                  wave_unit.manifest_frozen_at,wave_unit.entry_consolidation_id
             FROM attack_wave_units AS wave_unit
             JOIN attack_wave_runs AS wave
               ON wave.id=wave_unit.wave_run_id
              AND wave.operation_id=wave_unit.operation_id
              AND wave.scope_snapshot_id=wave_unit.scope_snapshot_id
            WHERE wave_unit.id=$1
              AND wave_unit.wave_run_id=$2
              AND wave_unit.operation_id=$3
              AND wave_unit.scope_snapshot_id=$4
              AND wave_unit.organization_id=$5
            FOR SHARE OF wave_unit,wave"#,
    )
    .bind(wave_unit_id)
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| invalid("attack candidate entry authority mismatch"))?;
    let entry_evidence_ids: Vec<i64> =
        if let Some(consolidation_id) = authority.entry_consolidation_id {
            sqlx::query_scalar(
                r#"SELECT DISTINCT evidence.evidence_id
                 FROM attack_wave_consolidations AS consolidation
                 JOIN attack_wave_consolidation_members AS member
                   ON member.consolidation_id=consolidation.id
                  AND member.operation_id=consolidation.operation_id
                  AND member.scope_snapshot_id=consolidation.scope_snapshot_id
                  AND member.source_wave_run_id=consolidation.source_wave_run_id
                 JOIN attack_fact_delta_decisions AS decision
                   ON decision.fact_delta_id=member.fact_delta_id
                  AND decision.disposition='accepted'
                 JOIN attack_fact_delta_evidence AS evidence
                   ON evidence.fact_delta_id=member.fact_delta_id
                  AND evidence.role='fact_delta'
                WHERE consolidation.id=$1
                  AND consolidation.decision_kind='opened_next_wave'
                  AND consolidation.target_wave_run_id=$2
                  AND consolidation.operation_id=$3
                  AND consolidation.scope_snapshot_id=$4
                  AND member.target_wave_unit_id=$5
                  AND member.organization_id=$6
                  AND member.target_work_item_id IS NOT NULL
                ORDER BY evidence.evidence_id"#,
            )
            .bind(consolidation_id)
            .bind(wave_run_id)
            .bind(operation_id)
            .bind(scope_snapshot_id)
            .bind(wave_unit_id)
            .bind(organization_id)
            .fetch_all(&mut *connection)
            .await?
        } else {
            sqlx::query_scalar(
                r#"SELECT handoff.evidence_ids
                 FROM attack_wave_units AS wave_unit
                 JOIN stage_run_units AS entry_unit
                   ON entry_unit.id=wave_unit.entry_stage_run_unit_id
                  AND entry_unit.operation_id=wave_unit.operation_id
                  AND entry_unit.stage_execution_id=wave_unit.entry_stage_execution_id
                  AND entry_unit.scope_snapshot_id=wave_unit.scope_snapshot_id
                  AND entry_unit.organization_id=wave_unit.organization_id
                  AND entry_unit.stage_kind=wave_unit.entry_stage_kind
                 JOIN stage_handoffs AS handoff
                   ON handoff.operation_id=entry_unit.operation_id
                  AND handoff.scope_snapshot_id=entry_unit.scope_snapshot_id
                  AND handoff.organization_id=entry_unit.organization_id
                  AND handoff.stage_execution_id=entry_unit.stage_execution_id
                  AND handoff.source_stage_run_unit_id=entry_unit.id
                  AND handoff.deliverable_submission_id=wave_unit.entry_deliverable_submission_id
                  AND handoff.from_stage_kind=entry_unit.stage_kind
                WHERE wave_unit.id=$1
                  AND wave_unit.wave_run_id=$2
                  AND wave_unit.operation_id=$3
                  AND wave_unit.scope_snapshot_id=$4
                  AND wave_unit.organization_id=$5
                  AND wave_unit.entry_stage_kind='vuln_triage'
                  AND entry_unit.status='passed'
                  AND entry_unit.terminal_at IS NOT NULL
                  AND handoff.invalidated_at IS NULL
                FOR SHARE OF wave_unit,entry_unit,handoff"#,
            )
            .bind(wave_unit_id)
            .bind(wave_run_id)
            .bind(operation_id)
            .bind(scope_snapshot_id)
            .bind(organization_id)
            .fetch_optional(&mut *connection)
            .await?
            .ok_or_else(|| invalid("attack candidate entry handoff authority mismatch"))?
        };
    if entry_evidence_ids.is_empty() {
        return Err(invalid("attack candidate entry evidence is empty"));
    }
    let (manifest_hash, manifest_count, _manifest_frozen_at) = match (
        authority.manifest_hash,
        authority.manifest_count,
        authority.manifest_frozen_at,
    ) {
        (Some(hash), Some(count), Some(frozen_at)) if !hash.trim().is_empty() && count > 0 => {
            (hash, count, frozen_at)
        }
        _ => return Err(invalid("attack candidate entry manifest is not frozen")),
    };
    let manifest = load_manifest_with_connection(
        connection,
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
    )
    .await?;
    if manifest.items.len() != manifest_count as usize
        || canonical_manifest_hash(&manifest) != manifest_hash
        || manifest
            .items
            .iter()
            .any(|item| item.evidence_ids.is_empty())
    {
        return Err(invalid(
            "attack candidate frozen manifest attestation mismatch",
        ));
    }
    let evidence_ids = manifest
        .items
        .iter()
        .flat_map(|item| item.evidence_ids.iter().copied())
        .collect::<BTreeSet<_>>();
    let entry_evidence = entry_evidence_ids.into_iter().collect::<BTreeSet<_>>();
    if evidence_ids.iter().any(|id| !entry_evidence.contains(id)) {
        return Err(invalid(
            "attack candidate manifest evidence is not linked by its exact sealed entry",
        ));
    }
    Ok(evidence_ids.into_iter().collect())
}

pub async fn load_for_wave_unit(
    pool: &sqlx::PgPool,
    operation_id: Uuid,
    scope_snapshot_id: Uuid,
    wave_run_id: Uuid,
    wave_unit_id: Uuid,
    organization_id: Uuid,
) -> crate::Result<CandidateManifestRow> {
    let attestation: (String, i32, chrono::DateTime<Utc>) = sqlx::query_as(
        r#"SELECT manifest_hash,manifest_count,manifest_frozen_at
             FROM attack_wave_units
            WHERE id=$1 AND wave_run_id=$2 AND operation_id=$3
              AND scope_snapshot_id=$4 AND organization_id=$5"#,
    )
    .bind(wave_unit_id)
    .bind(wave_run_id)
    .bind(operation_id)
    .bind(scope_snapshot_id)
    .bind(organization_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| crate::DbError::Other(anyhow::anyhow!("attack manifest is not frozen")))?;
    let sql = format!(
        "SELECT {COLUMNS} FROM attack_candidate_work_items
         WHERE wave_unit_id=$1 AND operation_id=$2 AND scope_snapshot_id=$3
           AND organization_id=$4 ORDER BY work_item_key,id"
    );
    let work_items = sqlx::query_as::<_, AttackCandidateWorkItemRow>(&sql)
        .bind(wave_unit_id)
        .bind(operation_id)
        .bind(scope_snapshot_id)
        .bind(organization_id)
        .fetch_all(pool)
        .await?;
    let mut items = Vec::with_capacity(work_items.len());
    for work_item in work_items {
        let technique: String =
            sqlx::query_scalar("SELECT technique FROM attack_candidate_seeds WHERE id=$1")
                .bind(work_item.seed_id)
                .fetch_one(pool)
                .await?;
        let evidence_ids = sqlx::query_scalar(
            r#"SELECT evidence_id FROM (
                   SELECT evidence_id FROM attack_candidate_seed_evidence WHERE seed_id=$1
                   UNION
                   SELECT evidence_id FROM attack_candidate_work_item_evidence
                    WHERE work_item_id=$2 AND role IN ('observation','support')
               ) evidence ORDER BY evidence_id"#,
        )
        .bind(work_item.seed_id)
        .bind(work_item.id)
        .fetch_all(pool)
        .await?;
        items.push(CandidateManifestItemRow {
            work_item,
            technique,
            evidence_ids,
        });
    }
    let manifest = CandidateManifestRow {
        operation_id,
        scope_snapshot_id,
        wave_run_id,
        wave_unit_id,
        organization_id,
        items,
    };
    let actual_hash = canonical_manifest_hash(&manifest);
    if attestation.0 != actual_hash || attestation.1 != manifest.items.len() as i32 {
        return Err(invalid("attack manifest attestation mismatch"));
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcomes(count: usize) -> Vec<FormulaicOutcomeRow> {
        (0..count)
            .map(|index| FormulaicOutcomeRow {
                asset: format!(
                    "https://app-{}.example.test",
                    index / FORMULAIC_TECHNIQUES.len()
                ),
                technique: FORMULAIC_TECHNIQUES[index % FORMULAIC_TECHNIQUES.len()].to_string(),
                outcome: if index % 2 == 0 { "found" } else { "empty" }.to_string(),
                source: Some("formulaic_sweep".to_string()),
                query: None,
                result_count: Some(i32::from(index % 2 == 0)),
                confidence: Some(1.0),
                evidence_ids: vec![index as i64 + 1],
                collected_at: Utc::now() - chrono::Duration::seconds(1),
            })
            .collect()
    }

    fn authority(organization_id: Uuid, rows: &[FormulaicOutcomeRow]) -> FormulaicHandoffAuthority {
        let assets = rows
            .iter()
            .map(|row| row.asset.clone())
            .collect::<BTreeSet<_>>();
        let techniques = rows
            .iter()
            .map(|row| row.technique.clone())
            .collect::<BTreeSet<_>>();
        FormulaicHandoffAuthority {
            stage_execution_id: Uuid::new_v4(),
            source_stage_run_unit_id: Uuid::new_v4(),
            deliverable_submission_id: Uuid::new_v4(),
            scope_snapshot_id: Uuid::new_v4(),
            source_generation: 0,
            evidence_ids: rows
                .iter()
                .flat_map(|row| row.evidence_ids.iter().copied())
                .collect(),
            coverage_watermark: serde_json::json!({
                "kind": "information_coverage_v1",
                "stage": "vuln_triage",
                "organization_id": organization_id,
                "terminal_cells": rows.len(),
                "canonical_ref_total": rows.len(),
                "canonical_ref_included": rows.len(),
                "canonical_ref_truncated": false,
                "evidence_id_total": rows.len(),
                "evidence_id_included": rows.len(),
                "evidence_id_truncated": false,
                "assets": assets,
                "techniques": techniques,
            }),
            gate_passed_at: Utc::now(),
        }
    }

    #[test]
    fn exact_formulaic_watermark_passes_but_truncation_fails_closed() {
        let organization_id = Uuid::new_v4();
        let rows = outcomes(FORMULAIC_TECHNIQUES.len());
        let mut exact = authority(organization_id, &rows);
        attest_formulaic_outcomes(organization_id, &exact, &rows)
            .expect("exact canonical formulaic cells");
        exact.coverage_watermark["canonical_ref_truncated"] = serde_json::json!(true);
        exact.coverage_watermark["canonical_ref_included"] = serde_json::json!(rows.len() - 1);
        assert!(attest_formulaic_outcomes(organization_id, &exact, &rows).is_err());
    }

    #[test]
    fn formulaic_manifest_over_policy_limit_fails_before_seeding() {
        let organization_id = Uuid::new_v4();
        let rows = outcomes(MAX_ATTACK_MANIFEST_ITEMS + 1);
        let exact = authority(organization_id, &rows);
        assert!(attest_formulaic_outcomes(organization_id, &exact, &rows).is_err());
    }

    #[test]
    fn frozen_target_snapshot_does_not_require_a_live_target_row() {
        let (target_type, value, hash) = frozen_target_snapshot("https://example.test/login");
        assert_eq!(target_type, "url");
        assert_eq!(value, "https://example.test/login");
        assert!(hash.starts_with("sha256:"));
    }
}
